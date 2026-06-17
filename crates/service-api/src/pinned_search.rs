//! Pinned-search write surfaces (Phase 6a-part-2).
//!
//! Four IPC methods - `pinned_search.create_or_update`,
//! `pinned_search.update`, `pinned_search.delete`,
//! `pinned_search.delete_all` - relocate the per-row pinned-search
//! writes Service-side. The expire-stale cadence (`pinned_search.kick`)
//! shipped earlier in 6a; this module covers the user-facing CRUD that
//! the search UI fires on snapshot persist / refresh / dismiss / clear.
//!
//! `create_or_update` and `update` are separate methods because their
//! semantics differ: `create_or_update` is query-keyed UPSERT (the UI
//! does not know the row id at first persist), `update` is id-keyed
//! and includes a query-conflict cleanup step (a row with the same
//! query as the updated one is deleted to preserve the UNIQUE on
//! `pinned_searches.query`). Folding them at this boundary would
//! require a sentinel for "create case" that the wire would have to
//! validate; separate methods keep each handler trivial.
//!
//! Smart folders live on a sibling table and ship their own module
//! (`smart_folder.create`).

use serde::{Deserialize, Serialize};

/// One thread reference inside a pinned-search snapshot. Today's
/// `pinned_search_threads` row is keyed `(pinned_search_id, thread_id,
/// account_id)`; the wire shape carries `(thread_id, account_id)` and
/// the Service handler joins it with the search id to populate the row.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedThreadRef {
    pub thread_id: String,
    pub account_id: String,
}

/// `pinned_search.create_or_update` request body. Query-keyed UPSERT:
/// if a row with this `query` already exists, its `updated_at` and
/// `scope_account_id` advance and its `pinned_search_threads` are
/// replaced; otherwise a new row is inserted.
///
/// The UI fires this when a search resolution carries
/// `SearchPersistenceBehavior::CreatePinnedSnapshot`. The first save
/// of a freshly-typed query takes the create path; subsequent re-pins
/// of the same query take the update path of this same UPSERT.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSearchCreateOrUpdateParams {
    pub query: String,
    pub thread_ids: Vec<PinnedThreadRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_account_id: Option<String>,
}

/// `pinned_search.create_or_update` ack. Carries the row id so the UI
/// can correlate the persist back to the in-memory snapshot for the
/// active search session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSearchCreateOrUpdateAck {
    pub id: i64,
}

/// `pinned_search.update` request body. Id-keyed UPDATE with a
/// query-conflict cleanup step inside the same DB transaction: if
/// another row already has the new `query`, that conflicting row is
/// deleted before this row's query is updated, preserving the UNIQUE
/// constraint on `pinned_searches.query`.
///
/// Used by the UI's `UpdatePinnedSnapshot` and `RefreshPinnedSnapshot`
/// search-persistence behaviors. Both paths re-run the query and
/// replace the snapshot; the difference is whether the row already
/// existed in the sidebar or was just promoted.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSearchUpdateParams {
    pub id: i64,
    pub query: String,
    pub thread_ids: Vec<PinnedThreadRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_account_id: Option<String>,
}

/// `pinned_search.update` ack. Empty - the id is already known to
/// the caller; failure surfaces through `ServiceResponse::Error`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSearchUpdateAck;

/// `pinned_search.delete` request body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSearchDeleteParams {
    pub id: i64,
}

/// `pinned_search.delete` ack. Empty - DELETE is idempotent on the DB
/// side (delete-of-missing returns `Ok`), so the only failure mode
/// the UI sees is wire / DB transport failure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSearchDeleteAck;

/// `pinned_search.delete_all` request body. No fields - the table is
/// global. Named struct (not unit) so a future scoped variant
/// (`delete_all_for_account`) extends the same wire shape.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSearchDeleteAllParams;

/// `pinned_search.delete_all` ack. Carries the row count for symmetry
/// with the underlying `db_delete_all_pinned_searches_sync` return.
/// The UI does not act on the count today (logs only); no-rollback
/// policy on failure is documented at the call site.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PinnedSearchDeleteAllAck {
    pub deleted: u64,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_or_update_params_round_trip_through_serde() {
        let original = PinnedSearchCreateOrUpdateParams {
            query: "from:atle".to_string(),
            thread_ids: vec![
                PinnedThreadRef {
                    thread_id: "t1".into(),
                    account_id: "acc-1".into(),
                },
                PinnedThreadRef {
                    thread_id: "t2".into(),
                    account_id: "acc-2".into(),
                },
            ],
            scope_account_id: Some("acc-1".into()),
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: PinnedSearchCreateOrUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn create_or_update_params_skip_none_scope() {
        let p = PinnedSearchCreateOrUpdateParams {
            query: "x".into(),
            thread_ids: Vec::new(),
            scope_account_id: None,
        };
        let json = serde_json::to_value(&p).expect("serialize");
        let obj = json.as_object().expect("object");
        assert!(
            !obj.contains_key("scope_account_id"),
            "scope_account_id=None must be omitted on the wire so an \
             AllAccounts snapshot serializes minimally"
        );
    }

    #[test]
    fn create_or_update_ack_round_trips() {
        let original = PinnedSearchCreateOrUpdateAck { id: 42 };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: PinnedSearchCreateOrUpdateAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn update_params_round_trip() {
        let original = PinnedSearchUpdateParams {
            id: 7,
            query: "in:inbox is:unread".into(),
            thread_ids: vec![PinnedThreadRef {
                thread_id: "t9".into(),
                account_id: "acc-1".into(),
            }],
            scope_account_id: None,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: PinnedSearchUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn update_ack_round_trips() {
        let original = PinnedSearchUpdateAck;
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: PinnedSearchUpdateAck = serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn delete_params_round_trip() {
        let original = PinnedSearchDeleteParams { id: 13 };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: PinnedSearchDeleteParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn delete_all_params_round_trip() {
        let original = PinnedSearchDeleteAllParams;
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: PinnedSearchDeleteAllParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn delete_all_ack_round_trips() {
        let original = PinnedSearchDeleteAllAck { deleted: 5 };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: PinnedSearchDeleteAllAck =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
    }

    #[test]
    fn create_or_update_params_carry_empty_thread_list() {
        // Snapshot with zero hits is legal: e.g. user pins a search
        // that ran against an empty inbox. The Vec must round-trip.
        let original = PinnedSearchCreateOrUpdateParams {
            query: "label:nonsense".into(),
            thread_ids: Vec::new(),
            scope_account_id: None,
        };
        let json = serde_json::to_value(&original).expect("serialize");
        let recovered: PinnedSearchCreateOrUpdateParams =
            serde_json::from_value(json).expect("deserialize");
        assert_eq!(original, recovered);
        assert!(recovered.thread_ids.is_empty());
    }
}
