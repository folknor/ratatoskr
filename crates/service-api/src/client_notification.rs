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
}

impl ClientNotification {
    pub fn method_name(&self) -> &'static str {
        match self {
            Self::PendingOpsKick => "pending_ops.kick",
        }
    }

    pub fn params_value(&self) -> Value {
        match self {
            Self::PendingOpsKick => Value::Null,
        }
    }

    /// Class controls Service-side dispatch behavior. Phase 2 only
    /// uses `Drop` (best-effort fire-and-forget): if the notification
    /// task pool is at capacity, drop the inbound rather than block
    /// the dispatch loop.
    pub fn class(&self) -> NotificationClass {
        match self {
            Self::PendingOpsKick => NotificationClass::Drop,
        }
    }

    pub fn from_method_params(method: &str, params: &Option<Value>) -> Result<Self, String> {
        match method {
            "pending_ops.kick" => match params {
                None | Some(Value::Null) => Ok(Self::PendingOpsKick),
                Some(_) => Err("pending_ops.kick must have no params".to_string()),
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
}
