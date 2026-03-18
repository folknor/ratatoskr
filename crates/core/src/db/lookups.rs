use rusqlite::Connection;
use rusqlite::OptionalExtension;

pub fn get_message_ids_for_thread(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM messages WHERE thread_id = ?1 AND account_id = ?2")
        .map_err(|e| format!("prepare: {e}"))?;
    stmt.query_map(rusqlite::params![thread_id, account_id], |row| row.get("id"))
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect: {e}"))
}

pub fn get_thread_id_for_message(
    conn: &Connection,
    account_id: &str,
    message_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT thread_id FROM messages WHERE account_id = ?1 AND id = ?2",
        rusqlite::params![account_id, message_id],
        |row| row.get::<_, String>("thread_id"),
    )
    .optional()
    .map_err(|e| format!("query thread id: {e}"))
}
