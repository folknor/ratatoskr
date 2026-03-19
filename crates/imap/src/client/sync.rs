use async_imap::types::Flag;
use futures::StreamExt;
use mail_parser::MessageParser;

use super::super::connection::{
    IMAP_CMD_TIMEOUT, IMAP_FETCH_TIMEOUT, IMAP_SEARCH_TIMEOUT, ImapSession,
};
use super::super::parse::parse_message;
use super::super::types::*;
use super::{mailbox_supports_custom_keywords, timeout_err};

/// Check multiple folders for new UIDs in a single IMAP session.
///
/// For each folder: SELECT, compare UIDVALIDITY, UID SEARCH for new messages.
/// This replaces N separate connections (status + fetch_new_uids per folder)
/// with a single connection that checks all folders.
#[allow(clippy::too_many_lines)]
pub async fn delta_check_folders(
    session: &mut ImapSession,
    folders: &[DeltaCheckRequest],
) -> Result<Vec<DeltaCheckResult>, String> {
    let mut results = Vec::with_capacity(folders.len());

    for req in folders {
        let mailbox =
            match tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(&req.folder)).await {
                Ok(Ok(m)) => m,
                Ok(Err(e)) => {
                    log::warn!("delta_check: SELECT {} failed: {e}", req.folder);
                    continue;
                }
                Err(_) => {
                    log::warn!(
                        "delta_check: SELECT {} timed out after {}s",
                        req.folder,
                        IMAP_CMD_TIMEOUT.as_secs()
                    );
                    continue;
                }
            };

        let current_uidvalidity = mailbox.uid_validity.unwrap_or(0);
        let server_modseq = mailbox.highest_modseq;
        let uidvalidity_changed = req.uidvalidity != 0 && current_uidvalidity != req.uidvalidity;

        if uidvalidity_changed {
            results.push(DeltaCheckResult {
                folder: req.folder.clone(),
                uidvalidity: current_uidvalidity,
                new_uids: vec![],
                uidvalidity_changed: true,
                highest_modseq: server_modseq,
                modseq_unchanged: false,
                modseq_reset: false,
            });
            continue;
        }

        // CONDSTORE: detect modseq reset (server < cached with same UIDVALIDITY).
        // This can happen during server migration or mailbox repair.
        let modseq_reset = matches!((req.last_modseq, server_modseq), (Some(cached), Some(server)) if server < cached);

        if modseq_reset {
            log::warn!(
                "delta_check: {} HIGHESTMODSEQ reset detected (cached {} > server {}), \
                 will trigger full flag resync",
                req.folder,
                req.last_modseq.unwrap_or(0),
                server_modseq.unwrap_or(0)
            );
            // Still do the UID SEARCH for new messages, but signal the reset
            // so the caller knows to do a full flag resync instead of CHANGEDSINCE.
        }

        // CONDSTORE fast path: if server's HIGHESTMODSEQ matches our cached
        // value, nothing changed (no new messages, no flag changes, no deletions).
        let modseq_unchanged = !modseq_reset && matches!((req.last_modseq, server_modseq), (Some(cached), Some(server)) if cached == server);

        if modseq_unchanged {
            log::debug!(
                "delta_check: {} modseq unchanged ({}), skipping UID SEARCH",
                req.folder,
                server_modseq.unwrap_or(0)
            );
            results.push(DeltaCheckResult {
                folder: req.folder.clone(),
                uidvalidity: current_uidvalidity,
                new_uids: vec![],
                uidvalidity_changed: false,
                highest_modseq: server_modseq,
                modseq_unchanged: true,
                modseq_reset: false,
            });
            continue;
        }

        // UID SEARCH for messages newer than last_uid
        let query = format!("{}:*", req.last_uid + 1);
        let new_uids = match tokio::time::timeout(IMAP_SEARCH_TIMEOUT, session.uid_search(&query))
            .await
        {
            Ok(Ok(uids)) => {
                let mut result: Vec<u32> = uids.into_iter().filter(|&u| u > req.last_uid).collect();
                result.sort();
                result
            }
            Ok(Err(e)) => {
                log::warn!("delta_check: UID SEARCH {} failed: {e}", req.folder);
                vec![]
            }
            Err(_) => {
                log::warn!(
                    "delta_check: UID SEARCH {} timed out after {}s",
                    req.folder,
                    IMAP_SEARCH_TIMEOUT.as_secs()
                );
                vec![]
            }
        };

        results.push(DeltaCheckResult {
            folder: req.folder.clone(),
            uidvalidity: current_uidvalidity,
            new_uids,
            uidvalidity_changed: false,
            highest_modseq: server_modseq,
            modseq_unchanged: false,
            modseq_reset,
        });
    }

    Ok(results)
}

/// Search a folder: SELECT -> UID SEARCH, returning UIDs and folder status without fetching bodies.
///
/// This is a lightweight alternative to `sync_folder` for callers that want to
/// fetch messages in smaller IPC-friendly chunks on the TypeScript side.
pub async fn search_folder(
    session: &mut ImapSession,
    folder: &str,
    since_date: Option<String>,
) -> Result<ImapFolderSearchResult, String> {
    // SELECT the folder
    let mailbox = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let folder_status = ImapFolderStatus {
        uidvalidity: mailbox.uid_validity.unwrap_or(0),
        uidnext: mailbox.uid_next.unwrap_or(0),
        exists: mailbox.exists,
        unseen: mailbox.unseen.unwrap_or(0),
        highest_modseq: mailbox.highest_modseq,
        supports_custom_keywords: mailbox_supports_custom_keywords(&mailbox),
    };

    // UID SEARCH with optional SINCE date filter (RFC 3501 §6.4.4)
    let search_query = match &since_date {
        Some(date) => format!("SINCE {date}"),
        None => "ALL".to_string(),
    };
    let uids_raw = tokio::time::timeout(IMAP_SEARCH_TIMEOUT, session.uid_search(&search_query))
        .await
        .map_err(|_| {
            timeout_err(
                &format!("UID SEARCH {search_query} {folder}"),
                IMAP_SEARCH_TIMEOUT,
            )
        })?
        .map_err(|e| format!("UID SEARCH {search_query} {folder} failed: {e}"))?;

    let mut uids: Vec<u32> = uids_raw.into_iter().collect();
    uids.sort();

    log::info!(
        "IMAP search_folder {folder}: {} UIDs found (search={search_query}), uidvalidity={}",
        uids.len(),
        folder_status.uidvalidity,
    );

    Ok(ImapFolderSearchResult {
        uids,
        folder_status,
    })
}

