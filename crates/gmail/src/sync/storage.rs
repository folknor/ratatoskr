use db::db::DbState;
use db::db::queries_extra::{AttachmentInsertRow, MessageInsertRow, insert_attachments, insert_messages};
use search::{SearchDocument, SearchState};
use store::body_store::BodyStoreState;
use store::inline_image_store::{InlineImage, InlineImageStoreState};

use super::super::client::GmailClient;
use super::super::parse::{ParsedGmailMessage, parse_gmail_message};
use sync::persistence as sync_persistence;

// ---------------------------------------------------------------------------
// Single-thread fetch + store
// ---------------------------------------------------------------------------

/// Fetch and store a single thread. Returns its history_id.
pub(super) async fn process_single_thread(
    client: &GmailClient,
    thread_id: &str,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
) -> Result<String, String> {
    let thread = client.get_thread(thread_id, "full", db).await?;

    let history_id = thread.history_id.clone().unwrap_or_default();

    if thread.messages.is_empty() {
        return Ok(history_id);
    }

    let parsed: Vec<ParsedGmailMessage> = thread.messages.iter().map(parse_gmail_message).collect();

    // DB writes
    let aid = account_id.to_string();
    let tid = thread_id.to_string();
    let parsed_clone = parsed.clone();
    db.with_conn(move |conn| store_thread_to_db(conn, &aid, &tid, &parsed_clone))
        .await?;

    // Fire-and-forget post-DB writes — all independent, run concurrently.
    tokio::join!(
        store_bodies(body_store, &parsed),
        store_inline_images(inline_images, &parsed),
        index_messages(search, account_id, &parsed),
        seen::ingest_from_messages(db, account_id, &parsed),
    );

    Ok(history_id)
}

// ---------------------------------------------------------------------------
// DB write helpers
// ---------------------------------------------------------------------------

fn store_thread_to_db(
    conn: &rusqlite::Connection,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin tx: {e}"))?;

    // upsert_thread_record calls upsert_messages internally before aggregating
    upsert_attachments(&tx, account_id, messages)?;
    upsert_thread_record(&tx, account_id, thread_id, messages)?;
    set_thread_labels(&tx, account_id, thread_id, messages)?;
    insert_reactions(&tx, account_id, messages)?;

    for msg in messages {
        sync_persistence::upsert_thread_participants(
            &tx,
            account_id,
            thread_id,
            msg.base.from_address.as_deref(),
            msg.base.to_addresses.as_deref(),
            msg.base.cc_addresses.as_deref(),
            msg.base.bcc_addresses.as_deref(),
        )?;
    }
    let user_emails = sync_persistence::query_user_emails(&tx)?;
    sync_persistence::maybe_update_chat_state(&tx, account_id, thread_id, &user_emails)?;

    tx.commit().map_err(|e| format!("commit: {e}"))?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn upsert_thread_record(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGmailMessage],
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
        None,
    )
}

fn set_thread_labels(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGmailMessage],
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
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    let rows: Vec<MessageInsertRow> = messages
        .iter()
        .map(|msg| {
            let b = &msg.base;
            MessageInsertRow {
                id: b.id.clone(),
                account_id: account_id.to_string(),
                thread_id: b.thread_id.clone(),
                from_address: b.from_address.clone(),
                from_name: b.from_name.clone(),
                to_addresses: b.to_addresses.clone(),
                cc_addresses: b.cc_addresses.clone(),
                bcc_addresses: b.bcc_addresses.clone(),
                reply_to: b.reply_to.clone(),
                subject: b.subject.clone(),
                snippet: b.snippet.clone(),
                date: b.date,
                is_read: b.is_read,
                is_starred: b.is_starred,
                raw_size: Some(i64::from(b.raw_size)),
                internal_date: Some(b.internal_date),
                list_unsubscribe: b.list_unsubscribe.clone(),
                list_unsubscribe_post: b.list_unsubscribe_post.clone(),
                auth_results: b.auth_results.clone(),
                message_id_header: b.message_id_header.clone(),
                references_header: b.references_header.clone(),
                in_reply_to_header: b.in_reply_to_header.clone(),
                body_cached: b.body_html.is_some() || b.body_text.is_some(),
                mdn_requested: b.mdn_requested,
                is_reaction: msg.is_reaction,
                imap_uid: None,
                imap_folder: None,
            }
        })
        .collect();
    insert_messages(tx, &rows)
}

