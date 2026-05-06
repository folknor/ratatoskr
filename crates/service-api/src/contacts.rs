//! Contact-group write surfaces (Phase 6a).
//!
//! Two IPC methods - `contacts.group_save` and `contacts.group_delete`
//! - relocate the user-facing group CRUD writes Service-side.
//!
//! The plan's original wording split this into three methods
//! (`group_create | group_update | group_delete`) but today's
//! underlying DB function (`save_group_sync`) is a true UPSERT and
//! the UI always pre-generates ids before any save. Splitting create
//! / update on the wire would not change behavior - both paths
//! would call the same sync helper - so the wire shape collapses to
//! one `group_save` method that carries the complete row + member
//! list. Future work can split if a real semantic difference emerges
//! (e.g. server-generated ids, audit trails that want create-vs-update
//! distinction).
//!
//! Out of scope for this surface:
//!   - Contact CRUD (`save_contact` / `delete_contact`): the active
//!     code routes through the action service for provider write-back
//!     to Google/Graph/CardDAV, which is a different relocation
//!     pattern than the simple-write surfaces in 6a.
//!   - Bulk import (`execute_contact_import`): calls
//!     `Db::save_group` in a loop. Leaving UI-side until a batch IPC
//!     pattern lands; the loop's per-call overhead would otherwise
//!     dominate the import time.

use serde::{Deserialize, Serialize};

/// `contacts.group_save` request body. UPSERT semantics: if a row
/// with `id` exists, name is updated and `updated_at` advances; if
/// not, a new row is inserted. `member_emails` always replaces the
/// existing member list inside the same DB transaction (the prior
/// API has no "amend members" form, only replace-all).
///
/// The UI pre-generates `id` for new groups so the create / update
/// distinction is invisible at this boundary - both paths land here.
/// `created_at` / `updated_at` are timestamp hints from the UI; the
/// Service handler may overwrite them with the canonical
/// transaction-time value (today the helper sets `updated_at` to
/// `unixepoch()` server-side).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroupSaveParams {
    pub id: String,
    pub name: String,
    pub member_emails: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub member_count: i64,
}

/// `contacts.group_save` ack. Empty struct - failure surfaces through
/// `ServiceResponse::Error`. The wire does not return the saved row
/// because today's UI re-lists groups from the DB after every save,
/// and the ack-with-row pattern is reserved for surfaces that benefit
/// from skipping the re-list (none of the current 6a surfaces do).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroupSaveAck;

/// `contacts.group_delete` request body. Carries only the id;
/// member rows and inbound nested-group references are cleaned up
/// inside the same DB transaction by the sync helper.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroupDeleteParams {
    pub id: String,
}

/// `contacts.group_delete` ack. Empty struct; idempotent on the DB
/// side (delete-of-missing returns `Ok`), so the only failure mode
/// the UI sees is wire / DB transport failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroupDeleteAck;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_save_params_round_trip_through_serde() {
        let original = ContactGroupSaveParams {
            id: "grp-1".into(),
            name: "Friends".into(),
            member_emails: vec!["a@example.com".into(), "b@example.com".into()],
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
            member_count: 2,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ContactGroupSaveParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn group_save_ack_round_trips() {
        let original = ContactGroupSaveAck;
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ContactGroupSaveAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn group_delete_params_round_trip() {
        let original = ContactGroupDeleteParams { id: "grp-9".into() };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ContactGroupDeleteParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn group_save_params_carry_empty_member_list() {
        // A group save with zero members is legal: replaces all
        // existing members with the empty set. The wire shape must
        // round-trip an empty Vec.
        let original = ContactGroupSaveParams {
            id: "grp-1".into(),
            name: "Empty".into(),
            member_emails: Vec::new(),
            created_at: 0,
            updated_at: 0,
            member_count: 0,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ContactGroupSaveParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
        assert!(recovered.member_emails.is_empty());
    }
}
