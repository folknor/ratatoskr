use std::collections::HashMap;

use ratatoskr_stores::attachment_cache::hash_bytes;
use ratatoskr_stores::body_store::BodyStoreState;
use ratatoskr_stores::inline_image_store::{InlineImage, MAX_INLINE_SIZE};
use ratatoskr_search::{SearchDocument, SearchState};

use super::super::parse::ParsedJmapMessage;
use super::SyncCtx;
use ratatoskr_sync::persistence as sync_persistence;

// ---------------------------------------------------------------------------
// DB persistence
// ---------------------------------------------------------------------------

/// Persist parsed messages to DB, body store, and search index.
pub(crate) async fn persist_messages(
    ctx: &SyncCtx<'_>,
    messages: &[ParsedJmapMessage],
    _mailbox_data: &[(String, Option<String>, String)],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }

    // Group messages by thread for thread-level aggregation
    let mut threads: HashMap<&str, Vec<&ParsedJmapMessage>> = HashMap::new();
    for msg in messages {
        threads.entry(&msg.base.thread_id).or_default().push(msg);
    }

    // 1. DB writes (metadata + thread aggregation)
    let aid = ctx.account_id.to_string();
    let shared_mb_id = ctx.shared_account_id().map(String::from);
    let thread_groups: Vec<(String, Vec<ParsedJmapMessage>)> = threads
        .into_iter()
        .map(|(tid, msgs)| (tid.to_string(), msgs.into_iter().cloned().collect()))
        .collect();

    ctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            let user_emails = sync_persistence::query_user_emails(&tx)?;
            for (thread_id, msgs) in &thread_groups {
                store_thread_to_db(&tx, &aid, thread_id, msgs, shared_mb_id.as_deref(), &user_emails)?;
            }
            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(())
        })
        .await?;

    // 2-5. Fire-and-forget post-DB writes -- all independent, run concurrently.
    tokio::join!(
        store_bodies(ctx.body_store, messages),
        store_inline_images(ctx, messages),
        index_messages(ctx.search, ctx.account_id, messages),
        ratatoskr_seen_addresses::ingest_from_messages(ctx.db, ctx.account_id, messages),
    );

    Ok(())
}

/// Delete messages from DB, body store, and search index.
/// Also updates or removes parent threads as needed.
pub(crate) async fn delete_messages(ctx: &SyncCtx<'_>, message_ids: &[&str]) -> Result<(), String> {
    let aid = ctx.account_id.to_string();
    let ids: Vec<String> = message_ids.iter().map(|s| (*s).to_string()).collect();

    // Delete from DB and update parent threads
    ctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            sync_persistence::delete_messages_and_cleanup_threads(&tx, &aid, &ids)?;
            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(())
        })
        .await?;

    // Delete from body store
    let body_ids: Vec<String> = message_ids.iter().map(|s| (*s).to_string()).collect();
    if let Err(e) = ctx.body_store.delete(body_ids).await {
        log::warn!("Failed to delete JMAP bodies: {e}");
    }

    // Delete from search index (batch -- single commit)
    if let Err(e) = ctx.search.delete_messages_batch(message_ids).await {
        log::warn!("Failed to batch-delete search documents: {e}");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// DB write helpers (mirrors gmail/sync patterns)
// ---------------------------------------------------------------------------

fn store_thread_to_db(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedJmapMessage],
    shared_mailbox_id: Option<&str>,
    user_emails: &[String],
) -> Result<(), String> {
    // upsert_thread_record calls upsert_messages internally before aggregating
    upsert_attachments(tx, account_id, messages)?;
    upsert_thread_record(tx, account_id, thread_id, messages, shared_mailbox_id)?;
    set_thread_labels(tx, account_id, thread_id, messages)?;
    sync_keyword_labels(tx, account_id, thread_id, messages)?;
    for msg in messages {
        sync_persistence::upsert_thread_participants(
            tx,
            account_id,
            thread_id,
            msg.base.from_address.as_deref(),
            msg.base.to_addresses.as_deref(),
            msg.base.cc_addresses.as_deref(),
            msg.base.bcc_addresses.as_deref(),
        )?;
    }
    sync_persistence::maybe_update_chat_state(tx, account_id, thread_id, user_emails)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn upsert_thread_record(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedJmapMessage],
    shared_mailbox_id: Option<&str>,
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }

    // First upsert the incoming messages so they are visible in DB queries
    upsert_messages(tx, account_id, messages)?;

    let is_important = messages
        .iter()
        .flat_map(|message| message.base.label_ids.iter().map(String::as_str))
        .any(|label| label == "IMPORTANT");

    let aggregate = sync_persistence::compute_thread_aggregate(tx, account_id, thread_id)?;
    sync_persistence::upsert_thread_aggregate(
        tx,
        account_id,
        thread_id,
        &aggregate,
        Some(is_important),
        shared_mailbox_id,
    )
}

