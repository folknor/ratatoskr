use std::collections::{HashMap, HashSet};

use crate::db::{DbState, lookups};

use super::super::client::GraphClient;
use super::super::parse::ParsedGraphMessage;
use super::super::types::{
    BatchRequest, BatchRequestItem, REACTIONS_GUID, SingleValueExtendedProperty,
};
use super::SyncCtx;
use super::stores::{index_messages, store_bodies, store_inline_images};
use crate::sync::persistence as sync_persistence;

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
        threads.entry(&msg.thread_id).or_default().push(msg);
    }

    // 1. DB writes (metadata + thread aggregation)
    let aid = sctx.account_id.to_string();
    let thread_groups: Vec<(String, Vec<ParsedGraphMessage>)> = threads
        .into_iter()
        .map(|(tid, msgs)| (tid.to_string(), msgs.into_iter().cloned().collect()))
        .collect();

    sctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            for (thread_id, msgs) in &thread_groups {
                store_thread_to_db(&tx, &aid, thread_id, msgs)?;
            }
            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(())
        })
        .await?;

    // 2-5. Fire-and-forget post-DB writes — all independent, run concurrently.
    tokio::join!(
        store_bodies(sctx.body_store, messages),
        store_inline_images(sctx.inline_images, messages),
        index_messages(sctx.search, sctx.account_id, messages),
        crate::seen_addresses::ingest_from_messages(sctx.db, sctx.account_id, messages),
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
    sctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;

            // Collect affected thread IDs before deleting
            let mut affected_threads = HashSet::new();
            for id in &ids {
                if let Ok(Some(tid)) = lookups::get_thread_id_for_message(&tx, &aid, id) {
                    affected_threads.insert(tid);
                }
            }

            // Delete the messages
            for id in &ids {
                tx.execute(
                    "DELETE FROM messages WHERE account_id = ?1 AND id = ?2",
                    rusqlite::params![aid, id],
                )
                .map_err(|e| format!("delete message: {e}"))?;
            }

            // Update or remove affected threads
            for tid in &affected_threads {
                let remaining: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND account_id = ?2",
                        rusqlite::params![tid, aid],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("count remaining: {e}"))?;

                if remaining == 0 {
                    // Orphan thread — remove it and its labels
                    tx.execute(
                        "DELETE FROM threads WHERE id = ?1 AND account_id = ?2",
                        rusqlite::params![tid, aid],
                    )
                    .map_err(|e| format!("delete orphan thread: {e}"))?;
                    tx.execute(
                        "DELETE FROM thread_labels WHERE thread_id = ?1 AND account_id = ?2",
                        rusqlite::params![tid, aid],
                    )
                    .map_err(|e| format!("delete orphan thread labels: {e}"))?;
                } else {
                    // Re-aggregate thread fields from remaining messages
                    reaggregate_thread(&tx, &aid, tid)?;
                }
            }

            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(())
        })
        .await?;

    // Delete from body store
    if let Err(e) = sctx.body_store.delete(message_ids.to_vec()).await {
        log::warn!("Failed to delete Graph bodies: {e}");
    }

    // Delete from search index (batch — single commit)
    let id_refs: Vec<&str> = message_ids.iter().map(String::as_str).collect();
    if let Err(e) = sctx.search.delete_messages_batch(&id_refs).await {
        log::warn!("Failed to batch-delete search documents: {e}");
    }

    Ok(())
}

/// Re-aggregate thread fields from remaining messages after deletion.
fn reaggregate_thread(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    let aggregate = sync_persistence::compute_thread_aggregate(tx, account_id, thread_id)?;
    sync_persistence::upsert_thread_aggregate(tx, account_id, thread_id, &aggregate, None)
}

// ---------------------------------------------------------------------------
// DB write helpers
// ---------------------------------------------------------------------------

