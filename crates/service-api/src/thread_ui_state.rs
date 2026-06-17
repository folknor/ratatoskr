//! Per-thread UI state wire types.
//!
//! Service owns writes to the `thread_ui_state` table, keyed on
//! `(account_id, thread_id)`. Today's only field is `attachments_collapsed`; the
//! IPC carries the full row so future thread-scoped UI flags get a
//! wire-shape they can extend.

use serde::{Deserialize, Serialize};

/// `thread_ui_state.set` request body.
///
/// `attachments_collapsed` is `Option<bool>` so the IPC can be extended
/// to carry partial updates if more fields land here later (today's
/// only caller always sets the field, but future fields can be `None`
/// to leave-as-is). The Service-side handler treats `Some(value)` as
/// "set to value" and `None` as "leave existing row column unchanged."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadUiStateSetParams {
    pub account_id: String,
    pub thread_id: String,
    pub attachments_collapsed: Option<bool>,
}

/// `thread_ui_state.set` ack. Empty struct; failure surfaces through
/// `ServiceResponse::Error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThreadUiStateSetAck;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thread_ui_state_set_params_round_trips_through_serde() {
        let params = ThreadUiStateSetParams {
            account_id: "acc-1".into(),
            thread_id: "thread-7".into(),
            attachments_collapsed: Some(true),
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: ThreadUiStateSetParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn thread_ui_state_set_params_round_trips_with_none() {
        let params = ThreadUiStateSetParams {
            account_id: "acc-1".into(),
            thread_id: "thread-7".into(),
            attachments_collapsed: None,
        };
        let json = serde_json::to_value(&params).expect("serialize");
        let recovered: ThreadUiStateSetParams = serde_json::from_value(json).expect("deserialize");
        assert_eq!(params, recovered);
    }

    #[test]
    fn thread_ui_state_set_ack_round_trips_through_serde() {
        let ack = ThreadUiStateSetAck;
        let json = serde_json::to_value(&ack).expect("serialize");
        let _recovered: ThreadUiStateSetAck = serde_json::from_value(json).expect("deserialize");
    }
}
