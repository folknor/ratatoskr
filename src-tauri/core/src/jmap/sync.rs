use std::collections::{HashMap, HashSet};

use crate::db::{DbState, lookups};
use crate::progress::ProgressReporter;
use jmap_client::core::query::QueryResponse;
use jmap_client::email;
use jmap_client::mailbox::Role;
use serde::Serialize;

use crate::attachment_cache::hash_bytes;
use crate::body_store::BodyStoreState;
use crate::inline_image_store::{InlineImage, InlineImageStoreState, MAX_INLINE_SIZE};
use crate::search::{SearchDocument, SearchState};

use super::client::JmapClient;
use super::mailbox_mapper::{MailboxInfo, map_mailbox_to_label};
use super::parse::{ParsedJmapMessage, email_get_properties, parse_jmap_email};
use crate::sync::{
    pending as sync_pending, persistence as sync_persistence, progress as sync_progress,
    state as sync_state,
};

const BATCH_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a JMAP delta sync, returned to TS for post-sync hooks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JmapSyncResult {
    pub new_inbox_email_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}

/// Bundle of shared state references.
struct SyncCtx<'a> {
    client: &'a JmapClient,
    account_id: &'a str,
    db: &'a DbState,
    body_store: &'a BodyStoreState,
    inline_images: &'a InlineImageStoreState,
    search: &'a SearchState,
    progress: &'a dyn ProgressReporter,
}

// ---------------------------------------------------------------------------
// Initial sync
// ---------------------------------------------------------------------------

/// Initial JMAP sync: mailboxes → batched Email/query + Email/get → DB writes.
#[allow(clippy::too_many_arguments)]
pub async fn jmap_initial_sync(
    client: &JmapClient,
    account_id: &str,
    days_back: i64,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    progress: &dyn ProgressReporter,
) -> Result<(), String> {
    let ctx = SyncCtx {
        client,
        account_id,
        db,
        body_store,
        inline_images,
        search,
        progress,
    };

    emit_progress(&ctx, "mailboxes", 0, 1);

    // Phase 1: Sync mailboxes → labels
    let (mailbox_map, mailbox_data) = sync_mailboxes(&ctx).await?;

    // Save mailbox state
    let mailbox_state = get_mailbox_state(client).await?;
    save_sync_state(db, account_id, "Mailbox", &mailbox_state).await?;

    emit_progress(&ctx, "mailboxes", 1, 1);

    // Phase 2: Paginated Email/query → batched Email/get → DB writes
    let since = chrono::Utc::now() - chrono::Duration::days(days_back);
    let since_ts = since.timestamp();

    let mut total_u64: u64 = 0;
    let mut fetched: u64 = 0;
    let mut position: usize = 0;

    loop {
        emit_progress(&ctx, "messages", fetched, total_u64);

        let query_result = query_email_page(client, since_ts, position, position == 0).await?;

        if position == 0 {
            #[allow(clippy::cast_possible_truncation)]
            {
                total_u64 = query_result.total().unwrap_or(0) as u64;
            }
        }

        let ids = query_result.ids();
        if ids.is_empty() {
            break;
        }

        let batch_ids: Vec<&str> = ids.iter().map(String::as_str).collect();

        let emails = fetch_email_batch(client, &batch_ids).await?;
        let parsed = parse_email_batch(&emails, &mailbox_map)?;

        persist_messages(&ctx, &parsed, &mailbox_data).await?;

        #[allow(clippy::cast_possible_truncation)]
        {
            fetched += parsed.len() as u64;
        }
        position += ids.len();
        if ids.len() < BATCH_SIZE {
            break;
        }
    }

    // Save email state
    let email_state = get_email_state(client).await?;
    save_sync_state(db, account_id, "Email", &email_state).await?;
    let aid = account_id.to_string();
    db.with_conn(move |conn| crate::sync::pipeline::mark_initial_sync_completed(conn, &aid))
        .await?;

    emit_progress(&ctx, "done", fetched, total_u64);

    Ok(())
}

