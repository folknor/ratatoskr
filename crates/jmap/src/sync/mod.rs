mod mailbox;
mod storage;

use std::collections::{HashMap, HashSet};

use jmap_client::core::query::QueryResponse;
use jmap_client::email;
use serde::Serialize;

use ratatoskr_stores::body_store::BodyStoreState;
use ratatoskr_db::db::DbState;
use ratatoskr_stores::inline_image_store::InlineImageStoreState;
use ratatoskr_db::progress::ProgressReporter;
use ratatoskr_search::SearchState;

use super::client::JmapClient;
use super::mailbox_mapper::MailboxInfo;
use super::parse::{ParsedJmapMessage, email_get_properties, parse_jmap_email};
use ratatoskr_sync::{
    pending as sync_pending, progress as sync_progress,
    state as sync_state,
};

const BATCH_SIZE: usize = 50;

// Re-export public items
pub(crate) use mailbox::{fetch_all_mailboxes, role_to_str};

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
pub(crate) struct SyncCtx<'a> {
    pub client: &'a JmapClient,
    pub account_id: &'a str,
    pub db: &'a DbState,
    pub body_store: &'a BodyStoreState,
    pub inline_images: &'a InlineImageStoreState,
    pub search: &'a SearchState,
    pub progress: &'a dyn ProgressReporter,
}

// ---------------------------------------------------------------------------
// Initial sync
// ---------------------------------------------------------------------------

/// Initial JMAP sync: mailboxes -> batched Email/query + Email/get -> DB writes.
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

    log::info!("[JMAP] Starting initial sync for account {account_id} (days_back={days_back})");
    emit_progress(&ctx, "mailboxes", 0, 1);

    // Phase 1: Sync mailboxes -> labels
    let (mailbox_map, mailbox_data) = mailbox::sync_mailboxes(&ctx).await?;

    // Save mailbox state
    let mailbox_state = mailbox::get_mailbox_state(client).await?;
    save_sync_state(db, account_id, "Mailbox", &mailbox_state).await?;

    emit_progress(&ctx, "mailboxes", 1, 1);

    // Phase 2: Paginated Email/query -> batched Email/get -> DB writes
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

        storage::persist_messages(&ctx, &parsed, &mailbox_data).await?;

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
    let email_state = mailbox::get_email_state(client).await?;
    save_sync_state(db, account_id, "Email", &email_state).await?;
    let aid = account_id.to_string();
    db.with_conn(move |conn| ratatoskr_sync::pipeline::mark_initial_sync_completed(conn, &aid))
        .await?;

    log::info!("[JMAP] Initial sync complete for account {account_id}: {fetched} messages synced");

    // Phase 3: Contacts sync
    match super::contacts_sync::jmap_contacts_initial_sync(client, account_id, db).await {
        Ok(count) => log::info!("[JMAP] Initial contacts sync: {count} contacts for account {account_id}"),
        Err(e) => log::warn!("[JMAP] Contacts initial sync failed for account {account_id}: {e}"),
    }

    // Phase 4: Calendar sync
    match super::calendar_sync::sync_calendars(client, account_id, db).await {
        Ok(()) => log::info!("[JMAP] Initial calendar sync complete for account {account_id}"),
        Err(e) => log::warn!("[JMAP] Calendar initial sync failed for account {account_id}: {e}"),
    }

    emit_progress(&ctx, "done", fetched, total_u64);

    Ok(())
}

async fn query_email_page(
    client: &JmapClient,
    since_ts: i64,
    position: usize,
    calculate_total: bool,
) -> Result<QueryResponse, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = request.default_account_id().to_string();
    let mut query = email::EmailQuery::new(&account_id);
    query.filter(email::query::Filter::after(since_ts));
    query.sort([email::query::Comparator::received_at()]);
    query.position(i32::try_from(position).unwrap_or(i32::MAX));
    query.limit(BATCH_SIZE);
    query.calculate_total(calculate_total);
    let handle = request
        .call(query)
        .map_err(|e| format!("Email/query: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Email/query: {e}"))?;
    response
        .get(&handle)
        .map_err(|e| format!("Email/query: {e}"))
}

// ---------------------------------------------------------------------------
// Delta sync
// ---------------------------------------------------------------------------

