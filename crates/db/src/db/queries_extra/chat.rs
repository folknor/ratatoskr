//! Chat contact storage and write-side transactions.
//!
//! Read-side queries (timeline, contact summaries, inline images, signature
//! texts) live in `db-read::db::queries_extra::chat`; this module owns the
//! write paths and the helpers they need.

use crate::db::{WriteTarget, WriteTransactionTarget};
use rusqlite::params;

const CHAT_UNREAD_AFFECTED_THREADS_SQL: &str = "SELECT DISTINCT m.account_id, m.thread_id \
     FROM messages m \
     INNER JOIN threads t \
       ON t.id = m.thread_id AND t.account_id = m.account_id \
     INNER JOIN thread_participants tp \
       ON tp.account_id = m.account_id AND tp.thread_id = m.thread_id \
     WHERE t.is_chat_thread = 1 AND tp.email = ?1 AND m.is_read = 0";
const CHAT_UNREAD_RECOMPUTE_SQL: &str = "SELECT COUNT(*) FROM messages m \
     INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id \
     WHERE t.is_chat_thread = 1 AND m.is_read = 0 \
       AND LOWER(m.from_address) = ?1";

/// Insert or update a chat contact designation and recompute chat flags/summary.
pub fn designate_chat_contact_sync(
    conn: &impl WriteTransactionTarget,
    email: &str,
    user_emails: &[String],
) -> Result<(), String> {
    let tx = conn
        .transaction()
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
pub fn undesignate_chat_contact_sync(
    conn: &impl WriteTransactionTarget,
    email: &str,
) -> Result<(), String> {
    let tx = conn
        .transaction()
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

/// Mark all messages in a contact's chat threads as read in one transaction.
///
/// Flips `messages.is_read = 1` for every unread row in any thread where the
/// contact is a participant and `threads.is_chat_thread = 1`, mirrors the
/// state on `threads.is_read`, and resets the denormalised
/// `chat_contacts.unread_count`. Returns the `(account_id, thread_id)` pairs
/// that had any unread messages, so the caller can dispatch provider
/// mark-read against each of them.
pub fn mark_chat_read_local_sync(
    conn: &impl WriteTransactionTarget,
    email: &str,
) -> Result<Vec<(String, String)>, String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("begin: {e}"))?;

    let affected: Vec<(String, String)> = {
        let mut stmt = tx
            .prepare(CHAT_UNREAD_AFFECTED_THREADS_SQL)
            .map_err(|e| format!("prepare affected: {e}"))?;
        stmt.query_map(params![email], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query affected: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect affected: {e}"))?
    };

    tx.execute(
        "UPDATE messages SET is_read = 1 \
         WHERE is_read = 0 \
           AND (account_id, thread_id) IN ( \
               SELECT t.account_id, t.id FROM threads t \
               INNER JOIN thread_participants tp \
                 ON tp.account_id = t.account_id AND tp.thread_id = t.id \
               WHERE t.is_chat_thread = 1 AND tp.email = ?1 \
           )",
        params![email],
    )
    .map_err(|e| format!("update messages: {e}"))?;

    tx.execute(
        "UPDATE threads SET is_read = 1 \
         WHERE is_read = 0 AND is_chat_thread = 1 \
           AND (account_id, id) IN ( \
               SELECT tp.account_id, tp.thread_id \
               FROM thread_participants tp \
               WHERE tp.email = ?1 \
           )",
        params![email],
    )
    .map_err(|e| format!("update threads: {e}"))?;

    tx.execute(
        "UPDATE chat_contacts SET unread_count = 0 WHERE email = ?1",
        params![email],
    )
    .map_err(|e| format!("update chat_contacts: {e}"))?;

    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(affected)
}

fn set_chat_thread_flags(
    tx: &impl WriteTarget,
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

fn update_chat_summary(tx: &impl WriteTarget, email: &str) -> Result<(), String> {
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
        .query_row(CHAT_UNREAD_RECOMPUTE_SQL, params![email], |row| row.get(0))
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
