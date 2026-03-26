use std::collections::HashSet;

use rusqlite::Transaction;

use ratatoskr_db::db::lookups;
use ratatoskr_stores::body_store::{BodyStoreState, MessageBody};
use ratatoskr_stores::inline_image_store::{InlineImage, InlineImageStoreState};
use ratatoskr_search::{SearchDocument, SearchState};

pub struct ThreadAggregate {
    pub subject: Option<String>,
    pub snippet: String,
    pub last_date: i64,
    pub message_count: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
}

pub fn compute_thread_aggregate(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<ThreadAggregate, String> {
    // Exclude reaction-only messages (is_reaction = 1) from thread aggregates
    // so emoji reactions don't inflate counts or override snippets.
    let message_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get("cnt"),
        )
        .map_err(|e| format!("count messages: {e}"))?;

    let is_read: bool = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_read = 0 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map(|unread| unread == 0)
        .map_err(|e| format!("check is_read: {e}"))?;

    let is_starred: bool = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_starred = 1 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map(|starred| starred > 0)
        .map_err(|e| format!("check is_starred: {e}"))?;

    let has_attachments: bool = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM attachments a \
             JOIN messages m ON a.message_id = m.id \
             WHERE m.thread_id = ?1 AND m.account_id = ?2 AND m.is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map(|count| count > 0)
        .map_err(|e| format!("check has_attachments: {e}"))?;

    let (snippet, last_date): (String, i64) = tx
        .query_row(
            "SELECT COALESCE(snippet, '') AS snippet, date FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0 \
             ORDER BY date DESC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| Ok((row.get("snippet")?, row.get("date")?)),
        )
        .map_err(|e| format!("get latest message: {e}"))?;

    let subject: Option<String> = tx
        .query_row(
            "SELECT subject FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0 \
             ORDER BY date ASC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get("subject"),
        )
        .map_err(|e| format!("get subject: {e}"))?;

    Ok(ThreadAggregate {
        subject,
        snippet,
        last_date,
        message_count,
        is_read,
        is_starred,
        has_attachments,
    })
}

pub fn upsert_thread_aggregate(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    aggregate: &ThreadAggregate,
    is_important: Option<bool>,
    shared_mailbox_id: Option<&str>,
) -> Result<(), String> {
    let exists: bool = tx
        .query_row(
            "SELECT COUNT(*) AS cnt FROM threads WHERE id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>("cnt"),
        )
        .map(|c| c > 0)
        .map_err(|e| format!("check thread exists: {e}"))?;

    if exists {
        // Use COALESCE so that NULL shared_mailbox_id param preserves the
        // existing value — important for re-aggregation paths that don't
        // know the mailbox context.
        match is_important {
            Some(is_important) => {
                tx.execute(
                    "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
                     message_count = ?4, is_read = ?5, is_starred = ?6, is_important = ?7, \
                     has_attachments = ?8, \
                     shared_mailbox_id = COALESCE(?11, shared_mailbox_id) \
                     WHERE id = ?9 AND account_id = ?10",
                    rusqlite::params![
                        aggregate.subject,
                        aggregate.snippet,
                        aggregate.last_date,
                        aggregate.message_count,
                        aggregate.is_read,
                        aggregate.is_starred,
                        is_important,
                        aggregate.has_attachments,
                        thread_id,
                        account_id,
                        shared_mailbox_id,
                    ],
                )
                .map_err(|e| format!("update thread: {e}"))?;
            }
            None => {
                tx.execute(
                    "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
                     message_count = ?4, is_read = ?5, is_starred = ?6, \
                     has_attachments = ?7, \
                     shared_mailbox_id = COALESCE(?10, shared_mailbox_id) \
                     WHERE id = ?8 AND account_id = ?9",
                    rusqlite::params![
                        aggregate.subject,
                        aggregate.snippet,
                        aggregate.last_date,
                        aggregate.message_count,
                        aggregate.is_read,
                        aggregate.is_starred,
                        aggregate.has_attachments,
                        thread_id,
                        account_id,
                        shared_mailbox_id,
                    ],
                )
                .map_err(|e| format!("update thread: {e}"))?;
            }
        }
    } else {
        tx.execute(
            "INSERT INTO threads \
             (id, account_id, subject, snippet, last_message_at, message_count, \
              is_read, is_starred, is_important, has_attachments, shared_mailbox_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![
                thread_id,
                account_id,
                aggregate.subject,
                aggregate.snippet,
                aggregate.last_date,
                aggregate.message_count,
                aggregate.is_read,
                aggregate.is_starred,
                is_important.unwrap_or(false),
                aggregate.has_attachments,
                shared_mailbox_id,
            ],
        )
        .map_err(|e| format!("insert thread: {e}"))?;
    }

    Ok(())
}

