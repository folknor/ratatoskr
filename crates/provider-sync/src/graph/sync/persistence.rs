use std::collections::HashMap;

use common::types::{FolderKind, ImportanceLevel, LabelKind, MailProviderKind};
use db::db::ReadDbState;
use db::db::queries_extra::{
    AttachmentInsertRow, LabelWriteRow, MessageInsertRow, delete_message_reaction,
    insert_attachments, insert_messages, upsert_labels, upsert_message_reaction,
    upsert_message_reaction_update_type,
};

use super::super::client::GraphClient;
use super::super::parse::ParsedGraphMessage;
use super::super::types::{
    BatchRequest, BatchRequestItem, REACTIONS_GUID, SingleValueExtendedProperty,
};
use super::SyncCtx;
use super::stores::{index_messages, store_bodies, store_inline_images};
use crate::persistence as sync_persistence;
use crate::thread_membership::replace_message_membership_and_recompute;

// ---------------------------------------------------------------------------
// DB persistence (mirrors jmap/sync.rs patterns)
// ---------------------------------------------------------------------------

/// Persist parsed messages to DB, body store, and search index.
pub(super) async fn persist_messages(
    sctx: &SyncCtx<'_>,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }

    // Group messages by thread for thread-level aggregation
    let mut threads: HashMap<&str, Vec<&ParsedGraphMessage>> = HashMap::new();
    for msg in messages {
        threads.entry(&msg.base.thread_id).or_default().push(msg);
    }

    // 1. DB writes (metadata + thread aggregation)
    let aid = sctx.account_id.to_string();
    let shared_mb_id = sctx.client.mailbox_id().map(String::from);
    let thread_groups: Vec<(String, Vec<ParsedGraphMessage>)> = threads
        .into_iter()
        .map(|(tid, msgs)| (tid.to_string(), msgs.into_iter().cloned().collect()))
        .collect();

    let reaction_writes: Vec<GraphReactionWrite> = messages
        .iter()
        .filter(|m| m.owner_reaction_type.is_some() || m.reactions_count.is_some())
        .map(|m| GraphReactionWrite {
            message_id: m.base.id.clone(),
            owner_reaction_type: m.owner_reaction_type.clone(),
            reactions_count: m.reactions_count,
            reacted_at: m.base.date,
        })
        .collect();

    sctx.write_db
        .with_write(move |conn| {
            let tx = conn.transaction().map_err(|e| format!("begin tx: {e}"))?;
            let user_emails = sync_persistence::query_user_emails(&tx)?;
            for (thread_id, msgs) in &thread_groups {
                store_thread_to_db(
                    &tx,
                    &aid,
                    thread_id,
                    msgs,
                    shared_mb_id.as_deref(),
                    &user_emails,
                )?;
            }
            insert_exchange_reactions(&tx, &aid, &reaction_writes)?;
            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(())
        })
        .await?;

    // 2-5. Fire-and-forget post-DB writes - all independent, run concurrently.
    tokio::join!(
        store_bodies(sctx.body_store, messages),
        store_inline_images(sctx.inline_images, messages),
        index_messages(sctx.search, sctx.account_id, messages),
        crate::seen_ingest::ingest_from_messages(sctx.write_db, sctx.account_id, messages),
    );

    Ok(())
}

/// Delete messages from DB, body store, and search index.
/// Also updates or removes parent threads as needed.
pub(super) async fn delete_messages(
    sctx: &SyncCtx<'_>,
    message_ids: &[String],
) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }

    let aid = sctx.account_id.to_string();
    let ids = message_ids.to_vec();

    // Delete from DB and update parent threads
    sctx.write_db
        .with_write(move |conn| {
            let tx = conn.transaction().map_err(|e| format!("begin tx: {e}"))?;
            sync_persistence::delete_messages_and_cleanup_threads(&tx, &aid, &ids)?;
            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(())
        })
        .await?;

    // Delete from body store
    if let Err(e) = sctx.body_store.delete(message_ids.to_vec()).await {
        log::warn!("Failed to delete Graph bodies: {e}");
    }

    // Delete from search index (batch - single commit)
    let owned_ids: Vec<String> = message_ids.to_vec();
    if let Err(e) = sctx.search.delete_messages_batch(owned_ids).await {
        log::warn!("Failed to batch-delete search documents: {e}");
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// DB write helpers
// ---------------------------------------------------------------------------

fn store_thread_to_db(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGraphMessage],
    shared_mailbox_id: Option<&str>,
    user_emails: &[String],
) -> Result<(), String> {
    upsert_thread_record(tx, account_id, thread_id, messages, shared_mailbox_id)?;
    upsert_graph_label_rows(tx, account_id, messages)?;
    set_thread_labels(tx, account_id, thread_id, messages)?;

    // Populate thread_participants from message address fields.
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
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGraphMessage],
    shared_mailbox_id: Option<&str>,
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
        shared_mailbox_id,
    )
}