async fn query_email_page(
    client: &JmapClient,
    since_ts: i64,
    position: usize,
    calculate_total: bool,
) -> Result<QueryResponse, String> {
    let mut request = client.inner().build();
    let query_request = request.query_email();
    query_request.filter(email::query::Filter::after(since_ts));
    query_request.sort([email::query::Comparator::received_at()]);
    #[allow(clippy::cast_possible_wrap)]
    {
        query_request.position(position as i32);
    }
    query_request.limit(BATCH_SIZE);
    query_request.calculate_total(calculate_total);
    request
        .send_single::<QueryResponse>()
        .await
        .map_err(|e| format!("Email/query: {e}"))
}

// ---------------------------------------------------------------------------
// Delta sync
// ---------------------------------------------------------------------------

/// Delta JMAP sync: Email/changes + Mailbox/changes → DB writes.
///
/// Returns new inbox email IDs and affected thread IDs for TS post-sync hooks.
#[allow(clippy::too_many_arguments)]
pub async fn jmap_delta_sync(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    progress: &dyn ProgressReporter,
) -> Result<JmapSyncResult, String> {
    let ctx = SyncCtx {
        client,
        account_id,
        db,
        body_store,
        inline_images,
        search,
        progress,
    };

    // Load current sync states
    let email_state = load_sync_state(db, account_id, "Email").await?;
    let mailbox_state = load_sync_state(db, account_id, "Mailbox").await?;

    let Some(email_state) = email_state else {
        return Err("JMAP_NO_STATE".to_string());
    };

    // 1. Mailbox changes
    if let Some(mb_state) = &mailbox_state {
        sync_mailbox_changes(&ctx, mb_state).await?;
    }

    // Refresh mailbox map for email parsing
    let (mailbox_map, mailbox_data) = sync_mailboxes(&ctx).await?;

    // 2. Email changes
    let mut since_state = email_state;
    let mut new_inbox_ids = Vec::new();
    let mut affected_thread_ids = HashSet::new();

    loop {
        let changes = client
            .inner()
            .email_changes(&since_state, None)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("cannotCalculateChanges") {
                    return "JMAP_STATE_EXPIRED".to_string();
                }
                format!("Email/changes: {msg}")
            })?;

        let created = changes.created();
        let updated = changes.updated();
        let destroyed = changes.destroyed();

        // Batch-fetch created + updated emails
        let ids_to_fetch: Vec<&str> = created
            .iter()
            .chain(updated.iter())
            .map(String::as_str)
            .collect();

        if !ids_to_fetch.is_empty() {
            for chunk in ids_to_fetch.chunks(BATCH_SIZE) {
                let emails = fetch_email_batch(client, chunk).await?;
                let parsed = parse_email_batch(&emails, &mailbox_map)?;

                // Check pending_operations before persisting
                let filtered = filter_pending_ops(&ctx, parsed).await?;

                for msg in &filtered {
                    affected_thread_ids.insert(msg.thread_id.clone());
                    if msg.label_ids.contains(&"INBOX".to_string())
                        && created.iter().any(|c| c == &msg.id)
                    {
                        new_inbox_ids.push(msg.id.clone());
                    }
                }

                persist_messages(&ctx, &filtered, &mailbox_data).await?;
            }
        }

        // Delete destroyed emails
        if !destroyed.is_empty() {
            let destroyed_refs: Vec<&str> = destroyed.iter().map(String::as_str).collect();
            delete_messages(&ctx, &destroyed_refs).await?;
        }

        since_state = changes.new_state().to_string();

        if !changes.has_more_changes() {
            break;
        }
    }

    // Save updated states
    save_sync_state(db, account_id, "Email", &since_state).await?;

    Ok(JmapSyncResult {
        new_inbox_email_ids: new_inbox_ids,
        affected_thread_ids: affected_thread_ids.into_iter().collect(),
    })
}

// ---------------------------------------------------------------------------
// Mailbox sync helpers
// ---------------------------------------------------------------------------

