use futures::StreamExt;

use super::super::connection::{IMAP_CMD_TIMEOUT, IMAP_FETCH_TIMEOUT, ImapSession};
use super::super::types::*;
use super::{mailbox_supports_custom_keywords, timeout_err};

use async_imap::types::Flag;

/// Get UIDs of messages newer than `last_uid`.
pub async fn fetch_new_uids(
    session: &mut ImapSession,
    folder: &str,
    last_uid: u32,
) -> Result<Vec<u32>, String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let query = format!("{}:*", last_uid + 1);
    let uids = tokio::time::timeout(
        super::super::connection::IMAP_SEARCH_TIMEOUT,
        session.uid_search(&query),
    )
    .await
    .map_err(|_| timeout_err("UID SEARCH", super::super::connection::IMAP_SEARCH_TIMEOUT))?
    .map_err(|e| format!("UID SEARCH failed: {e}"))?;

    // Filter out last_uid itself (IMAP returns it if it's the highest UID)
    let mut result: Vec<u32> = uids.into_iter().filter(|&u| u > last_uid).collect();
    result.sort();
    Ok(result)
}

/// Search for all UIDs in a folder using `UID SEARCH ALL`.
/// Returns real UIDs sorted ascending — avoids the sparse UID gap problem.
pub async fn search_all_uids(session: &mut ImapSession, folder: &str) -> Result<Vec<u32>, String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let uids = tokio::time::timeout(
        super::super::connection::IMAP_SEARCH_TIMEOUT,
        session.uid_search("ALL"),
    )
    .await
    .map_err(|_| timeout_err("UID SEARCH ALL", super::super::connection::IMAP_SEARCH_TIMEOUT))?
    .map_err(|e| format!("UID SEARCH ALL failed: {e}"))?;

    let mut result: Vec<u32> = uids.into_iter().collect();
    result.sort();
    Ok(result)
}

/// Set or remove flags on messages.
///
/// `flag_op`: "+FLAGS" to add, "-FLAGS" to remove
/// `flags`: e.g. "(\\Seen)" or "(\\Flagged)"
pub async fn set_flags(
    session: &mut ImapSession,
    folder: &str,
    uid_set: &str,
    flag_op: &str,
    flags: &str,
) -> Result<(), String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let query = format!("{flag_op} {flags}");
    tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
        let stream = session
            .uid_store(uid_set, &query)
            .await
            .map_err(|e| format!("UID STORE failed: {e}"))?;
        let _: Vec<_> = stream.collect().await;
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| timeout_err("UID STORE", IMAP_CMD_TIMEOUT))?
}

/// Set a custom keyword flag on a message, but only if the folder's
/// PERMANENTFLAGS includes `\*` (custom keywords allowed).
///
/// Returns `Ok(())` silently if the server does not support custom keywords.
pub async fn set_keyword_if_supported(
    session: &mut ImapSession,
    folder: &str,
    uid: u32,
    flag_op: &str,
    keyword: &str,
) -> Result<(), String> {
    let mailbox = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    if !mailbox_supports_custom_keywords(&mailbox) {
        log::debug!(
            "IMAP: folder {folder} does not support custom keywords, skipping keyword {keyword}"
        );
        return Ok(());
    }

    let uid_set = uid.to_string();
    let query = format!("{flag_op} ({keyword})");
    tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
        let stream = session
            .uid_store(&uid_set, &query)
            .await
            .map_err(|e| format!("UID STORE failed: {e}"))?;
        let _: Vec<_> = stream.collect().await;
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| timeout_err("UID STORE", IMAP_CMD_TIMEOUT))?
}

