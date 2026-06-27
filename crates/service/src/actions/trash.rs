//! Trash action.
//!
//! Trash is reversible (the underlying message stays in the provider
//! Trash mailbox until it ages out), so the search-index entry stays
//! around by design while in Trash. The Phase 2 search-index contract
//! described on `permanent_delete.rs` applies to the eventual
//! permanent-delete that resolves a Trash. No action-time Tantivy
//! writes happen here either.

use super::context::ActionContext;
use super::outcome::ActionError;
use db::db::queries_extra::{insert_folder, remove_folder};

/// Local DB mutation for trash (idempotent).
pub(crate) async fn trash_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| {
        remove_folder(conn, &aid, &tid, "INBOX")?;
        insert_folder(conn, &aid, &tid, "TRASH").map(|_| ())
    })
    .await
    .map_err(ActionError::db)
}
