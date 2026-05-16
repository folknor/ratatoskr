//! Thread folder/label mutation helpers for the email action pipeline.

use rusqlite::{Connection, params};

/// Remove a label from a thread. Returns the number of rows affected.
pub fn remove_label(
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

/// Add a label to a thread. Returns the number of rows affected.
pub fn insert_label(
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

/// Remove a folder from a thread. Returns the number of rows affected.
pub fn remove_folder(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    folder_id: &str,
) -> Result<usize, String> {
    conn.execute(
        "DELETE FROM thread_folders WHERE account_id = ?1 AND thread_id = ?2 AND folder_id = ?3",
        params![account_id, thread_id, folder_id],
    )
    .map_err(|e| e.to_string())
}

/// Add a folder to a thread. Returns the number of rows affected.
pub fn insert_folder(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    folder_id: &str,
) -> Result<usize, String> {
    conn.execute(
        "INSERT OR IGNORE INTO thread_folders (account_id, thread_id, folder_id) VALUES (?1, ?2, ?3)",
        params![account_id, thread_id, folder_id],
    )
    .map_err(|e| e.to_string())
}

/// Remove the INBOX folder from a thread.
pub fn remove_inbox_folder(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<usize, String> {
    remove_folder(conn, account_id, thread_id, "INBOX")
}
