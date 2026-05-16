//! Thread folder mutation helpers for the email action pipeline.
//!
//! Label-side equivalents have been retired: the action service writes
//! pending intent into `pending_thread_label_intents` and lets the
//! provider-truth path own `thread_labels` writes. See the doc comments
//! on `crates/db/src/db/queries_extra/label_intent.rs` for the overlay
//! lifecycle.

use rusqlite::{Connection, params};

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