fn set_thread_labels(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedJmapMessage],
) -> Result<(), String> {
    sync_persistence::replace_thread_labels(
        tx,
        account_id,
        thread_id,
        messages
            .iter()
            .flat_map(|message| message.base.label_ids.iter().map(String::as_str)),
    )
}

fn upsert_messages(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedJmapMessage],
) -> Result<(), String> {
    for msg in messages {
        let b = &msg.base;
        let has_body = b.body_html.is_some() || b.body_text.is_some();

        tx.execute(
            "INSERT OR REPLACE INTO messages \
             (id, account_id, thread_id, from_address, from_name, to_addresses, \
              cc_addresses, bcc_addresses, reply_to, subject, snippet, date, \
              is_read, is_starred, raw_size, internal_date, \
              list_unsubscribe, list_unsubscribe_post, auth_results, \
              message_id_header, references_header, in_reply_to_header, body_cached, \
              mdn_requested) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                     ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
            rusqlite::params![
                b.id,
                account_id,
                b.thread_id,
                b.from_address,
                b.from_name,
                b.to_addresses,
                b.cc_addresses,
                b.bcc_addresses,
                b.reply_to,
                b.subject,
                b.snippet,
                b.date,
                b.is_read,
                b.is_starred,
                b.raw_size,
                b.internal_date,
                b.list_unsubscribe,
                b.list_unsubscribe_post,
                b.auth_results,
                b.message_id_header,
                b.references_header,
                b.in_reply_to_header,
                if has_body { 1i64 } else { 0i64 },
                b.mdn_requested,
            ],
        )
        .map_err(|e| format!("upsert message: {e}"))?;
    }
    Ok(())
}

