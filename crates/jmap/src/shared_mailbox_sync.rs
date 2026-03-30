//! Orchestration layer for per-shared-account sync (JMAP Sharing — Phase 2).
//!
//! Each shared JMAP account syncs independently with its own Mailbox/Email
//! state tokens, following the same pattern as `graph/src/shared_mailbox_sync.rs`.
//! The existing sync pipeline is reused by passing the shared account's JMAP ID
//! through `SyncCtx::jmap_account_id`.

use std::collections::{HashMap, HashSet};

use jmap_client::mailbox::MailboxChanges;

use store::body_store::BodyStoreState;
use db::db::DbState;
use store::inline_image_store::InlineImageStoreState;
use db::progress::ProgressReporter;
use common::types::SyncResult;
use search::SearchState;
use sync::state as sync_state;

use crate::client::JmapClient;
use crate::mailbox_mapper::MailboxInfo;
use crate::parse::{parse_jmap_email, ParsedJmapMessage};
use crate::sync::{
    SyncCtx, emit_progress,
    fetch_email_batch_for, query_email_page_for,
    save_sync_state_ctx, load_sync_state_ctx,
};
use crate::sync::mailbox::{get_email_state_for, get_mailbox_state_for};

/// Default number of days to look back during initial sync of a shared account.
const SHARED_ACCOUNT_INITIAL_SYNC_DAYS: i64 = 30;

const BATCH_SIZE: usize = 50;

/// Run sync for a single shared JMAP account.
///
/// Uses the same client (credentials/session) as the primary account but
/// targets a different JMAP account ID in all method calls.
#[allow(clippy::too_many_arguments)]
pub async fn sync_shared_account(
    client: &JmapClient,
    jmap_account_id: &str,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    progress: &dyn ProgressReporter,
) -> Result<SyncResult, String> {
    let ctx = SyncCtx {
        client,
        account_id,
        db,
        body_store,
        inline_images,
        search,
        progress,
        jmap_account_id: Some(jmap_account_id.to_string()),
    };

    let now = chrono::Utc::now().timestamp();

    // Check if we have existing state tokens — if not, run initial sync.
    let email_state = load_sync_state_ctx(&ctx, "Email").await?;

    if email_state.is_none() {
        log::info!(
            "Shared JMAP account {jmap_account_id}: no state found, running initial sync"
        );
        match shared_initial_sync(&ctx).await {
            Ok(()) => {
                sync_state::update_shared_mailbox_sync_status(
                    db, account_id, jmap_account_id, now, None,
                )
                .await?;
                Ok(SyncResult::default())
            }
            Err(e) => {
                log::warn!(
                    "Shared JMAP account {jmap_account_id} initial sync failed: {e}"
                );
                sync_state::update_shared_mailbox_sync_status(
                    db, account_id, jmap_account_id, now, Some(&e),
                )
                .await?;
                Err(e)
            }
        }
    } else {
        log::info!(
            "Shared JMAP account {jmap_account_id}: state found, running delta sync"
        );
        match shared_delta_sync(&ctx).await {
            Ok(sync_result) => {
                sync_state::update_shared_mailbox_sync_status(
                    db, account_id, jmap_account_id, now, None,
                )
                .await?;
                Ok(sync_result)
            }
            Err(e) => {
                log::warn!(
                    "Shared JMAP account {jmap_account_id} delta sync failed: {e}"
                );
                sync_state::update_shared_mailbox_sync_status(
                    db, account_id, jmap_account_id, now, Some(&e),
                )
                .await?;
                Err(e)
            }
        }
    }
}

