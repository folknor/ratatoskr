use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::Serialize;

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::inline_image_store::{InlineImage, InlineImageStoreState};
use crate::progress::ProgressReporter;
use crate::search::{SearchDocument, SearchState};

use super::client::GmailClient;
use super::parse::{ParsedGmailMessage, parse_gmail_message};
use super::types::GmailLabel;
use crate::sync::{
    pending as sync_pending, persistence as sync_persistence, progress as sync_progress,
    state as sync_state,
};

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

/// Bundle of shared state references to stay under the 7-arg clippy limit.
struct SyncCtx<'a> {
    client: &'a GmailClient,
    account_id: &'a str,
    db: &'a DbState,
    body_store: &'a BodyStoreState,
    inline_images: &'a InlineImageStoreState,
    search: &'a SearchState,
    progress: &'a dyn ProgressReporter,
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
        sync_state::save_account_history_id(ctx.db, ctx.account_id, &history_id).await?;
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
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    progress: &dyn ProgressReporter,
) -> Result<GmailSyncResult, String> {
    let ctx = SyncCtx {
        client,
        account_id,
        db,
        body_store,
        inline_images,
        search,
        progress,
    };
    run_delta_sync(&ctx).await
}

async fn run_delta_sync(ctx: &SyncCtx<'_>) -> Result<GmailSyncResult, String> {
    // Read current history_id from account
    let last_history_id = { sync_state::load_account_history_id(ctx.db, ctx.account_id).await? };
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
            ctx.inline_images,
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
        crate::seen_addresses::ingest_from_messages(db, account_id, &parsed),
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
    messages: &[ParsedGmailMessage],
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
          is_read, is_starred, raw_size, internal_date, \
          list_unsubscribe, list_unsubscribe_post, auth_results, \
          message_id_header, references_header, in_reply_to_header, body_cached, \
          mdn_requested) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                 ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24)",
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
            msg.mdn_requested,
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

// ---------------------------------------------------------------------------
// Body store helper
// ---------------------------------------------------------------------------

async fn store_bodies(body_store: &BodyStoreState, messages: &[ParsedGmailMessage]) {
    sync_persistence::store_message_bodies(
        body_store,
        messages,
        "Gmail",
        |message| &message.id,
        |message| message.body_html.as_ref(),
        |message| message.body_text.as_ref(),
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

    sync_persistence::index_search_documents(search, docs, "Gmail").await;
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
    let skipped = sync_pending::blocked_thread_ids(ctx.db, ctx.account_id, tids).await?;

    Ok(thread_ids
        .iter()
        .filter(|thread_id| !skipped.contains(*thread_id))
        .cloned()
        .collect())
}

async fn update_history_id(ctx: &SyncCtx<'_>, history_id: &str) -> Result<(), String> {
    sync_state::save_account_history_id(ctx.db, ctx.account_id, history_id).await
}

// ---------------------------------------------------------------------------
// Progress event helpers
// ---------------------------------------------------------------------------

fn emit_progress(ctx: &SyncCtx<'_>, phase: &str, current: u64, total: u64) {
    sync_progress::emit_sync_progress(
        ctx.progress,
        "gmail-sync-progress",
        ctx.account_id,
        phase,
        current,
        total,
        None,
    );
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
