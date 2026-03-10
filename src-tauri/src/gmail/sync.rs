use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::body_store::{BodyStoreState, MessageBody};
use crate::db::DbState;
use crate::search::{SearchDocument, SearchState};

use super::client::GmailClient;
use super::parse::{ParsedGmailMessage, parse_gmail_message};
use super::types::GmailLabel;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a Gmail delta sync, returned to TS for post-sync hooks.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GmailSyncResult {
    pub new_inbox_message_ids: Vec<String>,
    pub affected_thread_ids: Vec<String>,
}

/// Progress event emitted during Gmail sync.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GmailSyncProgress {
    account_id: String,
    phase: String,
    current: u64,
    total: u64,
}

/// Bundle of shared state references to stay under the 7-arg clippy limit.
struct SyncCtx<'a> {
    client: &'a GmailClient,
    account_id: &'a str,
    db: &'a DbState,
    body_store: &'a BodyStoreState,
    search: &'a SearchState,
    app_handle: &'a AppHandle,
}

// ---------------------------------------------------------------------------
// Initial sync (public entry point)
// ---------------------------------------------------------------------------

/// Run initial Gmail sync: labels, thread list, parallel thread fetch.
#[allow(clippy::too_many_arguments)]
pub async fn gmail_initial_sync(
    client: &GmailClient,
    account_id: &str,
    days_back: i64,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<(), String> {
    let ctx = SyncCtx {
        client,
        account_id,
        db,
        body_store,
        search,
        app_handle,
    };
    run_initial_sync(&ctx, days_back).await
}

async fn run_initial_sync(ctx: &SyncCtx<'_>, days_back: i64) -> Result<(), String> {
    // Phase 1: Sync labels
    emit_progress(ctx, "labels", 0, 1);
    sync_labels(ctx).await?;
    emit_progress(ctx, "labels", 1, 1);

    // Phase 2: Paginated thread list
    let thread_ids = list_thread_ids(ctx, days_back).await?;

    // Phase 3: Parallel thread fetch + store
    let history_id = fetch_threads_parallel(ctx, &thread_ids, 10).await?;

    // Store history ID for delta sync
    if !history_id.is_empty() {
        let aid = ctx.account_id.to_string();
        ctx.db
            .with_conn(move |conn| update_account_history_id(conn, &aid, &history_id))
            .await?;
    }

    let total = thread_ids.len() as u64;
    emit_progress(ctx, "done", total, total);
    Ok(())
}

// ---------------------------------------------------------------------------
// Delta sync (public entry point)
// ---------------------------------------------------------------------------

/// Run delta Gmail sync via History API.
#[allow(clippy::too_many_arguments)]
pub async fn gmail_delta_sync(
    client: &GmailClient,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    app_handle: &AppHandle,
) -> Result<GmailSyncResult, String> {
    let ctx = SyncCtx {
        client,
        account_id,
        db,
        body_store,
        search,
        app_handle,
    };
    run_delta_sync(&ctx).await
}

async fn run_delta_sync(ctx: &SyncCtx<'_>) -> Result<GmailSyncResult, String> {
    // Read current history_id from account
    let last_history_id = {
        let aid = ctx.account_id.to_string();
        ctx.db
            .with_conn(move |conn| read_account_history_id(conn, &aid))
            .await?
    };
    let Some(last_history_id) = last_history_id else {
        return Err("No history_id found — run initial sync first".to_string());
    };

    // Paginate History API
    let history_result = collect_history(ctx, &last_history_id).await?;

    if history_result.affected_thread_ids.is_empty() {
        update_history_id(ctx, &history_result.latest_history_id).await?;
        return Ok(GmailSyncResult {
            new_inbox_message_ids: vec![],
            affected_thread_ids: vec![],
        });
    }

    // Filter out threads with pending local ops
    let thread_ids_to_sync = filter_pending_ops(ctx, &history_result.affected_thread_ids).await?;

    // Re-fetch affected threads in parallel (concurrency 5)
    if !thread_ids_to_sync.is_empty() {
        fetch_threads_parallel(ctx, &thread_ids_to_sync, 5).await?;
    }

    // Update history_id
    update_history_id(ctx, &history_result.latest_history_id).await?;

    Ok(GmailSyncResult {
        new_inbox_message_ids: history_result.new_inbox_message_ids.into_iter().collect(),
        affected_thread_ids: history_result.affected_thread_ids.into_iter().collect(),
    })
}

// ---------------------------------------------------------------------------
// Label sync
// ---------------------------------------------------------------------------

async fn sync_labels(ctx: &SyncCtx<'_>) -> Result<(), String> {
    let labels = ctx.client.list_labels(ctx.db).await?;

    let aid = ctx.account_id.to_string();
    ctx.db
        .with_conn(move |conn| persist_labels(conn, &aid, &labels))
        .await
}

fn persist_labels(
    conn: &rusqlite::Connection,
    account_id: &str,
    labels: &[GmailLabel],
) -> Result<(), String> {
    for label in labels {
        let color_bg = label.color.as_ref().map(|c| c.background_color.clone());
        let color_fg = label.color.as_ref().map(|c| c.text_color.clone());
        let label_type = label.label_type.as_deref().unwrap_or("user");

        conn.execute(
            "INSERT OR REPLACE INTO labels \
             (id, account_id, name, type, color_bg, color_fg) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                label.id, account_id, label.name, label_type, color_bg, color_fg,
            ],
        )
        .map_err(|e| format!("upsert label: {e}"))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Thread list (paginated)
// ---------------------------------------------------------------------------

async fn list_thread_ids(ctx: &SyncCtx<'_>, days_back: i64) -> Result<Vec<String>, String> {
    let after_date = chrono::Utc::now() - chrono::Duration::days(days_back);
    let after_str = format!(
        "{}/{}/{}",
        after_date.format("%Y"),
        after_date.format("%-m"),
        after_date.format("%-d")
    );
    let query = format!("after:{after_str}");

    let mut thread_ids = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let est = thread_ids.len() as u64 + if page_token.is_some() { 100 } else { 0 };
        emit_progress(ctx, "threads", thread_ids.len() as u64, est);

        let (stubs, next_page) = ctx
            .client
            .list_threads(Some(&query), Some(100), page_token.as_deref(), ctx.db)
            .await?;

        for stub in &stubs {
            thread_ids.push(stub.id.clone());
        }

        page_token = next_page;
        if page_token.is_none() {
            break;
        }
    }

    Ok(thread_ids)
}

// ---------------------------------------------------------------------------
// Parallel thread fetch + store
// ---------------------------------------------------------------------------

/// Fetch threads in parallel with a concurrency limit.
/// Returns the highest history_id seen.
///
/// Uses a worker-pool pattern similar to the TS `parallelLimit()`:
/// spawn `concurrency` workers that pull from a shared queue.
async fn fetch_threads_parallel(
    ctx: &SyncCtx<'_>,
    thread_ids: &[String],
    concurrency: usize,
) -> Result<String, String> {
    let highest_history = Arc::new(AtomicHistoryId::new());
    let progress = Arc::new(AtomicU64::new(0));
    let total = thread_ids.len() as u64;
    let index = Arc::new(AtomicU64::new(0));

    let mut workers = Vec::with_capacity(concurrency);
    for _ in 0..concurrency.min(thread_ids.len()) {
        workers.push(run_fetch_worker(
            ctx,
            thread_ids,
            &index,
            &highest_history,
            &progress,
            total,
        ));
    }

    futures::future::join_all(workers).await;

    Ok(highest_history.get())
}

async fn run_fetch_worker(
    ctx: &SyncCtx<'_>,
    thread_ids: &[String],
    index: &AtomicU64,
    highest_history: &AtomicHistoryId,
    progress: &AtomicU64,
    total: u64,
) {
    loop {
        let i = index.fetch_add(1, Ordering::Relaxed);
        if i >= thread_ids.len() as u64 {
            break;
        }

        let current = progress.fetch_add(1, Ordering::Relaxed) + 1;
        emit_progress(ctx, "messages", current, total);

        #[allow(clippy::cast_possible_truncation)]
        let tid = &thread_ids[i as usize];

        match process_single_thread(
            ctx.client,
            tid,
            ctx.account_id,
            ctx.db,
            ctx.body_store,
            ctx.search,
        )
        .await
        {
            Ok(h_id) => highest_history.update(&h_id),
            Err(e) => log::error!("Failed to sync thread {tid}: {e}"),
        }
    }
}

/// Fetch and store a single thread. Returns its history_id.
async fn process_single_thread(
    client: &GmailClient,
    thread_id: &str,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
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

    // Body store writes
    store_bodies(body_store, &parsed).await;

    // Search index writes
    index_messages(search, account_id, &parsed).await;

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

    // Now aggregate thread fields from ALL messages in the DB for this thread
    let message_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("count messages: {e}"))?;

    let is_read: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_read = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|unread| unread == 0)
        .map_err(|e| format!("check is_read: {e}"))?;

    let is_starred: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_starred = 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|starred| starred > 0)
        .map_err(|e| format!("check is_starred: {e}"))?;

    let has_attachments: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM attachments a \
             JOIN messages m ON a.message_id = m.id \
             WHERE m.thread_id = ?1 AND m.account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|e| format!("check has_attachments: {e}"))?;

    // Get snippet + date from the most recent message in the thread
    let (snippet, last_date): (String, i64) = tx
        .query_row(
            "SELECT COALESCE(snippet, ''), date FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 \
             ORDER BY date DESC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("get latest message: {e}"))?;

    // Get subject from the earliest message (thread subject)
    let subject: Option<String> = tx
        .query_row(
            "SELECT subject FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 \
             ORDER BY date ASC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("get subject: {e}"))?;

    let all_labels: HashSet<&str> = messages
        .iter()
        .flat_map(|m| m.label_ids.iter().map(String::as_str))
        .collect();
    let is_important = all_labels.contains("IMPORTANT");

    // Check if thread already exists to preserve fields like is_pinned, is_muted
    let exists: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .map_err(|e| format!("check thread exists: {e}"))?;

    if exists {
        tx.execute(
            "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
             message_count = ?4, is_read = ?5, is_starred = ?6, is_important = ?7, \
             has_attachments = ?8 \
             WHERE id = ?9 AND account_id = ?10",
            rusqlite::params![
                subject,
                snippet,
                last_date,
                message_count,
                is_read,
                is_starred,
                is_important,
                has_attachments,
                thread_id,
                account_id,
            ],
        )
        .map_err(|e| format!("update thread: {e}"))?;
    } else {
        tx.execute(
            "INSERT INTO threads \
             (id, account_id, subject, snippet, last_message_at, message_count, \
              is_read, is_starred, is_important, has_attachments) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                thread_id,
                account_id,
                subject,
                snippet,
                last_date,
                message_count,
                is_read,
                is_starred,
                is_important,
                has_attachments,
            ],
        )
        .map_err(|e| format!("insert thread: {e}"))?;
    }

    Ok(())
}