/// Sync all enabled shared JMAP accounts for a Ratatoskr account.
///
/// Each shared account syncs independently — one failure does not block others.
#[allow(clippy::too_many_arguments)]
pub async fn sync_all_shared_accounts(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    progress: &dyn ProgressReporter,
) -> Vec<(String, Result<SyncResult, String>)> {
    let enabled = match sync_state::get_enabled_shared_mailboxes(db, account_id).await {
        Ok(list) => list,
        Err(e) => {
            log::warn!("Failed to load enabled shared JMAP accounts: {e}");
            return Vec::new();
        }
    };

    if enabled.is_empty() {
        return Vec::new();
    }

    log::info!(
        "Syncing {} enabled shared JMAP account(s) for {account_id}",
        enabled.len()
    );

    let mut results = Vec::with_capacity(enabled.len());

    for entry in &enabled {
        let display = entry
            .display_name
            .as_deref()
            .unwrap_or(&entry.mailbox_id);
        log::info!("Starting sync for shared JMAP account: {display}");

        let result = sync_shared_account(
            client,
            &entry.mailbox_id,
            account_id,
            db,
            body_store,
            inline_images,
            search,
            progress,
        )
        .await;

        match &result {
            Ok(sr) => {
                log::info!(
                    "Shared JMAP account {display}: sync complete ({} new inbox, {} affected threads)",
                    sr.new_inbox_message_ids.len(),
                    sr.affected_thread_ids.len()
                );
            }
            Err(e) => {
                log::warn!("Shared JMAP account {display}: sync failed: {e}");
            }
        }

        results.push((entry.mailbox_id.clone(), result));
    }

    results
}

// ---------------------------------------------------------------------------
// Internal sync implementations
// ---------------------------------------------------------------------------

/// Initial sync for a shared JMAP account.
async fn shared_initial_sync(ctx: &SyncCtx<'_>) -> Result<(), String> {
    let jmap_id = ctx.jmap_account_id.as_deref()
        .ok_or("shared_initial_sync called without jmap_account_id")?;

    // Phase 1: Sync mailboxes -> labels
    emit_progress(ctx, "mailboxes", 0, 1);
    let (mailbox_map, mailbox_data) = crate::sync::mailbox::sync_mailboxes(ctx).await?;

    let mailbox_state = get_mailbox_state_for(ctx.client, Some(jmap_id)).await?;
    save_sync_state_ctx(ctx, "Mailbox", &mailbox_state).await?;
    emit_progress(ctx, "mailboxes", 1, 1);

    // Phase 2: Paginated Email/query -> batched Email/get -> DB writes
    let since = chrono::Utc::now()
        - chrono::Duration::days(SHARED_ACCOUNT_INITIAL_SYNC_DAYS);
    let since_ts = since.timestamp();

    let mut total_u64: u64 = 0;
    let mut fetched: u64 = 0;
    let mut position: usize = 0;

    loop {
        emit_progress(ctx, "messages", fetched, total_u64);

        let query_result =
            query_email_page_for(ctx.client, Some(jmap_id), since_ts, position, position == 0)
                .await?;

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
        let emails = fetch_email_batch_for(ctx.client, Some(jmap_id), &batch_ids).await?;
        let parsed = parse_email_batch(&emails, &mailbox_map)?;

        crate::sync::storage::persist_messages(ctx, &parsed, &mailbox_data).await?;

        #[allow(clippy::cast_possible_truncation)]
        {
            fetched += parsed.len() as u64;
        }
        position += ids.len();
        if ids.len() < BATCH_SIZE {
            break;
        }
    }

    let email_state = get_email_state_for(ctx.client, Some(jmap_id)).await?;
    save_sync_state_ctx(ctx, "Email", &email_state).await?;

    log::info!(
        "[JMAP] Shared account {jmap_id} initial sync complete: {fetched} messages"
    );

    Ok(())
}