/// Fetch all mailboxes, persist as labels, return (mailbox_map, mailbox_data).
async fn sync_mailboxes(
    ctx: &SyncCtx<'_>,
) -> Result<
    (
        HashMap<String, MailboxInfo>,
        Vec<(String, Option<String>, String)>,
    ),
    String,
> {
    let mailboxes = fetch_all_mailboxes(ctx.client).await?;

    let mut mailbox_map = HashMap::new();
    let mut mailbox_data = Vec::new();

    let aid = ctx.account_id.to_string();
    let mut label_rows: Vec<(String, String, String, String)> = Vec::new();

    for mb in &mailboxes {
        let Some(id) = mb.id() else { continue };
        let name = mb.name().unwrap_or("(unnamed)");
        let role = mb.role();
        let role_str = if role == Role::None {
            None
        } else {
            Some(role_to_str(&role))
        };

        mailbox_map.insert(
            id.to_string(),
            MailboxInfo {
                role: role_str.map(String::from),
                name: name.to_string(),
            },
        );

        mailbox_data.push((id.to_string(), role_str.map(String::from), name.to_string()));

        let mapping = map_mailbox_to_label(role_str, id, name);
        label_rows.push((
            mapping.label_id,
            aid.clone(),
            mapping.label_name,
            mapping.label_type.to_string(),
        ));
    }

    // Also add pseudo-labels
    label_rows.push((
        "UNREAD".to_string(),
        aid.clone(),
        "Unread".to_string(),
        "system".to_string(),
    ));

    // Persist labels + categories to DB
    let category_rows: Vec<(String, String)> = label_rows
        .iter()
        .filter(|(_, _, _, lt)| lt == "user")
        .map(|(id, _, name, _)| (id.clone(), name.clone()))
        .collect();

    ctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            for (label_id, account_id, name, label_type) in &label_rows {
                tx.execute(
                    "INSERT OR REPLACE INTO labels (id, account_id, name, type) \
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![label_id, account_id, name, label_type],
                )
                .map_err(|e| format!("upsert label: {e}"))?;
            }
            // Sync user mailboxes into categories table (no colors in JMAP)
            for (i, (provider_id, name)) in category_rows.iter().enumerate() {
                tx.execute(
                    "INSERT INTO categories \
                     (id, account_id, display_name, color_preset, color_bg, color_fg, \
                      provider_id, sync_state, sort_order) \
                     VALUES (?1, ?2, ?3, NULL, NULL, NULL, ?4, 'synced', ?5) \
                     ON CONFLICT(account_id, display_name) DO UPDATE SET \
                       provider_id = ?4, sync_state = 'synced'",
                    rusqlite::params![provider_id, aid, name, provider_id, i as i64],
                )
                .map_err(|e| format!("upsert jmap category: {e}"))?;
            }
            tx.commit().map_err(|e| format!("commit labels: {e}"))?;
            Ok(())
        })
        .await?;

    Ok((mailbox_map, mailbox_data))
}

