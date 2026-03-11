#![allow(clippy::let_underscore_must_use)]

use std::collections::{HashMap, HashSet};

use tauri::AppHandle;

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::imap::client;
use crate::imap::connection::connect;
use crate::imap::types::{DeltaCheckRequest, DeltaCheckResult, ImapConfig};
use crate::inline_image_store::InlineImageStoreState;
use crate::search::SearchState;
use crate::threading;

use super::convert::{ConvertedMessage, convert_imap_message};
use super::folder_mapper::{get_syncable_folders, map_folder_to_label};
use super::pipeline;
use super::pipeline::{CHUNK_SIZE, store_chunk};
use super::types::{ImapSyncResult, MessageMeta};

const CIRCUIT_BREAKER_THRESHOLD: u32 = 3;
const CIRCUIT_BREAKER_DELAY_MS: u64 = 15_000;
const CIRCUIT_BREAKER_MAX_FAILURES: u32 = 5;

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
    _app: &AppHandle,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    account_id: &str,
    config: &ImapConfig,
    days_back: i64,
) -> Result<ImapSyncResult, String> {
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
            pipeline::sync_folders_to_labels(conn, &aid, &refs)
        })
        .await?;
    }

    // Get saved folder sync states
    let sync_states = {
        let aid = account_id.to_string();
        db.with_conn(move |conn| pipeline::get_all_folder_sync_states(conn, &aid))
            .await?
    };
    let state_map: HashMap<String, pipeline::FolderSyncState> = sync_states
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

    if all_threadable.is_empty() && !delta_errors.is_empty() {
        return Err(format!("All folders failed: {}", delta_errors[0]));
    }

    if all_threadable.is_empty() {
        return Ok(ImapSyncResult {
            stored_count: 0,
            thread_count: 0,
            new_inbox_message_ids: vec![],
            affected_thread_ids: vec![],
        });
    }

    // Thread + store
    let thread_groups = threading::build_threads(&all_threadable);
    let tids: Vec<String> = thread_groups.iter().map(|g| g.thread_id.clone()).collect();

    let skipped = {
        let aid = account_id.to_string();
        let t = tids.clone();
        db.with_conn(move |conn| pipeline::get_skipped_thread_ids(conn, &aid, &t))
            .await?
    };

    let affected = {
        let aid = account_id.to_string();
        let tg = thread_groups.clone();
        let m = all_meta.clone();
        let l = labels_by_rfc_id.clone();
        let s = skipped;
        db.with_conn(move |conn| pipeline::store_threads(conn, &aid, &tg, &m, &l, &s))
            .await?
    };

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
        db.with_conn(move |conn| pipeline::update_account_sync_state(conn, &aid, &marker))
            .await?;
    }

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
    existing_folders: &[&&crate::imap::types::ImapFolder],
    state_map: &HashMap<String, pipeline::FolderSyncState>,
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
    saved: &pipeline::FolderSyncState,
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
        });
    }

    let new_uids = client::fetch_new_uids(&mut session, folder_path, saved.last_uid).await?;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;

    Ok(DeltaCheckResult {
        folder: folder_path.to_string(),
        uidvalidity: status.uidvalidity,
        new_uids,
        uidvalidity_changed: false,
    })
}

/// Fetch all UIDs from a folder and store them.
#[allow(clippy::too_many_arguments)]
async fn fetch_folder_uids(
    config: &ImapConfig,
    account_id: &str,
    folder: &crate::imap::types::ImapFolder,
    folder_label_id: &str,
    since_date: &str,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    all_threadable: &mut Vec<crate::threading::ThreadableMessage>,
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
        let sat = chrono::Utc::now().timestamp();
        db.with_conn(move |conn| pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, 0, sat))
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
    let sat = chrono::Utc::now().timestamp();
    db.with_conn(move |conn| {
        pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, last_uid, sat)
    })
    .await?;

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
    Ok(())
}

/// Fetch UIDs on an existing session, store messages. Returns (last_uid, uidvalidity).
#[allow(clippy::too_many_arguments)]
async fn fetch_uids_on_session(
    session: &mut crate::imap::connection::ImapSession,
    account_id: &str,
    folder: &crate::imap::types::ImapFolder,
    folder_label_id: &str,
    uids: &[u32],
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    all_threadable: &mut Vec<crate::threading::ThreadableMessage>,
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
    folder: &crate::imap::types::ImapFolder,
    folder_label_id: &str,
    saved: &pipeline::FolderSyncState,
    delta: &DeltaCheckResult,
    days_back: i64,
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    all_threadable: &mut Vec<crate::threading::ThreadableMessage>,
    all_meta: &mut HashMap<String, MessageMeta>,
    labels_by_rfc_id: &mut HashMap<String, HashSet<String>>,
) -> Result<(), String> {
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
            let sat = chrono::Utc::now().timestamp();
            db.with_conn(move |conn| {
                pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, 0, sat)
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
        let sat = chrono::Utc::now().timestamp();
        db.with_conn(move |conn| pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, lu, sat))
            .await?;

        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
        return Ok(());
    }

    if delta.new_uids.is_empty() {
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

    let last_uid = saved.last_uid.max(dlu);
    let aid = account_id.to_string();
    let fp = folder.raw_path.clone();
    let sat = chrono::Utc::now().timestamp();
    db.with_conn(move |conn| {
        pipeline::upsert_folder_sync_state(conn, &aid, &fp, uv, last_uid, sat)
    })
    .await?;

    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), session.logout()).await;
    Ok(())
}
