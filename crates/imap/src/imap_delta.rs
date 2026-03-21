#![allow(clippy::let_underscore_must_use)]

use std::collections::{HashMap, HashSet};

use ratatoskr_db::progress::ProgressReporter;

use ratatoskr_stores::body_store::BodyStoreState;
use ratatoskr_db::db::DbState;
use ratatoskr_stores::inline_image_store::InlineImageStoreState;
use ratatoskr_search::SearchState;
use ratatoskr_sync::pipeline;
use ratatoskr_sync::types::{ImapSyncResult, MessageMeta};
use ratatoskr_sync::threading;

use super::client;
use super::connection::connect;
use super::convert::{ConvertedMessage, convert_imap_message};
use super::folder_mapper::{get_syncable_folders, map_folder_to_label};
use super::sync_pipeline;
use super::sync_pipeline::{CHUNK_SIZE, store_chunk};
use super::types::{DeltaCheckRequest, DeltaCheckResult, ImapConfig};

const CIRCUIT_BREAKER_THRESHOLD: u32 = 3;
const CIRCUIT_BREAKER_DELAY_MS: u64 = 15_000;
const CIRCUIT_BREAKER_MAX_FAILURES: u32 = 5;

/// Minimum interval between deletion detection checks per folder (seconds).
/// UID SEARCH ALL can be expensive on large folders, so we throttle it.
const DELETION_CHECK_INTERVAL_SECS: i64 = 600; // 10 minutes

fn is_connection_error(err: &str) -> bool {
    let lower = err.to_lowercase();
    lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("tcp")
        || lower.contains("tls")
        || lower.contains("dns")
        || lower.contains("network")
        || lower.contains("socket")
}

fn compute_since_date(days_back: i64) -> String {
    use chrono::{Duration, Utc};
    let date = Utc::now() - Duration::days(days_back);
    date.format("%d-%b-%Y").to_string()
}