fn set_thread_labels(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    // Graph delta pages contain only changed messages. Replace membership at
    // the message grain, then recompute the thread aggregate from the
    // per-message union so removals observed by other clients are visible.
    for message in messages {
        let mut folders = Vec::new();
        let mut labels = Vec::new();
        for label_id in message.base.label_ids.iter().map(String::as_str) {
            if common::folder_roles::is_graph_tag_id(label_id) {
                labels.push(LabelKind::parse(label_id, MailProviderKind::Graph)?);
            } else {
                folders.push(FolderKind::parse(label_id, MailProviderKind::Graph)?);
            }
        }
        replace_message_membership_and_recompute(
            tx,
            account_id,
            thread_id,
            &message.base.id,
            &folders,
            &labels,
        )?;
    }
    Ok(())
}

fn upsert_graph_label_rows(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    let mut rows = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for msg in messages {
        for category in &msg.categories {
            let label_id = LabelKind::graph_category(category)?.storage_id();
            if seen.insert(label_id.clone()) {
                rows.push(LabelWriteRow {
                    id: label_id,
                    account_id: account_id.to_string(),
                    name: category.clone(),
                    visible: None,
                    sort_order: None,
                    server_color_bg: None,
                    server_color_fg: None,
                    user_color_bg: None,
                    user_color_fg: None,
                    is_undeletable: false,
                });
            }
        }

        for label_id in &msg.base.label_ids {
            let Some(level) = ImportanceLevel::parse_label_id(label_id) else {
                continue;
            };
            if seen.insert(level.label_id().to_string()) {
                rows.push(LabelWriteRow {
                    id: level.label_id().to_string(),
                    account_id: account_id.to_string(),
                    name: level.display_name().to_string(),
                    visible: None,
                    sort_order: Some(level.sort_order()),
                    server_color_bg: None,
                    server_color_fg: None,
                    user_color_bg: None,
                    user_color_fg: None,
                    is_undeletable: true,
                });
            }
        }
    }

    if rows.is_empty() {
        return Ok(());
    }

    upsert_labels(tx, &rows)
}

