// Re-export label-mutation helpers from db.
//
// The action service moved to `service::actions` (Phase 2 task 6) and
// now imports these directly from `db::db::queries_extra`. Other core
// callers may still go through this module path, so the re-export
// stays. `#[allow(unused_imports)]` because all current consumers
// happen to use the direct path - dropping this file entirely would
// also be valid; kept as a phased migration aid.
#[allow(unused_imports)]
pub(crate) use crate::db::queries_extra::{insert_label, remove_inbox_label, remove_label};