/// Delete messages from the `messages` table and clean up orphaned threads.
///
/// For each deleted message, looks up its parent thread. After deletion:
/// Populate the `thread_participants` table from a message's address fields.
///
/// Extracts unique lowercase email addresses from from_address, to_addresses,
/// cc_addresses, and bcc_addresses. Uses INSERT OR IGNORE so duplicates are
/// harmless.
pub fn upsert_thread_participants(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    from_address: Option<&str>,
    to_addresses: Option<&str>,
    cc_addresses: Option<&str>,
    bcc_addresses: Option<&str>,
) -> Result<(), String> {
    let mut emails = HashSet::new();

    // from_address is a single email (possibly with display name)
    if let Some(from) = from_address {
        let parsed = ratatoskr_seen_addresses::parse::parse_address_list(from);
        for (_, email) in parsed {
            emails.insert(email.to_lowercase());
        }
    }

    for field in [to_addresses, cc_addresses, bcc_addresses].into_iter().flatten() {
        let parsed = ratatoskr_seen_addresses::parse::parse_address_list(field);
        for (_, email) in parsed {
            emails.insert(email.to_lowercase());
        }
    }

    for email in &emails {
        tx.execute(
            "INSERT OR IGNORE INTO thread_participants (account_id, thread_id, email) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, email],
        )
        .map_err(|e| format!("insert thread_participant: {e}"))?;
    }

    Ok(())
}

/// Update `is_chat_thread` flag and chat contact summary after thread
/// participants change. Called from sync paths after `upsert_thread_participants`.
///
/// `user_emails` should be all email addresses across all user accounts,
/// lowercased.
pub fn maybe_update_chat_state(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    // Resolve all user email addresses from accounts table
    let mut email_stmt = tx
        .prepare("SELECT email FROM accounts")
        .map_err(|e| format!("prepare user emails: {e}"))?;
    let user_emails: Vec<String> = email_stmt
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query user emails: {e}"))?
        .filter_map(Result::ok)
        .map(|e| e.to_lowercase())
        .collect();
    drop(email_stmt);
    // Check if any participant in this thread is a designated chat contact
    let chat_email: Option<String> = tx
        .query_row(
            "SELECT cc.email FROM chat_contacts cc \
             INNER JOIN thread_participants tp ON LOWER(tp.email) = cc.email \
             WHERE tp.account_id = ?1 AND tp.thread_id = ?2 \
             LIMIT 1",
            rusqlite::params![account_id, thread_id],
            |row| row.get(0),
        )
        .ok();

    let Some(ref chat_email) = chat_email else {
        // No chat contact in this thread — ensure flag is cleared
        tx.execute(
            "UPDATE threads SET is_chat_thread = 0 \
             WHERE account_id = ?1 AND id = ?2 AND is_chat_thread = 1",
            rusqlite::params![account_id, thread_id],
        )
        .map_err(|e| format!("clear chat flag: {e}"))?;
        return Ok(());
    };

    // Count distinct participants
    let participant_count: i64 = tx
        .query_row(
            "SELECT COUNT(DISTINCT LOWER(email)) FROM thread_participants \
             WHERE account_id = ?1 AND thread_id = ?2",
            rusqlite::params![account_id, thread_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("count participants: {e}"))?;

    // Check if one participant is the user
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

    let is_chat = participant_count == 2 && has_user;

    tx.execute(
        "UPDATE threads SET is_chat_thread = ?3 \
         WHERE account_id = ?1 AND id = ?2 AND is_chat_thread != ?3",
        rusqlite::params![account_id, thread_id, i32::from(is_chat)],
    )
    .map_err(|e| format!("update chat flag: {e}"))?;

    // Update summary if this is a chat thread
    if is_chat {
        // Latest message (from either direction)
        let latest: Option<(Option<String>, i64)> = tx
            .query_row(
                "SELECT m.snippet, m.date FROM messages m \
                 INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id \
                 INNER JOIN thread_participants tp \
                   ON tp.account_id = m.account_id AND tp.thread_id = m.thread_id \
                 WHERE t.is_chat_thread = 1 AND LOWER(tp.email) = ?1 \
                 ORDER BY m.date DESC LIMIT 1",
                rusqlite::params![chat_email],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .ok();

        let unread: i64 = tx
            .query_row(
                "SELECT COUNT(*) FROM messages m \
                 INNER JOIN threads t ON m.thread_id = t.id AND m.account_id = t.account_id \
                 WHERE t.is_chat_thread = 1 AND m.is_read = 0 \
                   AND LOWER(m.from_address) = ?1",
                rusqlite::params![chat_email],
                |row| row.get(0),
            )
            .unwrap_or(0);

        if let Some((preview, ts)) = latest {
            let _ = tx.execute(
                "UPDATE chat_contacts SET latest_message_preview = ?2, \
                 latest_message_at = ?3, unread_count = ?4 WHERE email = ?1",
                rusqlite::params![chat_email, preview, ts, unread],
            );
        }
    }

    Ok(())
}

/// - Orphan threads (0 remaining messages) are removed along with their labels.
/// - Surviving threads are reaggregated from their remaining messages.
///
/// Returns the set of affected thread IDs (useful for UI refresh).
///
/// **Must be called inside a transaction** — the caller owns the transaction
/// boundary so it can combine this with other writes (body store, search, etc.).
pub fn delete_messages_and_cleanup_threads(
    tx: &Transaction,
    account_id: &str,
    message_ids: &[impl AsRef<str>],
) -> Result<Vec<String>, String> {
    if message_ids.is_empty() {
        return Ok(vec![]);
    }

    log::debug!(
        "Deleting {} messages and cleaning up threads for account {}",
        message_ids.len(),
        account_id
    );

    // Collect affected thread IDs before deleting
    let mut affected_threads: HashSet<String> = HashSet::new();
    for id in message_ids {
        if let Ok(Some(tid)) = lookups::get_thread_id_for_message(tx, account_id, id.as_ref()) {
            affected_threads.insert(tid);
        }
    }

    // Delete the messages
    for id in message_ids {
        tx.execute(
            "DELETE FROM messages WHERE account_id = ?1 AND id = ?2",
            rusqlite::params![account_id, id.as_ref()],
        )
        .map_err(|e| format!("delete message: {e}"))?;
    }

    // Update or remove affected threads
    for tid in &affected_threads {
        let remaining: i64 = tx
            .query_row(
                "SELECT COUNT(*) AS cnt FROM messages WHERE thread_id = ?1 AND account_id = ?2",
                rusqlite::params![tid, account_id],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("count remaining: {e}"))?;

        if remaining == 0 {
            // Orphan thread — remove it and its labels
            tx.execute(
                "DELETE FROM threads WHERE id = ?1 AND account_id = ?2",
                rusqlite::params![tid, account_id],
            )
            .map_err(|e| format!("delete orphan thread: {e}"))?;
            tx.execute(
                "DELETE FROM thread_labels WHERE thread_id = ?1 AND account_id = ?2",
                rusqlite::params![tid, account_id],
            )
            .map_err(|e| format!("delete orphan thread labels: {e}"))?;
        } else {
            // Re-aggregate thread fields from remaining messages
            let aggregate = compute_thread_aggregate(tx, account_id, tid)?;
            upsert_thread_aggregate(tx, account_id, tid, &aggregate, None, None)?;
        }
    }

    Ok(affected_threads.into_iter().collect())
}

pub fn replace_thread_labels<'a>(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    labels: impl IntoIterator<Item = &'a str>,
) -> Result<(), String> {
    let unique_labels: HashSet<&str> = labels.into_iter().collect();

    tx.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
        rusqlite::params![account_id, thread_id],
    )
    .map_err(|e| format!("delete thread labels: {e}"))?;

    for label_id in unique_labels {
        tx.execute(
            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, label_id],
        )
        .map_err(|e| format!("insert thread label: {e}"))?;
    }

    Ok(())
}

