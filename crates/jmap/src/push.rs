//! JMAP WebSocket push notifications (RFC 8887).
//!
//! Provides near-instant sync by subscribing to server-side state changes
//! over a persistent WebSocket connection. Falls back to polling when the
//! server does not advertise WebSocket capability or after repeated failures.
//!
//! The push manager is standalone — it does not drive the sync loop directly.
//! Instead it forwards `StateChange` events through a channel that the sync
//! layer can consume to trigger immediate delta syncs.

use std::collections::HashMap;
use std::sync::Arc;

use futures::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc, watch};

use db::db::DbState;

use super::client::JmapClient;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Current state of the push WebSocket connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PushState {
    Disconnected,
    Connecting,
    Connected,
    Error(String),
}

/// A JMAP state change notification received over WebSocket.
///
/// `changed` maps JMAP account IDs to a map of data type names (e.g. "Email",
/// "Mailbox") to their new state strings.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StateChange {
    #[serde(rename = "@type")]
    pub type_name: String,
    pub changed: HashMap<String, HashMap<String, String>>,
}

/// Manages a WebSocket push connection for one JMAP account.
pub struct JmapPushManager {
    account_id: String,
    ws_url: String,
    state: Arc<RwLock<PushState>>,
    shutdown_tx: watch::Sender<bool>,
}

// ---------------------------------------------------------------------------
// WebSocket message types (RFC 8887)
// ---------------------------------------------------------------------------

/// Sent to the server to enable push notifications.
#[derive(Debug, Serialize)]
struct WebSocketPushEnable {
    #[serde(rename = "@type")]
    type_name: String,
    #[serde(rename = "dataTypes")]
    #[serde(skip_serializing_if = "Option::is_none")]
    data_types: Option<Vec<String>>,
    #[serde(rename = "pushState")]
    #[serde(skip_serializing_if = "Option::is_none")]
    push_state: Option<String>,
}

/// Sent to the server to disable push notifications.
#[derive(Debug, Serialize)]
struct WebSocketPushDisable {
    #[serde(rename = "@type")]
    type_name: String,
}

/// Envelope for incoming WebSocket messages — we only care about StateChange.
#[derive(Debug, Deserialize)]
struct IncomingMessage {
    #[serde(rename = "@type")]
    type_name: String,
    #[serde(default)]
    changed: Option<HashMap<String, HashMap<String, String>>>,
    /// Per RFC 8620 section 7.1, a summary push state string that can be
    /// sent back in `WebSocketPushEnable.pushState` to avoid replays.
    #[serde(rename = "pushState")]
    #[serde(default)]
    push_state: Option<String>,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum consecutive failures before giving up and falling back to polling.
const MAX_CONSECUTIVE_FAILURES: u32 = 5;

/// Maximum reconnection backoff (seconds).
const MAX_BACKOFF_SECS: u64 = 60;

/// Data types to subscribe to for push notifications.
const PUSH_DATA_TYPES: &[&str] = &["Email", "Mailbox", "EmailSubmission"];

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a channel pair for forwarding state changes from the push manager
/// to the sync layer.
pub fn create_push_channel() -> (mpsc::Sender<StateChange>, mpsc::Receiver<StateChange>) {
    mpsc::channel(64)
}

/// Start a WebSocket push connection for the given JMAP account.
///
/// Returns a `JmapPushManager` that owns the background task. State changes
/// are forwarded to `change_tx`. The connection auto-reconnects with
/// exponential backoff on failure.
///
/// Returns an error if the server does not advertise WebSocket capability.
pub async fn start_push(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
    change_tx: mpsc::Sender<StateChange>,
) -> Result<JmapPushManager, String> {
    let inner = client.inner();
    let session = inner.session();

    // Extract WebSocket URL from session capabilities
    let ws_caps = session.websocket_capabilities().ok_or_else(|| {
        "JMAP server does not advertise WebSocket capability \
         (urn:ietf:params:jmap:websocket)"
            .to_string()
    })?;

    if !ws_caps.supports_push() {
        return Err("JMAP server WebSocket capability does not support push".to_string());
    }

    let ws_url = ws_caps.url().to_string();

    // Extract auth header from the client
    let auth_header = inner.authorization().to_string();

    // Load last known push state from DB
    let last_push_state = load_push_state(db, account_id).await?;

    let state = Arc::new(RwLock::new(PushState::Disconnected));
    let (shutdown_tx, shutdown_rx) = watch::channel(false);

    let manager = JmapPushManager {
        account_id: account_id.to_string(),
        ws_url: ws_url.clone(),
        state: Arc::clone(&state),
        shutdown_tx,
    };

    // Persist initial push state
    save_push_enabled(db, account_id, &ws_url).await;

    // Spawn the background connection loop
    let aid = account_id.to_string();
    let db_clone = db.clone();
    tokio::spawn(async move {
        push_connection_loop(
            &aid,
            &ws_url,
            &auth_header,
            last_push_state,
            state,
            shutdown_rx,
            change_tx,
            &db_clone,
        )
        .await;
    });

    Ok(manager)
}

impl JmapPushManager {
    /// Get the current connection state.
    pub async fn state(&self) -> PushState {
        self.state.read().await.clone()
    }

