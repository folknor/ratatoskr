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
}

impl ClientNotification {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::PendingOpsKick => "pending_ops.kick",
            Self::CalendarKick => "calendar.kick",
            Self::GalKick => "gal.kick",
            Self::PinnedSearchKick => "pinned_search.kick",
        }
    }

    pub fn params_value(&self) -> Value {
        match self {
            Self::PendingOpsKick
            | Self::CalendarKick
            | Self::GalKick
            | Self::PinnedSearchKick => Value::Null,
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
            | Self::CalendarKick
            | Self::GalKick
            | Self::PinnedSearchKick => NotificationClass::Drop,
        }
    }

    pub fn from_method_params(method: &str, params: &Option<Value>) -> Result<Self, String> {
        match method {
            "pending_ops.kick" => match params {
                None | Some(Value::Null) => Ok(Self::PendingOpsKick),
                Some(_) => Err("pending_ops.kick must have no params".to_string()),
            },
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
