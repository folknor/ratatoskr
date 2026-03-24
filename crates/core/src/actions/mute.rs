use super::context::ActionContext;
use super::outcome::{ActionError, ActionOutcome};
use crate::db::queries::set_thread_muted;

/// Set mute state on a single thread. Local-only by design — no provider
/// has a native mute equivalent.
pub async fn mute(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    muted: bool,
) -> ActionOutcome {
    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        set_thread_muted(&conn, &aid, &tid, muted)
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r| r.map_err(ActionError::db));

    match local_result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => ActionOutcome::Failed { error: e },
    }
}
