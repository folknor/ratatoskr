use std::collections::{HashMap, HashSet};

use db::db::ReadDbState;
use search::SearchState;
use store::body_store::BodyStoreReadState;

use super::client;
use super::connection::connect;
use super::sync_pipeline;
use super::types::ImapConfig;

use super::is_connection_error;

/// Minimum interval between deletion detection checks per folder (seconds).
/// UID SEARCH ALL can be expensive on large folders, so we throttle it.
const DELETION_CHECK_INTERVAL_SECS: i64 = 600; // 10 minutes

/// Minimum interval between non-CONDSTORE flag sync checks per folder (seconds).
/// `UID FETCH 1:* (FLAGS)` is cheaper than body fetches but still non-trivial
/// on large folders.
const FLAG_SYNC_INTERVAL_SECS: i64 = 300; // 5 minutes

/// Sync flags for servers that don't support CONDSTORE, reusing an existing
/// IMAP session.
///
/// Fetches all current flags via `UID FETCH 1:* (FLAGS)`, diffs against
/// locally cached flags, and applies any changes. This is the fallback for
/// servers like Exchange IMAP, Courier, and hMailServer that lack CONDSTORE.
///
/// The caller is responsible for connecting/disconnecting the session.
/// Returns the number of flags updated.
pub(crate) async fn sync_flags_on_session(
    session: &mut super::connection::ImapSession,
    folder_path: &str,
    account_id: &str,
    db: &ReadDbState,
) -> Result<u64, String> {
    // Throttle: only check every FLAG_SYNC_INTERVAL_SECS
    let now = chrono::Utc::now().timestamp();
    let aid = account_id.to_string();
    let fp = folder_path.to_string();
    let last_sync = db
        .with_conn(move |conn| sync_pipeline::get_last_deletion_check_at(conn, &aid, &fp))
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

    // Fetch current flags from server (SELECT + UID FETCH handled by callee)
    let server_flags = client::fetch_all_flags(session, folder_path).await?;

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
        log::debug!("[sync] Non-CONDSTORE flag sync for {folder_path}: no changes");
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

/// Sync flags for servers that don't support CONDSTORE.
///
/// Convenience wrapper that opens (and closes) its own IMAP connection.
/// Prefer `sync_flags_on_session` when a session is already available.
pub async fn sync_flags_without_condstore(
    config: &ImapConfig,
    folder_path: &str,
    account_id: &str,
    db: &ReadDbState,
) -> Result<u64, String> {
    let mut session = connect(config).await?;
    let result = sync_flags_on_session(&mut session, folder_path, account_id, db).await;
    let _ = tokio::time::timeout(crate::connection::IMAP_LOGOUT_TIMEOUT, session.logout()).await;
    result
}

/// Detect messages deleted on the IMAP server by comparing `UID SEARCH ALL`
/// results against locally-cached UIDs, reusing an existing IMAP session.
///
/// The caller is responsible for connecting/disconnecting the session.
/// `SELECT folder` is issued internally by `search_all_uids`.
///
/// Only runs if enough time has elapsed since the last check (controlled by
/// `DELETION_CHECK_INTERVAL_SECS`).
async fn detect_deleted_on_session(
    session: &mut super::connection::ImapSession,
    folder_path: &str,
    account_id: &str,
    db: &ReadDbState,
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

    // Get all UIDs currently on server (SELECT + UID SEARCH handled by callee)
    let server_uids = client::search_all_uids(session, folder_path).await?;

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

/// Detect messages deleted on the IMAP server by comparing `UID SEARCH ALL`
/// results against locally-cached UIDs.
///
/// Convenience wrapper that opens (and closes) its own IMAP connection.
/// Prefer `detect_deleted_on_session` when a session is already available.
pub async fn detect_deleted_messages(
    config: &ImapConfig,
    folder_path: &str,
    account_id: &str,
    db: &ReadDbState,
) -> Result<Vec<String>, String> {
    let mut session = connect(config).await?;
    let result = detect_deleted_on_session(&mut session, folder_path, account_id, db).await;
    let _ = tokio::time::timeout(crate::connection::IMAP_LOGOUT_TIMEOUT, session.logout()).await;
    result
}

/// Run deletion detection across all synced folders and remove deleted messages.
///
/// Opens a single IMAP connection and reuses it across all folders, using
/// `SELECT` to switch between them. This avoids the overhead of a separate
/// TLS handshake per folder.
///
/// Returns the list of affected thread IDs (for UI refresh).
pub async fn run_deletion_detection(
    config: &ImapConfig,
    account_id: &str,
    db: &ReadDbState,
    body_store: &BodyStoreReadState,
    search: &SearchState,
    syncable_folders: &[&super::types::ImapFolder],
    state_map: &HashMap<String, sync_pipeline::FolderSyncState>,
) -> Vec<String> {
    let mut all_affected = Vec::new();

    // Filter to folders that need checking (already synced)
    let folders_to_check: Vec<_> = syncable_folders
        .iter()
        .filter(|f| state_map.contains_key(&f.raw_path))
        .collect();

    if folders_to_check.is_empty() {
        return all_affected;
    }

    // Open one connection for all folder deletion checks
    let mut session = match connect(config).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("[sync] Deletion detection: failed to connect: {e}");
            return all_affected;
        }
    };

    for folder in &folders_to_check {
        match detect_deleted_on_session(&mut session, &folder.raw_path, account_id, db).await {
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
                log::warn!("[sync] Deletion detection failed for {}: {e}", folder.path);
                // Connection may be broken - try to reconnect and retry the failed folder
                if is_connection_error(&e) {
                    log::info!("[sync] Reconnecting for remaining deletion checks...");
                    match connect(config).await {
                        Ok(s) => {
                            session = s;
                            // Retry the folder that triggered the reconnect
                            match detect_deleted_on_session(
                                &mut session,
                                &folder.raw_path,
                                account_id,
                                db,
                            )
                            .await
                            {
                                Ok(deleted_ids) if !deleted_ids.is_empty() => {
                                    if let Err(e) = body_store.delete(deleted_ids.clone()).await {
                                        log::warn!(
                                            "[sync] Failed to delete bodies for removed messages: {e}"
                                        );
                                    }
                                    let id_refs: Vec<&str> =
                                        deleted_ids.iter().map(String::as_str).collect();
                                    if let Err(e) = search.delete_messages_batch(&id_refs).await {
                                        log::warn!(
                                            "[sync] Failed to remove deleted messages from search: {e}"
                                        );
                                    }
                                    let aid = account_id.to_string();
                                    let ids = deleted_ids;
                                    match db
                                        .with_conn(move |conn| {
                                            sync_pipeline::remove_deleted_messages(conn, &aid, &ids)
                                        })
                                        .await
                                    {
                                        Ok(affected) => all_affected.extend(affected),
                                        Err(e) => {
                                            log::error!(
                                                "[sync] Failed to remove deleted messages from DB: {e}"
                                            );
                                        }
                                    }
                                }
                                Ok(_) => {}
                                Err(e2) => {
                                    log::warn!(
                                        "[sync] Retry deletion detection for {} also failed: {e2}",
                                        folder.path
                                    );
                                }
                            }
                        }
                        Err(e2) => {
                            log::error!(
                                "[sync] Reconnect failed, aborting deletion detection: {e2}"
                            );
                            break;
                        }
                    }
                }
            }
        }
    }

    let _ = tokio::time::timeout(crate::connection::IMAP_LOGOUT_TIMEOUT, session.logout()).await;
    all_affected
}
