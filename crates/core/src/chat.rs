use ratatoskr_db::db::DbState;

/// Summary data for a chat contact in the sidebar.
#[derive(Debug, Clone)]
pub struct ChatContactSummary {
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_path: Option<String>,
    pub latest_message_preview: Option<String>,
    pub latest_message_at: Option<i64>,
    pub unread_count: i64,
    pub sort_order: i64,
}

/// A single message in a chat timeline.
#[derive(Debug, Clone)]
pub struct ChatMessage {
    pub message_id: String,
    pub account_id: String,
    pub thread_id: String,
    pub from_address: String,
    pub from_name: Option<String>,
    pub date: i64,
    pub subject: Option<String>,
    pub is_read: bool,
    pub is_from_user: bool,
}

/// Designate an email address as a chat contact.
///
/// Inserts into `chat_contacts`, scans existing threads for 1:1 eligibility,
/// sets `is_chat_thread` on qualifying threads, and computes initial summary.
pub async fn designate_chat_contact(
    db: &DbState,
    email: &str,
    user_emails: &[String],
) -> Result<(), String> {
    let email = email.to_lowercase();
    let user_emails: Vec<String> = user_emails.iter().map(|e| e.to_lowercase()).collect();

    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| format!("begin: {e}"))?;

        // Insert the chat contact
        tx.execute(
            "INSERT OR IGNORE INTO chat_contacts (email) VALUES (?1)",
            rusqlite::params![email],
        )
        .map_err(|e| format!("insert chat_contact: {e}"))?;

        // Resolve display name from contacts or seen_addresses
        let display_name: Option<String> = tx
            .query_row(
                "SELECT COALESCE(c.display_name, sa.display_name) \
                 FROM (SELECT ?1 AS email) q \
                 LEFT JOIN contacts c ON LOWER(c.email) = q.email \
                 LEFT JOIN seen_addresses sa ON LOWER(sa.email) = q.email \
                 LIMIT 1",
                rusqlite::params![email],
                |row| row.get(0),
            )
            .ok()
            .flatten();

        if let Some(ref name) = display_name {
            tx.execute(
                "UPDATE chat_contacts SET display_name = ?2 WHERE email = ?1",
                rusqlite::params![email, name],
            )
            .map_err(|e| format!("update display_name: {e}"))?;
        }

        // Find and flag qualifying 1:1 threads
        set_chat_thread_flags(&tx, &email, &user_emails)?;

        // Compute initial summary
        update_chat_summary(&tx, &email)?;

        tx.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    })
    .await
}

/// Remove chat contact designation.
///
/// Clears `is_chat_thread` on all affected threads and deletes the contact row.
pub async fn undesignate_chat_contact(
    db: &DbState,
    email: &str,
) -> Result<(), String> {
    let email = email.to_lowercase();

    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| format!("begin: {e}"))?;

        // Clear is_chat_thread on all threads involving this contact
        tx.execute(
            "UPDATE threads SET is_chat_thread = 0 \
             WHERE is_chat_thread = 1 \
               AND (account_id, id) IN ( \
                   SELECT account_id, thread_id FROM thread_participants \
                   WHERE LOWER(email) = ?1 \
               )",
            rusqlite::params![email],
        )
        .map_err(|e| format!("clear chat flags: {e}"))?;

        // Delete the chat contact row
        tx.execute(
            "DELETE FROM chat_contacts WHERE email = ?1",
            rusqlite::params![email],
        )
        .map_err(|e| format!("delete chat_contact: {e}"))?;

        tx.commit().map_err(|e| format!("commit: {e}"))?;
        Ok(())
    })
    .await
}

