use super::context::ActionContext;
use super::outcome::ActionError;
use db::db::queries::set_thread_read;
use rusqlite::params;

/// Local DB mutation for mark-read (idempotent).
pub(crate) async fn mark_read_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    read: bool,
) -> Result<(), ActionError> {
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| {
        let tx = conn
            .transaction()
            .map_err(|e| format!("begin mark-read transaction: {e}"))?;
        set_thread_read(&tx, &aid, &tid, read)?;
        tx.execute(
            "UPDATE messages SET is_read = ?1 WHERE account_id = ?2 AND thread_id = ?3",
            params![read, aid, tid],
        )
        .map_err(|e| format!("update message read flags: {e}"))?;
        tx.commit()
            .map_err(|e| format!("commit mark-read transaction: {e}"))?;
        Ok::<(), String>(())
    })
    .await
    .map_err(ActionError::db)
}
