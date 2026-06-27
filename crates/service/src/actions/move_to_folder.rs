use common::typed_ids::FolderId;

use super::context::ActionContext;
use super::outcome::ActionError;
use db::db::queries_extra::{insert_folder, remove_folder};

/// Local DB mutation for move-to-folder (idempotent).
pub(crate) async fn move_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    folder_id: &FolderId,
    source_folder_id: Option<&FolderId>,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let fid = folder_id.as_str().to_string();
    let source = source_folder_id.map(|s| s.as_str().to_string());
    db.with_write(move |conn| {
        if let Some(ref src) = source {
            remove_folder(conn, &aid, &tid, src)?;
        }
        insert_folder(conn, &aid, &tid, &fid).map(|_| ())
    })
    .await
    .map_err(ActionError::db)
}