/// Run delta IMAP sync for an account.
#[allow(clippy::too_many_lines)]
pub async fn imap_delta_sync(
    _progress: &dyn ProgressReporter,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    account_id: &str,
    config: &ImapConfig,
    days_back: i64,
) -> Result<ImapSyncResult, String> {
    log::info!("[IMAP] Starting delta sync for account {account_id} (days_back={days_back})");
    // List folders
    let all_folders = {
        let mut session = connect(config).await?;
        let folders = client::list_folders(&mut session).await?;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
        folders
    };
    let syncable_folders = get_syncable_folders(&all_folders);

    // Sync folders to labels
    {
        let aid = account_id.to_string();
        let fowned: Vec<_> = syncable_folders.iter().map(|f| (*f).clone()).collect();
        db.with_conn(move |conn| {
            let refs: Vec<_> = fowned.iter().collect();
            sync_pipeline::sync_folders_to_labels(conn, &aid, &refs)
        })
        .await?;
    }

    // Get saved folder sync states
    let sync_states = {
        let aid = account_id.to_string();
        db.with_conn(move |conn| sync_pipeline::get_all_folder_sync_states(conn, &aid))
            .await?
    };
    let state_map: HashMap<String, sync_pipeline::FolderSyncState> = sync_states
        .into_iter()
        .map(|s| (s.folder_path.clone(), s))
        .collect();

    let mut all_threadable = Vec::new();
    let mut all_meta: HashMap<String, MessageMeta> = HashMap::new();
    let mut labels_by_rfc_id: HashMap<String, HashSet<String>> = HashMap::new();
    let mut delta_errors: Vec<String> = Vec::new();

    let new_folders: Vec<_> = syncable_folders
        .iter()
        .filter(|f| !state_map.contains_key(&f.raw_path))
        .collect();
    let existing_folders: Vec<_> = syncable_folders
        .iter()
        .filter(|f| state_map.contains_key(&f.raw_path))
        .collect();

    // Handle new folders
    let mut consecutive_failures: u32 = 0;
    let since_date = compute_since_date(days_back);

    for folder in &new_folders {
        if consecutive_failures >= CIRCUIT_BREAKER_MAX_FAILURES {
            break;
        }
        if consecutive_failures >= CIRCUIT_BREAKER_THRESHOLD {
            tokio::time::sleep(std::time::Duration::from_millis(CIRCUIT_BREAKER_DELAY_MS)).await;
        }

        let mapping = map_folder_to_label(folder);
        match fetch_folder_uids(
            config,
            account_id,
            folder,
            &mapping.label_id,
            &since_date,
            db,
            body_store,
            inline_images,
            search,
            &mut all_threadable,
            &mut all_meta,
            &mut labels_by_rfc_id,
        )
        .await
        {
            Ok(()) => consecutive_failures = 0,
            Err(e) => {
                let s = e.clone();
                log::error!("[sync] Delta new folder {} failed: {s}", folder.path);
                delta_errors.push(format!("{}: {s}", folder.path));
                if is_connection_error(&s) {
                    consecutive_failures += 1;
                }
            }
        }
    }

    // Batch delta check existing folders
    if !existing_folders.is_empty() {
        let requests: Vec<DeltaCheckRequest> = existing_folders
            .iter()
            .filter_map(|f| {
                let s = state_map.get(&f.raw_path)?;
                Some(DeltaCheckRequest {
                    folder: f.raw_path.clone(),
                    last_uid: s.last_uid,
                    uidvalidity: s.uidvalidity.unwrap_or(0),
                    last_modseq: s.modseq,
                })
            })
            .collect();

        let result_map = batch_delta_check(config, &requests, &existing_folders, &state_map).await;

        for folder in &existing_folders {
            let mapping = map_folder_to_label(folder);
            let saved = match state_map.get(&folder.raw_path) {
                Some(s) => s,
                None => continue,
            };
            let delta = match result_map.get(&folder.raw_path) {
                Some(r) => r,
                None => continue,
            };

            if let Err(e) = process_folder_delta(
                config,
                account_id,
                folder,
                &mapping.label_id,
                saved,
                delta,
                days_back,
                db,
                body_store,
                inline_images,
                search,
                &mut all_threadable,
                &mut all_meta,
                &mut labels_by_rfc_id,
            )
            .await
            {
                let s = e.clone();
                log::error!("[sync] Delta {} failed: {s}", folder.path);
                delta_errors.push(format!("{}: {s}", folder.path));
            }
        }
    }

    // Run deletion detection (throttled per-folder, only checks every 10 min)
    let deletion_affected = run_deletion_detection(
        config,
        account_id,
        db,
        body_store,
        search,
        &syncable_folders,
        &state_map,
    )
    .await;

    if all_threadable.is_empty() && !delta_errors.is_empty() {
        return Err(format!("All folders failed: {}", delta_errors[0]));
    }

    if all_threadable.is_empty() {
        return Ok(ImapSyncResult {
            stored_count: 0,
            thread_count: 0,
            new_inbox_message_ids: vec![],
            affected_thread_ids: deletion_affected,
        });
    }

    // Thread + store
    let thread_groups = threading::build_threads(&all_threadable);
    let tids: Vec<String> = thread_groups.iter().map(|g| g.thread_id.clone()).collect();

    let skipped = {
        let aid = account_id.to_string();
        let t = tids.clone();
        db.with_conn(move |conn| {
            ratatoskr_sync::pending::get_blocked_thread_ids(conn, &aid, &t)
        })
        .await?
    };

    let mut affected = {
        let aid = account_id.to_string();
        let tg = thread_groups.clone();
        let m = all_meta.clone();
        let l = labels_by_rfc_id.clone();
        let s = skipped;
        db.with_conn(move |conn| pipeline::store_threads(conn, &aid, &tg, &m, &l, &s))
            .await?
    };

    // Merge deletion-affected thread IDs
    affected.extend(deletion_affected);

    let inbox_ids: Vec<String> = all_meta
        .values()
        .filter(|m| m.label_ids.contains(&"INBOX".to_string()))
        .map(|m| m.id.clone())
        .collect();

    let stored = all_meta.len() as u64;

    // Update sync state
    {
        let aid = account_id.to_string();
        let marker = format!("imap-synced-{}", chrono::Utc::now().timestamp_millis());
        db.with_conn(move |conn| {
            ratatoskr_sync::state::update_account_sync_state(conn, &aid, &marker)
        })
        .await?;
    }

    log::info!(
        "[IMAP] Delta sync complete for account {account_id}: {} messages stored, {} threads, {} affected",
        stored, thread_groups.len(), affected.len()
    );

    Ok(ImapSyncResult {
        stored_count: stored,
        thread_count: thread_groups.len() as u64,
        new_inbox_message_ids: inbox_ids,
        affected_thread_ids: affected,
    })
}