/// Move messages between folders.
///
/// Tries MOVE first; falls back to COPY + flag Deleted + EXPUNGE.
pub async fn move_messages(
    session: &mut ImapSession,
    source_folder: &str,
    uid_set: &str,
    dest_folder: &str,
) -> Result<(), String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(source_folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {source_folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {source_folder} failed: {e}"))?;

    // Try MOVE extension first
    match tokio::time::timeout(IMAP_CMD_TIMEOUT, session.uid_mv(uid_set, dest_folder)).await {
        Ok(Ok(())) => return Ok(()),
        _ => {
            // Fallback: COPY, then mark Deleted, then EXPUNGE
            tokio::time::timeout(IMAP_CMD_TIMEOUT, session.uid_copy(uid_set, dest_folder))
                .await
                .map_err(|_| timeout_err("UID COPY", IMAP_CMD_TIMEOUT))?
                .map_err(|e| format!("UID COPY failed: {e}"))?;

            tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
                let store_stream = session
                    .uid_store(uid_set, "+FLAGS (\\Deleted)")
                    .await
                    .map_err(|e| format!("UID STORE +Deleted failed: {e}"))?;
                let _: Vec<_> = store_stream.collect().await;
                Ok::<_, String>(())
            })
            .await
            .map_err(|_| timeout_err("UID STORE +Deleted", IMAP_CMD_TIMEOUT))??;

            tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
                let expunge_stream = session
                    .expunge()
                    .await
                    .map_err(|e| format!("EXPUNGE failed: {e}"))?;
                let _: Vec<_> = expunge_stream.collect().await;
                Ok::<_, String>(())
            })
            .await
            .map_err(|_| timeout_err("EXPUNGE", IMAP_CMD_TIMEOUT))??;
        }
    }

    Ok(())
}

/// Flag messages as deleted and expunge them.
pub async fn delete_messages(
    session: &mut ImapSession,
    folder: &str,
    uid_set: &str,
) -> Result<(), String> {
    tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
        let store_stream = session
            .uid_store(uid_set, "+FLAGS (\\Deleted)")
            .await
            .map_err(|e| format!("UID STORE +Deleted failed: {e}"))?;
        let _: Vec<_> = store_stream.collect().await;
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| timeout_err("UID STORE +Deleted", IMAP_CMD_TIMEOUT))??;

    tokio::time::timeout(IMAP_CMD_TIMEOUT, async {
        let expunge_stream = session
            .expunge()
            .await
            .map_err(|e| format!("EXPUNGE failed: {e}"))?;
        let _: Vec<_> = expunge_stream.collect().await;
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| timeout_err("EXPUNGE", IMAP_CMD_TIMEOUT))??;

    Ok(())
}

/// Append a raw message to a folder (for saving sent mail or drafts).
pub async fn append_message(
    session: &mut ImapSession,
    folder: &str,
    flags: Option<&str>,
    raw_message: &[u8],
) -> Result<(), String> {
    tokio::time::timeout(
        IMAP_FETCH_TIMEOUT,
        session.append(folder, flags, None, raw_message),
    )
    .await
    .map_err(|_| timeout_err("APPEND", IMAP_FETCH_TIMEOUT))?
    .map_err(|e| format!("APPEND failed: {e}"))
}

/// Get folder status (UIDVALIDITY, UIDNEXT, MESSAGES, UNSEEN).
pub async fn get_folder_status(
    session: &mut ImapSession,
    folder: &str,
) -> Result<ImapFolderStatus, String> {
    let mailbox = tokio::time::timeout(
        IMAP_CMD_TIMEOUT,
        session.status(folder, "(UIDVALIDITY UIDNEXT MESSAGES UNSEEN)"),
    )
    .await
    .map_err(|_| timeout_err("STATUS", IMAP_CMD_TIMEOUT))?
    .map_err(|e| format!("STATUS failed: {e}"))?;

    Ok(ImapFolderStatus {
        uidvalidity: mailbox.uid_validity.unwrap_or(0),
        uidnext: mailbox.uid_next.unwrap_or(0),
        exists: mailbox.exists,
        unseen: mailbox.unseen.unwrap_or(0),
        highest_modseq: mailbox.highest_modseq,
        // STATUS doesn't return PERMANENTFLAGS; caller should use SELECT-based
        // methods (fetch_messages, search_folder, etc.) to get this value.
        supports_custom_keywords: false,
    })
}

