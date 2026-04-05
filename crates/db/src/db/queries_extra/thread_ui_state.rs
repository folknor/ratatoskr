//! Per-thread UI state persistence (attachment collapse, etc.).

use rusqlite::{Connection, OptionalExtension, params};

/// Get whether the attachment group is collapsed for a thread.
/// Returns `false` (expanded) if no row exists.
pub fn get_attachments_collapsed(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, String> {
    let result: Option<bool> = conn
        .query_row(
            "SELECT attachments_collapsed FROM thread_ui_state \
             WHERE account_id = ?1 AND thread_id = ?2",
            params![account_id, thread_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    Ok(result.unwrap_or(false))
}

/// Set the attachment group collapse state for a thread.
/// Uses INSERT OR REPLACE — creates the row on first toggle.
pub fn set_attachments_collapsed(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    collapsed: bool,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO thread_ui_state (account_id, thread_id, attachments_collapsed) \
         VALUES (?1, ?2, ?3) \
         ON CONFLICT(account_id, thread_id) DO UPDATE SET attachments_collapsed = ?3",
        params![account_id, thread_id, collapsed],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::migrations;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        migrations::run_all(&conn).expect("migrations");
        conn
    }

    #[test]
    fn default_is_expanded() {
        let conn = setup_db();
        let collapsed = get_attachments_collapsed(&conn, "acc-1", "thread-1").expect("query");
        assert!(!collapsed);
    }

    #[test]
    fn set_and_get() {
        let conn = setup_db();
        set_attachments_collapsed(&conn, "acc-1", "thread-1", true).expect("set");
        let collapsed = get_attachments_collapsed(&conn, "acc-1", "thread-1").expect("get");
        assert!(collapsed);
    }

    #[test]
    fn toggle_back() {
        let conn = setup_db();
        set_attachments_collapsed(&conn, "acc-1", "thread-1", true).expect("set true");
        set_attachments_collapsed(&conn, "acc-1", "thread-1", false).expect("set false");
        let collapsed = get_attachments_collapsed(&conn, "acc-1", "thread-1").expect("get");
        assert!(!collapsed);
    }
}