/// Batch delta check with fallback to per-folder.
async fn batch_delta_check(
    config: &ImapConfig,
    requests: &[DeltaCheckRequest],
    existing_folders: &[&&super::types::ImapFolder],
    state_map: &HashMap<String, sync_pipeline::FolderSyncState>,
) -> HashMap<String, DeltaCheckResult> {
    let result = async {
        let mut session = connect(config).await?;
        let results = client::delta_check_folders(&mut session, requests).await;
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
        results
    }
    .await;

    match result {
        Ok(results) => {
            log::info!(
                "[sync] Batch delta: {}/{} folders checked",
                results.len(),
                existing_folders.len()
            );
            results.into_iter().map(|r| (r.folder.clone(), r)).collect()
        }
        Err(e) => {
            log::warn!("[sync] Batch delta failed, per-folder fallback: {e}");
            let mut map = HashMap::new();
            for folder in existing_folders {
                let saved = match state_map.get(&folder.raw_path) {
                    Some(s) => s,
                    None => continue,
                };
                match per_folder_check(config, &folder.raw_path, saved).await {
                    Ok(r) => {
                        map.insert(r.folder.clone(), r);
                    }
                    Err(e) => log::error!("[sync] Per-folder check {}: {e}", folder.path),
                }
            }
            map
        }
    }
}

async fn per_folder_check(
    config: &ImapConfig,
    folder_path: &str,
    saved: &sync_pipeline::FolderSyncState,
) -> Result<DeltaCheckResult, String> {
    let mut session = connect(config).await?;
    let status = client::get_folder_status(&mut session, folder_path).await?;

    let changed = saved.uidvalidity.is_some() && saved.uidvalidity != Some(status.uidvalidity);

    if changed {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
        return Ok(DeltaCheckResult {
            folder: folder_path.to_string(),
            uidvalidity: status.uidvalidity,
            new_uids: vec![],
            uidvalidity_changed: true,
            highest_modseq: status.highest_modseq,
            modseq_unchanged: false,
            modseq_reset: false,
        });
    }

    let modseq_reset = match (saved.modseq, status.highest_modseq) {
        (Some(cached), Some(server)) if server < cached => {
            log::warn!(
                "[sync] per_folder_check: {folder_path} HIGHESTMODSEQ reset (cached {cached} > server {server})",
            );
            true
        }
        _ => false,
    };

    let new_uids = client::fetch_new_uids(&mut session, folder_path, saved.last_uid).await?;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;

    Ok(DeltaCheckResult {
        folder: folder_path.to_string(),
        uidvalidity: status.uidvalidity,
        new_uids,
        uidvalidity_changed: false,
        highest_modseq: status.highest_modseq,
        modseq_unchanged: false,
        modseq_reset,
    })
}

