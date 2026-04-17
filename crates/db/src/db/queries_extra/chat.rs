//! Chat contact storage and timeline queries.

use rusqlite::{Connection, Transaction, params};

/// Sidebar summary row for a chat contact.
#[derive(Debug, Clone)]
pub struct DbChatContactSummary {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_path: Option<String>,
    pub latest_message_preview: Option<String>,
    pub latest_message_at: Option<i64>,
    pub unread_count: i64,
    pub sort_order: i64,
}

/// Timeline row for a chat message.
#[derive(Debug, Clone)]
pub struct DbChatMessage {
    pub message_id: String,
    pub account_id: String,
    pub thread_id: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub date: i64,
    pub subject: Option<String>,
    pub is_read: bool,
}

/// Insert or update a chat contact designation and recompute chat flags/summary.
pub fn designate_chat_contact_sync(
    conn: &Connection,
    email: &str,
    user_emails: &[String],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin: {e}"))?;

    tx.execute(
        "INSERT OR IGNORE INTO chat_contacts (email) VALUES (?1)",
        params![email],
    )
    .map_err(|e| format!("insert chat_contact: {e}"))?;

    let display_name: Option<String> = tx
        .query_row(
            "SELECT COALESCE(c.display_name, sa.display_name) \
             FROM (SELECT ?1 AS email) q \
             LEFT JOIN contacts c ON LOWER(c.email) = q.email \
             LEFT JOIN seen_addresses sa ON LOWER(sa.email) = q.email \
             LIMIT 1",
            params![email],
            |row| row.get(0),
        )
        .ok()
        .flatten();

    if let Some(ref name) = display_name {
        tx.execute(
            "UPDATE chat_contacts SET display_name = ?2 WHERE email = ?1",
            params![email, name],
        )
        .map_err(|e| format!("update display_name: {e}"))?;
    }

    set_chat_thread_flags(&tx, email, user_emails)?;
    update_chat_summary(&tx, email)?;

    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(())
}

/// Remove a chat contact designation and clear all related chat flags.
pub fn undesignate_chat_contact_sync(conn: &Connection, email: &str) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin: {e}"))?;

    tx.execute(
        "UPDATE threads SET is_chat_thread = 0 \
         WHERE is_chat_thread = 1 \
           AND (account_id, id) IN ( \
               SELECT account_id, thread_id FROM thread_participants \
               WHERE email = ?1 \
           )",
        params![email],
    )
    .map_err(|e| format!("clear chat flags: {e}"))?;

    tx.execute(
        "DELETE FROM chat_contacts WHERE email = ?1",
        params![email],
    )
    .map_err(|e| format!("delete chat_contact: {e}"))?;

    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(())
}

