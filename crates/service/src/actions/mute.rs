use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};
use db::db::queries::set_thread_muted;

/// Set mute state on a single thread. Local-only by design - no provider
/// has a native mute equivalent.
pub async fn mute(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    muted: bool,
) -> ActionOutcome {
    let mlog = MutationLog::begin("mute", account_id, thread_id);

    let db = ctx.write_db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = db
        .with_write(move |conn| set_thread_muted(conn, &aid, &tid, muted).map(|_| ()))
        .await
        .map_err(ActionError::db);

    let outcome = match local_result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => ActionOutcome::Failed { error: e },
    };
    mlog.emit(&outcome);
    outcome
}