/// Fetch all UIDs from a folder and store them.
#[allow(clippy::too_many_arguments)]
async fn fetch_folder_uids(
    config: &ImapConfig,
    account_id: &str,
    folder: &super::types::ImapFolder,
    folder_label_id: &str,
    since_date: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    all_threadable: &mut Vec<threading::ThreadableMessage>,
    all_meta: &mut HashMap<String, MessageMeta>,
    labels_by_rfc_id: &mut HashMap<String, HashSet<String>>,
) -> Result<(), String> {
    let mut session = connect(config).await?;
    let sr =
        client::search_folder(&mut session, &folder.raw_path, Some(since_date.to_string())).await?;

    if sr.uids.is_empty() {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
        // Still persist sync state so this folder isn't treated as "new" on every delta cycle
        let aid = account_id.to_string();
        let fp = folder.raw_path.clone();
        let uv = sr.folder_status.uidvalidity;
        let ms = sr.folder_status.highest_modseq;
        let sat = chrono::Utc::now().timestamp();
        db.with_conn(move |conn| {
            sync_pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, 0, sat, ms)
        })
        .await?;
        return Ok(());
    }

    let (last_uid, _) = fetch_uids_on_session(
        &mut session,
        account_id,
        folder,
        folder_label_id,
        &sr.uids,
        db,
        body_store,
        inline_images,
        search,
        all_threadable,
        all_meta,
        labels_by_rfc_id,
    )
    .await?;

    let aid = account_id.to_string();
    let fp = folder.raw_path.clone();
    let uv = sr.folder_status.uidvalidity;
    let ms = sr.folder_status.highest_modseq;
    let sat = chrono::Utc::now().timestamp();
    db.with_conn(move |conn| {
        sync_pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, last_uid, sat, ms)
    })
    .await?;

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
    Ok(())
}

/// Fetch UIDs on an existing session, store messages. Returns (last_uid, uidvalidity).
#[allow(clippy::too_many_arguments)]
async fn fetch_uids_on_session(
    session: &mut super::connection::ImapSession,
    account_id: &str,
    folder: &super::types::ImapFolder,
    folder_label_id: &str,
    uids: &[u32],
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    all_threadable: &mut Vec<threading::ThreadableMessage>,
    all_meta: &mut HashMap<String, MessageMeta>,
    labels_by_rfc_id: &mut HashMap<String, HashSet<String>>,
) -> Result<(u32, u32), String> {
    let mut last_uid: u32 = 0;
    let mut uidvalidity: u32 = 0;

    for chunk in uids.chunks(CHUNK_SIZE) {
        let uid_set: String = chunk
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");

        let fr = client::fetch_messages(session, &folder.raw_path, &uid_set).await?;
        uidvalidity = fr.folder_status.uidvalidity;

        let mut converted: Vec<ConvertedMessage> = Vec::new();
        for msg in fr.messages {
            if msg.uid > last_uid {
                last_uid = msg.uid;
            }
            converted.push(convert_imap_message(msg, account_id, folder_label_id));
        }

        if !converted.is_empty() {
            store_chunk(
                db,
                body_store,
                inline_images,
                search,
                &converted,
                account_id,
            )
            .await?;

            for c in &converted {
                all_meta.insert(c.id.clone(), c.meta.clone());
                all_threadable.push(c.threadable.clone());
                let rfc_labels = labels_by_rfc_id
                    .entry(c.threadable.message_id.clone())
                    .or_default();
                for lid in &c.label_ids {
                    rfc_labels.insert(lid.clone());
                }
            }
        }
    }

    Ok((last_uid, uidvalidity))
}

