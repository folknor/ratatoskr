use std::collections::HashSet;

use db::db::WriteConn;

/// Delete orphaned placeholder threads that are no longer referenced by any final thread group.
pub fn cleanup_orphan_threads(
    conn: &WriteConn<'_>,
    account_id: &str,
    all_message_ids: &HashSet<String>,
    final_thread_ids: &HashSet<String>,
) -> Result<u64, String> {
    log::debug!(
        "Cleaning up orphan threads for account {}: checking {} message IDs against {} final threads",
        account_id,
        all_message_ids.len(),
        final_thread_ids.len()
    );
    let mut count: u64 = 0;
    for msg_id in all_message_ids {
        if !final_thread_ids.contains(msg_id) {
            count +=
                db::db::queries_extra::delete_thread_by_account_and_id(conn, account_id, msg_id)?;
        }
    }
    if count > 0 {
        log::info!("Cleaned up {count} orphan threads for account {account_id}");
    }
    Ok(count)
}

/// Mark initial sync as completed for providers whose delta state is stored elsewhere.
pub fn mark_initial_sync_completed(conn: &WriteConn<'_>, account_id: &str) -> Result<(), String> {
    log::info!("Marking initial sync completed for account {account_id}");
    db::db::queries_extra::mark_account_initial_sync_completed(conn, account_id)
}

/// Reset initial-sync state so the next cycle starts from scratch.
pub fn reset_initial_sync_state(conn: &WriteConn<'_>, account_id: &str) -> Result<(), String> {
    log::info!("Resetting initial sync state for account {account_id}");
    db::db::queries_extra::clear_account_initial_sync_completed(conn, account_id)
}