/// List all chat contacts with sidebar summary data.
pub async fn get_chat_contacts(
    db: &DbState,
) -> Result<Vec<ChatContactSummary>, String> {
    db.with_conn(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT cc.email, cc.display_name, cc.latest_message_at, \
                        cc.latest_message_preview, cc.unread_count, cc.sort_order, \
                        cpc.file_path \
                 FROM chat_contacts cc \
                 LEFT JOIN contact_photo_cache cpc ON LOWER(cpc.email) = cc.email \
                 ORDER BY cc.sort_order ASC",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map([], |row| {
            Ok(ChatContactSummary {
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
    })
    .await
}

/// Get the chat timeline for a contact — paginated message stream.
///
/// Returns messages across all accounts and threads, ordered chronologically
/// (oldest first). Use `before` timestamp for pagination.
pub async fn get_chat_timeline(
    db: &DbState,
    email: &str,
    user_emails: &[String],
    limit: usize,
    before: Option<i64>,
) -> Result<Vec<ChatMessage>, String> {
    let email = email.to_lowercase();
    let user_emails: Vec<String> = user_emails.iter().map(|e| e.to_lowercase()).collect();

    db.with_conn(move |conn| {
        // Step 1: Find eligible chat thread IDs
        let mut thread_stmt = conn
            .prepare(
                "SELECT DISTINCT tp.account_id, tp.thread_id \
                 FROM thread_participants tp \
                 INNER JOIN threads t ON t.id = tp.thread_id AND t.account_id = tp.account_id \
                 WHERE LOWER(tp.email) = ?1 AND t.is_chat_thread = 1",
            )
            .map_err(|e| e.to_string())?;

        let thread_ids: Vec<(String, String)> = thread_stmt
            .query_map(rusqlite::params![email], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        if thread_ids.is_empty() {
            return Ok(Vec::new());
        }

        // Step 2: Build IN clause for thread IDs
        // Use a temp approach: query messages per thread and merge
        let mut all_messages: Vec<ChatMessage> = Vec::new();

        for (account_id, thread_id) in &thread_ids {
            let sql = if before.is_some() {
                "SELECT id, account_id, thread_id, from_address, from_name, \
                        date, is_read, subject \
                 FROM messages \
                 WHERE account_id = ?1 AND thread_id = ?2 AND date < ?3 \
                 ORDER BY date ASC"
            } else {
                "SELECT id, account_id, thread_id, from_address, from_name, \
                        date, is_read, subject \
                 FROM messages \
                 WHERE account_id = ?1 AND thread_id = ?2 \
                 ORDER BY date ASC"
            };

            let mut msg_stmt = conn.prepare(sql).map_err(|e| e.to_string())?;

            let params: Vec<Box<dyn rusqlite::types::ToSql>> = if let Some(before_ts) = before {
                vec![
                    Box::new(account_id.clone()),
                    Box::new(thread_id.clone()),
                    Box::new(before_ts),
                ]
            } else {
                vec![
                    Box::new(account_id.clone()),
                    Box::new(thread_id.clone()),
                ]
            };
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params.iter().map(AsRef::as_ref).collect();
            let rows: Vec<ChatMessage> = msg_stmt
                .query_map(param_refs.as_slice(), |row| {
                    chat_message_from_row(row, &user_emails)
                })
                .map_err(|e| e.to_string())?
                .filter_map(Result::ok)
                .collect();

            all_messages.extend(rows);
        }

        // Sort all messages chronologically and apply limit (take the LAST N)
        all_messages.sort_by_key(|m| m.date);
        let total = all_messages.len();
        if total > limit {
            all_messages = all_messages.split_off(total - limit);
        }

        Ok(all_messages)
    })
    .await
}

// ── Internal helpers ──────────────────────────────────────

/// Set `is_chat_thread = 1` on all qualifying 1:1 threads for a contact.
fn set_chat_thread_flags(
    tx: &rusqlite::Transaction,
    email: &str,
    user_emails: &[String],
) -> Result<(), String> {
    // Find all threads where this contact participates
    let mut stmt = tx
        .prepare(
            "SELECT DISTINCT account_id, thread_id FROM thread_participants \
             WHERE LOWER(email) = ?1",
        )
        .map_err(|e| format!("prepare: {e}"))?;

    let thread_ids: Vec<(String, String)> = stmt
        .query_map(rusqlite::params![email], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| format!("query: {e}"))?
        .filter_map(Result::ok)
        .collect();

    for (account_id, thread_id) in &thread_ids {
        let participant_count: i64 = tx
            .query_row(
                "SELECT COUNT(DISTINCT LOWER(email)) FROM thread_participants \
                 WHERE account_id = ?1 AND thread_id = ?2",
                rusqlite::params![account_id, thread_id],
                |row| row.get(0),
            )
            .map_err(|e| format!("count: {e}"))?;

        if participant_count != 2 {
            continue;
        }

        // Verify one of the two is the user
        let has_user = user_emails.iter().any(|ue| {
            tx.query_row(
                "SELECT COUNT(*) FROM thread_participants \
                 WHERE account_id = ?1 AND thread_id = ?2 AND LOWER(email) = ?3",
                rusqlite::params![account_id, thread_id, ue],
                |row| row.get::<_, i64>(0),
            )
            .unwrap_or(0)
                > 0
        });

        if has_user {
            tx.execute(
                "UPDATE threads SET is_chat_thread = 1 \
                 WHERE account_id = ?1 AND id = ?2",
                rusqlite::params![account_id, thread_id],
            )
            .map_err(|e| format!("set chat flag: {e}"))?;
        }
    }

    Ok(())
}

/// Update the denormalized summary columns on `chat_contacts`.
fn update_chat_summary(
    tx: &rusqlite::Transaction,
    email: &str,
) -> Result<(), String> {
    // Latest message (from either direction)
    let latest: Option<(Option<String>, i64)> = tx
        .query_row(
            "SELECT m.snippet, m.date FROM messages m \
             INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id \
             INNER JOIN thread_participants tp \
               ON tp.account_id = m.account_id AND tp.thread_id = m.thread_id \
             WHERE t.is_chat_thread = 1 AND LOWER(tp.email) = ?1 \
             ORDER BY m.date DESC LIMIT 1",
            rusqlite::params![email],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .ok();

    // Unread count (messages from the contact, not from the user)
    let unread: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM messages m \
             INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id \
             WHERE t.is_chat_thread = 1 AND m.is_read = 0 \
               AND LOWER(m.from_address) = ?1",
            rusqlite::params![email],
            |row| row.get(0),
        )
        .unwrap_or(0);

    match latest {
        Some((preview, ts)) => {
            tx.execute(
                "UPDATE chat_contacts SET latest_message_preview = ?2, \
                 latest_message_at = ?3, unread_count = ?4 WHERE email = ?1",
                rusqlite::params![email, preview, ts, unread],
            )
            .map_err(|e| format!("update summary: {e}"))?;
        }
        None => {
            tx.execute(
                "UPDATE chat_contacts SET latest_message_preview = NULL, \
                 latest_message_at = NULL, unread_count = 0 WHERE email = ?1",
                rusqlite::params![email],
            )
            .map_err(|e| format!("update summary: {e}"))?;
        }
    }

    Ok(())
}

fn chat_message_from_row(
    row: &rusqlite::Row<'_>,
    user_emails: &[String],
) -> rusqlite::Result<ChatMessage> {
    let from_address: String = row.get("from_address")?;
    let is_from_user = user_emails
        .iter()
        .any(|ue| ue.eq_ignore_ascii_case(&from_address));

    Ok(ChatMessage {
        message_id: row.get("id")?,
        account_id: row.get("account_id")?,
        thread_id: row.get("thread_id")?,
        from_address,
        from_name: row.get("from_name")?,
        date: row.get("date")?,
        subject: row.get("subject")?,
        is_read: row.get::<_, i64>("is_read")? != 0,
        is_from_user,
    })
}