/// Process delta result for a single existing folder.
#[allow(clippy::too_many_arguments)]
async fn process_folder_delta(
    config: &ImapConfig,
    account_id: &str,
    folder: &super::types::ImapFolder,
    folder_label_id: &str,
    saved: &sync_pipeline::FolderSyncState,
    delta: &DeltaCheckResult,
    days_back: i64,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    all_threadable: &mut Vec<threading::ThreadableMessage>,
    all_meta: &mut HashMap<String, MessageMeta>,
    labels_by_rfc_id: &mut HashMap<String, HashSet<String>>,
) -> Result<(), String> {
    // CONDSTORE fast path: modseq unchanged means no flags, no new messages,
    // no deletions. Update the modseq in sync state (in case it was None before)
    // and skip all further processing.
    if delta.modseq_unchanged {
        // Still update sync state to persist the modseq value
        if let Some(modseq) = delta.highest_modseq {
            let aid = account_id.to_string();
            let fp = folder.raw_path.clone();
            let uv = delta.uidvalidity;
            let lu = saved.last_uid;
            let sat = chrono::Utc::now().timestamp();
            db.with_conn(move |conn| {
                sync_pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, lu, sat, Some(modseq))
            })
            .await?;
        }
        return Ok(());
    }

    // HIGHESTMODSEQ reset: server's modseq went backwards while UIDVALIDITY
    // stayed the same.  This happens during server migration or mailbox repair.
    // Using CHANGEDSINCE with our stale (higher) cached value would return zero
    // results, silently missing all flag updates.  Fix: fetch ALL flags via
    // CHANGEDSINCE 1, then persist the server's new (lower) modseq.
    if delta.modseq_reset {
        log::warn!(
            "[sync] HIGHESTMODSEQ reset for {} (cached {:?} > server {:?}), full flag resync",
            folder.path,
            saved.modseq,
            delta.highest_modseq
        );

        let mut session = connect(config).await?;
        match client::fetch_changed_flags(&mut session, &folder.raw_path, 1).await {
            Ok(changes) if !changes.is_empty() => {
                log::info!(
                    "[sync] Modseq reset resync for {}: {} flag updates",
                    folder.path,
                    changes.len()
                );
                let aid = account_id.to_string();
                let fp = folder.raw_path.clone();
                let ch = changes;
                db.with_conn(move |conn| sync_pipeline::apply_flag_changes(conn, &aid, &fp, &ch))
                    .await?;
            }
            Ok(_) => {
                log::info!(
                    "[sync] Modseq reset resync for {}: no flag changes found",
                    folder.path
                );
            }
            Err(e) => {
                log::warn!(
                    "[sync] Modseq reset flag resync failed for {}: {e}",
                    folder.path
                );
            }
        }
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;

        // Persist the server's new (lower) modseq so future syncs use CHANGEDSINCE
        // with the correct baseline.
        let aid = account_id.to_string();
        let fp = folder.raw_path.clone();
        let uv = delta.uidvalidity;
        let lu = saved.last_uid;
        let ms = delta.highest_modseq;
        let sat = chrono::Utc::now().timestamp();
        db.with_conn(move |conn| {
            sync_pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, lu, sat, ms)
        })
        .await?;

        // If there are also new UIDs, fall through to fetch them below.
        // Otherwise we're done.
        if delta.new_uids.is_empty() {
            return Ok(());
        }
    }

    if delta.uidvalidity_changed {
        log::warn!(
            "[sync] UIDVALIDITY changed for {} ({:?} → {}), full resync",
            folder.path,
            saved.uidvalidity,
            delta.uidvalidity
        );

        let since_date = compute_since_date(days_back);
        let mut session = connect(config).await?;
        let sr = client::search_folder(&mut session, &folder.raw_path, Some(since_date)).await?;

        if sr.uids.is_empty() {
            let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
            // Persist the new uidvalidity even for empty folders, so we don't
            // repeat the expensive UIDVALIDITY recovery on every sync cycle
            let aid = account_id.to_string();
            let fp = folder.raw_path.clone();
            let uv = sr.folder_status.uidvalidity;
            let ms = sr.folder_status.highest_modseq;
            let sat = chrono::Utc::now().timestamp();
            db.with_conn(move |conn| {
                sync_pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, 0, sat, ms)
            })
            .await?;
            return Ok(());
        }

        let (lu, _) = fetch_uids_on_session(
            &mut session,
            account_id,
            folder,
            folder_label_id,
            &sr.uids,
            db,
            body_store,
            inline_images,
            search,
            all_threadable,
            all_meta,
            labels_by_rfc_id,
        )
        .await?;

        let aid = account_id.to_string();
        let fp = folder.raw_path.clone();
        let uv = sr.folder_status.uidvalidity;
        let ms = sr.folder_status.highest_modseq;
        let sat = chrono::Utc::now().timestamp();
        db.with_conn(move |conn| {
            sync_pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, lu, sat, ms)
        })
        .await?;

        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
        return Ok(());
    }

    if delta.new_uids.is_empty() {
        // No new UIDs — check for flag changes.
        if let (Some(cached_modseq), Some(server_modseq)) = (saved.modseq, delta.highest_modseq) {
            // CONDSTORE path: modseq changed → use CHANGEDSINCE for efficient diff.
            if server_modseq > cached_modseq {
                log::info!(
                    "[sync] {} modseq changed ({cached_modseq} → {server_modseq}), fetching flag changes",
                    folder.path
                );
                let mut session = connect(config).await?;
                match client::fetch_changed_flags(&mut session, &folder.raw_path, cached_modseq)
                    .await
                {
                    Ok(changes) if !changes.is_empty() => {
                        let aid = account_id.to_string();
                        let fp = folder.raw_path.clone();
                        let ch = changes;
                        db.with_conn(move |conn| {
                            sync_pipeline::apply_flag_changes(conn, &aid, &fp, &ch)
                        })
                        .await?;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        // CHANGEDSINCE may fail on servers that advertise modseq
                        // but don't fully support CONDSTORE. Log and continue.
                        log::warn!(
                            "[sync] CHANGEDSINCE failed for {}, falling back: {e}",
                            folder.path
                        );
                    }
                }
                let _ =
                    tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
            }
        } else if delta.highest_modseq.is_none() {
            // Non-CONDSTORE fallback: server doesn't support CONDSTORE, so we
            // periodically fetch all flags and diff against local cache.
            // This covers Exchange IMAP, Courier, hMailServer, etc.
            match sync_flags_without_condstore(config, &folder.raw_path, account_id, db).await {
                Ok(updated) if updated > 0 => {
                    log::info!(
                        "[sync] Non-CONDSTORE flag sync for {}: {updated} flags updated",
                        folder.path
                    );
                }
                Ok(_) => {}
                Err(e) => {
                    log::warn!(
                        "[sync] Non-CONDSTORE flag sync failed for {}: {e}",
                        folder.path
                    );
                }
            }
        }

        // Persist updated modseq regardless of whether flag sync succeeded
        if delta.highest_modseq != saved.modseq {
            let aid = account_id.to_string();
            let fp = folder.raw_path.clone();
            let uv = delta.uidvalidity;
            let lu = saved.last_uid;
            let ms = delta.highest_modseq;
            let sat = chrono::Utc::now().timestamp();
            db.with_conn(move |conn| {
                sync_pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, lu, sat, ms)
            })
            .await?;
        }
        return Ok(());
    }

    let mut session = connect(config).await?;
    let (dlu, uv) = fetch_uids_on_session(
        &mut session,
        account_id,
        folder,
        folder_label_id,
        &delta.new_uids,
        db,
        body_store,
        inline_images,
        search,
        all_threadable,
        all_meta,
        labels_by_rfc_id,
    )
    .await?;

    // Non-CONDSTORE: also sync flags on existing messages while we're at it,
    // since we can't rely on CHANGEDSINCE to detect changes.
    if delta.highest_modseq.is_none() {
        match sync_flags_without_condstore(config, &folder.raw_path, account_id, db).await {
            Ok(updated) if updated > 0 => {
                log::info!(
                    "[sync] Non-CONDSTORE flag sync for {} (with new UIDs): {updated} flags updated",
                    folder.path
                );
            }
            Ok(_) => {}
            Err(e) => {
                log::warn!(
                    "[sync] Non-CONDSTORE flag sync failed for {}: {e}",
                    folder.path
                );
            }
        }
    }

    let last_uid = saved.last_uid.max(dlu);
    let aid = account_id.to_string();
    let fp = folder.raw_path.clone();
    let ms = delta.highest_modseq;
    let sat = chrono::Utc::now().timestamp();
    db.with_conn(move |conn| {
        sync_pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, last_uid, sat, ms)
    })
    .await?;

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
    Ok(())
}