pub async fn store_message_bodies<T, FId, FHtml, FText>(
    body_store: &BodyStoreState,
    messages: &[T],
    provider_name: &str,
    id_of: FId,
    html_of: FHtml,
    text_of: FText,
) where
    FId: Fn(&T) -> &str,
    FHtml: Fn(&T) -> Option<&String>,
    FText: Fn(&T) -> Option<&String>,
{
    let bodies: Vec<MessageBody> = messages
        .iter()
        .filter(|message| html_of(message).is_some() || text_of(message).is_some())
        .map(|message| MessageBody {
            message_id: id_of(message).to_string(),
            body_html: html_of(message).cloned(),
            body_text: text_of(message).cloned(),
        })
        .collect();

    if bodies.is_empty() {
        return;
    }

    log::debug!("Storing {} message bodies for {}", bodies.len(), provider_name);
    if let Err(error) = body_store.put_batch(bodies).await {
        log::warn!("Failed to store {provider_name} bodies: {error}");
    }
}

pub async fn store_inline_images(
    inline_images: &InlineImageStoreState,
    images: Vec<InlineImage>,
    provider_name: &str,
) {
    if images.is_empty() {
        return;
    }

    log::debug!("Storing inline images for {provider_name}");
    if let Err(error) = inline_images.put_batch(images).await {
        log::warn!("Failed to store {provider_name} inline images: {error}");
    }
}

pub async fn index_search_documents(
    search: &SearchState,
    documents: Vec<SearchDocument>,
    provider_name: &str,
) {
    log::debug!("Indexing {} search documents for {}", documents.len(), provider_name);
    if let Err(error) = search.index_messages_batch(&documents).await {
        log::warn!("Failed to index {provider_name} messages: {error}");
    }
}
