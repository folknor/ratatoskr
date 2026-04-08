//! Microsoft Graph change notification subscriptions.
//!
//! Manages subscription CRUD lifecycle for near-real-time change notifications.
//! This is the management layer only — the HTTP listener that receives
//! notifications will be wired up separately.
//!
//! Graph subscriptions require a public HTTPS endpoint for notifications.
//! Desktop apps typically need a relay service; this module handles the
//! subscription lifecycle regardless of how notifications are delivered.

use serde::{Deserialize, Serialize};

use db::db::DbState;

use super::client::GraphClient;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default subscription duration in minutes (24 hours).
const DEFAULT_EXPIRATION_MINUTES: u32 = 1440;

/// Maximum subscription duration for message resources (~2.9 days = 4230 min).
const MAX_EXPIRATION_MINUTES: u32 = 4230;

/// Renew subscriptions when they expire within this many minutes.
const RENEWAL_THRESHOLD_MINUTES: i64 = 30;

/// Length of the random hex client_state string (16 bytes = 32 hex chars).
const CLIENT_STATE_BYTES: usize = 16;

// ---------------------------------------------------------------------------
// Request / response types
// ---------------------------------------------------------------------------

/// Request body for `POST /subscriptions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphSubscription {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub change_type: String,
    pub notification_url: String,
    pub resource: String,
    pub expiration_date_time: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub client_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle_notification_url: Option<String>,
}

/// Response from Graph subscription endpoints.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionResponse {
    pub id: String,
    pub change_type: String,
    pub notification_url: String,
    pub resource: String,
    pub expiration_date_time: String,
    pub client_state: Option<String>,
}

/// Wrapper for `GET /subscriptions` list response.
#[derive(Debug, Deserialize)]
pub struct SubscriptionListResponse {
    pub value: Vec<SubscriptionResponse>,
}

/// Request body for `PATCH /subscriptions/{id}` renewal.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RenewSubscriptionRequest {
    expiration_date_time: String,
}

// ---------------------------------------------------------------------------
// Notification types
// ---------------------------------------------------------------------------

/// Top-level notification payload sent by Graph to the notification URL.
#[derive(Debug, Clone, Deserialize)]
pub struct NotificationPayload {
    pub value: Vec<ChangeNotification>,
}

/// A single change notification within the payload.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangeNotification {
    pub subscription_id: String,
    pub change_type: String,
    pub resource: String,
    #[serde(default)]
    pub resource_data: Option<ResourceData>,
    #[serde(default)]
    pub client_state: Option<String>,
    #[serde(default)]
    pub subscription_expiration_date_time: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
}

/// Resource data included in a change notification.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceData {
    #[serde(rename = "@odata.type")]
    pub odata_type: Option<String>,
    #[serde(rename = "@odata.id")]
    pub odata_id: Option<String>,
    pub id: Option<String>,
}

// ---------------------------------------------------------------------------
// DB record
// ---------------------------------------------------------------------------