fn store_thread_to_db(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    // upsert_thread_record calls upsert_messages internally before aggregating
    upsert_attachments(tx, account_id, messages)?;
    upsert_thread_record(tx, account_id, thread_id, messages)?;
    set_thread_labels(tx, account_id, thread_id, messages)?;
    insert_exchange_reactions(tx, account_id, messages)?;
    sync_persistence::insert_message_categories(
        tx,
        account_id,
        messages
            .iter()
            .flat_map(|msg| {
                msg.categories
                    .iter()
                    .map(move |cat| (msg.id.as_str(), cat.as_str()))
            }),
    )?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn upsert_thread_record(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }

    // First upsert the incoming messages so they are visible in DB queries
    upsert_messages(tx, account_id, messages)?;

    let is_important = messages
        .iter()
        .flat_map(|message| message.label_ids.iter().map(String::as_str))
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
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    sync_persistence::replace_thread_labels(
        tx,
        account_id,
        thread_id,
        messages
            .iter()
            .flat_map(|message| message.label_ids.iter().map(String::as_str)),
    )
}

fn upsert_messages(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    for msg in messages {
        let has_body = msg.body_html.is_some() || msg.body_text.is_some();

        tx.execute(
            "INSERT OR REPLACE INTO messages \
             (id, account_id, thread_id, from_address, from_name, to_addresses, \
              cc_addresses, bcc_addresses, reply_to, subject, snippet, date, \
              is_read, is_starred, raw_size, internal_date, \
              list_unsubscribe, list_unsubscribe_post, auth_results, \
              message_id_header, references_header, in_reply_to_header, body_cached, \
              mdn_requested, is_mentioned) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                     ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25)",
            rusqlite::params![
                msg.id,
                account_id,
                msg.thread_id,
                msg.from_address,
                msg.from_name,
                msg.to_addresses,
                msg.cc_addresses,
                msg.bcc_addresses,
                msg.reply_to,
                msg.subject,
                msg.snippet,
                msg.date,
                msg.is_read,
                msg.is_starred,
                0i64, // raw_size — Graph doesn't expose message size directly
                msg.internal_date,
                msg.list_unsubscribe,
                msg.list_unsubscribe_post,
                msg.auth_results,
                msg.message_id_header,
                msg.references_header,
                msg.in_reply_to_header,
                if has_body { 1i64 } else { 0i64 },
                msg.mdn_requested,
                msg.is_mentioned,
            ],
        )
        .map_err(|e| format!("upsert message: {e}"))?;
    }
    Ok(())
}