/// List all chat contacts with sidebar summary data.
pub fn get_chat_contacts_sync(conn: &Connection) -> Result<Vec<DbChatContactSummary>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT cc.email, cc.display_name, cc.latest_message_at, \
                    cc.latest_message_preview, cc.unread_count, cc.sort_order, \
                    (SELECT file_path FROM contact_photo_cache \
                     WHERE email = cc.email \
                     ORDER BY last_accessed_at DESC LIMIT 1) AS file_path \
             FROM chat_contacts cc \
             ORDER BY cc.sort_order ASC",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map([], |row| {
        Ok(DbChatContactSummary {
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            latest_message_at: row.get("latest_message_at")?,
            latest_message_preview: row.get("latest_message_preview")?,
            unread_count: row.get::<_, i64>("unread_count")?,
            sort_order: row.get::<_, i64>("sort_order")?,
            avatar_path: row.get("file_path")?,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Get the chat timeline rows for a contact, newest first.
pub fn get_chat_timeline_sync(
    conn: &Connection,
    email: &str,
    limit: usize,
    before: Option<(i64, String)>,
) -> Result<Vec<DbChatMessage>, String> {
    let limit_i64 = i64::try_from(limit).unwrap_or(i64::MAX);
    let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) =
        if let Some((before_ts, before_id)) = before {
            (
                "SELECT m.id, m.account_id, m.thread_id, m.from_address, m.from_name, \
                    m.date, m.is_read, m.subject \
                 FROM messages m \
                 INNER JOIN threads t ON t.id = m.thread_id AND t.account_id = m.account_id \
                 INNER JOIN thread_participants tp \
                   ON tp.account_id = m.account_id AND tp.thread_id = m.thread_id \
                 WHERE t.is_chat_thread = 1 AND tp.email = ?1 \
                   AND (m.date < ?2 OR (m.date = ?2 AND m.id < ?3)) \
                 ORDER BY m.date DESC, m.id DESC \
                 LIMIT ?4"
                    .to_string(),
                vec![
                    Box::new(email.to_string()),
                    Box::new(before_ts),
                    Box::new(before_id),
                    Box::new(limit_i64),
                ],
            )
        } else {
            (
                "SELECT m.id, m.account_id, m.thread_id, m.from_address, m.from_name, \
                    m.date, m.is_read, m.subject \
                 FROM messages m \
                 INNER JOIN threads t ON t.id = m.thread_id AND t.account_id = m.account_id \
                 INNER JOIN thread_participants tp \
                   ON tp.account_id = m.account_id AND tp.thread_id = m.thread_id \
                 WHERE t.is_chat_thread = 1 AND tp.email = ?1 \
                 ORDER BY m.date DESC, m.id DESC \
                 LIMIT ?2"
                    .to_string(),
                vec![Box::new(email.to_string()), Box::new(limit_i64)],
            )
        };

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    stmt.query_map(param_refs.as_slice(), |row| {
        let from_address = row
            .get::<_, Option<String>>("from_address")?
            .unwrap_or_default();

        Ok(DbChatMessage {
            message_id: row.get("id")?,
            account_id: row.get("account_id")?,
            thread_id: row.get("thread_id")?,
            from_address,
            from_name: row.get("from_name")?,
            date: row.get("date")?,
            subject: row.get("subject")?,
            is_read: row.get::<_, i64>("is_read")? != 0,
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

fn set_chat_thread_flags(
    tx: &Transaction<'_>,
    email: &str,
    user_emails: &[String],
) -> Result<(), String> {
    if user_emails.is_empty() {
        return Ok(());
    }

    let placeholders: Vec<String> = (0..user_emails.len())
        .map(|i| format!("?{}", i + 2))
        .collect();
    let placeholders_csv = placeholders.join(", ");

    let sql = format!(
        "UPDATE threads SET is_chat_thread = 1 \
         WHERE (account_id, id) IN ( \
             SELECT tp.account_id, tp.thread_id \
             FROM thread_participants tp \
             WHERE tp.email = ?1 \
             GROUP BY tp.account_id, tp.thread_id \
             HAVING ( \
                 SELECT COUNT(DISTINCT tp2.email) \
                 FROM thread_participants tp2 \
                 WHERE tp2.account_id = tp.account_id \
                   AND tp2.thread_id = tp.thread_id \
             ) = 2 \
             AND EXISTS ( \
                 SELECT 1 FROM thread_participants tp3 \
                 WHERE tp3.account_id = tp.account_id \
                   AND tp3.thread_id = tp.thread_id \
                   AND tp3.email IN ({placeholders_csv}) \
             ) \
         )"
    );

    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(1 + user_emails.len());
    params.push(Box::new(email.to_string()));
    for ue in user_emails {
        params.push(Box::new(ue.clone()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(AsRef::as_ref).collect();

    tx.execute(&sql, param_refs.as_slice())
        .map_err(|e| format!("set chat flags: {e}"))?;
    Ok(())
}

fn update_chat_summary(tx: &Transaction<'_>, email: &str) -> Result<(), String> {
    let latest: Option<(Option<String>, i64)> = tx
        .query_row(
            "SELECT m.snippet, m.date FROM messages m \
             INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id \
             INNER JOIN thread_participants tp \
               ON tp.account_id = m.account_id AND tp.thread_id = m.thread_id \
             WHERE t.is_chat_thread = 1 AND tp.email = ?1 \
             ORDER BY m.date DESC LIMIT 1",
            params![email],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    let unread: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM messages m \
             INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id \
             WHERE t.is_chat_thread = 1 AND m.is_read = 0 \
               AND LOWER(m.from_address) = ?1",
            params![email],
            |row| row.get(0),
        )
        .unwrap_or(0);

    match latest {
        Some((preview, ts)) => {
            tx.execute(
                "UPDATE chat_contacts SET latest_message_preview = ?2, \
                 latest_message_at = ?3, unread_count = ?4 WHERE email = ?1",
                params![email, preview, ts, unread],
            )
            .map_err(|e| format!("update summary: {e}"))?;
        }
        None => {
            tx.execute(
                "UPDATE chat_contacts SET latest_message_preview = NULL, \
                 latest_message_at = NULL, unread_count = 0 WHERE email = ?1",
                params![email],
            )
            .map_err(|e| format!("update summary: {e}"))?;
        }
    }

    Ok(())
}