/// Minimum interval between non-CONDSTORE flag sync checks per folder (seconds).
/// `UID FETCH 1:* (FLAGS)` is cheaper than body fetches but still non-trivial
/// on large folders.
const FLAG_SYNC_INTERVAL_SECS: i64 = 300; // 5 minutes

/// Sync flags for servers that don't support CONDSTORE.
///
/// Fetches all current flags via `UID FETCH 1:* (FLAGS)`, diffs against
/// locally cached flags, and applies any changes. This is the fallback for
/// servers like Exchange IMAP, Courier, and hMailServer that lack CONDSTORE.
///
/// Returns the number of flags updated.
pub async fn sync_flags_without_condstore(
    config: &ImapConfig,
    folder_path: &str,
    account_id: &str,
    db: &DbState,
) -> Result<u64, String> {
    // Throttle: only check every FLAG_SYNC_INTERVAL_SECS
    let now = chrono::Utc::now().timestamp();
    let aid = account_id.to_string();
    let fp = folder_path.to_string();
    let last_sync = db
        .with_conn(move |conn| {
            sync_pipeline::get_last_deletion_check_at(conn, &aid, &fp)
        })
        .await;

    // Reuse the deletion check timestamp table for throttling. If we can't
    // read it, proceed anyway (first run).
    if let Ok(Some(last)) = &last_sync
        && now - last < FLAG_SYNC_INTERVAL_SECS
    {
        return Ok(0);
    }

    // Get local flags
    let aid = account_id.to_string();
    let fp = folder_path.to_string();
    let local_flags = db
        .with_conn(move |conn| sync_pipeline::get_local_flags_for_folder(conn, &aid, &fp))
        .await?;

    if local_flags.is_empty() {
        return Ok(0);
    }

    let local_map: std::collections::HashMap<u32, (bool, bool)> = local_flags
        .into_iter()
        .map(|(uid, is_read, is_starred)| (uid, (is_read, is_starred)))
        .collect();

    // Fetch current flags from server
    let mut session = connect(config).await?;
    let server_flags = client::fetch_all_flags(&mut session, folder_path).await?;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;

    // Diff: only include UIDs where flags actually changed
    let changes: Vec<super::types::FlagChange> = server_flags
        .into_iter()
        .filter(|sf| {
            match local_map.get(&sf.uid) {
                Some(&(local_read, local_starred)) => {
                    sf.is_read != local_read || sf.is_starred != local_starred
                }
                None => false, // UID not in local DB, skip (will be fetched as new)
            }
        })
        .collect();

    if changes.is_empty() {
        log::debug!(
            "[sync] Non-CONDSTORE flag sync for {folder_path}: no changes"
        );
        return Ok(0);
    }

    log::info!(
        "[sync] Non-CONDSTORE flag sync for {folder_path}: {} flag changes",
        changes.len()
    );

    let aid = account_id.to_string();
    let fp = folder_path.to_string();
    let updated = db
        .with_conn(move |conn| sync_pipeline::apply_flag_changes(conn, &aid, &fp, &changes))
        .await?;

    Ok(updated)
}