    /// The account ID this push manager is associated with.
    pub fn account_id(&self) -> &str {
        &self.account_id
    }

    /// The WebSocket URL being used.
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// Stop the push connection and shut down the background task.
    pub async fn stop_push(&self) {
        let _ = self.shutdown_tx.send(true);
    }
}

// ---------------------------------------------------------------------------
// Connection loop
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
async fn push_connection_loop(
    account_id: &str,
    ws_url: &str,
    auth_header: &str,
    initial_push_state: Option<String>,
    state: Arc<RwLock<PushState>>,
    mut shutdown_rx: watch::Receiver<bool>,
    change_tx: mpsc::Sender<StateChange>,
    db: &DbState,
) {
    let mut consecutive_failures: u32 = 0;
    let mut last_push_state = initial_push_state;

    loop {
        // Check shutdown before connecting
        if *shutdown_rx.borrow() {
            *state.write().await = PushState::Disconnected;
            log::info!("[JMAP push] {account_id}: shutdown requested");
            break;
        }

        if consecutive_failures >= MAX_CONSECUTIVE_FAILURES {
            let msg = format!(
                "giving up after {MAX_CONSECUTIVE_FAILURES} consecutive failures, \
                 falling back to polling"
            );
            log::warn!("[JMAP push] {account_id}: {msg}");
            *state.write().await = PushState::Error(msg);
            save_push_disabled(db, account_id, consecutive_failures).await;
            break;
        }

        *state.write().await = PushState::Connecting;
        log::info!("[JMAP push] {account_id}: connecting to {ws_url}");

        match connect_and_listen(
            account_id,
            ws_url,
            auth_header,
            &last_push_state,
            &state,
            &mut shutdown_rx,
            &change_tx,
            db,
        )
        .await
        {
            Ok(new_push_state) => {
                // Clean disconnect — update last push state
                last_push_state = new_push_state.or(last_push_state);
                consecutive_failures = 0;
                save_consecutive_failures(db, account_id, 0).await;
            }
            Err(e) => {
                consecutive_failures += 1;
                log::warn!(
                    "[JMAP push] {account_id}: connection failed \
                     (attempt {consecutive_failures}/{MAX_CONSECUTIVE_FAILURES}): {e}"
                );
                *state.write().await = PushState::Error(e);
                save_consecutive_failures(db, account_id, consecutive_failures).await;

                // Exponential backoff before reconnect
                let backoff = backoff_duration(consecutive_failures);
                log::info!(
                    "[JMAP push] {account_id}: reconnecting in {}s",
                    backoff.as_secs()
                );

                tokio::select! {
                    _ = tokio::time::sleep(backoff) => {}
                    _ = shutdown_rx.changed() => {
                        if *shutdown_rx.borrow() {
                            *state.write().await = PushState::Disconnected;
                            log::info!("[JMAP push] {account_id}: shutdown during backoff");
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Connect to the WebSocket, enable push, and listen for messages.
///
/// Returns the last push state received on clean exit, or an error on failure.
#[allow(clippy::too_many_arguments)]
async fn connect_and_listen(
    account_id: &str,
    ws_url: &str,
    auth_header: &str,
    last_push_state: &Option<String>,
    state: &Arc<RwLock<PushState>>,
    shutdown_rx: &mut watch::Receiver<bool>,
    change_tx: &mpsc::Sender<StateChange>,
    db: &DbState,
) -> Result<Option<String>, String> {
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::http::HeaderValue;

    // Build the WebSocket request with auth and subprotocol headers
    let mut request = ws_url
        .into_client_request()
        .map_err(|e| format!("invalid WebSocket URL: {e}"))?;

    let headers = request.headers_mut();
    headers.insert(
        "Authorization",
        HeaderValue::from_str(auth_header)
            .map_err(|e| format!("invalid auth header value: {e}"))?,
    );
    headers.insert("Sec-WebSocket-Protocol", HeaderValue::from_static("jmap"));

    // Connect using native-tls
    let tls_connector = native_tls::TlsConnector::new()
        .map_err(|e| format!("TLS connector creation failed: {e}"))?;
    let connector = tokio_tungstenite::Connector::NativeTls(tls_connector);

    let (ws_stream, _response) =
        tokio_tungstenite::connect_async_tls_with_config(request, None, false, Some(connector))
            .await
            .map_err(|e| format!("WebSocket connect failed: {e}"))?;

    let (mut ws_sink, mut ws_reader) = ws_stream.split();

    // Send WebSocketPushEnable
    let enable_msg = WebSocketPushEnable {
        type_name: "WebSocketPushEnable".to_string(),
        data_types: Some(PUSH_DATA_TYPES.iter().map(|s| (*s).to_string()).collect()),
        push_state: last_push_state.clone(),
    };
    let enable_json =
        serde_json::to_string(&enable_msg).map_err(|e| format!("serialize push enable: {e}"))?;

    {
        use futures::SinkExt;
        use tokio_tungstenite::tungstenite::Message;
        ws_sink
            .send(Message::Text(enable_json.into()))
            .await
            .map_err(|e| format!("send push enable: {e}"))?;
    }

    log::info!("[JMAP push] {account_id}: connected and push enabled");
    *state.write().await = PushState::Connected;

    // Record connection time
    save_connected_at(db, account_id).await;

    // Listen for messages
    let mut current_push_state: Option<String> = last_push_state.clone();

    loop {
        tokio::select! {
            msg = ws_reader.next() => {
                match msg {
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Text(text))) => {
                        match serde_json::from_str::<IncomingMessage>(&text) {
                            Ok(incoming) if incoming.type_name == "StateChange" => {
                                if let Some(changed) = incoming.changed {
                                    // Update push state from incoming message if present
                                    if let Some(ps) = incoming.push_state {
                                        current_push_state = Some(ps);
                                    }

                                    let state_change = StateChange {
                                        type_name: incoming.type_name,
                                        changed,
                                    };
                                    log::info!(
                                        "[JMAP push] {account_id}: received state change"
                                    );

                                    // Persist updated push state to DB
                                    if let Some(ref ps) = current_push_state {
                                        save_last_push_state(db, account_id, ps).await;
                                    }

                                    if change_tx.send(state_change).await.is_err() {
                                        log::warn!(
                                            "[JMAP push] {account_id}: \
                                             change channel closed, stopping"
                                        );
                                        return Ok(current_push_state);
                                    }
                                }
                            }
                            Ok(incoming) => {
                                log::debug!(
                                    "[JMAP push] {account_id}: \
                                     ignoring message type: {}",
                                    incoming.type_name
                                );
                            }
                            Err(e) => {
                                log::warn!(
                                    "[JMAP push] {account_id}: \
                                     failed to parse WebSocket message: {e}"
                                );
                            }
                        }
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Ping(data))) => {
                        use futures::SinkExt;
                        use tokio_tungstenite::tungstenite::Message;
                        let _ = ws_sink.send(Message::Pong(data)).await;
                    }
                    Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_))) => {
                        log::info!(
                            "[JMAP push] {account_id}: server sent close frame"
                        );
                        return Ok(current_push_state);
                    }
                    Some(Ok(_)) => {
                        // Binary, Pong, Frame — ignore
                    }
                    Some(Err(e)) => {
                        return Err(format!("WebSocket read error: {e}"));
                    }
                    None => {
                        // Stream ended
                        log::info!(
                            "[JMAP push] {account_id}: WebSocket stream ended"
                        );
                        return Ok(current_push_state);
                    }
                }
            }
            _ = shutdown_rx.changed() => {
                if *shutdown_rx.borrow() {
                    // Send disable + close
                    let disable_msg = WebSocketPushDisable {
                        type_name: "WebSocketPushDisable".to_string(),
                    };
                    if let Ok(json) = serde_json::to_string(&disable_msg) {
                        use futures::SinkExt;
                        use tokio_tungstenite::tungstenite::Message;
                        let _ = ws_sink.send(Message::Text(json.into())).await;
                        let _ = ws_sink.send(Message::Close(None)).await;
                    }
                    log::info!("[JMAP push] {account_id}: shutdown, closing WebSocket");
                    return Ok(current_push_state);
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Backoff
// ---------------------------------------------------------------------------

/// Calculate exponential backoff duration for reconnection attempts.
///
/// Formula: min(2^(failures-1), MAX_BACKOFF_SECS) seconds.
fn backoff_duration(consecutive_failures: u32) -> std::time::Duration {
    let secs = if consecutive_failures == 0 {
        1
    } else {
        let exp = 1u64
            .checked_shl(consecutive_failures - 1)
            .unwrap_or(MAX_BACKOFF_SECS);
        exp.min(MAX_BACKOFF_SECS)
    };
    std::time::Duration::from_secs(secs)
}

// ---------------------------------------------------------------------------
// DB persistence helpers
// ---------------------------------------------------------------------------

async fn load_push_state(db: &DbState, account_id: &str) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT push_state FROM jmap_push_state WHERE account_id = ?1")
            .map_err(|e| format!("prepare load_push_state: {e}"))?;
        stmt.query_row(rusqlite::params![aid], |row| {
            row.get::<_, Option<String>>(0)
        })
        .map_err(|e| {
            if matches!(e, rusqlite::Error::QueryReturnedNoRows) {
                return String::new(); // sentinel for "no row"
            }
            format!("load_push_state: {e}")
        })
        .or_else(|e| if e.is_empty() { Ok(None) } else { Err(e) })
    })
    .await
}

async fn save_push_enabled(db: &DbState, account_id: &str, ws_url: &str) {
    let aid = account_id.to_string();
    let url = ws_url.to_string();
    if let Err(e) = db
        .with_conn(move |conn| {
            conn.execute(
                "INSERT INTO jmap_push_state (account_id, ws_url, is_push_enabled) \
                 VALUES (?1, ?2, 1) \
                 ON CONFLICT(account_id) DO UPDATE SET \
                   ws_url = ?2, is_push_enabled = 1",
                rusqlite::params![aid, url],
            )
            .map_err(|e| format!("save_push_enabled: {e}"))
        })
        .await
    {
        log::warn!("[JMAP push] failed to save push enabled state: {e}");
    }
}

async fn save_push_disabled(db: &DbState, account_id: &str, failures: u32) {
    let aid = account_id.to_string();
    if let Err(e) = db
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE jmap_push_state SET is_push_enabled = 0, \
                 consecutive_failures = ?2 WHERE account_id = ?1",
                rusqlite::params![aid, failures],
            )
            .map_err(|e| format!("save_push_disabled: {e}"))
        })
        .await
    {
        log::warn!("[JMAP push] failed to save push disabled state: {e}");
    }
}

async fn save_connected_at(db: &DbState, account_id: &str) {
    let aid = account_id.to_string();
    if let Err(e) = db
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE jmap_push_state SET last_connected_at = unixepoch(), \
                 consecutive_failures = 0 WHERE account_id = ?1",
                rusqlite::params![aid],
            )
            .map_err(|e| format!("save_connected_at: {e}"))
        })
        .await
    {
        log::warn!("[JMAP push] failed to save connected_at: {e}");
    }
}

