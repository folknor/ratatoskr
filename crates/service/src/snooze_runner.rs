//! Snooze resurfacing runner (Phase 2 task 17).
//!
//! On each action-worker wakeup the runner walks `threads` for rows
//! whose `snooze_until` has passed, unsnoozes them via the existing
//! `snooze::unsnooze` action (local-only, mutates DB + clears snooze
//! state), and logs the count. The trigger is shared with
//! `pending_ops.kick`: the UI fires a kick on its 60s `SnoozeTick`
//! and the action worker's wakeup pass calls `drain_due_snoozes`
//! alongside the journal and pending-ops drains.
//!
//! No notification fires when threads resurface today: the UI's
//! `SnoozeTick` handler schedules a follow-up nav reload after the
//! kick so the unsnoozed thread reappears in the inbox view. A
//! Phase-3+ refinement could add a dedicated `nav.changed`
//! notification that closes the ~1 s window between the kick and
//! the reload. Acceptable for v1 - the `SnoozeTick` cadence is
//! 60 s, so a 1 s reload lag is invisible.

use super::actions::{ActionContext, ActionOutcome, unsnooze};

pub(crate) async fn drain_due_snoozes(ctx: &ActionContext) {
    let now = chrono::Utc::now().timestamp();
    let due = match db::db::queries_extra::db_get_snoozed_threads_due(&ctx.db, now).await {
        Ok(threads) => threads,
        Err(error) => {
            log::warn!("snooze runner: query failed: {error}");
            return;
        }
    };
    if due.is_empty() {
        return;
    }
    let mut count = 0usize;
    for thread in &due {
        match unsnooze(ctx, &thread.account_id, &thread.id).await {
            ActionOutcome::Success => count += 1,
            ActionOutcome::Failed { error } => {
                log::warn!(
                    "snooze runner: unsnooze {}/{} failed: {}",
                    thread.account_id,
                    thread.id,
                    error.user_message(),
                );
            }
            _ => {}
        }
    }
    if count > 0 {
        log::info!("snooze runner: unsnoozed {count} thread(s)");
    }
}
