use std::collections::HashSet;

use rusqlite::Transaction;

use crate::body_store::{BodyStoreState, MessageBody};
use crate::inline_image_store::{InlineImage, InlineImageStoreState};
use crate::search::{SearchDocument, SearchState};

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
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("count messages: {e}"))?;

    let is_read: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_read = 0 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|unread| unread == 0)
        .map_err(|e| format!("check is_read: {e}"))?;

    let is_starred: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_starred = 1 AND is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|starred| starred > 0)
        .map_err(|e| format!("check is_starred: {e}"))?;

    let has_attachments: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM attachments a \
             JOIN messages m ON a.message_id = m.id \
             WHERE m.thread_id = ?1 AND m.account_id = ?2 AND m.is_reaction = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|e| format!("check has_attachments: {e}"))?;

    let (snippet, last_date): (String, i64) = tx
        .query_row(
            "SELECT COALESCE(snippet, ''), date FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0 \
             ORDER BY date DESC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("get latest message: {e}"))?;

    let subject: Option<String> = tx
        .query_row(
            "SELECT subject FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_reaction = 0 \
             ORDER BY date ASC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
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
) -> Result<(), String> {
    let exists: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .map_err(|e| format!("check thread exists: {e}"))?;

    if exists {
        match is_important {
            Some(is_important) => {
                tx.execute(
                    "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
                     message_count = ?4, is_read = ?5, is_starred = ?6, is_important = ?7, \
                     has_attachments = ?8 \
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
                    ],
                )
                .map_err(|e| format!("update thread: {e}"))?;
            }
            None => {
                tx.execute(
                    "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
                     message_count = ?4, is_read = ?5, is_starred = ?6, \
                     has_attachments = ?7 \
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
                    ],
                )
                .map_err(|e| format!("update thread: {e}"))?;
            }
        }
    } else {
        tx.execute(
            "INSERT INTO threads \
             (id, account_id, subject, snippet, last_message_at, message_count, \
              is_read, is_starred, is_important, has_attachments) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
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
            ],
        )
        .map_err(|e| format!("insert thread: {e}"))?;
    }

    Ok(())
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

    if let Err(error) = inline_images.put_batch(images).await {
        log::warn!("Failed to store {provider_name} inline images: {error}");
    }
}

pub async fn index_search_documents(
    search: &SearchState,
    documents: Vec<SearchDocument>,
    provider_name: &str,
) {
    if let Err(error) = search.index_messages_batch(&documents).await {
        log::warn!("Failed to index {provider_name} messages: {error}");
    }
}
