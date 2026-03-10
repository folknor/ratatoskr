pub mod commands;

use rusqlite::{params, Connection};

// ── DB helpers ────────────────────────────────────────────────

pub fn remove_label(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3",
        params![account_id, thread_id, label_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn insert_label(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) VALUES (?1, ?2, ?3)",
        params![account_id, thread_id, label_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub fn remove_inbox_label(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    remove_label(conn, account_id, thread_id, "INBOX")
}

/// Queue a pending operation for remote execution.
pub fn queue_pending_op(
    conn: &Connection,
    op_id: &str,
    account_id: &str,
    operation_type: &str,
    resource_id: &str,
    params_json: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO pending_operations (id, account_id, operation_type, resource_id, params, status)
         VALUES (?1, ?2, ?3, ?4, ?5, 'pending')",
        params![op_id, account_id, operation_type, resource_id, params_json],
    )
    .map_err(|e| format!("queue pending op: {e}"))?;
    Ok(())
}
