#![allow(clippy::let_underscore_must_use)]

use std::collections::{HashMap, HashSet};

use ratatoskr_db::progress::{self, ProgressReporter};

use ratatoskr_stores::body_store::BodyStoreState;
use ratatoskr_db::db::DbState;
use ratatoskr_stores::inline_image_store::InlineImageStoreState;
use ratatoskr_search::SearchState;
use ratatoskr_sync::pipeline;
use ratatoskr_sync::types::{ImapSyncResult, MessageMeta, SyncProgressEvent};
use ratatoskr_sync::threading;

use super::client;
use super::connection::connect;
use super::convert::convert_imap_message;
use super::folder_mapper::{get_syncable_folders, map_folder_to_label};
use super::sync_pipeline;
use super::sync_pipeline::{CHUNK_SIZE, store_chunk};
use super::types::ImapConfig;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Consecutive connection failures before adding cooldown.
const CIRCUIT_BREAKER_THRESHOLD: u32 = 3;
/// Cooldown delay (ms) after threshold failures.
const CIRCUIT_BREAKER_DELAY_MS: u64 = 15_000;
/// Skip remaining folders after this many consecutive failures.
const CIRCUIT_BREAKER_MAX_FAILURES: u32 = 5;
/// Delay (ms) between folder syncs to avoid connection bursts.
const INTER_FOLDER_DELAY_MS: u64 = 1_000;

fn is_connection_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("tcp")
        || lower.contains("tls")
        || lower.contains("dns")
        || lower.contains("econnrefused")
        || lower.contains("network")
        || lower.contains("socket")
}

fn compute_since_date(days_back: i64) -> String {
    use chrono::{Duration, Utc};
    let date = Utc::now() - Duration::days(days_back);
    date.format("%d-%b-%Y").to_string()
}

fn emit_progress(progress: &dyn ProgressReporter, event: &SyncProgressEvent) {
    progress::emit_event(progress, "imap-sync-progress", event);
}

// ---------------------------------------------------------------------------
// Initial sync entry point
// ---------------------------------------------------------------------------

