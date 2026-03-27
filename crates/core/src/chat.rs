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

    // Refuse to designate one of the user's own emails as a chat contact —
    // threads between two of the user's own accounts are not 1:1 chats.
    if user_emails.iter().any(|ue| ue == &email) {
        return Err(
            "Cannot designate your own email address as a chat contact".to_string(),
        );
    }

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
                   WHERE email = ?1 \
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
                 LEFT JOIN contact_photo_cache cpc ON cpc.email = cc.email \
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
    before: Option<(i64, String)>,
) -> Result<Vec<ChatMessage>, String> {
    let email = email.to_lowercase();
    let user_emails: Vec<String> = user_emails.iter().map(|e| e.to_lowercase()).collect();

    let limit_i64 = limit as i64;

    db.with_conn(move |conn| {
        // Single query: join messages against chat threads for this contact,
        // ORDER BY date DESC LIMIT N to get the most recent N messages,
        // then reverse in Rust for chronological order.
        let (sql, params): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = if let Some(
            (before_ts, before_id),
        ) = before
        {
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
                    Box::new(email.clone()),
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
                vec![Box::new(email.clone()), Box::new(limit_i64)],
            )
        };

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(AsRef::as_ref).collect();

        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let mut messages: Vec<ChatMessage> = stmt
            .query_map(param_refs.as_slice(), |row| {
                chat_message_from_row(row, &user_emails)
            })
            .map_err(|e| e.to_string())?
            .filter_map(Result::ok)
            .collect();

        // Reverse from DESC to chronological (oldest first)
        messages.reverse();

        Ok(messages)
    })
    .await
}

// ── Internal helpers ──────────────────────────────────────

/// Set `is_chat_thread = 1` on all qualifying 1:1 threads for a contact.
///
/// A qualifying thread has exactly 2 distinct participants: the contact and
/// one of the user's own email addresses. This is done in a single UPDATE
/// with a subquery to avoid N+1 queries.
fn set_chat_thread_flags(
    tx: &rusqlite::Transaction,
    email: &str,
    user_emails: &[String],
) -> Result<(), String> {
    if user_emails.is_empty() {
        return Ok(());
    }

    // Defensive: if the contact email is one of the user's own emails, no
    // thread qualifies — skip to avoid flagging inter-account threads.
    if user_emails.iter().any(|ue| ue == email) {
        return Ok(());
    }

    // Build placeholders for user_emails (?2, ?3, …)
    let placeholders: Vec<String> = (0..user_emails.len())
        .map(|i| format!("?{}", i + 2))
        .collect();
    let placeholders_csv = placeholders.join(", ");

    // Single UPDATE: find threads where the contact participates, that have
    // exactly 2 distinct participants, and where at least one participant is
    // a user email.
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

    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        Vec::with_capacity(1 + user_emails.len());
    params.push(Box::new(email.to_string()));
    for ue in user_emails {
        params.push(Box::new(ue.clone()));
    }
    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(AsRef::as_ref).collect();

    tx.execute(&sql, param_refs.as_slice())
        .map_err(|e| format!("set chat flags: {e}"))?;

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
             WHERE t.is_chat_thread = 1 AND tp.email = ?1 \
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
    let from_address: String = row.get::<_, Option<String>>("from_address")?.unwrap_or_default();
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
