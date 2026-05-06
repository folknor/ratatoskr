//! Account write surfaces (Phase 6a).
//!
//! Phase 6a relocates the small / non-envelope account writes
//! (`account.update`, `account.reorder`) that don't need
//! orchestration. The bigger surfaces - `account.create` (with the
//! `Plaintext | Encrypted` credential envelope) and `account.delete`
//! (with the runner-cancel-and-await orchestration) - land as their
//! own modules so the wire shape for the cancel/delete state machine
//! does not bleed into the simple update path.
//!
//! `caldav_password` is passed through verbatim today (the column
//! stores it without encryption); when the encryption-key handle
//! bundle (`internal.encrypt_for_storage`) lands, the wire shape
//! stays unchanged but the Service handler can route the value
//! through the cipher before writing.

use serde::{Deserialize, Serialize};

/// `account.update` request body. Each `Option` field is "no change"
/// if `None`, else "set to value." Mirrors the existing
/// `UpdateAccountParams` struct from `db::queries_extra::accounts_crud`,
/// scoped to fields the settings panel exposes today (account
/// metadata + caldav credentials). Provider tokens / mailbox
/// password are deliberately not on this surface - those mutate via
/// the account-create flow and the future
/// `internal.encrypt_for_storage` IPC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUpdateParams {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caldav_password: Option<String>,
}

/// `account.update` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountUpdateAck;

/// One `(account_id, sort_order)` reassignment for the batch reorder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReorderEntry {
    pub account_id: String,
    pub sort_order: i64,
}

/// `account.reorder` request body. Account ids absent from `orders`
/// keep their existing `sort_order` - same convention as
/// `signature.reorder`. Per-account ordering hazard is the same as
/// signature reorder: rapid drag-reorder clicks can land out of
/// order; today's tolerance is "next reload reconciles."
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReorderParams {
    pub orders: Vec<AccountReorderEntry>,
}

/// `account.reorder` ack.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountReorderAck;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_update_round_trip_full() {
        let original = AccountUpdateParams {
            id: "acc-1".into(),
            account_name: Some("Work".into()),
            display_name: Some("Atle".into()),
            account_color: Some("#abcdef".into()),
            caldav_url: Some("https://example.com/dav".into()),
            caldav_username: Some("atle".into()),
            caldav_password: Some("secret".into()),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_update_round_trip_partial_skips_none() {
        let original = AccountUpdateParams {
            id: "acc-1".into(),
            account_name: None,
            display_name: Some("Atle".into()),
            account_color: None,
            caldav_url: None,
            caldav_username: None,
            caldav_password: None,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let obj = json.as_object().expect("object");
        assert!(!obj.contains_key("account_name"));
        assert!(obj.contains_key("display_name"));
        assert!(!obj.contains_key("account_color"));
        assert!(!obj.contains_key("caldav_url"));
        assert!(!obj.contains_key("caldav_username"));
        assert!(!obj.contains_key("caldav_password"));
        let recovered: AccountUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn account_reorder_round_trip() {
        let original = AccountReorderParams {
            orders: vec![
                AccountReorderEntry {
                    account_id: "a".into(),
                    sort_order: 0,
                },
                AccountReorderEntry {
                    account_id: "b".into(),
                    sort_order: 1,
                },
            ],
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: AccountReorderParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