/// Fetch only messages whose flags changed since the given mod-sequence (RFC 7162 CONDSTORE).
///
/// Issues `UID FETCH 1:* (FLAGS) (CHANGEDSINCE <modseq>)` which returns only messages
/// whose metadata changed. The folder must already be SELECTed.
///
/// Returns an empty vec if the server doesn't support CONDSTORE or no flags changed.
pub async fn fetch_changed_flags(
    session: &mut ImapSession,
    folder: &str,
    since_modseq: u64,
) -> Result<Vec<FlagChange>, String> {
    // SELECT the folder first (needed for UID FETCH)
    let _mailbox = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    // Use uid_fetch with the CHANGEDSINCE modifier appended to the query.
    // async-imap passes the query string directly, so this produces:
    //   UID FETCH 1:* (FLAGS) (CHANGEDSINCE <modseq>)
    let query = format!("(FLAGS) (CHANGEDSINCE {since_modseq})");
    let stream = tokio::time::timeout(IMAP_FETCH_TIMEOUT, session.uid_fetch("1:*", &query))
        .await
        .map_err(|_| timeout_err(&format!("UID FETCH CHANGEDSINCE {folder}"), IMAP_FETCH_TIMEOUT))?
        .map_err(|e| format!("UID FETCH CHANGEDSINCE {folder} failed: {e}"))?;

    let raw: Vec<_> = tokio::time::timeout(
        IMAP_FETCH_TIMEOUT,
        stream.collect::<Vec<_>>(),
    )
    .await
    .map_err(|_| timeout_err(&format!("CHANGEDSINCE stream {folder}"), IMAP_FETCH_TIMEOUT))?;

    let mut changes = Vec::new();
    for item in raw {
        match item {
            Ok(fetch) => {
                let uid = match fetch.uid {
                    Some(u) => u,
                    None => continue,
                };
                let flags: Vec<_> = fetch.flags().collect();
                let is_read = flags.iter().any(|f| matches!(f, Flag::Seen));
                let is_starred = flags.iter().any(|f| matches!(f, Flag::Flagged));
                let keywords: Vec<String> = flags
                    .iter()
                    .filter_map(|f| match f {
                        Flag::Custom(cow) => Some(cow.to_string()),
                        _ => None,
                    })
                    .collect();
                changes.push(FlagChange {
                    uid,
                    is_read,
                    is_starred,
                    keywords,
                });
            }
            Err(e) => {
                log::warn!("CHANGEDSINCE fetch stream error in {folder}: {e}");
            }
        }
    }

    log::info!(
        "IMAP CHANGEDSINCE {folder} (modseq={since_modseq}): {} flag changes",
        changes.len()
    );
    Ok(changes)
}

/// Fetch flags for all messages in a folder (non-CONDSTORE fallback).
///
/// Issues `UID FETCH 1:* (FLAGS)` to get the current flag state for every
/// message, then diffs against the locally cached flags to produce a list
/// of changes. This is the fallback for servers that don't support
/// CONDSTORE (e.g. Exchange IMAP, Courier, hMailServer).
///
/// The folder must NOT already be SELECTed — this function SELECTs it.
pub async fn fetch_all_flags(
    session: &mut ImapSession,
    folder: &str,
) -> Result<Vec<FlagChange>, String> {
    let _mailbox = tokio::time::timeout(IMAP_CMD_TIMEOUT, session.select(folder))
        .await
        .map_err(|_| timeout_err(&format!("SELECT {folder}"), IMAP_CMD_TIMEOUT))?
        .map_err(|e| format!("SELECT {folder} failed: {e}"))?;

    let stream = tokio::time::timeout(IMAP_FETCH_TIMEOUT, session.uid_fetch("1:*", "(FLAGS)"))
        .await
        .map_err(|_| timeout_err(&format!("UID FETCH FLAGS {folder}"), IMAP_FETCH_TIMEOUT))?
        .map_err(|e| format!("UID FETCH FLAGS {folder} failed: {e}"))?;

    let raw: Vec<_> = tokio::time::timeout(
        IMAP_FETCH_TIMEOUT,
        stream.collect::<Vec<_>>(),
    )
    .await
    .map_err(|_| timeout_err(&format!("FLAGS stream {folder}"), IMAP_FETCH_TIMEOUT))?;

    let mut flags = Vec::new();
    for item in raw {
        match item {
            Ok(fetch) => {
                let uid = match fetch.uid {
                    Some(u) => u,
                    None => continue,
                };
                let flag_list: Vec<_> = fetch.flags().collect();
                let is_read = flag_list.iter().any(|f| matches!(f, Flag::Seen));
                let is_starred = flag_list.iter().any(|f| matches!(f, Flag::Flagged));
                let keywords: Vec<String> = flag_list
                    .iter()
                    .filter_map(|f| match f {
                        Flag::Custom(cow) => Some(cow.to_string()),
                        _ => None,
                    })
                    .collect();
                flags.push(FlagChange {
                    uid,
                    is_read,
                    is_starred,
                    keywords,
                });
            }
            Err(e) => {
                log::warn!("FLAGS fetch stream error in {folder}: {e}");
            }
        }
    }

    log::info!(
        "IMAP fetch_all_flags {folder}: {} messages",
        flags.len()
    );
    Ok(flags)
}