fn upsert_messages(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    let rows: Vec<MessageInsertRow> = messages
        .iter()
        .map(|msg| {
            let b = &msg.base;
            let invite_idx = msg.attachments.iter().position(|att| {
                att.mime_type
                    .as_deref()
                    .is_some_and(common::email_parsing::is_calendar_content_type)
            });
            let invite_method = invite_idx
                .and_then(|i| msg.attachments[i].mime_type.as_deref())
                .and_then(common::email_parsing::extract_imip_method);
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
                is_reaction: false,
                imap_uid: None,
                imap_folder: None,
                imap_uidvalidity: None,
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
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    let rows: Vec<AttachmentInsertRow> = messages
        .iter()
        .flat_map(|msg| {
            msg.attachments.iter().map(move |att| AttachmentInsertRow {
                id: format!("{}_{}", msg.base.id, att.id),
                message_id: msg.base.id.clone(),
                account_id: account_id.to_string(),
                filename: att.filename.clone(),
                mime_type: att.mime_type.clone(),
                size: att.size,
                remote_attachment_id: Some(att.id.clone()),
                content_hash: att.content_hash,
                content_id: att.content_id.clone(),
                is_inline: att.is_inline,
            })
        })
        .collect();
    insert_attachments(tx, &rows)
}

#[derive(Clone)]
struct GraphReactionWrite {
    message_id: String,
    owner_reaction_type: Option<String>,
    reactions_count: Option<i64>,
    reacted_at: i64,
}

fn insert_exchange_reactions(
    tx: &db::db::WriteTxn<'_>,
    account_id: &str,
    writes: &[GraphReactionWrite],
) -> Result<(), String> {
    if writes.is_empty() {
        return Ok(());
    }

    let owner_email: String = tx
        .query_row(
            "SELECT email FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| row.get("email"),
        )
        .map_err(|e| format!("lookup account email for reactions: {e}"))?;

    for write in writes {
        if let Some(emoji) = &write.owner_reaction_type {
            upsert_message_reaction(
                tx,
                &write.message_id,
                account_id,
                &owner_email,
                emoji,
                Some(write.reacted_at),
                "exchange_native",
            )?;
        }

        if let Some(count) = write.reactions_count {
            upsert_message_reaction_update_type(
                tx,
                &write.message_id,
                account_id,
                "__count__",
                &count.to_string(),
                "exchange_native",
            )?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Reaction delta refresh
// ---------------------------------------------------------------------------

/// Re-fetch Exchange reaction extended properties for messages that already
/// have reactions in the DB.
///
/// Exchange reactions do NOT update `lastModifiedDateTime` or `changeKey` on
/// messages, so delta queries miss reaction changes entirely. This function
/// compensates by periodically polling reaction properties for messages that
/// we know have had reactions before (i.e., have rows in `message_reactions`).
///
/// Uses `$batch` to fetch up to 20 messages per API call (Graph batch limit).
/// Returns the number of messages whose reactions were updated.
pub(super) async fn refresh_reactions_for_recent_messages(
    client: &GraphClient,
    db: &ReadDbState,
    write_db: &service_state::WriteDbState,
    account_id: &str,
) -> Result<usize, String> {
    // Find message IDs that have existing reaction rows (excluding the __count__ metadata)
    // or were recently viewed. Limit to 60 to keep API cost bounded (3 batch calls max).
    let aid = account_id.to_string();
    let message_ids: Vec<String> = db
        .with_read(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT mr.message_id FROM message_reactions mr \
                     JOIN messages m ON m.id = mr.message_id AND m.account_id = mr.account_id \
                     WHERE mr.account_id = ?1 AND mr.source = 'exchange_native' \
                     ORDER BY m.date DESC \
                     LIMIT 60",
                )
                .map_err(|e| format!("prepare reaction refresh query: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![aid], |row| {
                    row.get::<_, String>("message_id")
                })
                .map_err(|e| format!("query reaction messages: {e}"))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row.map_err(|e| format!("read reaction message id: {e}"))?);
            }
            Ok(ids)
        })
        .await?;

    if message_ids.is_empty() {
        return Ok(0);
    }

    let owner_reaction_id = format!("String {REACTIONS_GUID} Name OwnerReactionType");
    let reactions_count_id = format!("Integer {REACTIONS_GUID} Name ReactionsCount");
    let expand_filter =
        format!("$filter=id eq '{owner_reaction_id}' or id eq '{reactions_count_id}'");

    // Look up the authenticated user's email for reaction rows
    let aid2 = account_id.to_string();
    let owner_email: String = db
        .with_read(move |conn| {
            conn.query_row(
                "SELECT email FROM accounts WHERE id = ?1",
                rusqlite::params![aid2],
                |row| row.get("email"),
            )
            .map_err(|e| format!("lookup account email: {e}"))
        })
        .await?;

    let mut updated_count: usize = 0;

    // Process in batches of 20 (Graph batch limit)
    let me = client.api_path_prefix();
    for chunk in message_ids.chunks(20) {
        let requests: Vec<BatchRequestItem> = chunk
            .iter()
            .enumerate()
            .map(|(i, msg_id)| {
                let enc_id = urlencoding::encode(msg_id);
                BatchRequestItem {
                    id: i.to_string(),
                    method: "GET".to_string(),
                    url: format!(
                        "{me}/messages/{enc_id}/singleValueExtendedProperties?{expand_filter}"
                    ),
                    body: None,
                    headers: None,
                }
            })
            .collect();

        let batch_req = BatchRequest { requests };
        let batch_resp = client.post_batch(&batch_req, db).await?;

        // Collect reaction updates from batch responses
        let mut reaction_updates: Vec<(String, Option<String>, Option<i64>)> = Vec::new();

        for resp_item in &batch_resp.responses {
            if resp_item.status != 200 {
                continue;
            }

            let idx: usize = resp_item.id.parse().unwrap_or(usize::MAX);
            let Some(msg_id) = chunk.get(idx) else {
                continue;
            };

            // Parse the extended properties from the response
            let mut owner_reaction: Option<String> = None;
            let mut reactions_count: Option<i64> = None;

            if let Some(body) = &resp_item.body
                && let Some(values) = body.get("value").and_then(|v| v.as_array())
            {
                for prop_val in values {
                    if let Ok(prop) =
                        serde_json::from_value::<SingleValueExtendedProperty>(prop_val.clone())
                    {
                        if prop.id.eq_ignore_ascii_case(&owner_reaction_id) {
                            let val = prop.value.trim();
                            if !val.is_empty() {
                                owner_reaction = Some(val.to_string());
                            }
                        } else if prop.id.eq_ignore_ascii_case(&reactions_count_id) {
                            reactions_count = prop.value.trim().parse::<i64>().ok();
                        }
                    }
                }
            }

            reaction_updates.push((msg_id.clone(), owner_reaction, reactions_count));
        }

        // Write updates to DB
        if !reaction_updates.is_empty() {
            let aid3 = account_id.to_string();
            let email = owner_email.clone();
            let batch_updated = write_db
                .with_write(move |conn| {
                    let tx = conn.transaction().map_err(|e| format!("begin tx: {e}"))?;

                    let mut count: usize = 0;
                    for (msg_id, owner_reaction, reactions_count) in &reaction_updates {
                        if let Some(emoji) = owner_reaction {
                            // Use update-type variant: on conflict, update reaction_type
                            // (emoji may change even if the row already exists).
                            upsert_message_reaction_update_type(
                                &tx,
                                msg_id,
                                &aid3,
                                &email,
                                emoji,
                                "exchange_native",
                            )?;
                            count += 1;
                        } else {
                            // Owner reaction was removed - delete the row if it exists
                            delete_message_reaction(&tx, msg_id, &aid3, &email, "exchange_native")?;
                        }

                        if let Some(c) = reactions_count {
                            upsert_message_reaction_update_type(
                                &tx,
                                msg_id,
                                &aid3,
                                "__count__",
                                &c.to_string(),
                                "exchange_native",
                            )?;
                        }
                    }

                    tx.commit()
                        .map_err(|e| format!("commit reaction refresh: {e}"))?;
                    Ok(count)
                })
                .await?;

            updated_count += batch_updated;
        }
    }

    Ok(updated_count)
}