/// Sync a folder in a single IMAP session: SELECT -> UID SEARCH -> batched UID FETCH.
///
/// When `since_date` is provided (format `DD-Mon-YYYY`), uses `UID SEARCH SINCE <date>`
/// to only fetch messages from that date onward, avoiding timeouts on large folders.
///
/// This avoids creating multiple TCP connections per folder (one for search,
/// one per batch for fetch) which causes connection storms on servers with
/// many folders.
#[allow(clippy::too_many_lines)]
pub async fn sync_folder(
    session: &mut ImapSession,
    folder: &str,
    batch_size: u32,
    since_date: Option<String>,
) -> Result<ImapFolderSyncResult, String> {
    // SELECT the folder
    let mailbox = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let folder_status = ImapFolderStatus {
        uidvalidity: mailbox.uid_validity.unwrap_or(0),
        uidnext: mailbox.uid_next.unwrap_or(0),
        exists: mailbox.exists,
        unseen: mailbox.unseen.unwrap_or(0),
        highest_modseq: mailbox.highest_modseq,
        supports_custom_keywords: mailbox_supports_custom_keywords(&mailbox),
    };

    // UID SEARCH with optional SINCE date filter (RFC 3501 §6.4.4)
    let search_query = match &since_date {
        Some(date) => format!("SINCE {date}"),
        None => "ALL".to_string(),
    };
    let uids_raw = tokio::time::timeout(IMAP_SEARCH_TIMEOUT, session.uid_search(&search_query))
        .await
        .map_err(|_| {
            timeout_err(
                &format!("UID SEARCH {search_query} {folder}"),
                IMAP_SEARCH_TIMEOUT,
            )
        })?
        .map_err(|e| format!("UID SEARCH {search_query} {folder} failed: {e}"))?;

    let mut uids: Vec<u32> = uids_raw.into_iter().collect();
    uids.sort();

    log::info!(
        "IMAP sync_folder {folder}: {} UIDs found (search={search_query}), uidvalidity={}, batch_size={}",
        uids.len(),
        folder_status.uidvalidity,
        batch_size,
    );

    if uids.is_empty() {
        return Ok(ImapFolderSyncResult {
            uids,
            messages: vec![],
            folder_status,
        });
    }

    // Fetch in batches on the SAME session
    let parser = MessageParser::default();
    let mut all_messages = Vec::new();
    let bs = batch_size as usize;

    for chunk in uids.chunks(bs) {
        let uid_set: String = chunk
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",");

        let fetches = tokio::time::timeout(IMAP_FETCH_TIMEOUT, async {
            let stream = session
                .uid_fetch(&uid_set, "UID FLAGS INTERNALDATE BODY.PEEK[]")
                .await
                .map_err(|e| format!("UID FETCH {folder} uids={uid_set} failed: {e}"))?;
            Ok::<_, String>(stream.collect::<Vec<_>>().await)
        })
        .await
        .map_err(|_| timeout_err(&format!("UID FETCH {folder}"), IMAP_FETCH_TIMEOUT))?;

        let raw_fetches: Vec<_> = fetches?;
        for r in raw_fetches {
            match r {
                Ok(f) => {
                    let uid = match f.uid {
                        Some(u) => u,
                        None => {
                            log::warn!("IMAP sync_folder {folder}: response missing UID");
                            continue;
                        }
                    };
                    let raw = match f.body() {
                        Some(b) => b,
                        None => {
                            log::warn!("IMAP sync_folder {folder}: UID {uid} has no body");
                            continue;
                        }
                    };
                    #[allow(clippy::cast_possible_truncation)]
                    let raw_size = raw.len() as u32;
                    let flags: Vec<_> = f.flags().collect();
                    let is_read = flags.iter().any(|fl| matches!(fl, Flag::Seen));
                    let is_starred = flags.iter().any(|fl| matches!(fl, Flag::Flagged));
                    let is_draft = flags.iter().any(|fl| matches!(fl, Flag::Draft));
                    let internal_date = f.internal_date().map(|dt| dt.timestamp());

                    match parse_message(
                        &parser,
                        raw,
                        uid,
                        folder,
                        raw_size,
                        is_read,
                        is_starred,
                        is_draft,
                        internal_date,
                    ) {
                        Ok(msg) => all_messages.push(msg),
                        Err(e) => log::warn!("sync_folder: failed to parse UID {uid}: {e}"),
                    }
                }
                Err(e) => log::warn!("IMAP sync_folder fetch stream error in {folder}: {e}"),
            }
        }
    }

    log::info!(
        "IMAP sync_folder {folder}: fetched {} messages",
        all_messages.len()
    );

    Ok(ImapFolderSyncResult {
        uids,
        messages: all_messages,
        folder_status,
    })
}
