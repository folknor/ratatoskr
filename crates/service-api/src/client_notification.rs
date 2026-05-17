//! UI -> Service notifications (Phase 2 plan scope item 11).
//!
//! Asymmetrical to `Notification` (Service -> UI). The UI fires these
//! fire-and-forget so a chatty tick like `pending_ops.kick` on
//! `Message::SyncTick` doesn't await a Service round-trip. The Service
//! runs them on a separate task pool with `Drop`-class admission so a
//! slow notification handler cannot starve the request dispatcher's
//! semaphore.
//!
//! Wire shape: an `id`-less JSON-RPC envelope. The framing parser
//! (`parse_client_message`) routes on `id IS NULL` to the notification
//! path; messages with an `id` continue to use the request path.

use crate::notification::NotificationClass;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// UI -> Service notifications.
///
/// Phase 2 ships exactly one variant; future phases extend (e.g. a
/// `chat.viewing_changed` for read-state hinting).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum ClientNotification {
    /// "The UI's tick fired; please consider draining
    /// `pending_operations`." Phase 2 task 18 relocates the periodic
    /// drainer Service-side; the trigger remains UI-driven so the
    /// existing tick policy (focus / online state gating) stays
    /// UI-owned.
    #[serde(rename = "pending_ops.kick")]
    PendingOpsKick,
    /// Best-effort cancellation of a retry-queue row before the UI
    /// dispatches an undo inverse plan. The mutation belongs in the
    /// Service process; the app only sends the intent.
    #[serde(rename = "pending_ops.cancel_for_resource")]
    PendingOpsCancelForResource {
        account_id: String,
        resource_id: String,
        operation_type: String,
    },
    /// Phase 5: "The UI's tick fired; please consider running calendar
    /// sync for any account whose `last_calendar_sync` is stale." The
    /// Service handler enumerates accounts and gates each on a 1 h
    /// staleness check before spawning a `CalendarRuntime` runner.
    /// `Drop` class - a missed kick is harmless because the next tick
    /// will re-cover.
    #[serde(rename = "calendar.kick")]
    CalendarKick,
    /// Phase 5: "The UI's tick fired; please consider refreshing GAL
    /// caches." The Service handler enumerates all accounts and calls
    /// `refresh_gal_for_account`, which self-gates on the 24 h cache
    /// age check. `Drop` class - same forgiveness as the other kicks.
    #[serde(rename = "gal.kick")]
    GalKick,
    /// Phase 6a: "The UI's tick fired; please consider expiring stale
    /// pinned searches." The Service handler runs a single global
    /// DELETE keyed on the 14-day staleness window (matches today's
    /// UI-side `expire_stale_pinned_searches(1_209_600)` call).
    /// `Drop` class - missed kicks self-heal on the next `SyncTick`,
    /// and the DELETE is idempotent so duplicate kicks are harmless.
    #[serde(rename = "pinned_search.kick")]
    PinnedSearchKick,
    /// Phase 6b: "The UI's tick fired; please consider sweeping the
    /// attachment cache." The Service handler runs a single global
    /// LRU eviction sweep gated by an in-memory `Mutex` (so a slow
    /// sweep on one tick is not re-entered when the next tick lands
    /// within `NOTIFY_CAP=4` queued kicks). The sweep drops orphans
    /// first regardless of age, then evicts in `last_accessed_at`
    /// order until the cache is under cap, with a per-kick reclaim
    /// budget so a 50 GB cache reduction does not stall one tick.
    /// `Drop` class - missed kicks self-heal on the next
    /// `SyncTick`.
    #[serde(rename = "attachment.eviction_kick")]
    AttachmentEvictionKick,
    /// Attachments roadmap Phase 3: idle cleanup of
    /// `<app_data>/attachment_fetch_tmp/`. The Service handler walks
    /// the directory and unlinks entries whose mtime is older than 10
    /// minutes. `Drop` class - missed kicks self-heal on the next
    /// `SyncTick`.
    #[serde(rename = "attachment.tmp_cleanup_kick")]
    AttachmentTmpCleanupKick,
    /// Phase 7-6: "The UI's tick fired (or boot.ready just resolved);
    /// please consider scanning cached + unindexed attachments and
    /// enqueuing them into the ExtractRuntime." The Service handler
    /// SELECTs up to 1000 `attachments` rows JOINed against
    /// `attachment_blobs` (filtered on `tombstoned_at IS NULL`) where
    /// `text_indexed_at IS NULL` and enqueues each. NOT fanned out
    /// from the 5-min `Message::SyncTick` always-on path - the UI
    /// emits this once on `boot.ready` plus from a separate hourly
    /// subscription (event-driven cadence per the post-review
    /// revision). `Drop` class - missed kicks self-heal on the next
    /// hourly trigger.
    #[serde(rename = "extract.backfill_kick")]
    ExtractBackfillKick,
}