async fn save_consecutive_failures(db: &DbState, account_id: &str, failures: u32) {
    let aid = account_id.to_string();
    if let Err(e) = db
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE jmap_push_state SET consecutive_failures = ?2 \
                 WHERE account_id = ?1",
                rusqlite::params![aid, failures],
            )
            .map_err(|e| format!("save_consecutive_failures: {e}"))
        })
        .await
    {
        log::warn!("[JMAP push] failed to save consecutive failures: {e}");
    }
}

async fn save_last_push_state(db: &DbState, account_id: &str, push_state: &str) {
    let aid = account_id.to_string();
    let ps = push_state.to_string();
    if let Err(e) = db
        .with_conn(move |conn| {
            conn.execute(
                "UPDATE jmap_push_state SET push_state = ?2 WHERE account_id = ?1",
                rusqlite::params![aid, ps],
            )
            .map_err(|e| format!("save_last_push_state: {e}"))
        })
        .await
    {
        log::warn!("[JMAP push] failed to save last push state: {e}");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_change_deserialization() {
        let json = r#"{
            "@type": "StateChange",
            "changed": {
                "a1": {
                    "Email": "state_abc",
                    "Mailbox": "state_def"
                }
            }
        }"#;

        let sc: StateChange = serde_json::from_str(json).expect("should deserialize StateChange");
        assert_eq!(sc.type_name, "StateChange");
        assert_eq!(sc.changed.len(), 1);
        let account = sc.changed.get("a1").expect("should have account a1");
        assert_eq!(account.get("Email").map(String::as_str), Some("state_abc"));
        assert_eq!(
            account.get("Mailbox").map(String::as_str),
            Some("state_def")
        );
    }

    #[test]
    fn state_change_multiple_accounts() {
        let json = r#"{
            "@type": "StateChange",
            "changed": {
                "account1": { "Email": "s1" },
                "account2": { "Mailbox": "s2", "EmailSubmission": "s3" }
            }
        }"#;

        let sc: StateChange = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(sc.changed.len(), 2);
        assert_eq!(sc.changed["account2"].len(), 2);
    }

    #[test]
    fn state_change_empty_changed() {
        let json = r#"{ "@type": "StateChange", "changed": {} }"#;
        let sc: StateChange = serde_json::from_str(json).expect("should deserialize empty");
        assert!(sc.changed.is_empty());
    }

    #[test]
    fn incoming_message_state_change() {
        let json = r#"{
            "@type": "StateChange",
            "changed": { "a": { "Email": "x" } }
        }"#;
        let msg: IncomingMessage = serde_json::from_str(json).expect("should parse");
        assert_eq!(msg.type_name, "StateChange");
        assert!(msg.changed.is_some());
    }

    #[test]
    fn incoming_message_non_state_change() {
        let json = r#"{ "@type": "Response", "methodResponses": [] }"#;
        let msg: IncomingMessage = serde_json::from_str(json).expect("should parse");
        assert_eq!(msg.type_name, "Response");
        assert!(msg.changed.is_none());
    }

    #[test]
    fn push_enable_serialization() {
        let msg = WebSocketPushEnable {
            type_name: "WebSocketPushEnable".to_string(),
            data_types: Some(vec!["Email".to_string(), "Mailbox".to_string()]),
            push_state: Some("prev_state".to_string()),
        };
        let json = serde_json::to_string(&msg).expect("should serialize");
        assert!(json.contains("\"@type\":\"WebSocketPushEnable\""));
        assert!(json.contains("\"dataTypes\""));
        assert!(json.contains("\"pushState\":\"prev_state\""));
    }

    #[test]
    fn push_enable_no_optional_fields() {
        let msg = WebSocketPushEnable {
            type_name: "WebSocketPushEnable".to_string(),
            data_types: None,
            push_state: None,
        };
        let json = serde_json::to_string(&msg).expect("should serialize");
        assert!(!json.contains("dataTypes"));
        assert!(!json.contains("pushState"));
    }

    #[test]
    fn push_disable_serialization() {
        let msg = WebSocketPushDisable {
            type_name: "WebSocketPushDisable".to_string(),
        };
        let json = serde_json::to_string(&msg).expect("should serialize");
        assert!(json.contains("\"@type\":\"WebSocketPushDisable\""));
    }

    #[test]
    fn push_state_transitions() {
        assert_eq!(PushState::Disconnected, PushState::Disconnected);
        assert_eq!(PushState::Connecting, PushState::Connecting);
        assert_eq!(PushState::Connected, PushState::Connected);
        assert_eq!(
            PushState::Error("test".to_string()),
            PushState::Error("test".to_string())
        );
        assert_ne!(PushState::Connected, PushState::Disconnected);
        assert_ne!(
            PushState::Error("a".to_string()),
            PushState::Error("b".to_string())
        );
    }

    #[test]
    fn backoff_exponential() {
        assert_eq!(backoff_duration(0), std::time::Duration::from_secs(1));
        assert_eq!(backoff_duration(1), std::time::Duration::from_secs(1));
        assert_eq!(backoff_duration(2), std::time::Duration::from_secs(2));
        assert_eq!(backoff_duration(3), std::time::Duration::from_secs(4));
        assert_eq!(backoff_duration(4), std::time::Duration::from_secs(8));
        assert_eq!(backoff_duration(5), std::time::Duration::from_secs(16));
        assert_eq!(backoff_duration(6), std::time::Duration::from_secs(32));
        assert_eq!(backoff_duration(7), std::time::Duration::from_secs(60));
        assert_eq!(backoff_duration(100), std::time::Duration::from_secs(60));
    }

    #[test]
    fn create_push_channel_works() {
        let (tx, mut rx) = create_push_channel();
        // Channel should be empty
        assert!(rx.try_recv().is_err());

        // Send a state change through the channel
        let sc = StateChange {
            type_name: "StateChange".to_string(),
            changed: HashMap::new(),
        };
        tx.try_send(sc).expect("should send");
        let received = rx.try_recv().expect("should receive");
        assert_eq!(received.type_name, "StateChange");
    }
}
