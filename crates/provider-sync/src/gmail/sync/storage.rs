use db::db::ReadDbState;
use db::db::queries_extra::{
    AttachmentInsertRow, LabelWriteRow, MessageInsertRow, insert_attachments, insert_messages,
    upsert_labels,
};
use common::types::{FolderKind, LabelKind, MailProviderKind};
use search::SearchDocument;
use service_state::{BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState};
use store::inline_image_store::InlineImage;

use super::super::client::GmailClient;
use super::super::parse::{ParsedGmailMessage, parse_gmail_message};
use crate::persistence as sync_persistence;
use crate::thread_membership::replace_thread_membership_from_full_coverage;

// ---------------------------------------------------------------------------
// Single-thread fetch + store
// ---------------------------------------------------------------------------

/// Fetch and store a single thread. Returns its history_id.
#[allow(clippy::too_many_arguments)]
pub(super) async fn process_single_thread(
    client: &GmailClient,
    thread_id: &str,
    account_id: &str,
    db: &ReadDbState,
    write_db: &WriteDbState,
    body_store: &BodyStoreWriteState,
    inline_images: &InlineImageStoreWriteState,
    search: &SearchWriteHandle,
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
    write_db.with_write(move |conn| store_thread_to_db(conn, &aid, &tid, &parsed_clone))
        .await?;

    // Fire-and-forget post-DB writes - all independent, run concurrently.
    tokio::join!(
        store_bodies(body_store, &parsed),
        store_inline_images(inline_images, &parsed),
        index_messages(search, account_id, &parsed),
        crate::seen_ingest::ingest_from_messages(write_db, account_id, &parsed),
    );

    Ok(history_id)
}

// ---------------------------------------------------------------------------
// DB write helpers
// ---------------------------------------------------------------------------

fn store_thread_to_db(
    conn: &db::db::WriteConn<'_>,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    let tx = conn
        .transaction()
        .map_err(|e| format!("begin tx: {e}"))?;

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
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }

    // The messages table has an FK to threads; create a placeholder row
    // before inserting messages, then overwrite it with the real aggregate
    // computed from those messages below.
    sync_persistence::ensure_thread_exists(tx, account_id, thread_id)?;

    // First upsert the incoming messages so attachments can satisfy their FK,
    // then insert attachments before computing the thread aggregate.
    upsert_messages(tx, account_id, messages)?;
    upsert_attachments(tx, account_id, messages)?;

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
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    // BTreeSet keyed on the canonical storage id keeps the write order
    // deterministic across runs (HashSet ordering is randomized), which
    // matters for harness/log diffing even though INSERT OR IGNORE itself
    // is order-insensitive.
    let mut folders_by_id = std::collections::BTreeMap::new();
    let mut labels_by_id = std::collections::BTreeMap::new();
    for label_id in messages
        .iter()
        .flat_map(|message| message.base.label_ids.iter().map(String::as_str))
    {
        if common::folder_roles::is_message_state_label_id(label_id) {
            continue;
        }
        if common::folder_roles::is_gmail_system_folder_label_id(label_id) {
            let folder = FolderKind::parse(label_id, MailProviderKind::Gmail)?;
            folders_by_id.insert(folder.storage_id(), folder);
        } else {
            let label = LabelKind::parse(label_id, MailProviderKind::Gmail)?;
            labels_by_id.insert(label.storage_id(), label);
        }
    }
    let folders: Vec<_> = folders_by_id.into_values().collect();
    let labels: Vec<_> = labels_by_id.into_values().collect();

    // Pre-create `labels` rows for any user-label IDs referenced by these
    // messages. Thread-label replacement inserts FK-constrained rows; a
    // user label observed on a message before the next `sync_labels` pass
    // would otherwise FK-fail the whole transaction. Placeholder name is
    // the label id - sync_labels overwrites it with the real display name
    // and colour on its next cycle.
    let placeholder_rows: Vec<LabelWriteRow> = labels
        .iter()
        .map(|label| {
            let id = label.storage_id();
            LabelWriteRow {
                id: id.clone(),
                account_id: account_id.to_string(),
                name: id,
                visible: None,
                sort_order: None,
                server_color_bg: None,
                server_color_fg: None,
                user_color_bg: None,
                user_color_fg: None,
                is_undeletable: false,
            }
        })
        .collect();
    if !placeholder_rows.is_empty() {
        upsert_labels(tx, &placeholder_rows)?;
    }

    replace_thread_membership_from_full_coverage(tx, account_id, thread_id, &folders, &labels)
}

fn upsert_messages(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    let rows: Vec<MessageInsertRow> = messages
        .iter()
        .map(|msg| {
            let b = &msg.base;
            let invite_idx = msg.attachments.iter().position(|att| {
                common::email_parsing::is_calendar_content_type(&att.mime_type)
            });
            let invite_method = invite_idx.and_then(|i| {
                common::email_parsing::extract_imip_method(&msg.attachments[i].mime_type)
            });
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
                is_replied: b.is_replied,
                is_forwarded: b.is_forwarded,
                raw_size: Some(b.raw_size),
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
                has_meeting_invite: invite_idx.is_some(),
                meeting_invite_method: invite_method,
                meeting_invite_uid: None,
            }
        })
        .collect();
    insert_messages(tx, &rows)
}

fn upsert_attachments(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    let rows: Vec<AttachmentInsertRow> = messages
        .iter()
        .flat_map(|msg| {
            msg.attachments.iter().map(move |att| AttachmentInsertRow {
                id: format!("{}_{}", msg.base.id, att.remote_attachment_id),
                message_id: msg.base.id.clone(),
                account_id: account_id.to_string(),
                filename: Some(att.filename.clone()),
                mime_type: Some(att.mime_type.clone()),
                size: Some(att.size),
                remote_attachment_id: Some(att.remote_attachment_id.clone()),
                content_hash: att.content_hash,
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
    tx: &db::db::WriteTxn<'_>,
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

async fn store_bodies(body_store: &BodyStoreWriteState, messages: &[ParsedGmailMessage]) {
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
    inline_images: &InlineImageStoreWriteState,
    messages: &[ParsedGmailMessage],
) {
    let images: Vec<InlineImage> = messages
        .iter()
        .flat_map(|m| &m.attachments)
        .filter_map(|att| {
            let data = att.inline_data.as_ref()?;
            let hash = att.content_hash.as_ref()?;
            Some(InlineImage {
                content_hash: hash.to_hex(),
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

async fn index_messages(search: &SearchWriteHandle, account_id: &str, messages: &[ParsedGmailMessage]) {
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
            // Phase 7: provider crates emit thin docs without
            // attachment fragments; writer task enriches at apply
            // time (lands 7-3c).
            attachments: Vec::new(),
        })
        .collect();

    sync_persistence::index_search_documents(search, docs, "Gmail").await;
}
