use std::collections::HashSet;

use serde::Serialize;

use sync::{pending as sync_pending, state as sync_state};

use super::SyncCtx;
use super::labels;

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

// ---------------------------------------------------------------------------
// Delta sync implementation
// ---------------------------------------------------------------------------

pub(super) async fn run_delta_sync(ctx: &SyncCtx<'_>) -> Result<GmailSyncResult, String> {
    log::info!("[Gmail] Starting delta sync for account {}", ctx.account_id);
    // Read current history_id from account
    let last_history_id = { sync_state::load_account_history_id(ctx.db, ctx.account_id).await? };
    let Some(last_history_id) = last_history_id else {
        log::error!("[Gmail] No history_id found for account {} — run initial sync first", ctx.account_id);
        return Err("No history_id found — run initial sync first".to_string());
    };
    log::debug!("[Gmail] Delta sync from history_id={last_history_id}");

    let cycle = ctx.client.increment_sync_cycle();

    // Sync signatures on each delta (lightweight — single API call)
    labels::sync_signatures(ctx).await?;

    // Calendar delta sync: every 5th cycle (calendar events change moderately)
    if cycle.is_multiple_of(5)
        && let Err(e) =
            super::super::calendar::sync_calendars(ctx.client, ctx.account_id, ctx.db).await
    {
        log::warn!("Google Calendar delta sync failed (non-fatal): {e}");
    }

    // Contacts delta sync: every 20th cycle (contacts change rarely)
    if cycle.is_multiple_of(20) {
        if let Err(e) =
            super::super::contacts::sync_google_contacts(ctx.client, ctx.account_id, ctx.db).await
        {
            log::warn!("Google contacts delta sync failed (non-fatal): {e}");
        }
        if let Err(e) =
            super::super::contacts::sync_google_other_contacts(ctx.client, ctx.account_id, ctx.db)
                .await
        {
            log::warn!("Google otherContacts delta sync failed (non-fatal): {e}");
        }
    }

    // Paginate History API
    let history_result = collect_history(ctx, &last_history_id).await?;

    if history_result.affected_thread_ids.is_empty() {
        log::info!("[Gmail] Delta sync complete for account {}: no changes", ctx.account_id);
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
        super::fetch_threads_parallel(ctx, &thread_ids_to_sync, 5).await?;
    }

    // Update history_id
    update_history_id(ctx, &history_result.latest_history_id).await?;

    log::info!(
        "[Gmail] Delta sync complete for account {}: {} threads affected, {} new inbox messages",
        ctx.account_id,
        history_result.affected_thread_ids.len(),
        history_result.new_inbox_message_ids.len()
    );

    Ok(GmailSyncResult {
        new_inbox_message_ids: history_result.new_inbox_message_ids.into_iter().collect(),
        affected_thread_ids: history_result.affected_thread_ids.into_iter().collect(),
    })
}

// ---------------------------------------------------------------------------
// History API
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
                log::warn!("[Gmail] History expired for account {}, full re-sync needed", ctx.account_id);
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
    item: &crate::types::GmailHistoryItem,
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
