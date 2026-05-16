// Re-export folder mutation helpers from db.
//
// The action service moved to `service::actions` (Phase 2 task 6) and
// now imports these directly from `db::db::queries_extra`. Label-side
// helpers have been retired because the action service writes pending
// intent into `pending_thread_label_intents` instead of mutating
// `thread_labels` directly; see `crates/db/src/db/queries_extra/label_intent.rs`.
#[allow(unused_imports)]
pub(crate) use crate::db::queries_extra::{
    insert_folder, remove_folder, remove_inbox_folder,
};
