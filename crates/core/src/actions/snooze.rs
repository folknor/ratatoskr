use super::context::ActionContext;
use super::log::MutationLog;
use super::outcome::{ActionError, ActionOutcome};

/// Snooze a single thread: remove from inbox, set snooze timestamp.
/// Local-only by design — no provider has a universal snooze equivalent.
pub async fn snooze(
    ctx: &ActionContext,
    account_id: &str,
    thread_id: &str,
    until: i64,
) -> ActionOutcome {
    let mlog = MutationLog::begin("snooze", account_id, thread_id);

    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        crate::email_actions::remove_label(&conn, &aid, &tid, "INBOX")?;
        crate::db::queries_extra::action_helpers::snooze_thread_sync(&conn, &aid, &tid, until)?;
        Ok(())
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r: Result<(), String>| r.map_err(ActionError::db));

    let outcome = match local_result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => ActionOutcome::Failed { error: e },
    };
    mlog.emit(&outcome);
    outcome
}

/// Unsnooze a single thread: restore to inbox, clear snooze state.
/// Local-only by design.
pub async fn unsnooze(ctx: &ActionContext, account_id: &str, thread_id: &str) -> ActionOutcome {
    let mlog = MutationLog::begin("unsnooze", account_id, thread_id);

    let db = ctx.db.clone();
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let local_result = tokio::task::spawn_blocking(move || {
        let conn = db.conn();
        let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
        crate::email_actions::insert_label(&conn, &aid, &tid, "INBOX")?;
        crate::db::queries_extra::action_helpers::unsnooze_thread_sync(&conn, &aid, &tid)?;
        Ok(())
    })
    .await
    .map_err(|e| ActionError::db(format!("spawn_blocking: {e}")))
    .and_then(|r: Result<(), String>| r.map_err(ActionError::db));

    let outcome = match local_result {
        Ok(()) => ActionOutcome::Success,
        Err(e) => ActionOutcome::Failed { error: e },
    };
    mlog.emit(&outcome);
    outcome
}
