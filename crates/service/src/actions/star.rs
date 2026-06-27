use super::context::ActionContext;
use super::outcome::ActionError;
use db::db::queries::{set_thread_messages_starred, set_thread_starred};

/// Local DB mutation for star. Returns true if state changed.
pub(crate) async fn star_local(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    starred: bool,
) -> Result<bool, ActionError> {
    ctx.verify_thread_exists(account_id, thread_id)?;
    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    db.with_write(move |conn| {
        let tx = conn
            .transaction()
            .map_err(|e| format!("begin star transaction: {e}"))?;
        let thread_changed = set_thread_starred(&tx, &aid, &tid, starred).map(|n| n > 0)?;
        let message_changed =
            set_thread_messages_starred(&tx, &aid, &tid, starred).map(|n| n > 0)?;
        tx.commit()
            .map_err(|e| format!("commit star transaction: {e}"))?;
        Ok(thread_changed || message_changed)
    })
    .await
    .map_err(ActionError::db)
}