/// Detect messages deleted on the IMAP server by comparing `UID SEARCH ALL`
/// results against locally-cached UIDs.
///
/// This is the core detection function: it connects to the server, gets all
/// UIDs for the folder, diffs against the local DB, and returns the local
/// message IDs whose server-side UIDs no longer exist.
///
/// Only runs if enough time has elapsed since the last check (controlled by
/// `DELETION_CHECK_INTERVAL_SECS`).
pub async fn detect_deleted_messages(
    config: &ImapConfig,
    folder_path: &str,
    account_id: &str,
    db: &DbState,
) -> Result<Vec<String>, String> {
    // Throttle: only check every DELETION_CHECK_INTERVAL_SECS
    let now = chrono::Utc::now().timestamp();
    let aid = account_id.to_string();
    let fp = folder_path.to_string();
    let should_run = db
        .with_conn(move |conn| {
            match sync_pipeline::get_last_deletion_check_at(conn, &aid, &fp) {
                Ok(Some(last)) if now - last < DELETION_CHECK_INTERVAL_SECS => Ok(false),
                Ok(_) => Ok(true),
                Err(e) => {
                    // If the row doesn't exist yet (new folder), skip
                    log::debug!("get_last_deletion_check_at: {e}");
                    Ok(false)
                }
            }
        })
        .await?;

    if !should_run {
        return Ok(vec![]);
    }

    // Get all UIDs currently on server
    let mut session = connect(config).await?;
    let server_uids = client::search_all_uids(&mut session, folder_path).await?;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;

    let server_uid_set: HashSet<u32> = server_uids.into_iter().collect();

    // Get locally-cached UIDs for this folder
    let aid = account_id.to_string();
    let fp = folder_path.to_string();
    let local_entries = db
        .with_conn(move |conn| sync_pipeline::get_local_uids_for_folder(conn, &aid, &fp))
        .await?;

    // Diff: local UIDs not on server = deleted
    let deleted_ids: Vec<String> = local_entries
        .into_iter()
        .filter(|(_, uid)| !server_uid_set.contains(uid))
        .map(|(id, _)| id)
        .collect();

    // Update the last check timestamp
    let aid = account_id.to_string();
    let fp = folder_path.to_string();
    db.with_conn(move |conn| sync_pipeline::set_last_deletion_check_at(conn, &aid, &fp, now))
        .await?;

    if !deleted_ids.is_empty() {
        log::info!(
            "[sync] Deletion detection for {folder_path}: {} messages deleted on server",
            deleted_ids.len()
        );
    }

    Ok(deleted_ids)
}