/// Handle Mailbox/changes during delta sync.
async fn sync_mailbox_changes(ctx: &SyncCtx<'_>, since_state: &str) -> Result<(), String> {
    let result = ctx.client.inner().mailbox_changes(since_state, 500).await;

    match result {
        Ok(changes) => {
            let new_state = changes.new_state().to_string();
            if new_state != since_state {
                // State changed — re-sync all mailboxes
                sync_mailboxes(ctx).await?;
                save_sync_state(ctx.db, ctx.account_id, "Mailbox", &new_state).await?;
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("cannotCalculateChanges") {
                // Full mailbox refresh
                let (_, _) = sync_mailboxes(ctx).await?;
                let new_state = get_mailbox_state(ctx.client).await?;
                save_sync_state(ctx.db, ctx.account_id, "Mailbox", &new_state).await?;
            } else {
                return Err(format!("Mailbox/changes: {msg}"));
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Email fetch + parse helpers
// ---------------------------------------------------------------------------

/// Fetch a batch of emails by ID with all properties + body values.
async fn fetch_email_batch(
    client: &JmapClient,
    ids: &[&str],
) -> Result<Vec<jmap_client::email::Email>, String> {
    // Use request builder to specify all needed properties + body values
    let mut request = client.inner().build();
    let get_req = request.get_email();
    get_req.ids(ids.iter().copied());
    get_req.properties(email_get_properties());
    get_req.arguments().fetch_text_body_values(true);
    get_req.arguments().fetch_html_body_values(true);

    let response = request
        .send()
        .await
        .map_err(|e| format!("Email/get batch: {e}"))?;

    response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_email().ok())
        .map(|mut r| r.take_list())
        .ok_or_else(|| "No Email/get response".to_string())
}

/// Parse a batch of emails into our internal structs.
fn parse_email_batch(
    emails: &[jmap_client::email::Email],
    mailbox_map: &HashMap<String, MailboxInfo>,
) -> Result<Vec<ParsedJmapMessage>, String> {
    let mut results = Vec::with_capacity(emails.len());
    for email in emails {
        match parse_jmap_email(email, mailbox_map) {
            Ok(parsed) => results.push(parsed),
            Err(e) => log::warn!("Failed to parse JMAP email: {e}"),
        }
    }
    Ok(results)
}

// ---------------------------------------------------------------------------
// Pending operations filter (sync vs queue coordination)
// ---------------------------------------------------------------------------

/// Filter out messages whose thread has pending operations.
///
/// This prevents sync from overwriting optimistic local state applied by
/// the TS queue processor.
async fn filter_pending_ops(
    ctx: &SyncCtx<'_>,
    messages: Vec<ParsedJmapMessage>,
) -> Result<Vec<ParsedJmapMessage>, String> {
    if messages.is_empty() {
        return Ok(messages);
    }

    let thread_ids: HashSet<String> = messages
        .iter()
        .map(|message| message.thread_id.clone())
        .collect();
    let blocked_threads =
        sync_pending::blocked_thread_ids(ctx.db, ctx.account_id, thread_ids.into_iter().collect())
            .await?;

    if blocked_threads.is_empty() {
        return Ok(messages);
    }

    log::info!(
        "JMAP delta sync: skipping {} threads with pending operations",
        blocked_threads.len()
    );

    Ok(sync_pending::filter_by_blocked_threads(
        messages,
        &blocked_threads,
        |message| &message.thread_id,
    ))
}

// ---------------------------------------------------------------------------
// DB persistence
// ---------------------------------------------------------------------------

/// Persist parsed messages to DB, body store, and search index.
async fn persist_messages(
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
        threads.entry(&msg.thread_id).or_default().push(msg);
    }

    // 1. DB writes (metadata + thread aggregation)
    let aid = ctx.account_id.to_string();
    let thread_groups: Vec<(String, Vec<ParsedJmapMessage>)> = threads
        .into_iter()
        .map(|(tid, msgs)| (tid.to_string(), msgs.into_iter().cloned().collect()))
        .collect();

    ctx.db
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
        store_bodies(ctx.body_store, messages),
        store_inline_images(ctx, messages),
        index_messages(ctx.search, ctx.account_id, messages),
        crate::seen_addresses::ingest_from_messages(ctx.db, ctx.account_id, messages),
    );

    Ok(())
}

/// Delete messages from DB, body store, and search index.
/// Also updates or removes parent threads as needed.
async fn delete_messages(ctx: &SyncCtx<'_>, message_ids: &[&str]) -> Result<(), String> {
    let aid = ctx.account_id.to_string();
    let ids: Vec<String> = message_ids.iter().map(|s| (*s).to_string()).collect();

    // Delete from DB and update parent threads
    ctx.db
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
    let body_ids: Vec<String> = message_ids.iter().map(|s| (*s).to_string()).collect();
    if let Err(e) = ctx.body_store.delete(body_ids).await {
        log::warn!("Failed to delete JMAP bodies: {e}");
    }

    // Delete from search index (batch — single commit)
    if let Err(e) = ctx.search.delete_messages_batch(message_ids).await {
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
// DB write helpers (mirrors gmail/sync.rs patterns)
// ---------------------------------------------------------------------------

fn store_thread_to_db(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedJmapMessage],
) -> Result<(), String> {
    // upsert_thread_record calls upsert_messages internally before aggregating
    upsert_attachments(tx, account_id, messages)?;
    upsert_thread_record(tx, account_id, thread_id, messages)?;
    set_thread_labels(tx, account_id, thread_id, messages)?;
    sync_keyword_categories(tx, account_id, thread_id, messages)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn upsert_thread_record(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedJmapMessage],
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
    messages: &[ParsedJmapMessage],
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
    messages: &[ParsedJmapMessage],
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
              mdn_requested) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                     ?13, ?14, ?15, ?16, ?17, ?18, NULL, ?19, ?20, ?21, ?22, ?23)",
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
                msg.raw_size,
                msg.internal_date,
                msg.list_unsubscribe,
                msg.list_unsubscribe_post,
                msg.message_id_header,
                msg.references_header,
                msg.in_reply_to_header,
                if has_body { 1i64 } else { 0i64 },
                msg.mdn_requested,
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
            let att_id = format!("{}_{}", msg.id, att.blob_id);
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
                    msg.id,
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
// Keyword → category sync
// ---------------------------------------------------------------------------

/// Ensure non-system JMAP keywords exist in the `categories` table and link
/// the thread to the first keyword category found across its messages.
///
/// JMAP keywords have no color metadata, so colors are left NULL.
/// Only updates `thread_categories` when the existing assignment is not manual.
fn sync_keyword_categories(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedJmapMessage],
) -> Result<(), String> {
    // Collect all unique keyword categories across messages in this thread
    let mut unique_keywords: Vec<String> = messages
        .iter()
        .flat_map(|msg| msg.keyword_categories.iter().cloned())
        .collect();
    unique_keywords.sort();
    unique_keywords.dedup();

    if unique_keywords.is_empty() {
        return Ok(());
    }

    // Upsert each keyword into the categories table
    for keyword in &unique_keywords {
        tx.execute(
            "INSERT INTO categories \
             (id, account_id, display_name, color_preset, color_bg, color_fg, \
              provider_id, sync_state, sort_order) \
             VALUES (?1, ?2, ?3, NULL, NULL, NULL, ?4, 'synced', 0) \
             ON CONFLICT(account_id, display_name) DO UPDATE SET \
               provider_id = ?4, sync_state = 'synced'",
            rusqlite::params![
                format!("kw_{keyword}"),
                account_id,
                keyword,
                keyword,
            ],
        )
        .map_err(|e| format!("upsert jmap keyword category: {e}"))?;
    }

    // Link thread to the first keyword category (don't overwrite manual assignments)
    let first_category = &unique_keywords[0];
    tx.execute(
        "INSERT INTO thread_categories (account_id, thread_id, category, is_manual) \
         VALUES (?1, ?2, ?3, 0) \
         ON CONFLICT(account_id, thread_id) DO UPDATE SET \
           category = ?3 \
         WHERE is_manual = 0",
        rusqlite::params![account_id, thread_id, first_category],
    )
    .map_err(|e| format!("upsert jmap thread keyword category: {e}"))?;

    // Link each message to its keyword categories
    sync_persistence::insert_message_categories(
        tx,
        account_id,
        messages.iter().flat_map(|msg| {
            msg.keyword_categories
                .iter()
                .map(move |kw| (msg.id.as_str(), kw.as_str()))
        }),
    )?;

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
        |message| &message.id,
        |message| message.body_html.as_ref(),
        |message| message.body_text.as_ref(),
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
                    format!("{}_{}", msg.id, att.blob_id),
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
    let mut unique_blobs: HashMap<String, String> = HashMap::new(); // blob_id → mime_type
    for (_, blob_id, mime_type) in &eligible {
        unique_blobs.entry(blob_id.clone()).or_insert_with(|| mime_type.clone());
    }

    // Download unique blobs in parallel with bounded concurrency
    let blob_cache: HashMap<String, (String, Vec<u8>, String)> = stream::iter(unique_blobs)
        .map(|(blob_id, mime_type)| async move {
            match ctx.client.inner().download(&blob_id).await {
                Ok(data) if data.len() <= MAX_INLINE_SIZE => {
                    let content_hash = hash_bytes(&data);
                    Some((blob_id, (content_hash, data, mime_type)))
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

    // Build attachment → content_hash mapping for DB updates
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
            message_id: m.id.clone(),
            account_id: account_id.to_string(),
            thread_id: m.thread_id.clone(),
            subject: m.subject.clone(),
            from_name: m.from_name.clone(),
            from_address: m.from_address.clone(),
            to_addresses: m.to_addresses.clone(),
            body_text: m.body_text.clone(),
            snippet: Some(m.snippet.clone()),
            date: m.date / 1000, // tantivy expects seconds
            is_read: m.is_read,
            is_starred: m.is_starred,
            has_attachment: m.has_attachments,
        })
        .collect();

    sync_persistence::index_search_documents(search, docs, "JMAP").await;
}

// ---------------------------------------------------------------------------
// Sync state persistence (jmap_sync_state table)
// ---------------------------------------------------------------------------

async fn save_sync_state(
    db: &DbState,
    account_id: &str,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    sync_state::save_jmap_sync_state(db, account_id, state_type, state).await
}

async fn load_sync_state(
    db: &DbState,
    account_id: &str,
    state_type: &str,
) -> Result<Option<String>, String> {
    sync_state::load_jmap_sync_state(db, account_id, state_type).await
}

// ---------------------------------------------------------------------------
// JMAP state getters
// ---------------------------------------------------------------------------

async fn get_mailbox_state(client: &JmapClient) -> Result<String, String> {
    // Fetch mailboxes to get the state string
    let mut request = client.inner().build();
    request.get_mailbox();
    let response = request
        .send()
        .await
        .map_err(|e| format!("Mailbox state: {e}"))?;

    response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_mailbox().ok())
        .map(|r| r.state().to_string())
        .ok_or_else(|| "No Mailbox/get response for state".to_string())
}

async fn get_email_state(client: &JmapClient) -> Result<String, String> {
    let mut request = client.inner().build();
    let get_req = request.get_email();
    get_req.ids(std::iter::empty::<&str>());

    let response = request
        .send()
        .await
        .map_err(|e| format!("Email state: {e}"))?;

    response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_email().ok())
        .map(|r| r.state().to_string())
        .ok_or_else(|| "No Email/get response for state".to_string())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Fetch all mailboxes using the builder pattern (no filter = all mailboxes).
pub async fn fetch_all_mailboxes(
    client: &JmapClient,
) -> Result<Vec<jmap_client::mailbox::Mailbox<jmap_client::Get>>, String> {
    let mut request = client.inner().build();
    request.get_mailbox();
    let response = request
        .send()
        .await
        .map_err(|e| format!("Mailbox/get: {e}"))?;

    Ok(response
        .unwrap_method_responses()
        .pop()
        .and_then(|r| r.unwrap_get_mailbox().ok())
        .map(|mut r| r.take_list())
        .unwrap_or_default())
}

fn emit_progress(ctx: &SyncCtx<'_>, phase: &str, current: u64, total: u64) {
    sync_progress::emit_sync_progress(
        ctx.progress,
        "jmap-sync-progress",
        ctx.account_id,
        phase,
        current,
        total,
        None,
    );
}

pub fn role_to_str(role: &jmap_client::mailbox::Role) -> &'static str {
    use jmap_client::mailbox::Role;
    match role {
        Role::Inbox => "inbox",
        Role::Archive => "archive",
        Role::Drafts => "drafts",
        Role::Sent => "sent",
        Role::Trash => "trash",
        Role::Junk => "junk",
        Role::Important => "important",
        _ => "other",
    }
}