fn set_thread_labels(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGmailMessage],
) -> Result<(), String> {
    let all_labels: HashSet<&str> = messages
        .iter()
        .flat_map(|m| m.label_ids.iter().map(String::as_str))
        .collect();

    tx.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
        rusqlite::params![account_id, thread_id],
    )
    .map_err(|e| format!("delete thread labels: {e}"))?;

    for label_id in &all_labels {
        tx.execute(
            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, label_id],
        )
        .map_err(|e| format!("insert thread label: {e}"))?;
    }

    Ok(())
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
    let has_body = msg.body_html.is_some() || msg.body_text.is_some();

    tx.execute(
        "INSERT OR REPLACE INTO messages \
         (id, account_id, thread_id, from_address, from_name, to_addresses, \
          cc_addresses, bcc_addresses, reply_to, subject, snippet, date, \
          is_read, is_starred, body_html, body_text, raw_size, internal_date, \
          list_unsubscribe, list_unsubscribe_post, auth_results, \
          message_id_header, references_header, in_reply_to_header, body_cached) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                 ?13, ?14, NULL, NULL, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
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
            msg.auth_results,
            msg.message_id_header,
            msg.references_header,
            msg.in_reply_to_header,
            if has_body { 1i64 } else { 0i64 },
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
            let att_id = format!("{}_{}", msg.id, att.gmail_attachment_id);
            tx.execute(
                "INSERT OR REPLACE INTO attachments \
                 (id, message_id, account_id, filename, mime_type, size, \
                  gmail_attachment_id, content_id, is_inline) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    att_id,
                    msg.id,
                    account_id,
                    att.filename,
                    att.mime_type,
                    att.size,
                    att.gmail_attachment_id,
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
// Body store helper
// ---------------------------------------------------------------------------

async fn store_bodies(body_store: &BodyStoreState, messages: &[ParsedGmailMessage]) {
    let bodies: Vec<MessageBody> = messages
        .iter()
        .filter(|m| m.body_html.is_some() || m.body_text.is_some())
        .map(|m| MessageBody {
            message_id: m.id.clone(),
            body_html: m.body_html.clone(),
            body_text: m.body_text.clone(),
        })
        .collect();

    if bodies.is_empty() {
        return;
    }

    if let Err(e) = body_store.put_batch(bodies).await {
        log::warn!("Failed to store Gmail bodies: {e}");
    }
}

// ---------------------------------------------------------------------------
// Search index helper
// ---------------------------------------------------------------------------

async fn index_messages(search: &SearchState, account_id: &str, messages: &[ParsedGmailMessage]) {
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

    if let Err(e) = search.index_messages_batch(&docs).await {
        log::warn!("Failed to index Gmail messages: {e}");
    }
}

// ---------------------------------------------------------------------------
// History API (delta sync)
// ---------------------------------------------------------------------------

struct HistoryResult {
    affected_thread_ids: HashSet<String>,
    new_inbox_message_ids: HashSet<String>,
    latest_history_id: String,
}

/// Paginate History API and collect affected thread IDs.
async fn collect_history(
    ctx: &SyncCtx<'_>,
    start_history_id: &str,
) -> Result<HistoryResult, String> {
    let mut affected_thread_ids = HashSet::new();
    let mut new_inbox_message_ids = HashSet::new();
    let mut latest_history_id = start_history_id.to_string();
    let mut page_token: Option<String> = None;

    loop {
        let response = match ctx
            .client
            .get_history(start_history_id, page_token.as_deref(), ctx.db)
            .await
        {
            Ok(r) => r,
            Err(e) if is_history_expired(&e) => {
                return Err("HISTORY_EXPIRED".to_string());
            }
            Err(e) => return Err(e),
        };

        latest_history_id.clone_from(&response.history_id);

        for item in &response.history {
            collect_from_history_item(item, &mut affected_thread_ids, &mut new_inbox_message_ids);
        }

        page_token = response.next_page_token.clone();
        if page_token.is_none() {
            break;
        }
    }

    Ok(HistoryResult {
        affected_thread_ids,
        new_inbox_message_ids,
        latest_history_id,
    })
}

fn collect_from_history_item(
    item: &super::types::GmailHistoryItem,
    affected: &mut HashSet<String>,
    new_inbox: &mut HashSet<String>,
) {
    for added in &item.messages_added {
        affected.insert(added.message.thread_id.clone());
        let labels = &added.message.label_ids;
        if labels.contains(&"INBOX".to_string()) && labels.contains(&"UNREAD".to_string()) {
            new_inbox.insert(added.message.id.clone());
        }
    }
    for deleted in &item.messages_deleted {
        affected.insert(deleted.message.thread_id.clone());
    }
    for labeled in &item.labels_added {
        affected.insert(labeled.message.thread_id.clone());
    }
    for unlabeled in &item.labels_removed {
        affected.insert(unlabeled.message.thread_id.clone());
    }
}

fn is_history_expired(error: &str) -> bool {
    error.contains("404") || error.contains("historyId")
}

// ---------------------------------------------------------------------------
// Pending ops filter
// ---------------------------------------------------------------------------

/// Filter out thread IDs that have pending local operations.
async fn filter_pending_ops(
    ctx: &SyncCtx<'_>,
    thread_ids: &HashSet<String>,
) -> Result<Vec<String>, String> {
    let tids: Vec<String> = thread_ids.iter().cloned().collect();
    let aid = ctx.account_id.to_string();

    let skipped = ctx
        .db
        .with_conn(move |conn| check_pending_ops(conn, &aid, &tids))
        .await?;

    Ok(thread_ids
        .iter()
        .filter(|tid| !skipped.contains(*tid))
        .cloned()
        .collect())
}

fn check_pending_ops(
    conn: &rusqlite::Connection,
    account_id: &str,
    thread_ids: &[String],
) -> Result<HashSet<String>, String> {
    let mut skipped = HashSet::new();
    for tid in thread_ids {
        let count: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM pending_operations \
                 WHERE account_id = ?1 AND resource_id = ?2 AND status != 'failed'",
                rusqlite::params![account_id, tid],
                |row| row.get(0),
            )
            .map_err(|e| format!("check pending ops: {e}"))?;
        if count > 0 {
            log::info!("Skipping thread {tid}: has {count} pending local ops");
            skipped.insert(tid.clone());
        }
    }
    Ok(skipped)
}

// ---------------------------------------------------------------------------
// Account history_id helpers
// ---------------------------------------------------------------------------

fn update_account_history_id(
    conn: &rusqlite::Connection,
    account_id: &str,
    history_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET history_id = ?1 WHERE id = ?2",
        rusqlite::params![history_id, account_id],
    )
    .map_err(|e| format!("update history_id: {e}"))?;
    Ok(())
}