fn upsert_attachments(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    for msg in messages {
        for att in &msg.attachments {
            let att_id = format!("{}_{}", msg.id, att.id);
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
                    msg.id,
                    account_id,
                    att.filename,
                    att.mime_type,
                    att.size,
                    att.id,
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

/// Insert Exchange reactions from extended properties into `message_reactions`.
///
/// For each message with `owner_reaction_type` set, inserts a reaction row with
/// `source = 'exchange_native'`. The reactor email is looked up from the
/// `accounts` table. `reactions_count` is stored as a separate metadata row
/// with `reactor_email = '__count__'` so it can be read back without a
/// separate column.
fn insert_exchange_reactions(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    // Check if any message has reaction data before looking up the account email
    let has_reactions = messages
        .iter()
        .any(|m| m.owner_reaction_type.is_some() || m.reactions_count.is_some());
    if !has_reactions {
        return Ok(());
    }

    // Look up the authenticated user's email address
    let owner_email: String = tx
        .query_row(
            "SELECT email FROM accounts WHERE id = ?1",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("lookup account email for reactions: {e}"))?;

    for msg in messages {
        // Insert the owner's reaction if present
        if let Some(emoji) = &msg.owner_reaction_type {
            tx.execute(
                "INSERT INTO message_reactions \
                 (message_id, account_id, reactor_email, reactor_name, reaction_type, reacted_at, source) \
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, 'exchange_native') \
                 ON CONFLICT(message_id, account_id, reactor_email, reaction_type) DO UPDATE SET \
                   reacted_at = ?5",
                rusqlite::params![msg.id, account_id, owner_email, emoji, msg.date],
            )
            .map_err(|e| format!("insert exchange reaction: {e}"))?;
        }

        // Store the total reactions count as a metadata row so we know
        // there are other reactions beyond the owner's.
        if let Some(count) = msg.reactions_count {
            tx.execute(
                "INSERT INTO message_reactions \
                 (message_id, account_id, reactor_email, reactor_name, reaction_type, reacted_at, source) \
                 VALUES (?1, ?2, '__count__', NULL, ?3, NULL, 'exchange_native') \
                 ON CONFLICT(message_id, account_id, reactor_email, reaction_type) DO UPDATE SET \
                   reaction_type = ?3",
                rusqlite::params![msg.id, account_id, count.to_string()],
            )
            .map_err(|e| format!("insert exchange reaction count: {e}"))?;
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
    db: &DbState,
    account_id: &str,
) -> Result<usize, String> {
    // Find message IDs that have existing reaction rows (excluding the __count__ metadata)
    // or were recently viewed. Limit to 60 to keep API cost bounded (3 batch calls max).
    let aid = account_id.to_string();
    let message_ids: Vec<String> = db
        .with_conn(move |conn| {
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
                .query_map(rusqlite::params![aid], |row| row.get::<_, String>(0))
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
    let expand_filter = format!(
        "$filter=id eq '{owner_reaction_id}' or id eq '{reactions_count_id}'"
    );

    // Look up the authenticated user's email for reaction rows
    let aid2 = account_id.to_string();
    let owner_email: String = db
        .with_conn(move |conn| {
            conn.query_row(
                "SELECT email FROM accounts WHERE id = ?1",
                rusqlite::params![aid2],
                |row| row.get(0),
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
            let batch_updated = db
                .with_conn(move |conn| {
                    let tx = conn
                        .unchecked_transaction()
                        .map_err(|e| format!("begin tx: {e}"))?;

                    let mut count: usize = 0;
                    for (msg_id, owner_reaction, reactions_count) in &reaction_updates {
                        if let Some(emoji) = owner_reaction {
                            let changes = tx
                                .execute(
                                    "INSERT INTO message_reactions \
                                     (message_id, account_id, reactor_email, reactor_name, \
                                      reaction_type, reacted_at, source) \
                                     VALUES (?1, ?2, ?3, NULL, ?4, NULL, 'exchange_native') \
                                     ON CONFLICT(message_id, account_id, reactor_email, reaction_type) \
                                     DO UPDATE SET reaction_type = ?4",
                                    rusqlite::params![msg_id, aid3, email, emoji],
                                )
                                .map_err(|e| format!("upsert reaction: {e}"))?;
                            if changes > 0 {
                                count += 1;
                            }
                        } else {
                            // Owner reaction was removed — delete the row if it exists
                            tx.execute(
                                "DELETE FROM message_reactions \
                                 WHERE message_id = ?1 AND account_id = ?2 \
                                   AND reactor_email = ?3 AND source = 'exchange_native'",
                                rusqlite::params![msg_id, aid3, email],
                            )
                            .map_err(|e| format!("delete removed reaction: {e}"))?;
                        }

                        if let Some(c) = reactions_count {
                            tx.execute(
                                "INSERT INTO message_reactions \
                                 (message_id, account_id, reactor_email, reactor_name, \
                                  reaction_type, reacted_at, source) \
                                 VALUES (?1, ?2, '__count__', NULL, ?3, NULL, 'exchange_native') \
                                 ON CONFLICT(message_id, account_id, reactor_email, reaction_type) \
                                 DO UPDATE SET reaction_type = ?3",
                                rusqlite::params![msg_id, aid3, c.to_string()],
                            )
                            .map_err(|e| format!("upsert reaction count: {e}"))?;
                        }
                    }

                    tx.commit().map_err(|e| format!("commit reaction refresh: {e}"))?;
                    Ok(count)
                })
                .await?;

            updated_count += batch_updated;
        }
    }

    Ok(updated_count)
}
