//! Account-side writes that previously lived inline in the sync / imap
//! crates. Agent-owned scaffold for Phase 1.6 - functions get added here
//! as call sites in `crates/sync/src/state.rs`, `crates/sync/src/pipeline.rs`,
//! `crates/imap/src/imap_delta.rs`, and `crates/imap/src/imap_initial.rs`
//! are routed through `db` APIs.
//!
//! Each function takes typed writer access; callers wrap in
//! `WriteDbState::with_write(...)` if they need async dispatch.

use rusqlite::params;

use crate::db::from_row::{query_one, FromRow, QuerySource};
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

/// Update the `history_id` column on an account and mark initial sync completed.
///
/// Used by Gmail and IMAP after a successful sync pass to record the new
/// history cursor, ensuring the next delta starts from the right point.
pub fn set_account_history_id(
    conn: &WriteConn<'_>,
    account_id: &str,
    history_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET history_id = ?1, initial_sync_completed = 1 WHERE id = ?2",
        params![history_id, account_id],
    )
    .map_err(|e| format!("set_account_history_id: {e}"))?;
    Ok(())
}

/// Read the current `history_id` for an account.
///
/// Returns `Ok(None)` when no history cursor has been stored yet (pre-initial-sync).
pub fn get_account_history_id(
    conn: &(impl QuerySource + ?Sized),
    account_id: &str,
) -> Result<Option<String>, String> {
    struct HistoryRow {
        history_id: Option<String>,
    }

    impl FromRow for HistoryRow {
        fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
            Ok(Self {
                history_id: row.get("history_id")?,
            })
        }
    }

    query_one::<HistoryRow>(
        conn,
        "SELECT history_id FROM accounts WHERE id = ?1",
        &[&account_id],
    )
    .map(|row| row.and_then(|row| row.history_id))
    .map_err(|e| format!("get_account_history_id: {e}"))
}

/// Mark `initial_sync_completed = 1` for an account without changing `history_id`.
///
/// Used by providers (e.g. IMAP) whose delta cursor lives in a separate
/// protocol-owned table rather than in the `history_id` column.
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

/// Clear `history_id` and reset `initial_sync_completed = 0` for an account.
///
/// Forces the next sync cycle to run a full initial sync from scratch.
pub fn clear_account_sync_state(
    conn: &WriteConn<'_>,
    account_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET history_id = NULL, initial_sync_completed = 0, \
         updated_at = unixepoch() WHERE id = ?1",
        params![account_id],
    )
    .map_err(|e| format!("clear_account_sync_state: {e}"))?;
    Ok(())
}

/// Set the `supports_keywords` flag on an account.
///
/// Used by the IMAP provider to record whether the server advertises
/// IMAP KEYWORD capability, enabling custom flag sync.
pub fn set_account_supports_keywords(
    conn: &WriteConn<'_>,
    account_id: &str,
    supports: bool,
) -> Result<(), String> {
    let val = i64::from(supports);
    conn.execute(
        "UPDATE accounts SET supports_keywords = ?1 WHERE id = ?2",
        params![val, account_id],
    )
    .map_err(|e| format!("set_account_supports_keywords: {e}"))?;
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
    conn.execute(
        "DELETE FROM settings WHERE key = ?1",
        params![key],
    )
    .map_err(|e| format!("delete_setting: {e}"))?;
    Ok(())
}