/// Local record of a Graph subscription stored in the database.
#[derive(Debug, Clone)]
pub struct SubscriptionRecord {
    pub id: String,
    pub account_id: String,
    pub resource: String,
    pub notification_url: String,
    pub client_state: String,
    pub expiration_date_time: String,
    pub created_at: i64,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Create a new Graph change notification subscription.
///
/// Sends `POST /subscriptions` and persists the result in the local DB.
/// The `resource` should be an API-path-prefixed resource string, e.g.
/// `"/me/messages"` or `"/me/mailFolders/{id}/messages"`.
pub async fn create_subscription(
    client: &GraphClient,
    db: &DbState,
    account_id: &str,
    resource: &str,
    notification_url: &str,
    expiration_minutes: Option<u32>,
) -> Result<SubscriptionResponse, String> {
    let minutes = expiration_minutes
        .unwrap_or(DEFAULT_EXPIRATION_MINUTES)
        .min(MAX_EXPIRATION_MINUTES);

    let expiry = compute_expiry_iso8601(minutes);
    let client_state = generate_client_state()?;

    let body = GraphSubscription {
        id: None,
        change_type: "created,updated,deleted".to_string(),
        notification_url: notification_url.to_string(),
        resource: resource.to_string(),
        expiration_date_time: expiry,
        client_state: Some(client_state.clone()),
        lifecycle_notification_url: None,
    };

    let response: SubscriptionResponse = client.post("/subscriptions", &body, db).await?;

    log::info!(
        "[Graph webhooks] Created subscription {} for resource '{}' (expires {})",
        response.id,
        resource,
        response.expiration_date_time
    );

    save_graph_subscription(
        db,
        account_id,
        &response.id,
        resource,
        notification_url,
        &client_state,
        &response.expiration_date_time,
    )
    .await?;

    Ok(response)
}

/// Renew an existing subscription by extending its expiration.
///
/// Sends `PATCH /subscriptions/{id}` with a new expiration time.
pub async fn renew_subscription(
    client: &GraphClient,
    db: &DbState,
    subscription_id: &str,
    expiration_minutes: Option<u32>,
) -> Result<String, String> {
    let minutes = expiration_minutes
        .unwrap_or(DEFAULT_EXPIRATION_MINUTES)
        .min(MAX_EXPIRATION_MINUTES);

    let new_expiry = compute_expiry_iso8601(minutes);
    let path = format!("/subscriptions/{subscription_id}");

    let body = RenewSubscriptionRequest {
        expiration_date_time: new_expiry.clone(),
    };

    client.patch(&path, &body, db).await?;

    log::info!(
        "[Graph webhooks] Renewed subscription {subscription_id} (new expiry: {new_expiry})"
    );

    update_graph_subscription_expiry(db, subscription_id, &new_expiry).await?;

    Ok(new_expiry)
}

/// Delete a subscription.
///
/// Sends `DELETE /subscriptions/{id}` and removes the local DB record.
pub async fn delete_subscription(
    client: &GraphClient,
    db: &DbState,
    subscription_id: &str,
) -> Result<(), String> {
    let path = format!("/subscriptions/{subscription_id}");

    // Best-effort delete on the server — the subscription may have already
    // expired, in which case Graph returns 404. Either way, clean up locally.
    let server_result = client.delete(&path, db).await;
    if let Err(ref e) = server_result {
        if !e.contains("404") {
            return Err(e.clone());
        }
        log::info!("[Graph webhooks] Subscription {subscription_id} already gone on server (404)");
    }

    delete_graph_subscription_record(db, subscription_id).await?;

    log::info!("[Graph webhooks] Deleted subscription {subscription_id}");
    Ok(())
}

/// List all active subscriptions from the Graph API.
pub async fn list_subscriptions(
    client: &GraphClient,
    db: &DbState,
) -> Result<Vec<SubscriptionResponse>, String> {
    let response: SubscriptionListResponse = client.get_json("/subscriptions", db).await?;
    Ok(response.value)
}

/// Validate that a notification's `client_state` matches the expected secret.
pub fn validate_notification(
    notification: &ChangeNotification,
    expected_client_state: &str,
) -> bool {
    notification
        .client_state
        .as_deref()
        .is_some_and(|cs| cs == expected_client_state)
}

/// Parse a raw JSON notification payload from the request body.
pub fn parse_notification_payload(body: &str) -> Result<NotificationPayload, String> {
    serde_json::from_str(body).map_err(|e| format!("Failed to parse notification payload: {e}"))
}

/// Check all subscriptions for an account and renew or recreate as needed.
///
/// - Subscriptions expiring within 30 minutes are renewed.
/// - Expired subscriptions are deleted locally (the server already removed them).
///
/// Should be called periodically (e.g. every sync cycle).
pub async fn check_and_renew_subscriptions(
    client: &GraphClient,
    db: &DbState,
    account_id: &str,
) -> Result<(), String> {
    let subs = load_graph_subscriptions(db, account_id).await?;
    if subs.is_empty() {
        return Ok(());
    }

    let now = now_unix();

    for sub in &subs {
        let expiry_unix = parse_iso8601_to_unix(&sub.expiration_date_time);

        let minutes_remaining = (expiry_unix - now) / 60;

        if minutes_remaining < 0 {
            // Already expired — clean up local record
            log::info!(
                "[Graph webhooks] Subscription {} expired, removing local record",
                sub.id
            );
            if let Err(e) = delete_graph_subscription_record(db, &sub.id).await {
                log::warn!(
                    "[Graph webhooks] failed to delete expired subscription record {}: {e}",
                    sub.id
                );
            }
        } else if minutes_remaining < RENEWAL_THRESHOLD_MINUTES {
            log::info!(
                "[Graph webhooks] Subscription {} expires in {minutes_remaining}min, renewing",
                sub.id
            );
            match renew_subscription(client, db, &sub.id, None).await {
                Ok(new_expiry) => {
                    log::info!(
                        "[Graph webhooks] Renewed subscription {} until {new_expiry}",
                        sub.id
                    );
                }
                Err(e) => {
                    log::warn!(
                        "[Graph webhooks] Failed to renew subscription {}: {e}",
                        sub.id
                    );
                    // If renewal fails (e.g. 404), clean up
                    if e.contains("404")
                        && let Err(e) = delete_graph_subscription_record(db, &sub.id).await
                    {
                        log::warn!(
                            "[Graph webhooks] failed to delete stale subscription record {}: {e}",
                            sub.id
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Generate a cryptographically random 32-char hex string for `client_state`.
fn generate_client_state() -> Result<String, String> {
    let mut buf = [0u8; CLIENT_STATE_BYTES];
    getrandom::fill(&mut buf).map_err(|e| format!("RNG failed: {e}"))?;
    Ok(hex_encode(&buf))
}

/// Encode bytes as lowercase hex.
fn hex_encode(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{b:02x}");
    }
    s
}

/// Compute an ISO 8601 expiration timestamp `minutes` from now.
fn compute_expiry_iso8601(minutes: u32) -> String {
    let secs = now_unix() + i64::from(minutes) * 60;
    unix_to_iso8601(secs)
}

/// Current time as Unix epoch seconds.
fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        .cast_signed()
}

/// Convert Unix epoch seconds to ISO 8601 UTC string.
///
/// Format: `2024-01-15T12:00:00Z`
fn unix_to_iso8601(secs: i64) -> String {
    // Manual conversion without chrono dependency
    const SECS_PER_DAY: i64 = 86400;
    let days = secs.div_euclid(SECS_PER_DAY);
    let day_secs = secs.rem_euclid(SECS_PER_DAY);

    let hours = day_secs / 3600;
    let minutes = (day_secs % 3600) / 60;
    let seconds = day_secs % 60;

    // Days since Unix epoch to Y-M-D (algorithm from Howard Hinnant)
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

/// Parse an ISO 8601 UTC timestamp to Unix epoch seconds.
///
/// Accepts `YYYY-MM-DDTHH:MM:SSZ` and `YYYY-MM-DDTHH:MM:SS.fffZ`.
fn parse_iso8601_to_unix(s: &str) -> i64 {
    // Strip trailing 'Z' and any fractional seconds
    let s = s.trim_end_matches('Z');
    let s = if let Some(dot_pos) = s.rfind('.') {
        &s[..dot_pos]
    } else {
        s
    };

    // Parse components: YYYY-MM-DDTHH:MM:SS
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return 0;
    }

    let date_parts: Vec<i64> = parts[0].split('-').filter_map(|p| p.parse().ok()).collect();
    let time_parts: Vec<i64> = parts[1].split(':').filter_map(|p| p.parse().ok()).collect();

    if date_parts.len() != 3 || time_parts.len() != 3 {
        return 0;
    }

    let (y, m, d) = (date_parts[0], date_parts[1], date_parts[2]);
    let (hh, mm, ss) = (time_parts[0], time_parts[1], time_parts[2]);

    // Inverse of Howard Hinnant's algorithm
    let y_adj = if m <= 2 { y - 1 } else { y };
    let era = if y_adj >= 0 { y_adj } else { y_adj - 399 } / 400;
    let yoe = y_adj - era * 400;
    let m_adj = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * m_adj + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;

    days * 86400 + hh * 3600 + mm * 60 + ss
}

/// Check whether a subscription is expiring within the given threshold.
pub fn is_expiring_soon(expiration_iso: &str, threshold_minutes: i64) -> bool {
    let expiry = parse_iso8601_to_unix(expiration_iso);
    let remaining = expiry - now_unix();
    remaining < threshold_minutes * 60
}

// ---------------------------------------------------------------------------
// DB persistence
// ---------------------------------------------------------------------------

async fn save_graph_subscription(
    db: &DbState,
    account_id: &str,
    subscription_id: &str,
    resource: &str,
    notification_url: &str,
    client_state: &str,
    expiration_date_time: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let sid = subscription_id.to_string();
    let res = resource.to_string();
    let url = notification_url.to_string();
    let cs = client_state.to_string();
    let exp = expiration_date_time.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO graph_subscriptions \
             (id, account_id, resource, notification_url, client_state, expiration_date_time) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![sid, aid, res, url, cs, exp],
        )
        .map_err(|e| format!("save_graph_subscription: {e}"))?;
        Ok(())
    })
    .await
}

/// Load all subscriptions for a given account from the local DB.
pub async fn load_graph_subscriptions(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<SubscriptionRecord>, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, account_id, resource, notification_url, \
                 client_state, expiration_date_time, created_at \
                 FROM graph_subscriptions WHERE account_id = ?1",
            )
            .map_err(|e| format!("prepare load_graph_subscriptions: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params![aid], |row| {
                Ok(SubscriptionRecord {
                    id: row.get("id")?,
                    account_id: row.get("account_id")?,
                    resource: row.get("resource")?,
                    notification_url: row.get("notification_url")?,
                    client_state: row.get("client_state")?,
                    expiration_date_time: row.get("expiration_date_time")?,
                    created_at: row.get("created_at")?,
                })
            })
            .map_err(|e| format!("query load_graph_subscriptions: {e}"))?;

        let mut result = Vec::new();
        for row in rows {
            result.push(row.map_err(|e| format!("row load_graph_subscriptions: {e}"))?);
        }
        Ok(result)
    })
    .await
}

