use ratatoskr_stores::body_store::BodyStoreState;
use ratatoskr_db::db::DbState;
use ratatoskr_stores::inline_image_store::{InlineImage, InlineImageStoreState};
use ratatoskr_search::{SearchDocument, SearchState};

use super::super::client::GmailClient;
use super::super::parse::{ParsedGmailMessage, parse_gmail_message};
use ratatoskr_sync::persistence as sync_persistence;

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
        ratatoskr_seen_addresses::ingest_from_messages(db, account_id, &parsed),
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
    for msg in messages {
        upsert_single_message(tx, account_id, msg)?;
    }
    Ok(())
}

fn upsert_single_message(
    tx: &rusqlite::Transaction,
    account_id: &str,
    msg: &ParsedGmailMessage,
) -> Result<(), String> {
    let b = &msg.base;
    let has_body = b.body_html.is_some() || b.body_text.is_some();

    tx.execute(
        "INSERT OR REPLACE INTO messages \
         (id, account_id, thread_id, from_address, from_name, to_addresses, \
          cc_addresses, bcc_addresses, reply_to, subject, snippet, date, \
          is_read, is_starred, raw_size, internal_date, \
          list_unsubscribe, list_unsubscribe_post, auth_results, \
          message_id_header, references_header, in_reply_to_header, body_cached, \
          mdn_requested, is_reaction) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                 ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
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
            msg.is_reaction,
        ],
    )
    .map_err(|e| format!("upsert message: {e}"))?;
    Ok(())
}

fn upsert_attachments(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    for msg in messages {
        for att in &msg.attachments {
            let att_id = format!("{}_{}", msg.base.id, att.gmail_attachment_id);
            tx.execute(
                "INSERT INTO attachments \
                 (id, message_id, account_id, filename, mime_type, size, \
                  gmail_attachment_id, content_hash, content_id, is_inline) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) \
                 ON CONFLICT(id) DO UPDATE SET \
                   filename = ?4, mime_type = ?5, size = ?6, \
                   gmail_attachment_id = ?7, content_hash = ?8, content_id = ?9, is_inline = ?10",
                rusqlite::params![
                    att_id,
                    msg.base.id,
                    account_id,
                    att.filename,
                    att.mime_type,
                    att.size,
                    att.gmail_attachment_id,
                    att.content_hash,
                    att.content_id,
                    att.is_inline,
                ],
            )
            .map_err(|e| format!("upsert attachment: {e}"))?;
        }
    }
    Ok(())
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