impl ClientNotification {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::PendingOpsKick => "pending_ops.kick",
            Self::PendingOpsCancelForResource { .. } => "pending_ops.cancel_for_resource",
            Self::CalendarKick => "calendar.kick",
            Self::GalKick => "gal.kick",
            Self::PinnedSearchKick => "pinned_search.kick",
            Self::AttachmentEvictionKick => "attachment.eviction_kick",
            Self::AttachmentTmpCleanupKick => "attachment.tmp_cleanup_kick",
            Self::ExtractBackfillKick => "extract.backfill_kick",
        }
    }

    pub fn params_value(&self) -> Value {
        match self {
            Self::PendingOpsCancelForResource {
                account_id,
                resource_id,
                operation_type,
            } => serde_json::json!({
                "account_id": account_id,
                "resource_id": resource_id,
                "operation_type": operation_type,
            }),
            Self::PendingOpsKick
            | Self::CalendarKick
            | Self::GalKick
            | Self::PinnedSearchKick
            | Self::AttachmentEvictionKick
            | Self::AttachmentTmpCleanupKick
            | Self::ExtractBackfillKick => Value::Null,
        }
    }

    /// Class controls Service-side dispatch behavior. Phase 2 only
    /// uses `Drop` (best-effort fire-and-forget): if the notification
    /// task pool is at capacity, drop the inbound rather than block
    /// the dispatch loop. Phase 5's `calendar.kick` and `gal.kick` and
    /// Phase 6a's `pinned_search.kick` follow the same shape - missed
    /// kicks self-heal on the next `Message::SyncTick`.
    pub fn class(&self) -> NotificationClass {
        match self {
            Self::PendingOpsKick
            | Self::PendingOpsCancelForResource { .. }
            | Self::CalendarKick
            | Self::GalKick
            | Self::PinnedSearchKick
            | Self::AttachmentEvictionKick
            | Self::AttachmentTmpCleanupKick
            | Self::ExtractBackfillKick => NotificationClass::Drop,
        }
    }

    pub fn from_method_params(method: &str, params: &Option<Value>) -> Result<Self, String> {
        match method {
            "pending_ops.kick" => match params {
                None | Some(Value::Null) => Ok(Self::PendingOpsKick),
                Some(_) => Err("pending_ops.kick must have no params".to_string()),
            },
            "pending_ops.cancel_for_resource" => {
                #[derive(Deserialize)]
                struct CancelParams {
                    account_id: String,
                    resource_id: String,
                    operation_type: String,
                }
                let params = params
                    .clone()
                    .ok_or_else(|| "pending_ops.cancel_for_resource requires params".to_string())?;
                let params: CancelParams = serde_json::from_value(params)
                    .map_err(|e| format!("pending_ops.cancel_for_resource params: {e}"))?;
                Ok(Self::PendingOpsCancelForResource {
                    account_id: params.account_id,
                    resource_id: params.resource_id,
                    operation_type: params.operation_type,
                })
            }
            "calendar.kick" => match params {
                None | Some(Value::Null) => Ok(Self::CalendarKick),
                Some(_) => Err("calendar.kick must have no params".to_string()),
            },
            "gal.kick" => match params {
                None | Some(Value::Null) => Ok(Self::GalKick),
                Some(_) => Err("gal.kick must have no params".to_string()),
            },
            "pinned_search.kick" => match params {
                None | Some(Value::Null) => Ok(Self::PinnedSearchKick),
                Some(_) => Err("pinned_search.kick must have no params".to_string()),
            },
            "attachment.eviction_kick" => match params {
                None | Some(Value::Null) => Ok(Self::AttachmentEvictionKick),
                Some(_) => Err("attachment.eviction_kick must have no params".to_string()),
            },
            "attachment.tmp_cleanup_kick" => match params {
                None | Some(Value::Null) => Ok(Self::AttachmentTmpCleanupKick),
                Some(_) => Err("attachment.tmp_cleanup_kick must have no params".to_string()),
            },
            "extract.backfill_kick" => match params {
                None | Some(Value::Null) => Ok(Self::ExtractBackfillKick),
                Some(_) => Err("extract.backfill_kick must have no params".to_string()),
            },
            _ => Err(format!("unknown client notification method: {method}")),
        }
    }
}

/// Wire envelope for a UI -> Service notification.
///
/// Always `id`-less. The Service's framing parser branches on
/// `id IS NULL` to route to the notification dispatch path.
#[derive(Debug, Serialize)]
pub struct JsonRpcClientNotification {
    pub jsonrpc: &'static str,
    pub method: &'static str,
    pub params: Value,
}