/// Run initial IMAP sync for an account.
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
#[allow(clippy::too_many_arguments)]
pub async fn imap_initial_sync(
    progress: &dyn ProgressReporter,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    account_id: &str,
    config: &ImapConfig,
    days_back: i64,
) -> Result<ImapSyncResult, String> {
    log::info!("[IMAP] Starting initial sync for account {account_id} (days_back={days_back})");
    // Phase 1: List and sync folders
    emit_progress(
        progress,
        &SyncProgressEvent {
            account_id: account_id.to_string(),
            phase: "folders".to_string(),
            current: 0,
            total: 1,
            folder: None,
        },
    );

    let all_folders = {
        let mut session = connect(config).await?;
        let folders = client::list_folders(&mut session).await?;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
        folders
    };

    let syncable_folders = get_syncable_folders(&all_folders);

    // Sync folders to labels
    {
        let account_id = account_id.to_string();
        let folders_owned: Vec<_> = syncable_folders.iter().map(|f| (*f).clone()).collect();
        db.with_conn(move |conn| {
            let refs: Vec<&super::types::ImapFolder> = folders_owned.iter().collect();
            sync_pipeline::sync_folders_to_labels(conn, &account_id, &refs)
        })
        .await?;
    }

    log::info!(
        "[sync] Initial sync for {}: {} syncable folders",
        account_id,
        syncable_folders.len()
    );

    emit_progress(
        progress,
        &SyncProgressEvent {
            account_id: account_id.to_string(),
            phase: "folders".to_string(),
            current: 1,
            total: 1,
            folder: None,
        },
    );

    // Phase 2: Streaming fetch & store
    let mut all_threadable = Vec::new();
    let mut all_meta: HashMap<String, MessageMeta> = HashMap::new();
    let mut labels_by_rfc_id: HashMap<String, HashSet<String>> = HashMap::new();

    let total_estimate: u64 = syncable_folders.iter().map(|f| u64::from(f.exists)).sum();
    let mut fetched_total: u64 = 0;
    let mut stored_count: u64 = 0;
    let mut total_messages_found: u64 = 0;
    let mut consecutive_failures: u32 = 0;
    let mut folder_errors: Vec<String> = Vec::new();

    let cutoff_seconds = chrono::Utc::now().timestamp() - days_back * 86400;
    let now_seconds = chrono::Utc::now().timestamp();
    let since_date = compute_since_date(days_back);

    for (folder_idx, folder) in syncable_folders.iter().enumerate() {
        if folder.exists == 0 {
            continue;
        }

        // Circuit breaker
        if consecutive_failures >= CIRCUIT_BREAKER_MAX_FAILURES {
            log::warn!(
                "[sync] Circuit breaker: {} failures, skipping {} remaining folders",
                consecutive_failures,
                syncable_folders.len() - folder_idx
            );
            break;
        }
        if consecutive_failures >= CIRCUIT_BREAKER_THRESHOLD {
            log::warn!(
                "[sync] Circuit breaker cooldown: {}s",
                CIRCUIT_BREAKER_DELAY_MS / 1000
            );
            tokio::time::sleep(std::time::Duration::from_millis(CIRCUIT_BREAKER_DELAY_MS)).await;
        }

        if folder_idx > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(INTER_FOLDER_DELAY_MS)).await;
        }

        let folder_mapping = map_folder_to_label(folder);

        match sync_single_folder(
            progress,
            db,
            body_store,
            inline_images,
            search,
            config,
            account_id,
            folder,
            &folder_mapping.label_id,
            &since_date,
            cutoff_seconds,
            now_seconds,
            fetched_total,
            total_estimate,
            &mut all_threadable,
            &mut all_meta,
            &mut labels_by_rfc_id,
        )
        .await
        {
            Ok((folder_fetched, folder_stored, folder_uid_count)) => {
                consecutive_failures = 0;
                total_messages_found += folder_fetched;
                stored_count += folder_stored;
                fetched_total += folder_uid_count;
            }
            Err(e) => {
                let err_str = e.clone();
                log::error!("[sync] Failed to sync folder {}: {err_str}", folder.path);
                folder_errors.push(format!("{}: {err_str}", folder.path));
                if is_connection_error(&err_str) {
                    consecutive_failures += 1;
                }
            }
        }
    }

    if stored_count == 0 && !folder_errors.is_empty() {
        return Err(format!("All folders failed: {}", folder_errors[0]));
    }

    // Phase 3: Threading
    emit_progress(
        progress,
        &SyncProgressEvent {
            account_id: account_id.to_string(),
            phase: "threading".to_string(),
            current: 0,
            total: all_threadable.len() as u64,
            folder: None,
        },
    );

    let thread_groups = threading::build_threads(&all_threadable);
    log::info!(
        "[sync] Threading: {} messages → {} threads",
        all_threadable.len(),
        thread_groups.len()
    );

    // Phase 4: Store threads
    emit_progress(
        progress,
        &SyncProgressEvent {
            account_id: account_id.to_string(),
            phase: "storing_threads".to_string(),
            current: 0,
            total: thread_groups.len() as u64,
            folder: None,
        },
    );

    let thread_ids: Vec<String> = thread_groups.iter().map(|g| g.thread_id.clone()).collect();
    let skipped = {
        let aid = account_id.to_string();
        let tids = thread_ids.clone();
        db.with_conn(move |conn| {
            ratatoskr_sync::pending::get_blocked_thread_ids(conn, &aid, &tids)
        })
        .await?
    };

    let affected_thread_ids = {
        let aid = account_id.to_string();
        let tg = thread_groups.clone();
        let meta = all_meta.clone();
        let lbr = labels_by_rfc_id.clone();
        let sk = skipped;
        db.with_conn(move |conn| pipeline::store_threads(conn, &aid, &tg, &meta, &lbr, &sk))
            .await?
    };

    emit_progress(
        progress,
        &SyncProgressEvent {
            account_id: account_id.to_string(),
            phase: "storing_threads".to_string(),
            current: thread_groups.len() as u64,
            total: thread_groups.len() as u64,
            folder: None,
        },
    );

    // Phase 5: Orphan cleanup
    let final_ids: HashSet<String> = thread_groups.iter().map(|g| g.thread_id.clone()).collect();
    let msg_ids: HashSet<String> = all_meta.keys().cloned().collect();
    let orphans = {
        let aid = account_id.to_string();
        db.with_conn(move |conn| pipeline::cleanup_orphan_threads(conn, &aid, &msg_ids, &final_ids))
            .await?
    };
    if orphans > 0 {
        log::info!("[sync] Cleaned up {orphans} orphaned placeholder threads");
    }

    // Update sync state
    if stored_count > 0 || total_messages_found == 0 {
        let aid = account_id.to_string();
        let marker = format!("imap-synced-{}", chrono::Utc::now().timestamp_millis());
        db.with_conn(move |conn| {
            ratatoskr_sync::state::update_account_sync_state(conn, &aid, &marker)
        })
        .await?;
    }

    let new_inbox_message_ids: Vec<String> = all_meta
        .values()
        .filter(|m| m.label_ids.contains(&"INBOX".to_string()))
        .map(|m| m.id.clone())
        .collect();

    emit_progress(
        progress,
        &SyncProgressEvent {
            account_id: account_id.to_string(),
            phase: "done".to_string(),
            current: stored_count,
            total: stored_count,
            folder: None,
        },
    );

    log::info!(
        "[sync] Complete: {} messages, {} threads ({} found on server)",
        stored_count,
        thread_groups.len(),
        total_messages_found
    );

    log::info!(
        "[IMAP] Initial sync complete for account {account_id}: {} messages stored, {} threads",
        stored_count, thread_groups.len()
    );

    Ok(ImapSyncResult {
        stored_count,
        thread_count: thread_groups.len() as u64,
        new_inbox_message_ids,
        affected_thread_ids,
    })
}

