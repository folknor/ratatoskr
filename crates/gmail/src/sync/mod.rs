mod delta;
mod labels;
mod storage;

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use ratatoskr_stores::body_store::BodyStoreState;
use ratatoskr_db::db::DbState;
use ratatoskr_stores::inline_image_store::InlineImageStoreState;
use ratatoskr_db::progress::ProgressReporter;
use ratatoskr_search::SearchState;

use super::client::GmailClient;
use ratatoskr_sync::{progress as sync_progress, state as sync_state};

pub(crate) use delta::GmailSyncResult;

// ---------------------------------------------------------------------------
// Shared context (crate-visible for submodules)
// ---------------------------------------------------------------------------

/// Bundle of shared state references to stay under the 7-arg clippy limit.
pub(crate) struct SyncCtx<'a> {
    pub client: &'a GmailClient,
    pub account_id: &'a str,
    pub db: &'a DbState,
    pub body_store: &'a BodyStoreState,
    pub inline_images: &'a InlineImageStoreState,
    pub search: &'a SearchState,
    pub progress: &'a dyn ProgressReporter,
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
    labels::sync_labels(ctx).await?;
    emit_progress(ctx, "labels", 1, 1);

    // Phase 1b: Sync signatures from sendAs aliases
    labels::sync_signatures(ctx).await?;

    // Phase 2: Paginated thread list
    let thread_ids = list_thread_ids(ctx, days_back).await?;

    // Phase 3: Parallel thread fetch + store
    let history_id = fetch_threads_parallel(ctx, &thread_ids, 10).await?;

    // Store history ID for delta sync
    if !history_id.is_empty() {
        sync_state::save_account_history_id(ctx.db, ctx.account_id, &history_id).await?;
    }

    // Phase 4: Sync Google contacts (non-fatal)
    if let Err(e) =
        super::contacts::sync_google_contacts(ctx.client, ctx.account_id, ctx.db).await
    {
        log::warn!("Google contacts initial sync failed (non-fatal): {e}");
    }

    // Phase 4b: Sync Google otherContacts (non-fatal)
    if let Err(e) =
        super::contacts::sync_google_other_contacts(ctx.client, ctx.account_id, ctx.db).await
    {
        log::warn!("Google otherContacts initial sync failed (non-fatal): {e}");
    }

    let total = thread_ids.len() as u64;
    emit_progress(ctx, "done", total, total);
    Ok(())
}

// ---------------------------------------------------------------------------
// Delta sync (public entry point)
// ---------------------------------------------------------------------------

/// Run delta Gmail sync via History API.
///
/// Note on reactions: Gmail reactions appear as new messages with a special
/// MIME type (detected by `is_reaction` / `reaction_emoji` during parsing).
/// Because they are actual messages, they show up in `history.list` like any
/// other new message and are handled by the normal incremental sync path —
/// no special reaction polling is needed (unlike Exchange, where reactions
/// update extended properties without changing `lastModifiedDateTime`).
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
    delta::run_delta_sync(&ctx).await
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
pub(crate) async fn fetch_threads_parallel(
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

        match storage::process_single_thread(
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

// ---------------------------------------------------------------------------
// Progress event helpers
// ---------------------------------------------------------------------------

pub(crate) fn emit_progress(ctx: &SyncCtx<'_>, phase: &str, current: u64, total: u64) {
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