impl JsonRpcClientNotification {
    pub fn new(notification: &ClientNotification) -> Self {
        Self {
            jsonrpc: "2.0",
            method: notification.method_name(),
            params: notification.params_value(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_ops_kick_round_trips_through_serde() {
        let original = ClientNotification::PendingOpsKick;
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ClientNotification = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn pending_ops_kick_method_name_is_dotted() {
        assert_eq!(
            ClientNotification::PendingOpsKick.method_name(),
            "pending_ops.kick",
        );
    }

    #[test]
    fn pending_ops_kick_classifies_as_drop() {
        assert!(matches!(
            ClientNotification::PendingOpsKick.class(),
            NotificationClass::Drop,
        ));
    }

    #[test]
    fn from_method_params_accepts_null_and_missing_params() {
        let n = ClientNotification::from_method_params("pending_ops.kick", &None)
            .expect("missing ok");
        assert_eq!(n, ClientNotification::PendingOpsKick);
        let n = ClientNotification::from_method_params("pending_ops.kick", &Some(Value::Null))
            .expect("null ok");
        assert_eq!(n, ClientNotification::PendingOpsKick);
    }

    #[test]
    fn from_method_params_rejects_non_null_params() {
        let result = ClientNotification::from_method_params(
            "pending_ops.kick",
            &Some(serde_json::json!({"foo": "bar"})),
        );
        match result {
            Err(message) => assert!(message.contains("no params")),
            Ok(other) => panic!("expected Err, got Ok({other:?})"),
        }
    }

    #[test]
    fn from_method_params_rejects_unknown_method() {
        let result = ClientNotification::from_method_params("nope.unknown", &None);
        match result {
            Err(message) => assert!(message.contains("unknown")),
            Ok(other) => panic!("expected Err, got Ok({other:?})"),
        }
    }

    // -- Phase 5 catalog cases --------------------------------------------

    #[test]
    fn calendar_kick_round_trips_through_serde() {
        let original = ClientNotification::CalendarKick;
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ClientNotification = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn calendar_kick_method_name_is_dotted() {
        assert_eq!(
            ClientNotification::CalendarKick.method_name(),
            "calendar.kick",
        );
    }

    #[test]
    fn calendar_kick_classifies_as_drop() {
        assert!(matches!(
            ClientNotification::CalendarKick.class(),
            NotificationClass::Drop,
        ));
    }

    #[test]
    fn calendar_kick_from_method_params_accepts_null_and_missing() {
        let n = ClientNotification::from_method_params("calendar.kick", &None).expect("missing ok");
        assert_eq!(n, ClientNotification::CalendarKick);
        let n = ClientNotification::from_method_params("calendar.kick", &Some(Value::Null))
            .expect("null ok");
        assert_eq!(n, ClientNotification::CalendarKick);
    }

    #[test]
    fn calendar_kick_from_method_params_rejects_non_null_params() {
        let result = ClientNotification::from_method_params(
            "calendar.kick",
            &Some(serde_json::json!({"foo": "bar"})),
        );
        match result {
            Err(message) => assert!(message.contains("no params")),
            Ok(other) => panic!("expected Err, got Ok({other:?})"),
        }
    }

    #[test]
    fn gal_kick_round_trips_through_serde() {
        let original = ClientNotification::GalKick;
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ClientNotification = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn gal_kick_method_name_is_dotted() {
        assert_eq!(ClientNotification::GalKick.method_name(), "gal.kick",);
    }

    #[test]
    fn gal_kick_classifies_as_drop() {
        assert!(matches!(
            ClientNotification::GalKick.class(),
            NotificationClass::Drop,
        ));
    }

    #[test]
    fn gal_kick_from_method_params_accepts_null_and_missing() {
        let n = ClientNotification::from_method_params("gal.kick", &None).expect("missing ok");
        assert_eq!(n, ClientNotification::GalKick);
        let n = ClientNotification::from_method_params("gal.kick", &Some(Value::Null))
            .expect("null ok");
        assert_eq!(n, ClientNotification::GalKick);
    }

    // -- Phase 6a catalog cases -------------------------------------------

    #[test]
    fn pinned_search_kick_round_trips_through_serde() {
        let original = ClientNotification::PinnedSearchKick;
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ClientNotification = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn pinned_search_kick_method_name_is_dotted() {
        assert_eq!(
            ClientNotification::PinnedSearchKick.method_name(),
            "pinned_search.kick",
        );
    }

    #[test]
    fn pinned_search_kick_classifies_as_drop() {
        assert!(matches!(
            ClientNotification::PinnedSearchKick.class(),
            NotificationClass::Drop,
        ));
    }

    #[test]
    fn pinned_search_kick_from_method_params_accepts_null_and_missing() {
        let n = ClientNotification::from_method_params("pinned_search.kick", &None)
            .expect("missing ok");
        assert_eq!(n, ClientNotification::PinnedSearchKick);
        let n = ClientNotification::from_method_params("pinned_search.kick", &Some(Value::Null))
            .expect("null ok");
        assert_eq!(n, ClientNotification::PinnedSearchKick);
    }

    #[test]
    fn pinned_search_kick_from_method_params_rejects_non_null_params() {
        let result = ClientNotification::from_method_params(
            "pinned_search.kick",
            &Some(serde_json::json!({"foo": "bar"})),
        );
        match result {
            Err(message) => assert!(message.contains("no params")),
            Ok(other) => panic!("expected Err, got Ok({other:?})"),
        }
    }
}
