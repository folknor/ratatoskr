//! JMAP push notification setup.
//!
//! Moved to core so the app crate doesn't need direct JMAP provider dependencies.
//! Each account gets a long-lived bridge task that forwards state changes as
//! account-ID notifications through a caller-provided channel.
//!
//! ## Lifecycle
//!
//! The bridge task owns the `JmapPushManager`, keeping its `shutdown_tx` alive.
//! The task exits when either:
//!
//! - The push connection dies (`rx.recv()` returns `None`) — the push manager's
//!   background WebSocket loop ended (server disconnect, max failures).
//! - The app shuts down — the iced subscription drops the `UnboundedReceiver`,
//!   which causes `notify_tx.send()` to return `Err`, breaking the loop.
//!
//! On exit, the manager drops, `shutdown_tx` closes, and the push connection
//! loop terminates with a clean WebSocket close frame.
//!
//! ## Debounce
//!
//! JMAP servers can emit rapid-fire state changes (e.g., batch email import).
//! The bridge task coalesces notifications within a 500ms window to avoid
//! spawning concurrent syncs for the same account.

use crate::db::DbState;

/// How long to wait after a state change before forwarding the notification,
/// coalescing any additional changes that arrive in the window.
const PUSH_DEBOUNCE: std::time::Duration = std::time::Duration::from_millis(500);

/// Start JMAP push for a single account.
///
/// Constructs the JMAP client, starts the push manager, and spawns a
/// bridge task that forwards state-change notifications as `account_id`
/// strings through `notify_tx`. Returns immediately after setup; the
/// bridge task runs until the push connection dies or the app shuts down.
pub async fn start_jmap_push_for_account(
    db: &DbState,
    account_id: &str,
    email: &str,
    encryption_key: [u8; 32],
    notify_tx: tokio::sync::mpsc::UnboundedSender<String>,
) -> Result<(), String> {
    let client = jmap::client::JmapClient::from_account(db, account_id, &encryption_key).await?;

    let (tx, mut rx) = jmap::push::create_push_channel();
    let manager = jmap::push::start_push(&client, account_id, db, tx).await?;

    let aid = account_id.to_string();
    let email = email.to_string();
    tokio::spawn(async move {
        // Moving the manager into this task keeps shutdown_tx alive.
        let _manager = manager;
        log::info!("[JMAP Push] Listening for changes on {email}");
        while let Some(change) = rx.recv().await {
            log::info!(
                "[JMAP Push] State change for {email}: {} data types changed",
                change.changed.len()
            );
            // Debounce: drain rapid-fire changes within the window so we
            // send a single sync notification per burst.
            let mut coalesced = 0u32;
            let deadline = tokio::time::Instant::now() + PUSH_DEBOUNCE;
            while let Ok(Some(_)) = tokio::time::timeout_at(deadline, rx.recv()).await {
                coalesced += 1;
            }
            if coalesced > 0 {
                log::debug!("[JMAP Push] Coalesced {coalesced} additional changes for {email}");
            }
            if notify_tx.send(aid.clone()).is_err() {
                log::info!("[JMAP Push] Notify channel closed for {email}");
                break;
            }
        }
        log::info!("[JMAP Push] Push ended for {email}");
    });

    Ok(())
}
