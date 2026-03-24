mod delta_tokens;
mod folders;
mod persistence;
mod stores;

use std::collections::HashSet;

use ratatoskr_stores::body_store::BodyStoreState;
use ratatoskr_db::db::DbState;
use ratatoskr_stores::inline_image_store::InlineImageStoreState;
use ratatoskr_db::progress::ProgressReporter;
use ratatoskr_provider_utils::types::{ProviderCtx, SyncResult};
use ratatoskr_search::SearchState;

use super::client::GraphClient;
use super::folder_mapper::FolderMap;
use super::parse::{ParsedGraphMessage, parse_graph_message};
use super::types::{GraphMessage, ODataCollection};
use ratatoskr_sync::pending as sync_pending;

use self::delta_tokens::{
    bootstrap_delta_token, bootstrap_delta_token_latest, delete_delta_token,
    load_delta_tokens, save_delta_token,
};
use self::folders::{fetch_folder_messages, sync_folders};
use self::persistence::{delete_messages, persist_messages};
use self::stores::emit_progress;

const BATCH_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Internal context bundle for sync.
struct SyncCtx<'a> {
    client: &'a GraphClient,
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

/// Initial Graph sync: folders → per-folder message fetch → delta token bootstrap.
pub(crate) async fn graph_initial_sync(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    days_back: i64,
) -> Result<(), String> {
    let sctx = SyncCtx {
        client,
        account_id: ctx.account_id,
        db: ctx.db,
        body_store: ctx.body_store,
        inline_images: ctx.inline_images,
        search: ctx.search,
        progress: ctx.progress,
    };

    log::info!("[Graph] Starting initial sync for account {} (days_back={days_back})", ctx.account_id);
    // Phase 1: Sync folders → labels → build folder map
    emit_progress(&sctx, "folders", "", 0, 1, 0);

    let folder_map = sync_folders(client, ctx).await?;
    client.set_folder_map(folder_map.clone()).await;

    emit_progress(&sctx, "folders", "", 1, 1, 0);

    // Phase 2: Fetch messages per folder (prioritized)
    let since = chrono::Utc::now() - chrono::Duration::days(days_back);
    let since_iso = since.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

    let mut folder_list: Vec<(&str, &str)> = folder_map
        .folder_entries()
        .map(|(fid, m)| (fid, m.label_id.as_str()))
        .collect();
    folder_list.sort_by_key(|(_, label)| stores::folder_priority(label));

    #[allow(clippy::cast_possible_truncation)]
    let total_folders = folder_list.len() as u64;
    let mut total_messages: u64 = 0;

    for (i, &(folder_id, _label_id)) in folder_list.iter().enumerate() {
        let folder_name = folder_map
            .get_by_folder_id(folder_id)
            .map(|m| m.label_name.as_str())
            .unwrap_or("Unknown");

        #[allow(clippy::cast_possible_truncation)]
        let current = i as u64;
        emit_progress(
            &sctx,
            "messages",
            folder_name,
            current,
            total_folders,
            total_messages,
        );

        let messages =
            fetch_folder_messages(client, ctx.db, folder_id, &since_iso, &folder_map).await?;

        #[allow(clippy::cast_possible_truncation)]
        let count = messages.len() as u64;
        total_messages += count;

        if !messages.is_empty() {
            persist_messages(&sctx, &messages).await?;
        }
    }

    // Phase 3: Bootstrap delta tokens for each folder
    emit_progress(
        &sctx,
        "delta",
        "",
        0,
        total_folders,
        total_messages,
    );

    for (i, &(folder_id, _)) in folder_list.iter().enumerate() {
        match bootstrap_delta_token(client, ctx.db, folder_id).await {
            Ok(delta_link) => {
                save_delta_token(client, ctx.db, ctx.account_id, folder_id, &delta_link).await?;
            }
            Err(e) => {
                log::warn!(
                    "Failed to bootstrap delta token for folder {folder_id}: {e}"
                );
            }
        }

        #[allow(clippy::cast_possible_truncation)]
        let current = (i + 1) as u64;
        emit_progress(
            &sctx,
            "delta",
            "",
            current,
            total_folders,
            total_messages,
        );
    }

    let aid = ctx.account_id.to_string();
    ctx.db
        .with_conn(move |conn| ratatoskr_sync::pipeline::mark_initial_sync_completed(conn, &aid))
        .await?;

    log::info!(
        "[Graph] Initial sync complete for account {}: {} folders, {} messages",
        ctx.account_id, total_folders, total_messages
    );
    emit_progress(
        &sctx,
        "done",
        "",
        total_folders,
        total_folders,
        total_messages,
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Delta sync
// ---------------------------------------------------------------------------

/// Delta Graph sync: per-folder delta queries → targeted updates.
///
/// Returns new inbox message IDs and affected thread IDs for TS post-sync hooks.
///
/// Uses priority-based scheduling to reduce API calls:
/// - Tier 0 (INBOX, SENT, DRAFT): every cycle
/// - Tier 1 (TRASH, SPAM, archive): every 5th cycle
/// - Tier 2 (user folders): every 20th cycle
///
/// Every 10th cycle, refreshes the folder tree and bootstraps new folders.
pub(crate) async fn graph_delta_sync(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
) -> Result<SyncResult, String> {
    let sctx = SyncCtx {
        client,
        account_id: ctx.account_id,
        db: ctx.db,
        body_store: ctx.body_store,
        inline_images: ctx.inline_images,
        search: ctx.search,
        progress: ctx.progress,
    };

    let cycle = client.increment_sync_cycle();
    log::info!("[Graph] Starting delta sync for account {} (cycle={cycle})", ctx.account_id);

    // Load stored delta tokens
    let mut tokens = load_delta_tokens(client, ctx.db, ctx.account_id).await?;
    if tokens.is_empty() {
        log::error!("[Graph] No delta tokens for account {} — run initial sync first", ctx.account_id);
        return Err("GRAPH_NO_DELTA_STATE".to_string());
    }

    // Every 10th cycle, refresh the folder tree to discover new/removed folders
    let folder_map = if cycle.is_multiple_of(10) {
        let map = sync_folders(client, ctx).await?;
        client.set_folder_map(map.clone()).await;
        client.set_folder_map_synced().await;

        // Bootstrap delta tokens for newly discovered folders
        let known_folder_ids: HashSet<&str> = map.folder_entries().map(|(fid, _)| fid).collect();
        let token_folder_ids: HashSet<String> = tokens.keys().cloned().collect();

        for folder_id in &known_folder_ids {
            if !token_folder_ids.contains(*folder_id) {
                log::info!("Graph delta sync: bootstrapping new folder {folder_id}");
                match bootstrap_delta_token_latest(client, ctx.db, folder_id).await {
                    Ok(delta_link) => {
                        save_delta_token(client, ctx.db, ctx.account_id, folder_id, &delta_link).await?;
                        tokens.insert(folder_id.to_string(), delta_link);
                    }
                    Err(e) => {
                        log::warn!("Graph delta sync: failed to bootstrap folder {folder_id}: {e}");
                    }
                }
            }
        }

        // Clean up delta tokens for folders that no longer exist
        let stale_ids: Vec<String> = token_folder_ids
            .iter()
            .filter(|fid| !known_folder_ids.contains(fid.as_str()))
            .cloned()
            .collect();
        for stale_id in &stale_ids {
            log::info!("Graph delta sync: removing stale delta token for folder {stale_id}");
            delete_delta_token(client, ctx.db, ctx.account_id, stale_id).await?;
            tokens.remove(stale_id);
        }

        map
    } else if let Some(map) = client.folder_map().await {
        map
    } else {
        let map = sync_folders(client, ctx).await?;
        client.set_folder_map(map.clone()).await;
        client.set_folder_map_synced().await;
        map
    };

    let mut new_inbox_ids = Vec::new();
    let mut affected_thread_ids = HashSet::new();

    // Process each folder with a stored delta token, filtered by priority tier
    for (folder_id, delta_link) in &tokens {
        let label_id = folder_map
            .get_by_folder_id(folder_id)
            .map(|m| m.label_id.as_str())
            .unwrap_or("");

        if !should_sync_folder(label_id, cycle) {
            continue;
        }

        let (folder_new, folder_affected) =
            sync_folder_delta(&sctx, folder_id, delta_link, &folder_map).await?;
        new_inbox_ids.extend(folder_new);
        affected_thread_ids.extend(folder_affected);
    }

    // Reaction refresh: every 5th cycle.
    // Exchange reactions do NOT update lastModifiedDateTime or changeKey on messages,
    // so delta queries miss reaction changes entirely. To compensate, we periodically
    // re-fetch reaction extended properties for messages that already have reactions.
    if cycle.is_multiple_of(5) {
        match persistence::refresh_reactions_for_recent_messages(client, ctx.db, ctx.account_id).await {
            Ok(count) => {
                if count > 0 {
                    log::info!("Graph reaction refresh: updated {count} message(s)");
                }
            }
            Err(e) => log::warn!("Graph reaction refresh failed (non-fatal): {e}"),
        }
    }

    // Contacts + categories + calendar delta sync: every 20th cycle (change rarely)
    if cycle.is_multiple_of(20) {
        if let Err(e) =
            super::contact_sync::graph_contacts_delta_sync(client, ctx.account_id, ctx.db).await
        {
            log::warn!("Contact delta sync failed (non-fatal): {e}");
        }
        if let Err(e) =
            super::category_sync::graph_label_sync(client, ctx.account_id, ctx.db).await
        {
            log::warn!("Category delta sync failed (non-fatal): {e}");
        }
        match super::group_sync::sync_exchange_groups(client, ctx.db, ctx.account_id).await {
            Ok(count) => {
                if count > 0 {
                    log::info!("Exchange group delta sync: {count} groups");
                }
            }
            Err(e) => log::warn!("Exchange group delta sync failed (non-fatal): {e}"),
        }
        if let Err(e) =
            graph_calendar_delta_sync(client, ctx.account_id, ctx.db).await
        {
            log::warn!("Graph calendar delta sync failed (non-fatal): {e}");
        }
    }

    log::info!(
        "[Graph] Delta sync complete for account {}: {} new inbox, {} threads affected",
        ctx.account_id, new_inbox_ids.len(), affected_thread_ids.len()
    );

    Ok(SyncResult {
        new_inbox_message_ids: new_inbox_ids,
        affected_thread_ids: affected_thread_ids.into_iter().collect(),
    })
}

/// Decide whether a folder should be synced this cycle based on its priority tier.
fn should_sync_folder(label_id: &str, cycle: u32) -> bool {
    match stores::folder_priority(label_id) {
        0 => true,                     // Tier 0: every cycle
        1 => cycle.is_multiple_of(5),  // Tier 1: every 5th cycle
        _ => cycle.is_multiple_of(20), // Tier 2: every 20th cycle
    }
}

/// Process delta changes for a single folder.
///
/// Returns (new_inbox_message_ids, affected_thread_ids).
async fn sync_folder_delta(
    sctx: &SyncCtx<'_>,
    folder_id: &str,
    delta_link: &str,
    folder_map: &FolderMap,
) -> Result<(Vec<String>, HashSet<String>), String> {
    let mut new_inbox_ids = Vec::new();
    let mut affected_thread_ids = HashSet::new();

    let mut current_link = delta_link.to_string();

    loop {
        let page: ODataCollection<serde_json::Value> =
            sctx.client.get_absolute(&current_link, sctx.db).await?;

        let mut created_or_updated = Vec::new();
        let mut deleted_ids = Vec::new();

        for item in &page.value {
            let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
                continue;
            };

            if item.get("@removed").is_some() {
                deleted_ids.push(id.to_string());
            } else {
                // Deserialize full message
                match serde_json::from_value::<GraphMessage>(item.clone()) {
                    Ok(msg) => match parse_graph_message(&msg, folder_map) {
                        Ok(parsed) => created_or_updated.push(parsed),
                        Err(e) => log::warn!("Failed to parse Graph delta message {id}: {e}"),
                    },
                    Err(e) => log::warn!("Failed to deserialize Graph delta message {id}: {e}"),
                }
            }
        }

        // Filter pending ops before persisting
        let filtered = filter_pending_ops(sctx, created_or_updated).await?;

        for msg in &filtered {
            affected_thread_ids.insert(msg.base.thread_id.clone());
            if msg.base.label_ids.contains(&"INBOX".to_string()) {
                new_inbox_ids.push(msg.base.id.clone());
            }
        }

        if !filtered.is_empty() {
            persist_messages(sctx, &filtered).await?;
        }

        if !deleted_ids.is_empty() {
            delete_messages(sctx, &deleted_ids).await?;
        }

        // Follow pagination or store new delta link
        if let Some(ref next_link) = page.next_link {
            current_link = next_link.clone();
        } else if let Some(ref new_delta) = page.delta_link {
            save_delta_token(sctx.client, sctx.db, sctx.account_id, folder_id, new_delta).await?;
            break;
        } else {
            // No next or delta link — shouldn't happen, but break to avoid infinite loop
            log::warn!("Graph delta response for folder {folder_id} has no nextLink or deltaLink");
            break;
        }
    }

    Ok((new_inbox_ids, affected_thread_ids))
}

// ---------------------------------------------------------------------------
// Pending operations filter (sync vs queue coordination)
// ---------------------------------------------------------------------------

/// Filter out messages whose thread has pending operations.
///
/// Prevents sync from overwriting optimistic local state applied by
/// the TS queue processor. Same pattern as JMAP sync.
async fn filter_pending_ops(
    sctx: &SyncCtx<'_>,
    messages: Vec<ParsedGraphMessage>,
) -> Result<Vec<ParsedGraphMessage>, String> {
    if messages.is_empty() {
        return Ok(messages);
    }

    let thread_ids: HashSet<String> = messages
        .iter()
        .map(|message| message.base.thread_id.clone())
        .collect();
    let blocked_threads = sync_pending::blocked_thread_ids(
        sctx.db,
        sctx.account_id,
        thread_ids.into_iter().collect(),
    )
    .await?;

    if blocked_threads.is_empty() {
        return Ok(messages);
    }

    log::info!(
        "Graph delta sync: skipping {} threads with pending operations",
        blocked_threads.len()
    );

    Ok(sync_pending::filter_by_blocked_threads(
        messages,
        &blocked_threads,
        |message| &message.base.thread_id,
    ))
}

// ---------------------------------------------------------------------------
// Calendar delta sync
// ---------------------------------------------------------------------------

/// Run a calendar delta sync for a Graph account.
///
/// Lists calendars, upserts them into the DB, then syncs events for each
/// visible calendar using delta queries. Delta links are stored in the
/// calendar's `sync_token` column.
async fn graph_calendar_delta_sync(
    client: &GraphClient,
    account_id: &str,
    db: &DbState,
) -> Result<(), String> {
    use super::calendar_sync::{graph_list_calendars, graph_sync_calendar_events};

    let calendars = graph_list_calendars(client, db).await?;
    let aid = account_id.to_string();

    upsert_graph_calendars(db, &aid, &calendars).await?;
    let visible = load_visible_graph_calendars(db, &aid).await?;

    for (calendar_id, remote_id, sync_token) in &visible {
        let result =
            graph_sync_calendar_events(client, db, remote_id, sync_token.as_deref()).await?;
        persist_graph_calendar_events(db, &aid, calendar_id, result).await?;
        log::info!(
            "Graph calendar sync: synced calendar '{remote_id}' (cal_id={calendar_id})"
        );
    }

    Ok(())
}

/// Upsert discovered Graph calendars into the database.
async fn upsert_graph_calendars(
    db: &DbState,
    account_id: &str,
    calendars: &[super::calendar_sync::GraphCalendarInfo],
) -> Result<(), String> {
    let aid = account_id.to_string();
    let data: Vec<(String, Option<String>, bool)> = calendars
        .iter()
        .map(|c| (c.remote_id.clone(), c.color.clone(), c.is_primary))
        .collect();
    let names: Vec<String> = calendars.iter().map(|c| c.display_name.clone()).collect();

    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (i, (remote_id, color, is_primary)) in data.iter().enumerate() {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   display_name = ?5, color = ?6, is_primary = ?7, updated_at = unixepoch()",
                rusqlite::params![id, aid, "graph", remote_id, names[i], color, *is_primary as i64],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Load visible calendars (id, remote_id, sync_token) for an account.
async fn load_visible_graph_calendars(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<(String, String, Option<String>)>, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, remote_id, sync_token FROM calendars \
                 WHERE account_id = ?1 AND is_visible = 1 \
                 ORDER BY is_primary DESC, display_name ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(rusqlite::params![aid], |row| {
            Ok((
                row.get::<_, String>("id")?,
                row.get::<_, String>("remote_id")?,
                row.get::<_, Option<String>>("sync_token")?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

/// Persist synced calendar events and update the delta link.
#[allow(clippy::too_many_lines)]
async fn persist_graph_calendar_events(
    db: &DbState,
    account_id: &str,
    calendar_id: &str,
    result: super::calendar_sync::GraphCalendarSyncResult,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let cal_id = calendar_id.to_string();
    let new_delta_link = result.new_delta_link;
    let created = result.created;
    let updated = result.updated;
    let deleted_ids = result.deleted_remote_ids;

    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        for event in created.into_iter().chain(updated) {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO calendar_events (id, account_id, google_event_id, summary, description, location, start_time, end_time, is_all_day, status, organizer_email, attendees_json, html_link, calendar_id, remote_event_id, etag, ical_data, uid)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                 ON CONFLICT(account_id, google_event_id) DO UPDATE SET
                   summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
                   is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
                   html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
                   ical_data = ?17, uid = ?18, updated_at = unixepoch()",
                rusqlite::params![
                    id, aid, event.remote_event_id, event.summary, event.description,
                    event.location, event.start_time, event.end_time, event.is_all_day as i64,
                    event.status, event.organizer_email, event.attendees_json, event.html_link,
                    cal_id, event.remote_event_id, event.etag, event.ical_data, event.uid,
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        for remote_event_id in &deleted_ids {
            tx.execute(
                "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
                rusqlite::params![cal_id, remote_event_id],
            )
            .map_err(|e| e.to_string())?;
        }

        if let Some(ref delta_link) = new_delta_link {
            tx.execute(
                "UPDATE calendars SET sync_token = ?1, updated_at = unixepoch() WHERE id = ?2",
                rusqlite::params![delta_link, cal_id],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Public entry point for folder sync (used by ops.rs list_folders).
pub(crate) async fn sync_folders_public(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
) -> Result<FolderMap, String> {
    sync_folders(client, ctx).await
}
