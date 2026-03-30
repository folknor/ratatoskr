use rusqlite::{Connection, params};

// ── DB helpers ────────────────────────────────────────────────

/// Remove a label from a thread. Returns the number of rows affected
/// (0 = label wasn't present, 1 = removed).
pub(crate) fn remove_label(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2 AND label_id = ?3",
        params![account_id, thread_id, label_id],
    )
    .map_err(|e| e.to_string())
}

/// Add a label to a thread. Returns the number of rows affected
/// (0 = label already present, 1 = inserted).
pub(crate) fn insert_label(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    label_id: &str,
) -> Result<usize, String> {
    conn.execute(
        "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) VALUES (?1, ?2, ?3)",
        params![account_id, thread_id, label_id],
    )
    .map_err(|e| e.to_string())
}

/// Remove the INBOX label. Returns affected rows (0 = already not in inbox).
pub(crate) fn remove_inbox_label(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<usize, String> {
    remove_label(conn, account_id, thread_id, "INBOX")
}