fn upsert_attachments(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedJmapMessage],
) -> Result<(), String> {
    for msg in messages {
        for att in &msg.attachments {
            let att_id = format!("{}_{}", msg.base.id, att.blob_id);
            tx.execute(
                "INSERT INTO attachments \
                 (id, message_id, account_id, filename, mime_type, size, \
                  gmail_attachment_id, content_id, is_inline) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) \
                 ON CONFLICT(id) DO UPDATE SET \
                   filename = ?4, mime_type = ?5, size = ?6, \
                   gmail_attachment_id = ?7, content_id = ?8, is_inline = ?9",
                rusqlite::params![
                    att_id,
                    msg.base.id,
                    account_id,
                    att.filename,
                    att.mime_type,
                    att.size,
                    att.blob_id, // stored in gmail_attachment_id column (reused for blob_id)
                    att.content_id,
                    att.is_inline,
                ],
            )
            .map_err(|e| format!("upsert attachment: {e}"))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Keyword -> category sync
// ---------------------------------------------------------------------------

/// Ensure non-system JMAP keywords exist in the unified labels system.
///
/// Upserts each keyword as a `label_kind = 'tag'` label with a `kw:` prefix
/// and links it to the thread via `thread_labels`.
fn sync_keyword_labels(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedJmapMessage],
) -> Result<(), String> {
    let mut unique_keywords: Vec<String> = messages
        .iter()
        .flat_map(|msg| msg.keyword_categories.iter().cloned())
        .collect();
    unique_keywords.sort();
    unique_keywords.dedup();

    if unique_keywords.is_empty() {
        return Ok(());
    }

    for keyword in &unique_keywords {
        let label_id = format!("kw:{keyword}");
        tx.execute(
            "INSERT OR IGNORE INTO labels (id, account_id, name, type, label_kind) \
             VALUES (?1, ?2, ?3, 'user', 'tag')",
            rusqlite::params![label_id, account_id, keyword],
        )
        .map_err(|e| format!("upsert jmap keyword label: {e}"))?;
        tx.execute(
            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, label_id],
        )
        .map_err(|e| format!("insert jmap keyword thread_label: {e}"))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Body store helper
// ---------------------------------------------------------------------------

async fn store_bodies(body_store: &BodyStoreState, messages: &[ParsedJmapMessage]) {
    sync_persistence::store_message_bodies(
        body_store,
        messages,
        "JMAP",
        |message| &message.base.id,
        |message| message.base.body_html.as_ref(),
        |message| message.base.body_text.as_ref(),
    )
    .await;
}

/// Max concurrent JMAP blob downloads for inline images.
const INLINE_BLOB_CONCURRENCY: usize = 5;

async fn store_inline_images(ctx: &SyncCtx<'_>, messages: &[ParsedJmapMessage]) {
    use futures::stream::{self, StreamExt};

    let eligible: Vec<(String, String, String)> = messages
        .iter()
        .flat_map(|msg| {
            msg.attachments.iter().filter_map(|att| {
                if !att.is_inline
                    || !att.mime_type.starts_with("image/")
                    || att.size <= 0
                    || usize::try_from(att.size)
                        .ok()
                        .is_none_or(|size| size > MAX_INLINE_SIZE)
                {
                    return None;
                }

                Some((
                    format!("{}_{}", msg.base.id, att.blob_id),
                    att.blob_id.clone(),
                    att.mime_type.clone(),
                ))
            })
        })
        .collect();

    if eligible.is_empty() {
        return;
    }

    // Deduplicate blob IDs so each unique blob is downloaded once
    let mut unique_blobs: HashMap<String, String> = HashMap::new(); // blob_id -> mime_type
    for (_, blob_id, mime_type) in &eligible {
        unique_blobs.entry(blob_id.clone()).or_insert_with(|| mime_type.clone());
    }

    // Download unique blobs in parallel with bounded concurrency
    let blob_cache: HashMap<String, (String, Vec<u8>, String)> = stream::iter(unique_blobs)
        .map(|(blob_id, mime_type)| async move {
            let inner = ctx.client.inner();
            match inner.download(&blob_id).await {
                Ok(data) if data.len() <= MAX_INLINE_SIZE => {
                    let content_hash = hash_bytes(&data);
                    Some((blob_id, (content_hash, data.to_vec(), mime_type)))
                }
                Ok(_) => None,
                Err(error) => {
                    log::warn!("Failed to download JMAP inline blob {blob_id}: {error}");
                    None
                }
            }
        })
        .buffer_unordered(INLINE_BLOB_CONCURRENCY)
        .filter_map(|opt| async { opt })
        .collect()
        .await;

    if blob_cache.is_empty() {
        return;
    }

    // Build attachment -> content_hash mapping for DB updates
    let updates: Vec<(String, String)> = eligible
        .iter()
        .filter_map(|(attachment_row_id, blob_id, _)| {
            blob_cache
                .get(blob_id)
                .map(|(content_hash, _, _)| (attachment_row_id.clone(), content_hash.clone()))
        })
        .collect();

    let images: Vec<InlineImage> = blob_cache
        .into_values()
        .map(|(content_hash, data, mime_type)| InlineImage {
            content_hash,
            data,
            mime_type,
        })
        .collect();

    if let Err(error) = ctx.inline_images.put_batch(images).await {
        log::warn!("Failed to store JMAP inline images: {error}");
        return;
    }

    if let Err(error) = ctx
        .db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("jmap inline image update tx: {e}"))?;
            for (attachment_row_id, content_hash) in updates {
                tx.execute(
                    "UPDATE attachments SET content_hash = ?1 WHERE id = ?2",
                    rusqlite::params![content_hash, attachment_row_id],
                )
                .map_err(|e| format!("update JMAP inline image hash: {e}"))?;
            }
            tx.commit()
                .map_err(|e| format!("commit JMAP inline image hashes: {e}"))?;
            Ok(())
        })
        .await
    {
        log::warn!("Failed to persist JMAP inline image hashes: {error}");
    }
}

// ---------------------------------------------------------------------------
// Search index helper
// ---------------------------------------------------------------------------

async fn index_messages(search: &SearchState, account_id: &str, messages: &[ParsedJmapMessage]) {
    let docs: Vec<SearchDocument> = messages
        .iter()
        .map(|m| SearchDocument {
            message_id: m.base.id.clone(),
            account_id: account_id.to_string(),
            thread_id: m.base.thread_id.clone(),
            subject: m.base.subject.clone(),
            from_name: m.base.from_name.clone(),
            from_address: m.base.from_address.clone(),
            to_addresses: m.base.to_addresses.clone(),
            body_text: m.base.body_text.clone(),
            snippet: Some(m.base.snippet.clone()),
            date: m.base.date / 1000, // tantivy expects seconds
            is_read: m.base.is_read,
            is_starred: m.base.is_starred,
            has_attachment: m.base.has_attachments,
        })
        .collect();

    sync_persistence::index_search_documents(search, docs, "JMAP").await;
}