/// Run deletion detection across all synced folders and remove deleted messages.
///
/// Designed to be called from the delta sync flow. For each folder that has
/// a saved sync state, checks if enough time has elapsed, then runs
/// `UID SEARCH ALL` to find deletions.
///
/// Returns the list of affected thread IDs (for UI refresh).
pub async fn run_deletion_detection(
    config: &ImapConfig,
    account_id: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    search: &SearchState,
    syncable_folders: &[&super::types::ImapFolder],
    state_map: &HashMap<String, sync_pipeline::FolderSyncState>,
) -> Vec<String> {
    let mut all_affected = Vec::new();

    for folder in syncable_folders {
        // Only check folders we've already synced
        if !state_map.contains_key(&folder.raw_path) {
            continue;
        }

        match detect_deleted_messages(config, &folder.raw_path, account_id, db).await {
            Ok(deleted_ids) if !deleted_ids.is_empty() => {
                // Remove from body store
                if let Err(e) = body_store.delete(deleted_ids.clone()).await {
                    log::warn!("[sync] Failed to delete bodies for removed messages: {e}");
                }

                // Remove from search index
                let id_refs: Vec<&str> = deleted_ids.iter().map(String::as_str).collect();
                if let Err(e) = search.delete_messages_batch(&id_refs).await {
                    log::warn!("[sync] Failed to remove deleted messages from search: {e}");
                }

                // Remove from DB and update threads
                let aid = account_id.to_string();
                let ids = deleted_ids;
                match db
                    .with_conn(move |conn| sync_pipeline::remove_deleted_messages(conn, &aid, &ids))
                    .await
                {
                    Ok(affected) => all_affected.extend(affected),
                    Err(e) => {
                        log::error!("[sync] Failed to remove deleted messages from DB: {e}");
                    }
                }
            }
            Ok(_) => {} // No deletions or throttled
            Err(e) => {
                log::warn!(
                    "[sync] Deletion detection failed for {}: {e}",
                    folder.path
                );
            }
        }
    }

    all_affected
}