fn read_account_history_id(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT history_id FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| row.get(0),
    )
    .map_err(|e| format!("read history_id: {e}"))
}

async fn update_history_id(ctx: &SyncCtx<'_>, history_id: &str) -> Result<(), String> {
    let aid = ctx.account_id.to_string();
    let hid = history_id.to_string();
    ctx.db
        .with_conn(move |conn| update_account_history_id(conn, &aid, &hid))
        .await
}

// ---------------------------------------------------------------------------
// Progress event helpers
// ---------------------------------------------------------------------------

fn emit_progress(ctx: &SyncCtx<'_>, phase: &str, current: u64, total: u64) {
    let event = GmailSyncProgress {
        account_id: ctx.account_id.to_string(),
        phase: phase.to_string(),
        current,
        total,
    };
    if let Err(e) = ctx.app_handle.emit("gmail-sync-progress", &event) {
        log::warn!("Failed to emit gmail sync progress: {e}");
    }
}

// ---------------------------------------------------------------------------
// Atomic history ID tracker (thread-safe max)
// ---------------------------------------------------------------------------

struct AtomicHistoryId {
    value: std::sync::Mutex<String>,
}

impl AtomicHistoryId {
    fn new() -> Self {
        Self {
            value: std::sync::Mutex::new(String::from("0")),
        }
    }

    fn update(&self, new_id: &str) {
        let Ok(mut current) = self.value.lock() else {
            return;
        };
        let new_val = new_id.parse::<u64>().unwrap_or(0);
        let cur_val = current.parse::<u64>().unwrap_or(0);
        if new_val > cur_val {
            *current = new_id.to_string();
        }
    }

    fn get(&self) -> String {
        self.value
            .lock()
            .map(|v| v.clone())
            .unwrap_or_else(|_| String::from("0"))
    }
}