fn upsert_attachments(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    let rows: Vec<AttachmentInsertRow> = messages
        .iter()
        .flat_map(|msg| {
            msg.attachments.iter().map(move |att| AttachmentInsertRow {
                id: format!("{}_{}", msg.base.id, att.gmail_attachment_id),
                message_id: msg.base.id.clone(),
                account_id: account_id.to_string(),
                filename: Some(att.filename.clone()),
                mime_type: Some(att.mime_type.clone()),
                size: Some(i64::from(att.size)),
                remote_attachment_id: Some(att.gmail_attachment_id.clone()),
                content_hash: att.content_hash.clone(),
                content_id: att.content_id.clone(),
                is_inline: att.is_inline,
            })
        })
        .collect();
    insert_attachments(tx, &rows)
}

/// For each reaction message, resolve the target message via `In-Reply-To` header
/// and insert into `message_reactions`.
fn insert_reactions(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    for msg in messages {
        let Some(emoji) = &msg.reaction_emoji else {
            continue;
        };
        let Some(in_reply_to) = &msg.base.in_reply_to_header else {
            log::warn!(
                "Reaction message {} has no In-Reply-To header, skipping",
                msg.base.id
            );
            continue;
        };
        let Some(reactor_email) = &msg.base.from_address else {
            continue;
        };

        // Look up the target message by its Message-ID header
        let target_message_id: Option<String> = tx
            .query_row(
                "SELECT id FROM messages WHERE message_id_header = ?1 AND account_id = ?2 LIMIT 1",
                rusqlite::params![in_reply_to, account_id],
                |row| row.get("id"),
            )
            .ok();

        let target_id = target_message_id.as_deref().unwrap_or(in_reply_to.as_str());

        tx.execute(
            "INSERT INTO message_reactions \
             (message_id, account_id, reactor_email, reactor_name, reaction_type, reacted_at, source) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'gmail_mime') \
             ON CONFLICT(message_id, account_id, reactor_email, reaction_type) DO UPDATE SET \
               reactor_name = ?4, reacted_at = ?6",
            rusqlite::params![
                target_id,
                account_id,
                reactor_email,
                msg.base.from_name,
                emoji,
                msg.base.date,
            ],
        )
        .map_err(|e| format!("insert reaction: {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Body store helper
// ---------------------------------------------------------------------------

async fn store_bodies(body_store: &BodyStoreState, messages: &[ParsedGmailMessage]) {
    sync_persistence::store_message_bodies(
        body_store,
        messages,
        "Gmail",
        |message| &message.base.id,
        |message| message.base.body_html.as_ref(),
        |message| message.base.body_text.as_ref(),
    )
    .await;
}

async fn store_inline_images(
    inline_images: &InlineImageStoreState,
    messages: &[ParsedGmailMessage],
) {
    let images: Vec<InlineImage> = messages
        .iter()
        .flat_map(|m| &m.attachments)
        .filter_map(|att| {
            let data = att.inline_data.as_ref()?;
            let hash = att.content_hash.as_ref()?;
            Some(InlineImage {
                content_hash: hash.clone(),
                data: data.clone(),
                mime_type: att.mime_type.clone(),
            })
        })
        .collect();

    sync_persistence::store_inline_images(inline_images, images, "Gmail").await;
}

// ---------------------------------------------------------------------------
// Search index helper
// ---------------------------------------------------------------------------

async fn index_messages(search: &SearchState, account_id: &str, messages: &[ParsedGmailMessage]) {
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

    sync_persistence::index_search_documents(search, docs, "Gmail").await;
}