/// Delta sync for a shared JMAP account.
async fn shared_delta_sync(ctx: &SyncCtx<'_>) -> Result<SyncResult, String> {
    let jmap_id = ctx.jmap_account_id.as_deref()
        .ok_or("shared_delta_sync called without jmap_account_id")?;

    let email_state = load_sync_state_ctx(ctx, "Email")
        .await?
        .ok_or("JMAP_NO_STATE")?;
    let mailbox_state = load_sync_state_ctx(ctx, "Mailbox").await?;

    // 1. Mailbox changes
    if let Some(mb_state) = &mailbox_state {
        shared_mailbox_changes(ctx, jmap_id, mb_state).await?;
    }

    // Refresh mailbox map
    let (mailbox_map, _mailbox_data) =
        crate::sync::mailbox::sync_mailboxes(ctx).await?;

    // 2. Email changes
    let mut since_state = email_state;
    let mut new_inbox_ids = Vec::new();
    let mut affected_thread_ids = HashSet::new();

    loop {
        let inner = ctx.client.inner();
        let mut request = inner.build();
        let mut changes = jmap_client::email::EmailChanges::new(jmap_id, &since_state);
        changes.max_changes(crate::JMAP_MAX_CHANGES);
        let handle = request
            .call(changes)
            .map_err(|e| format!("Email/changes: {e}"))?;
        let mut response = request
            .send()
            .await
            .map_err(|e| format!("Email/changes: {e}"))?;

        let changes = response
            .get(&handle)
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("cannotCalculateChanges") {
                    log::warn!(
                        "[JMAP] Email state expired for shared account {jmap_id}, full re-sync needed"
                    );
                    return "JMAP_STATE_EXPIRED".to_string();
                }
                format!("Email/changes: {msg}")
            })?;

        let created = changes.created();
        let updated = changes.updated();
        let destroyed = changes.destroyed();

        let ids_to_fetch: Vec<&str> = created
            .iter()
            .chain(updated.iter())
            .map(String::as_str)
            .collect();

        if !ids_to_fetch.is_empty() {
            for chunk in ids_to_fetch.chunks(BATCH_SIZE) {
                let emails =
                    fetch_email_batch_for(ctx.client, Some(jmap_id), chunk).await?;
                let parsed = parse_email_batch(&emails, &mailbox_map)?;

                for msg in &parsed {
                    affected_thread_ids.insert(msg.base.thread_id.clone());
                    if msg.base.label_ids.contains(&"INBOX".to_string())
                        && created.iter().any(|c| c == &msg.base.id)
                    {
                        new_inbox_ids.push(msg.base.id.clone());
                    }
                }

                crate::sync::storage::persist_messages(ctx, &parsed, &[]).await?;
            }
        }

        if !destroyed.is_empty() {
            let destroyed_refs: Vec<&str> =
                destroyed.iter().map(String::as_str).collect();
            crate::sync::storage::delete_messages(ctx, &destroyed_refs).await?;
        }

        since_state = changes.new_state().to_string();

        if !changes.has_more_changes() {
            break;
        }
    }

    save_sync_state_ctx(ctx, "Email", &since_state).await?;

    log::info!(
        "[JMAP] Shared account {jmap_id} delta sync complete: {} new inbox, {} threads affected",
        new_inbox_ids.len(),
        affected_thread_ids.len()
    );

    Ok(SyncResult {
        new_inbox_message_ids: new_inbox_ids,
        affected_thread_ids: affected_thread_ids.into_iter().collect(),
    })
}

/// Handle Mailbox/changes for a shared account.
async fn shared_mailbox_changes(
    ctx: &SyncCtx<'_>,
    jmap_id: &str,
    since_state: &str,
) -> Result<(), String> {
    let inner = ctx.client.inner();
    let mut request = inner.build();
    let mut changes = MailboxChanges::new(jmap_id, since_state);
    changes.max_changes(crate::JMAP_MAX_CHANGES);
    let handle = request
        .call(changes)
        .map_err(|e| format!("Mailbox/changes: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Mailbox/changes: {e}"))?;

    match response.get(&handle) {
        Ok(changes) => {
            let new_state = changes.new_state().to_string();
            if new_state != since_state {
                crate::sync::mailbox::sync_mailboxes(ctx).await?;
                save_sync_state_ctx(ctx, "Mailbox", &new_state).await?;
            }
        }
        Err(e) => {
            let msg = e.to_string();
            if msg.contains("cannotCalculateChanges") {
                crate::sync::mailbox::sync_mailboxes(ctx).await?;
                let new_state =
                    get_mailbox_state_for(ctx.client, Some(jmap_id)).await?;
                save_sync_state_ctx(ctx, "Mailbox", &new_state).await?;
            } else {
                return Err(format!("Mailbox/changes: {msg}"));
            }
        }
    }

    Ok(())
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
