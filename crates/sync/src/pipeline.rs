use std::collections::HashSet;

use db::db::WriteConn;
use rusqlite::Connection;

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
            count += db::db::queries_extra::delete_thread_by_account_and_id(conn, account_id, msg_id)?;
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

/// Clear account history_id (forces next sync to be initial).
pub fn clear_account_history_id(conn: &WriteConn<'_>, account_id: &str) -> Result<(), String> {
    log::info!("Clearing history_id for account {account_id} (forcing initial sync)");
    db::db::queries_extra::clear_account_sync_state(conn, account_id)
}

/// Clear all folder sync states for an account (forces full folder resync).
pub fn clear_all_folder_sync_states(conn: &Connection, account_id: &str) -> Result<(), String> {
    log::info!("Clearing all folder sync states for account {account_id}");
    conn.execute(
        "DELETE FROM folder_sync_state WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("clear folder sync states: {e}"))?;
    Ok(())
}