async fn delete_graph_subscription_record(
    db: &DbState,
    subscription_id: &str,
) -> Result<(), String> {
    let sid = subscription_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM graph_subscriptions WHERE id = ?1",
            rusqlite::params![sid],
        )
        .map_err(|e| format!("delete_graph_subscription: {e}"))?;
        Ok(())
    })
    .await
}

async fn update_graph_subscription_expiry(
    db: &DbState,
    subscription_id: &str,
    new_expiry: &str,
) -> Result<(), String> {
    let sid = subscription_id.to_string();
    let exp = new_expiry.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE graph_subscriptions SET expiration_date_time = ?2 WHERE id = ?1",
            rusqlite::params![sid, exp],
        )
        .map_err(|e| format!("update_graph_subscription_expiry: {e}"))?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_request_serialization() {
        let sub = GraphSubscription {
            id: None,
            change_type: "created,updated,deleted".to_string(),
            notification_url: "https://example.com/notify".to_string(),
            resource: "/me/messages".to_string(),
            expiration_date_time: "2024-06-15T12:00:00Z".to_string(),
            client_state: Some("abc123".to_string()),
            lifecycle_notification_url: None,
        };

        let json = serde_json::to_string(&sub).expect("should serialize");
        assert!(json.contains("\"changeType\":\"created,updated,deleted\""));
        assert!(json.contains("\"notificationUrl\":\"https://example.com/notify\""));
        assert!(json.contains("\"resource\":\"/me/messages\""));
        assert!(json.contains("\"clientState\":\"abc123\""));
        // id is None, should be skipped
        assert!(!json.contains("\"id\""));
        // lifecycleNotificationUrl is None, should be skipped
        assert!(!json.contains("lifecycleNotificationUrl"));
    }

    #[test]
    fn subscription_response_deserialization() {
        let json = r#"{
            "id": "sub-123",
            "changeType": "created,updated,deleted",
            "notificationUrl": "https://example.com/notify",
            "resource": "/me/messages",
            "expirationDateTime": "2024-06-15T12:00:00Z",
            "clientState": "secret123"
        }"#;

        let resp: SubscriptionResponse = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(resp.id, "sub-123");
        assert_eq!(resp.change_type, "created,updated,deleted");
        assert_eq!(resp.resource, "/me/messages");
        assert_eq!(resp.client_state.as_deref(), Some("secret123"));
    }

    #[test]
    fn subscription_response_without_client_state() {
        let json = r#"{
            "id": "sub-456",
            "changeType": "created",
            "notificationUrl": "https://example.com/notify",
            "resource": "/me/mailFolders/inbox/messages",
            "expirationDateTime": "2024-06-15T12:00:00Z"
        }"#;

        let resp: SubscriptionResponse = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(resp.id, "sub-456");
        assert!(resp.client_state.is_none());
    }

    #[test]
    fn notification_payload_deserialization() {
        let json = r##"{
            "value": [
                {
                    "subscriptionId": "sub-123",
                    "changeType": "created",
                    "resource": "messages/AAMk123",
                    "resourceData": {
                        "@odata.type": "#Microsoft.Graph.Message",
                        "@odata.id": "messages/AAMk123",
                        "id": "AAMk123"
                    },
                    "clientState": "secret123",
                    "tenantId": "tenant-abc"
                }
            ]
        }"##;

        let payload = parse_notification_payload(json).expect("should parse");
        assert_eq!(payload.value.len(), 1);

        let notif = &payload.value[0];
        assert_eq!(notif.subscription_id, "sub-123");
        assert_eq!(notif.change_type, "created");
        assert_eq!(notif.resource, "messages/AAMk123");
        assert_eq!(notif.client_state.as_deref(), Some("secret123"));
        assert_eq!(notif.tenant_id.as_deref(), Some("tenant-abc"));

        let data = notif.resource_data.as_ref().expect("should have data");
        assert_eq!(data.odata_type.as_deref(), Some("#Microsoft.Graph.Message"));
        assert_eq!(data.id.as_deref(), Some("AAMk123"));
    }

    #[test]
    fn notification_multiple_changes() {
        let json = r#"{
            "value": [
                {
                    "subscriptionId": "sub-1",
                    "changeType": "created",
                    "resource": "messages/A"
                },
                {
                    "subscriptionId": "sub-1",
                    "changeType": "updated",
                    "resource": "messages/B"
                },
                {
                    "subscriptionId": "sub-1",
                    "changeType": "deleted",
                    "resource": "messages/C"
                }
            ]
        }"#;

        let payload = parse_notification_payload(json).expect("should parse");
        assert_eq!(payload.value.len(), 3);
        assert_eq!(payload.value[0].change_type, "created");
        assert_eq!(payload.value[1].change_type, "updated");
        assert_eq!(payload.value[2].change_type, "deleted");
    }

    #[test]
    fn notification_minimal_fields() {
        let json = r#"{
            "value": [
                {
                    "subscriptionId": "sub-1",
                    "changeType": "updated",
                    "resource": "messages/X"
                }
            ]
        }"#;

        let payload = parse_notification_payload(json).expect("should parse");
        let notif = &payload.value[0];
        assert!(notif.resource_data.is_none());
        assert!(notif.client_state.is_none());
        assert!(notif.tenant_id.is_none());
        assert!(notif.subscription_expiration_date_time.is_none());
    }

    #[test]
    fn validate_notification_matches() {
        let notif = ChangeNotification {
            subscription_id: "sub-1".to_string(),
            change_type: "created".to_string(),
            resource: "messages/A".to_string(),
            resource_data: None,
            client_state: Some("my_secret".to_string()),
            subscription_expiration_date_time: None,
            tenant_id: None,
        };
        assert!(validate_notification(&notif, "my_secret"));
        assert!(!validate_notification(&notif, "wrong_secret"));
    }

    #[test]
    fn validate_notification_missing_state() {
        let notif = ChangeNotification {
            subscription_id: "sub-1".to_string(),
            change_type: "created".to_string(),
            resource: "messages/A".to_string(),
            resource_data: None,
            client_state: None,
            subscription_expiration_date_time: None,
            tenant_id: None,
        };
        assert!(!validate_notification(&notif, "any_secret"));
    }

    #[test]
    fn client_state_generation() {
        let state = generate_client_state().expect("should generate");
        assert_eq!(state.len(), CLIENT_STATE_BYTES * 2); // hex encoding doubles length
        assert!(state.chars().all(|c| c.is_ascii_hexdigit()));

        // Should be unique each time
        let state2 = generate_client_state().expect("should generate");
        assert_ne!(state, state2);
    }

    #[test]
    fn hex_encode_works() {
        assert_eq!(hex_encode(&[0x00, 0xff, 0xab, 0x12]), "00ffab12");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn unix_to_iso8601_known_values() {
        // 2024-01-15T00:00:00Z
        assert_eq!(unix_to_iso8601(1705276800), "2024-01-15T00:00:00Z");
        // Unix epoch
        assert_eq!(unix_to_iso8601(0), "1970-01-01T00:00:00Z");
        // 2000-01-01T00:00:00Z
        assert_eq!(unix_to_iso8601(946684800), "2000-01-01T00:00:00Z");
    }

    #[test]
    fn parse_iso8601_to_unix_known_values() {
        assert_eq!(parse_iso8601_to_unix("1970-01-01T00:00:00Z"), 0);
        assert_eq!(parse_iso8601_to_unix("2024-01-15T00:00:00Z"), 1705276800);
        assert_eq!(parse_iso8601_to_unix("2000-01-01T00:00:00Z"), 946684800);
    }

    #[test]
    fn parse_iso8601_with_fractional_seconds() {
        // Should strip fractional seconds and still parse
        assert_eq!(
            parse_iso8601_to_unix("2024-01-15T00:00:00.000Z"),
            1705276800
        );
        assert_eq!(
            parse_iso8601_to_unix("2024-01-15T12:30:45.123456Z"),
            parse_iso8601_to_unix("2024-01-15T12:30:45Z")
        );
    }

    #[test]
    fn iso8601_roundtrip() {
        let timestamps = [0i64, 1705276800, 946684800, 1718448000, 86400];
        for ts in timestamps {
            let iso = unix_to_iso8601(ts);
            let parsed = parse_iso8601_to_unix(&iso);
            assert_eq!(parsed, ts, "roundtrip failed for {ts} -> {iso} -> {parsed}");
        }
    }

    #[test]
    fn expiry_computation() {
        let expiry = compute_expiry_iso8601(60); // 1 hour from now
        let expiry_unix = parse_iso8601_to_unix(&expiry);
        let now = now_unix();
        // Should be approximately 3600 seconds in the future (allow 5s tolerance)
        let diff = expiry_unix - now;
        assert!((3595..=3605).contains(&diff), "expected ~3600, got {diff}");
    }

    #[test]
    fn is_expiring_soon_logic() {
        // Already expired
        let past = unix_to_iso8601(now_unix() - 3600);
        assert!(is_expiring_soon(&past, 30));

        // Expiring in 10 minutes (within 30-minute threshold)
        let soon = unix_to_iso8601(now_unix() + 600);
        assert!(is_expiring_soon(&soon, 30));

        // Expiring in 2 hours (outside 30-minute threshold)
        let later = unix_to_iso8601(now_unix() + 7200);
        assert!(!is_expiring_soon(&later, 30));
    }

    #[test]
    fn subscription_list_response_deserialization() {
        let json = r#"{
            "value": [
                {
                    "id": "sub-1",
                    "changeType": "created,updated,deleted",
                    "notificationUrl": "https://a.com/n",
                    "resource": "/me/messages",
                    "expirationDateTime": "2024-06-15T12:00:00Z"
                },
                {
                    "id": "sub-2",
                    "changeType": "created",
                    "notificationUrl": "https://b.com/n",
                    "resource": "/me/mailFolders/inbox/messages",
                    "expirationDateTime": "2024-06-16T12:00:00Z",
                    "clientState": "secret"
                }
            ]
        }"#;

        let resp: SubscriptionListResponse =
            serde_json::from_str(json).expect("should deserialize");
        assert_eq!(resp.value.len(), 2);
        assert_eq!(resp.value[0].id, "sub-1");
        assert_eq!(resp.value[1].id, "sub-2");
    }

    #[test]
    fn renew_request_serialization() {
        let req = RenewSubscriptionRequest {
            expiration_date_time: "2024-06-16T12:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&req).expect("should serialize");
        assert!(json.contains("\"expirationDateTime\":\"2024-06-16T12:00:00Z\""));
        // Should only have the one field
        assert!(!json.contains("changeType"));
        assert!(!json.contains("resource"));
    }

    #[test]
    fn parse_invalid_iso8601() {
        assert_eq!(parse_iso8601_to_unix("not-a-date"), 0);
        assert_eq!(parse_iso8601_to_unix(""), 0);
        assert_eq!(parse_iso8601_to_unix("2024-01-15"), 0); // missing time
    }
}
