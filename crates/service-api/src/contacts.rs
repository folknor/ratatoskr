//! Contact write surfaces.
//!
//! Five IPC methods cover the user-facing contact + group writes:
//!
//! - **`contacts.group_save`** / **`contacts.group_delete`** (Phase 6a):
//!   user-facing group CRUD. UPSERT semantics; ids pre-generated UI-side.
//!
//! - **`contacts.contact_save`** (Phase 6a-part-2): local-only contact
//!   UPSERT. Used by the bulk-import path - import is high-volume and
//!   provider write-back per row would be O(N) HTTPS round-trips.
//!
//! - **`contacts.contact_save_with_writeback`** (Phase 6d-A): full
//!   single-contact save pipeline including provider write-back to
//!   JMAP / Google People / Graph for synced contacts. Replaces the
//!   pre-6d `service::actions::contacts::save_contact` UI-side call
//!   that ran through the `action_ctx` field. The local UPSERT runs
//!   first; provider failure surfaces as `WritebackOutcome::LocalOnly`
//!   (contact stays in the local DB; user-visible state is degraded
//!   but not lost). CardDAV remains stubbed.
//!
//! - **`contacts.contact_delete`** (Phase 6d-A): full single-contact
//!   delete pipeline. Provider-first for synced JMAP / Google / Graph
//!   contacts (matches the pre-6d UI-side behavior); on provider
//!   failure the local row is preserved and the wire returns an
//!   error. CardDAV stub returns `LocalOnly`. Local-only contacts
//!   delete locally and return `Success`.
//!
//! The plan's original wording split groups into three methods
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
//! Out of scope:
//!   - Bulk import (`execute_contact_import`): calls
//!     `contacts.contact_save` (local-only) in a loop. Leaving the
//!     per-row IPC shape; a batch IPC is its own follow-up if perf
//!     pressure surfaces. Provider write-back for imported contacts
//!     is a future Settings affordance ("sync uploaded contacts to
//!     provider"), not an implicit side effect of import.

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

/// `contacts.contact_save` request body. UPSERT on `contacts` keyed
/// on `id` - UI / import path always pre-generates the id, so the
/// underlying sync helper (`save_contact_sync`) is a true UPSERT and
/// the IPC is identical for create + update. Used by both the UI
/// single-contact save handler and the bulk-import path; the wire
/// shape is one row per call (the import path issues N calls). Per-
/// row IPC keeps the wire envelope shape simple and lets the import
/// loop log + continue on individual failures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactSaveParams {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub account_id: Option<String>,
    pub account_color: Option<String>,
    pub groups: Vec<String>,
    pub source: Option<String>,
    pub server_id: Option<String>,
}

/// `contacts.contact_save` ack. Empty struct - same rationale as
/// `ContactGroupSaveAck`: today's UI re-lists contacts after every
/// save, the ack-with-row pattern is reserved for surfaces that
/// benefit (none current 6a).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactSaveAck;

/// `contacts.group_delete` ack. Empty struct; idempotent on the DB
/// side (delete-of-missing returns `Ok`), so the only failure mode
/// the UI sees is wire / DB transport failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroupDeleteAck;

/// Outcome of the provider-side leg of a contact save / delete.
///
/// Used by `contacts.contact_save_with_writeback` and
/// `contacts.contact_delete` (Phase 6d-A). The local DB leg of each
/// pipeline either succeeded (and we're reporting on the provider
/// leg) or surfaced as `ServiceError` and the caller never sees a
/// `WritebackOutcome` at all.
///
/// `Success` means the provider acknowledged the change. `LocalOnly`
/// means the local DB write committed but the provider call failed
/// (or was skipped: missing `account_id` / `server_id` for a synced
/// contact, or CardDAV which is still stubbed). The `reason` carries
/// a wire-friendly message - `ActionError` (in `action-types`) is not
/// serde and never crosses the boundary.
///
/// `retryable` is preserved from `ActionOutcome::LocalOnly` so the UI
/// can decide whether to surface "retry" affordances later. Today's
/// UI does not branch on it, but the field is part of the wire shape
/// for forward-compatibility - shrinking it would be a breaking
/// change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WritebackOutcome {
    Success,
    LocalOnly { reason: String, retryable: bool },
}

/// `contacts.contact_save_with_writeback` ack. Carries the
/// provider-leg outcome; the local DB leg always succeeded by the
/// time this ack is constructed (a local-leg failure surfaces as
/// `ServiceError` instead).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactSaveWithWritebackAck {
    pub writeback: WritebackOutcome,
}

/// `contacts.contact_delete` request body. Carries only the contact
/// id; account / server identity is looked up from the contact row
/// inside the handler (matches the pre-6d
/// `service::actions::contacts::delete_contact` shape).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactDeleteParams {
    pub id: String,
}

/// `contacts.contact_delete` ack. Same shape as
/// `ContactSaveWithWritebackAck`: `Success` means provider + local
/// both committed; `LocalOnly` means the local row was deleted but
/// the provider call failed or was skipped (CardDAV stub, or a
/// non-synced contact).
///
/// Provider-first failures for JMAP / Google / Graph short-circuit
/// before the local delete and surface as `ServiceError`, not a
/// `LocalOnly` ack. The local row stays.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactDeleteAck {
    pub writeback: WritebackOutcome,
}

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

    #[test]
    fn contact_save_params_round_trip() {
        let original = ContactSaveParams {
            id: "c-1".into(),
            email: "alice@example.com".into(),
            display_name: Some("Alice".into()),
            email2: None,
            phone: None,
            company: Some("Acme".into()),
            notes: None,
            account_id: Some("a-1".into()),
            account_color: None,
            groups: vec!["G".into()],
            source: Some("import".into()),
            server_id: None,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ContactSaveParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn writeback_outcome_success_round_trips() {
        let original = WritebackOutcome::Success;
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: WritebackOutcome =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn writeback_outcome_local_only_round_trips() {
        let original = WritebackOutcome::LocalOnly {
            reason: "provider 503".into(),
            retryable: false,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: WritebackOutcome =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn writeback_outcome_uses_tagged_kind_field() {
        // The serde tag is "kind" so the wire JSON is self-describing
        // and a future variant addition does not need a wire-format
        // bump. Pin the shape so a tag rename surfaces as a test
        // failure rather than a silent compatibility break.
        let success_json = serde_json::to_value(WritebackOutcome::Success).expect("serialize");
        assert_eq!(success_json["kind"], "success");
        let local_json = serde_json::to_value(WritebackOutcome::LocalOnly {
            reason: "x".into(),
            retryable: true,
        })
        .expect("serialize");
        assert_eq!(local_json["kind"], "local_only");
        assert_eq!(local_json["reason"], "x");
        assert_eq!(local_json["retryable"], true);
    }

    #[test]
    fn contact_save_with_writeback_ack_round_trips() {
        let original = ContactSaveWithWritebackAck {
            writeback: WritebackOutcome::LocalOnly {
                reason: "Google 429".into(),
                retryable: true,
            },
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ContactSaveWithWritebackAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn contact_delete_params_round_trip() {
        let original = ContactDeleteParams { id: "c-9".into() };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ContactDeleteParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn contact_delete_ack_round_trips() {
        let original = ContactDeleteAck {
            writeback: WritebackOutcome::Success,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: ContactDeleteAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }
}