/// Delta JMAP sync: Email/changes + Mailbox/changes -> DB writes.
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

    log::info!("[JMAP] Starting delta sync for account {account_id}");
    let Some(email_state) = email_state else {
        log::error!("[JMAP] No email state for account {account_id} — run initial sync first");
        return Err("JMAP_NO_STATE".to_string());
    };

    // 1. Mailbox changes
    if let Some(mb_state) = &mailbox_state {
        mailbox::sync_mailbox_changes(&ctx, mb_state).await?;
    }

    // Refresh mailbox map for email parsing
    let (mailbox_map, mailbox_data) = mailbox::sync_mailboxes(&ctx).await?;

    // 2. Email changes
    let mut since_state = email_state;
    let mut new_inbox_ids = Vec::new();
    let mut affected_thread_ids = HashSet::new();

    loop {
        let inner = client.inner();
        let changes = inner
            .email_changes(&since_state, None)
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("cannotCalculateChanges") {
                    log::warn!("[JMAP] Email state expired for account {account_id}, full re-sync needed");
                    return "JMAP_STATE_EXPIRED".to_string();
                }
                log::error!("[JMAP] Email/changes failed for account {account_id}: {msg}");
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
                    affected_thread_ids.insert(msg.base.thread_id.clone());
                    if msg.base.label_ids.contains(&"INBOX".to_string())
                        && created.iter().any(|c| c == &msg.base.id)
                    {
                        new_inbox_ids.push(msg.base.id.clone());
                    }
                }

                storage::persist_messages(&ctx, &filtered, &mailbox_data).await?;
            }
        }

        // Delete destroyed emails
        if !destroyed.is_empty() {
            let destroyed_refs: Vec<&str> = destroyed.iter().map(String::as_str).collect();
            storage::delete_messages(&ctx, &destroyed_refs).await?;
        }

        since_state = changes.new_state().to_string();

        if !changes.has_more_changes() {
            break;
        }
    }

    // Save updated states
    save_sync_state(db, account_id, "Email", &since_state).await?;

    // 3. Contacts delta sync
    match super::contacts_sync::jmap_contacts_delta_sync(client, account_id, db).await {
        Ok(count) => {
            if count > 0 {
                log::info!("[JMAP] Contacts delta sync: {count} affected for account {account_id}");
            }
        }
        Err(e) => log::warn!("[JMAP] Contacts delta sync failed for account {account_id}: {e}"),
    }

    // 4. Calendar delta sync
    match super::calendar_sync::sync_calendars(client, account_id, db).await {
        Ok(()) => log::debug!("[JMAP] Calendar delta sync complete for account {account_id}"),
        Err(e) => log::warn!("[JMAP] Calendar delta sync failed for account {account_id}: {e}"),
    }

    log::info!(
        "[JMAP] Delta sync complete for account {account_id}: {} new inbox, {} threads affected",
        new_inbox_ids.len(), affected_thread_ids.len()
    );

    Ok(JmapSyncResult {
        new_inbox_email_ids: new_inbox_ids,
        affected_thread_ids: affected_thread_ids.into_iter().collect(),
    })
}

// ---------------------------------------------------------------------------
// Email fetch + parse helpers
// ---------------------------------------------------------------------------

/// Fetch a batch of emails by ID with all properties + body values.
pub(crate) async fn fetch_email_batch(
    client: &JmapClient,
    ids: &[&str],
) -> Result<Vec<jmap_client::email::Email>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let account_id = request.default_account_id().to_string();
    let mut get = email::EmailGet::new(&account_id);
    get.ids(ids.iter().copied());
    get.properties(email_get_properties());
    get.arguments().fetch_text_body_values(true);
    get.arguments().fetch_html_body_values(true);
    let handle = request
        .call(get)
        .map_err(|e| format!("Email/get batch: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Email/get batch: {e}"))?;

    response
        .get(&handle)
        .map(|mut r| r.take_list())
        .map_err(|e| format!("Email/get batch: {e}"))
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
        .map(|message| message.base.thread_id.clone())
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
        |message| &message.base.thread_id,
    ))
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
// Helpers
// ---------------------------------------------------------------------------

pub(crate) fn emit_progress(ctx: &SyncCtx<'_>, phase: &str, current: u64, total: u64) {
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