// ---------------------------------------------------------------------------
// Per-folder sync
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
#[allow(clippy::cognitive_complexity, clippy::too_many_lines)]
async fn sync_single_folder(
    progress: &dyn ProgressReporter,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    config: &ImapConfig,
    account_id: &str,
    folder: &super::types::ImapFolder,
    folder_label_id: &str,
    since_date: &str,
    cutoff_seconds: i64,
    now_seconds: i64,
    fetched_total: u64,
    total_estimate: u64,
    all_threadable: &mut Vec<threading::ThreadableMessage>,
    all_meta: &mut HashMap<String, MessageMeta>,
    labels_by_rfc_id: &mut HashMap<String, HashSet<String>>,
) -> Result<(u64, u64, u64), String> {
    let mut session = connect(config).await?;

    let search_result =
        client::search_folder(&mut session, &folder.raw_path, Some(since_date.to_string())).await?;

    let uids = search_result.uids;
    if uids.is_empty() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
        return Ok((0, 0, 0));
    }

    let uidvalidity = search_result.folder_status.uidvalidity;
    let highest_modseq = search_result.folder_status.highest_modseq;
    let mut last_uid: u32 = 0;
    let mut folder_fetched: u64 = 0;
    let mut folder_stored: u64 = 0;
    let mut date_fallback_count: u64 = 0;

    for chunk_start in (0..uids.len()).step_by(CHUNK_SIZE) {
        let chunk_end = (chunk_start + CHUNK_SIZE).min(uids.len());
        let chunk_uids = &uids[chunk_start..chunk_end];

        let uid_set: String = chunk_uids
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");

        let fetch_result = match client::fetch_messages(&mut session, &folder.raw_path, &uid_set)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                if is_connection_error(&e) {
                    log::warn!(
                        "[sync] Chunk fetch failed in {}, retrying: {e}",
                        folder.path
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(2000)).await;
                    session = connect(config).await?;
                    match client::fetch_messages(&mut session, &folder.raw_path, &uid_set).await {
                        Ok(r) => r,
                        Err(e2) => {
                            log::error!("[sync] Chunk retry failed in {}: {e2}", folder.path);
                            continue;
                        }
                    }
                } else {
                    log::error!("[sync] Chunk error in {}: {e}", folder.path);
                    continue;
                }
            }
        };

        let mut chunk_converted = Vec::new();

        for mut msg in fetch_result.messages {
            if msg.uid > last_uid {
                last_uid = msg.uid;
            }
            folder_fetched += 1;

            if msg.date == 0 {
                date_fallback_count += 1;
                msg.date = now_seconds;
            }
            if msg.date < cutoff_seconds {
                continue;
            }

            chunk_converted.push(convert_imap_message(msg, account_id, folder_label_id));
        }

        if !chunk_converted.is_empty() {
            store_chunk(
                db,
                body_store,
                inline_images,
                search,
                &chunk_converted,
                account_id,
            )
            .await?;

            for c in &chunk_converted {
                all_meta.insert(c.id.clone(), c.meta.clone());
                all_threadable.push(c.threadable.clone());
                let rfc_labels = labels_by_rfc_id
                    .entry(c.threadable.message_id.clone())
                    .or_default();
                for lid in &c.label_ids {
                    rfc_labels.insert(lid.clone());
                }
            }

            folder_stored += chunk_converted.len() as u64;
        }

        emit_progress(
            progress,
            &SyncProgressEvent {
                account_id: account_id.to_string(),
                phase: "messages".to_string(),
                current: fetched_total + (chunk_end as u64).min(uids.len() as u64),
                total: total_estimate,
                folder: Some(folder.path.clone()),
            },
        );
    }

    if date_fallback_count > 0 {
        log::warn!(
            "[sync] Folder {}: {}/{} had unparseable dates",
            folder.path,
            date_fallback_count,
            folder_fetched
        );
    }

    log::info!(
        "[sync] Folder {}: {} UIDs, {} fetched, {} stored",
        folder.path,
        uids.len(),
        folder_fetched,
        folder_stored
    );

    // Update folder sync state
    {
        let aid = account_id.to_string();
        let fp = folder.raw_path.clone();
        let sync_at = chrono::Utc::now().timestamp();
        db.with_conn(move |conn| {
            sync_pipeline::upsert_folder_sync_state(
                conn, &aid, &fp, uidvalidity, last_uid, sync_at, highest_modseq,
            )
        })
        .await?;
    }

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;

    Ok((folder_fetched, folder_stored, uids.len() as u64))
}
