//! Account-side writes that previously lived inline in the sync / imap
//! crates. Agent-owned scaffold for Phase 1.6 - functions get added here
//! as call sites in `crates/sync/src/state.rs`, `crates/sync/src/pipeline.rs`,
//! `crates/imap/src/imap_delta.rs`, and `crates/imap/src/imap_initial.rs`
//! are routed through `db` APIs.
//!
//! Each function takes typed writer access; callers wrap in
//! `WriteDbState::with_write(...)` if they need async dispatch.

use rusqlite::params;

use crate::db::{WriteConn, WriteTarget};

/// Mark every `local_drafts` row whose `sync_status = 'sending'` as `'failed'`.
/// Used by Phase 1.5's boot recovery to clear stale "sending" state from a
/// crashed previous Service incarnation; returns the number of rows updated.
pub fn mark_sending_drafts_failed(conn: &impl WriteTarget) -> Result<usize, String> {
    conn.execute(
        "UPDATE local_drafts SET sync_status = 'failed' WHERE sync_status = 'sending'",
        [],
    )
    .map_err(|e| format!("mark_sending_drafts_failed: {e}"))
}

/// Mark `initial_sync_completed = 1` for an account.
///
/// Used by providers (e.g. IMAP) whose delta cursor lives in a separate
/// protocol-owned cursor store.
pub fn mark_account_initial_sync_completed(
    conn: &WriteConn<'_>,
    account_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET initial_sync_completed = 1, updated_at = unixepoch() WHERE id = ?1",
        params![account_id],
    )
    .map_err(|e| format!("mark_account_initial_sync_completed: {e}"))?;
    Ok(())
}

/// Reset `initial_sync_completed = 0` for an account.
///
/// Forces the next sync cycle to run a full initial sync from scratch.
pub fn clear_account_initial_sync_completed(
    conn: &WriteConn<'_>,
    account_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET initial_sync_completed = 0, \
         updated_at = unixepoch() WHERE id = ?1",
        params![account_id],
    )
    .map_err(|e| format!("clear_account_initial_sync_completed: {e}"))?;
    Ok(())
}

/// Delete a single orphaned placeholder thread for an account.
///
/// Used during initial sync orphan cleanup to remove threads whose message IDs
/// no longer appear in any final thread group after JWZ re-threading.
pub fn delete_thread_by_account_and_id(
    conn: &WriteConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<u64, String> {
    let deleted = conn
        .execute(
            "DELETE FROM threads WHERE id = ?1 AND account_id = ?2",
            params![thread_id, account_id],
        )
        .map_err(|e| format!("delete_thread_by_account_and_id: {e}"))?;
    Ok(deleted as u64)
}

/// Delete a row from the `settings` table by key.
///
/// Returns `Ok(())` whether or not the key existed.
pub fn delete_setting(conn: &impl WriteTarget, key: &str) -> Result<(), String> {
    conn.execute("DELETE FROM settings WHERE key = ?1", params![key])
        .map_err(|e| format!("delete_setting: {e}"))?;
    Ok(())
}
