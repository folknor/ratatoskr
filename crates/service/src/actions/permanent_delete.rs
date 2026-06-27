//! Permanent delete action.
//!
//! **Phase 2 search-index contract (scope item 18b / task 16).** This
//! action does NOT touch the Tantivy index. The local DB row is
//! removed and the provider is asked to delete server-side; the
//! search-index entry for the message survives the action. The
//! cross-store invariant pass drops the orphaned doc on the next
//! sentinel-absent boot. The temporary inconsistency window is
//! intentional: relocating the Tantivy writer in lock-step with
//! actions would tangle Phase 2 with the Phase 3 sync surgery for no
//! UI-visible benefit (search readers see "the deleted message
//! disappears" once the next reload follows the next commit; in the
//! meantime, the search-result row's parent thread is gone from
//! `messages` and gets filtered out at result-render time).

use super::context::ActionContext;
use super::outcome::ActionError;
use db::db::queries::delete_thread;

/// Local DB mutation for permanent delete (idempotent).
pub(crate) async fn permanent_delete_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| delete_thread(conn, &aid, &tid))
        .await
        .map_err(ActionError::db)
}
