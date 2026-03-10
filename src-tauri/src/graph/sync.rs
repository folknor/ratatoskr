use std::collections::{HashMap, HashSet};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::body_store::{BodyStoreState, MessageBody};
use crate::db::DbState;
use crate::provider::types::{ProviderCtx, SyncResult};
use crate::search::{SearchDocument, SearchState};

use super::client::GraphClient;
use super::folder_mapper::FolderMap;
use super::parse::{ParsedGraphMessage, parse_graph_message};
use super::types::{GraphMailFolder, GraphMessage, MESSAGE_SELECT, ODataCollection};

const BATCH_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Progress event emitted during Graph sync.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct GraphSyncProgress {
    account_id: String,
    phase: String,
    folder_name: String,
    current_folder: u64,
    total_folders: u64,
    messages_processed: u64,
}

/// Internal context bundle for sync.
struct SyncCtx<'a> {
    client: &'a GraphClient,
    account_id: &'a str,
    db: &'a DbState,
    body_store: &'a BodyStoreState,
    search: &'a SearchState,
    app_handle: &'a AppHandle,
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
        search: ctx.search,
        app_handle: ctx.app_handle,
    };

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
    folder_list.sort_by_key(|(_, label)| folder_priority(label));

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
        {
            total_messages += messages.len() as u64;
        }

        persist_messages(&sctx, &messages).await?;
    }

    // Phase 3: Bootstrap delta tokens for all folders
    emit_progress(
        &sctx,
        "delta-bootstrap",
        "",
        0,
        total_folders,
        total_messages,
    );

    for (i, &(folder_id, _)) in folder_list.iter().enumerate() {
        let delta_link = bootstrap_delta_token(client, ctx.db, folder_id).await?;
        save_delta_token(ctx.db, ctx.account_id, folder_id, &delta_link).await?;

        #[allow(clippy::cast_possible_truncation)]
        let current = (i + 1) as u64;
        emit_progress(
            &sctx,
            "delta-bootstrap",
            "",
            current,
            total_folders,
            total_messages,
        );
    }

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
        search: ctx.search,
        app_handle: ctx.app_handle,
    };

    let cycle = client.increment_sync_cycle();

    // Load stored delta tokens
    let mut tokens = load_delta_tokens(ctx.db, ctx.account_id).await?;
    if tokens.is_empty() {
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
                        save_delta_token(ctx.db, ctx.account_id, folder_id, &delta_link).await?;
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
            delete_delta_token(ctx.db, ctx.account_id, stale_id).await?;
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

    Ok(SyncResult {
        new_inbox_message_ids: new_inbox_ids,
        affected_thread_ids: affected_thread_ids.into_iter().collect(),
    })
}

/// Decide whether a folder should be synced this cycle based on its priority tier.
fn should_sync_folder(label_id: &str, cycle: u32) -> bool {
    match folder_priority(label_id) {
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
            affected_thread_ids.insert(msg.thread_id.clone());
            if msg.label_ids.contains(&"INBOX".to_string()) {
                new_inbox_ids.push(msg.id.clone());
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
            save_delta_token(sctx.db, sctx.account_id, folder_id, new_delta).await?;
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
// Folder sync
// ---------------------------------------------------------------------------

/// Public entry point for folder sync (used by ops.rs list_folders).
pub(crate) async fn sync_folders_public(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
) -> Result<FolderMap, String> {
    sync_folders(client, ctx).await
}

/// Resolve well-known folders, fetch full tree, persist labels, return FolderMap.
async fn sync_folders(client: &GraphClient, ctx: &ProviderCtx<'_>) -> Result<FolderMap, String> {
    // Phase 1: Resolve well-known aliases to opaque IDs
    let mut resolved = HashMap::new();
    for &(alias, label_id, label_name) in FolderMap::well_known_aliases() {
        match client
            .get_json::<GraphMailFolder>(&format!("/me/mailFolders/{alias}"), ctx.db)
            .await
        {
            Ok(folder) => {
                resolved.insert(folder.id, (label_id, label_name));
            }
            Err(_) => {
                log::debug!("Well-known folder '{alias}' not found, skipping");
            }
        }
    }

    // Phase 2: Fetch full folder tree
    let all_folders = fetch_all_folders(client, ctx.db).await?;

    let folder_map = FolderMap::build(&resolved, &all_folders);

    // Phase 3: Persist folders as labels to DB
    persist_labels(ctx, &folder_map).await?;

    Ok(folder_map)
}

/// Persist folder-derived labels to the DB.
async fn persist_labels(ctx: &ProviderCtx<'_>, folder_map: &FolderMap) -> Result<(), String> {
    let aid = ctx.account_id.to_string();

    let label_rows: Vec<(String, String, String, String)> = folder_map
        .all_mappings()
        .map(|m| {
            (
                m.label_id.clone(),
                aid.clone(),
                m.label_name.clone(),
                m.label_type.to_string(),
            )
        })
        .chain(std::iter::once((
            "UNREAD".to_string(),
            aid.clone(),
            "Unread".to_string(),
            "system".to_string(),
        )))
        .collect();

    ctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            for (label_id, account_id, name, label_type) in &label_rows {
                tx.execute(
                    "INSERT OR REPLACE INTO labels (id, account_id, name, type) \
                     VALUES (?1, ?2, ?3, ?4)",
                    rusqlite::params![label_id, account_id, name, label_type],
                )
                .map_err(|e| format!("upsert label: {e}"))?;
            }
            tx.commit().map_err(|e| format!("commit labels: {e}"))?;
            Ok(())
        })
        .await
}

// ---------------------------------------------------------------------------
// Message fetch
// ---------------------------------------------------------------------------

/// Fetch messages from a single folder with a date filter.
async fn fetch_folder_messages(
    client: &GraphClient,
    db: &DbState,
    folder_id: &str,
    since_iso: &str,
    folder_map: &FolderMap,
) -> Result<Vec<ParsedGraphMessage>, String> {
    let mut messages = Vec::new();
    let enc_folder_id = urlencoding::encode(folder_id);
    let initial_url = format!(
        "/me/mailFolders/{enc_folder_id}/messages\
         ?$filter=receivedDateTime ge {since_iso}\
         &$select={MESSAGE_SELECT}\
         &$expand=attachments($select=id,name,contentType,size,isInline,contentId)\
         &$top={BATCH_SIZE}\
         &$orderby=receivedDateTime desc"
    );

    let mut next_link: Option<String> = None;

    loop {
        let page: ODataCollection<GraphMessage> = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client.get_json(&initial_url, db).await?
        };

        for msg in &page.value {
            match parse_graph_message(msg, folder_map) {
                Ok(parsed) => messages.push(parsed),
                Err(e) => log::warn!("Failed to parse Graph message {}: {e}", msg.id),
            }
        }

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => break,
        }
    }

    Ok(messages)
}

/// Recursively fetch all folders in the mailbox.
///
/// NOTE: This replaces the buggy version in ops.rs. The previous implementation
/// mixed relative-path and absolute-URL pagination incorrectly. This version
/// uses `get_absolute()` consistently for OData pagination.
async fn fetch_all_folders(
    client: &GraphClient,
    db: &DbState,
) -> Result<Vec<GraphMailFolder>, String> {
    let mut all = Vec::new();
    let mut next_link: Option<String> = None;

    loop {
        let page: ODataCollection<GraphMailFolder> = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client.get_json("/me/mailFolders?$top=250", db).await?
        };

        for folder in &page.value {
            if folder.child_folder_count.unwrap_or(0) > 0 {
                let children = fetch_child_folders(client, db, &folder.id).await?;
                all.extend(children);
            }
        }

        all.extend(page.value);

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => break,
        }
    }

    Ok(all)
}

/// Recursively fetch child folders of a given parent.
fn fetch_child_folders<'a>(
    client: &'a GraphClient,
    db: &'a DbState,
    parent_id: &'a str,
) -> futures::future::BoxFuture<'a, Result<Vec<GraphMailFolder>, String>> {
    Box::pin(async move {
        let mut children = Vec::new();
        let enc_parent_id = urlencoding::encode(parent_id);
        let initial_url = format!("/me/mailFolders/{enc_parent_id}/childFolders?$top=250");
        let mut next_link: Option<String> = None;

        loop {
            let page: ODataCollection<GraphMailFolder> = if let Some(ref link) = next_link {
                client.get_absolute(link, db).await?
            } else {
                client.get_json(&initial_url, db).await?
            };

            for folder in &page.value {
                if folder.child_folder_count.unwrap_or(0) > 0 {
                    let sub = fetch_child_folders(client, db, &folder.id).await?;
                    children.extend(sub);
                }
            }

            children.extend(page.value);

            match page.next_link {
                Some(link) => next_link = Some(link),
                None => break,
            }
        }

        Ok(children)
    })
}

// ---------------------------------------------------------------------------
// Delta token management
// ---------------------------------------------------------------------------

/// Bootstrap a delta token for a folder by paging through the delta endpoint
/// until the server returns a `@odata.deltaLink` (no more `nextLink`).
///
/// Uses `$select=id` to minimize payload — we already have the messages from
/// the initial fetch.
async fn bootstrap_delta_token(
    client: &GraphClient,
    db: &DbState,
    folder_id: &str,
) -> Result<String, String> {
    let enc_folder_id = urlencoding::encode(folder_id);
    let initial_url = format!("/me/mailFolders/{enc_folder_id}/messages/delta?$select=id");
    let mut next_link: Option<String> = None;

    loop {
        let page: ODataCollection<serde_json::Value> = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client.get_json(&initial_url, db).await?
        };

        if let Some(ref delta) = page.delta_link {
            return Ok(delta.clone());
        }

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => {
                return Err(format!(
                    "Delta bootstrap for folder {folder_id} ended without a deltaLink"
                ));
            }
        }
    }
}

/// Save a delta token for a folder.
async fn save_delta_token(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    delta_link: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();
    let dl = delta_link.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO graph_folder_delta_tokens \
             (account_id, folder_id, delta_link, updated_at) \
             VALUES (?1, ?2, ?3, strftime('%s', 'now'))",
            rusqlite::params![aid, fid, dl],
        )
        .map_err(|e| format!("save delta token: {e}"))?;
        Ok(())
    })
    .await
}

/// Load all delta tokens for an account.
async fn load_delta_tokens(
    db: &DbState,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    let aid = account_id.to_string();

    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT folder_id, delta_link FROM graph_folder_delta_tokens \
                 WHERE account_id = ?1",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        let map: HashMap<String, String> = stmt
            .query_map(rusqlite::params![aid], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("query: {e}"))?
            .filter_map(Result::ok)
            .collect();
        Ok(map)
    })
    .await
}

/// Bootstrap a delta token for a folder using `$deltatoken=latest`.
///
/// This asks the server for a fresh delta token without fetching any existing
/// messages. Ideal for newly discovered folders during delta sync — we'll
/// pick up new messages starting from the next cycle.
async fn bootstrap_delta_token_latest(
    client: &GraphClient,
    db: &DbState,
    folder_id: &str,
) -> Result<String, String> {
    let enc_folder_id = urlencoding::encode(folder_id);
    let url = format!("/me/mailFolders/{enc_folder_id}/messages/delta?$deltatoken=latest");
    let page: ODataCollection<serde_json::Value> = client.get_json(&url, db).await?;

    page.delta_link.ok_or_else(|| {
        format!("Delta bootstrap (latest) for folder {folder_id} returned no deltaLink")
    })
}

/// Delete a delta token for a folder that no longer exists.
async fn delete_delta_token(db: &DbState, account_id: &str, folder_id: &str) -> Result<(), String> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM graph_folder_delta_tokens \
             WHERE account_id = ?1 AND folder_id = ?2",
            rusqlite::params![aid, fid],
        )
        .map_err(|e| format!("delete delta token: {e}"))?;
        Ok(())
    })
    .await
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

    let thread_ids: HashSet<String> = messages.iter().map(|m| m.thread_id.clone()).collect();
    let aid = sctx.account_id.to_string();

    let blocked_threads: HashSet<String> = sctx
        .db
        .with_conn(move |conn| {
            let mut blocked = HashSet::new();
            for tid in &thread_ids {
                let count: i64 = conn
                    .query_row(
                        "SELECT COUNT(*) FROM pending_operations \
                         WHERE account_id = ?1 AND resource_id = ?2 \
                         AND status != 'failed'",
                        rusqlite::params![aid, tid],
                        |row| row.get(0),
                    )
                    .unwrap_or(0);
                if count > 0 {
                    blocked.insert(tid.clone());
                }
            }
            Ok(blocked)
        })
        .await?;

    if blocked_threads.is_empty() {
        return Ok(messages);
    }

    log::info!(
        "Graph delta sync: skipping {} threads with pending operations",
        blocked_threads.len()
    );

    Ok(messages
        .into_iter()
        .filter(|m| !blocked_threads.contains(&m.thread_id))
        .collect())
}

// ---------------------------------------------------------------------------
// DB persistence (mirrors jmap/sync.rs patterns)
// ---------------------------------------------------------------------------

/// Persist parsed messages to DB, body store, and search index.
async fn persist_messages(
    sctx: &SyncCtx<'_>,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }

    // Group messages by thread for thread-level aggregation
    let mut threads: HashMap<&str, Vec<&ParsedGraphMessage>> = HashMap::new();
    for msg in messages {
        threads.entry(&msg.thread_id).or_default().push(msg);
    }

    // 1. DB writes (metadata + thread aggregation)
    let aid = sctx.account_id.to_string();
    let thread_groups: Vec<(String, Vec<ParsedGraphMessage>)> = threads
        .into_iter()
        .map(|(tid, msgs)| (tid.to_string(), msgs.into_iter().cloned().collect()))
        .collect();

    sctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            for (thread_id, msgs) in &thread_groups {
                store_thread_to_db(&tx, &aid, thread_id, msgs)?;
            }
            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(())
        })
        .await?;

    // 2. Body store writes
    store_bodies(sctx.body_store, messages).await;

    // 3. Search index writes
    index_messages(sctx.search, sctx.account_id, messages).await;

    Ok(())
}

/// Delete messages from DB, body store, and search index.
/// Also updates or removes parent threads as needed.
async fn delete_messages(sctx: &SyncCtx<'_>, message_ids: &[String]) -> Result<(), String> {
    if message_ids.is_empty() {
        return Ok(());
    }

    let aid = sctx.account_id.to_string();
    let ids = message_ids.to_vec();

    // Delete from DB and update parent threads
    sctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;

            // Collect affected thread IDs before deleting
            let mut affected_threads = HashSet::new();
            for id in &ids {
                if let Ok(tid) = tx.query_row(
                    "SELECT thread_id FROM messages WHERE account_id = ?1 AND id = ?2",
                    rusqlite::params![aid, id],
                    |row| row.get::<_, String>(0),
                ) {
                    affected_threads.insert(tid);
                }
            }

            // Delete the messages
            for id in &ids {
                tx.execute(
                    "DELETE FROM messages WHERE account_id = ?1 AND id = ?2",
                    rusqlite::params![aid, id],
                )
                .map_err(|e| format!("delete message: {e}"))?;
            }

            // Update or remove affected threads
            for tid in &affected_threads {
                let remaining: i64 = tx
                    .query_row(
                        "SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND account_id = ?2",
                        rusqlite::params![tid, aid],
                        |row| row.get(0),
                    )
                    .map_err(|e| format!("count remaining: {e}"))?;

                if remaining == 0 {
                    // Orphan thread — remove it and its labels
                    tx.execute(
                        "DELETE FROM threads WHERE id = ?1 AND account_id = ?2",
                        rusqlite::params![tid, aid],
                    )
                    .map_err(|e| format!("delete orphan thread: {e}"))?;
                    tx.execute(
                        "DELETE FROM thread_labels WHERE thread_id = ?1 AND account_id = ?2",
                        rusqlite::params![tid, aid],
                    )
                    .map_err(|e| format!("delete orphan thread labels: {e}"))?;
                } else {
                    // Re-aggregate thread fields from remaining messages
                    reaggregate_thread(&tx, &aid, tid)?;
                }
            }

            tx.commit().map_err(|e| format!("commit: {e}"))?;
            Ok(())
        })
        .await?;

    // Delete from body store
    if let Err(e) = sctx.body_store.delete(message_ids.to_vec()).await {
        log::warn!("Failed to delete Graph bodies: {e}");
    }

    // Delete from search index
    for id in message_ids {
        if let Err(e) = sctx.search.delete_message(id).await {
            log::warn!("Failed to delete search document {id}: {e}");
        }
    }

    Ok(())
}

/// Re-aggregate thread fields from remaining messages after deletion.
fn reaggregate_thread(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    let message_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("count messages: {e}"))?;

    let is_read: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_read = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|unread| unread == 0)
        .map_err(|e| format!("check is_read: {e}"))?;

    let is_starred: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_starred = 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|starred| starred > 0)
        .map_err(|e| format!("check is_starred: {e}"))?;

    let has_attachments: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM attachments a \
             JOIN messages m ON a.message_id = m.id \
             WHERE m.thread_id = ?1 AND m.account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|e| format!("check has_attachments: {e}"))?;

    let (snippet, last_date): (String, i64) = tx
        .query_row(
            "SELECT COALESCE(snippet, ''), date FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 \
             ORDER BY date DESC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("get latest message: {e}"))?;

    let subject: Option<String> = tx
        .query_row(
            "SELECT subject FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 \
             ORDER BY date ASC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("get subject: {e}"))?;

    tx.execute(
        "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
         message_count = ?4, is_read = ?5, is_starred = ?6, \
         has_attachments = ?7 \
         WHERE id = ?8 AND account_id = ?9",
        rusqlite::params![
            subject,
            snippet,
            last_date,
            message_count,
            is_read,
            is_starred,
            has_attachments,
            thread_id,
            account_id,
        ],
    )
    .map_err(|e| format!("reaggregate thread: {e}"))?;

    Ok(())
}

// ---------------------------------------------------------------------------
// DB write helpers
// ---------------------------------------------------------------------------

fn store_thread_to_db(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    // upsert_thread_record calls upsert_messages internally before aggregating
    upsert_attachments(tx, account_id, messages)?;
    upsert_thread_record(tx, account_id, thread_id, messages)?;
    set_thread_labels(tx, account_id, thread_id, messages)?;
    Ok(())
}

#[allow(clippy::too_many_lines)]
fn upsert_thread_record(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    if messages.is_empty() {
        return Ok(());
    }

    // First upsert the incoming messages so they are visible in DB queries
    upsert_messages(tx, account_id, messages)?;

    // Now aggregate thread fields from ALL messages in the DB for this thread
    let message_count: i64 = tx
        .query_row(
            "SELECT COUNT(*) FROM messages WHERE thread_id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("count messages: {e}"))?;

    let is_read: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_read = 0",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|unread| unread == 0)
        .map_err(|e| format!("check is_read: {e}"))?;

    let is_starred: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 AND is_starred = 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|starred| starred > 0)
        .map_err(|e| format!("check is_starred: {e}"))?;

    let has_attachments: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM attachments a \
             JOIN messages m ON a.message_id = m.id \
             WHERE m.thread_id = ?1 AND m.account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|count| count > 0)
        .map_err(|e| format!("check has_attachments: {e}"))?;

    // Get snippet + date from the most recent message in the thread
    let (snippet, last_date): (String, i64) = tx
        .query_row(
            "SELECT COALESCE(snippet, ''), date FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 \
             ORDER BY date DESC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| format!("get latest message: {e}"))?;

    // Get subject from the earliest message (thread subject)
    let subject: Option<String> = tx
        .query_row(
            "SELECT subject FROM messages \
             WHERE thread_id = ?1 AND account_id = ?2 \
             ORDER BY date ASC LIMIT 1",
            rusqlite::params![thread_id, account_id],
            |row| row.get(0),
        )
        .map_err(|e| format!("get subject: {e}"))?;

    let all_labels: HashSet<&str> = messages
        .iter()
        .flat_map(|m| m.label_ids.iter().map(String::as_str))
        .collect();
    let is_important = all_labels.contains("IMPORTANT");

    // Check if thread already exists to preserve fields like is_pinned, is_muted
    let exists: bool = tx
        .query_row(
            "SELECT COUNT(*) FROM threads WHERE id = ?1 AND account_id = ?2",
            rusqlite::params![thread_id, account_id],
            |row| row.get::<_, i64>(0),
        )
        .map(|c| c > 0)
        .map_err(|e| format!("check thread exists: {e}"))?;

    if exists {
        tx.execute(
            "UPDATE threads SET subject = ?1, snippet = ?2, last_message_at = ?3, \
             message_count = ?4, is_read = ?5, is_starred = ?6, is_important = ?7, \
             has_attachments = ?8 \
             WHERE id = ?9 AND account_id = ?10",
            rusqlite::params![
                subject,
                snippet,
                last_date,
                message_count,
                is_read,
                is_starred,
                is_important,
                has_attachments,
                thread_id,
                account_id,
            ],
        )
        .map_err(|e| format!("update thread: {e}"))?;
    } else {
        tx.execute(
            "INSERT INTO threads \
             (id, account_id, subject, snippet, last_message_at, message_count, \
              is_read, is_starred, is_important, has_attachments) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                thread_id,
                account_id,
                subject,
                snippet,
                last_date,
                message_count,
                is_read,
                is_starred,
                is_important,
                has_attachments,
            ],
        )
        .map_err(|e| format!("insert thread: {e}"))?;
    }

    Ok(())
}

fn set_thread_labels(
    tx: &rusqlite::Transaction,
    account_id: &str,
    thread_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    let all_labels: HashSet<&str> = messages
        .iter()
        .flat_map(|m| m.label_ids.iter().map(String::as_str))
        .collect();

    tx.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND thread_id = ?2",
        rusqlite::params![account_id, thread_id],
    )
    .map_err(|e| format!("delete thread labels: {e}"))?;

    for label_id in &all_labels {
        tx.execute(
            "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![account_id, thread_id, label_id],
        )
        .map_err(|e| format!("insert thread label: {e}"))?;
    }

    Ok(())
}

fn upsert_messages(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    for msg in messages {
        let has_body = msg.body_html.is_some() || msg.body_text.is_some();

        tx.execute(
            "INSERT OR REPLACE INTO messages \
             (id, account_id, thread_id, from_address, from_name, to_addresses, \
              cc_addresses, bcc_addresses, reply_to, subject, snippet, date, \
              is_read, is_starred, body_html, body_text, raw_size, internal_date, \
              list_unsubscribe, list_unsubscribe_post, auth_results, \
              message_id_header, references_header, in_reply_to_header, body_cached) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                     ?13, ?14, NULL, NULL, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
            rusqlite::params![
                msg.id,
                account_id,
                msg.thread_id,
                msg.from_address,
                msg.from_name,
                msg.to_addresses,
                msg.cc_addresses,
                msg.bcc_addresses,
                msg.reply_to,
                msg.subject,
                msg.snippet,
                msg.date,
                msg.is_read,
                msg.is_starred,
                0i64, // raw_size — Graph doesn't expose message size directly
                msg.internal_date,
                msg.list_unsubscribe,
                msg.list_unsubscribe_post,
                msg.auth_results,
                msg.message_id_header,
                msg.references_header,
                msg.in_reply_to_header,
                if has_body { 1i64 } else { 0i64 },
            ],
        )
        .map_err(|e| format!("upsert message: {e}"))?;
    }
    Ok(())
}

fn upsert_attachments(
    tx: &rusqlite::Transaction,
    account_id: &str,
    messages: &[ParsedGraphMessage],
) -> Result<(), String> {
    for msg in messages {
        for att in &msg.attachments {
            let att_id = format!("{}_{}", msg.id, att.id);
            tx.execute(
                "INSERT OR REPLACE INTO attachments \
                 (id, message_id, account_id, filename, mime_type, size, \
                  gmail_attachment_id, content_id, is_inline) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                rusqlite::params![
                    att_id,
                    msg.id,
                    account_id,
                    att.filename,
                    att.mime_type,
                    att.size,
                    att.id,
                    att.content_id,
                    att.is_inline,
                ],
            )
            .map_err(|e| format!("upsert attachment: {e}"))?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Body store helper
// ---------------------------------------------------------------------------

async fn store_bodies(body_store: &BodyStoreState, messages: &[ParsedGraphMessage]) {
    let bodies: Vec<MessageBody> = messages
        .iter()
        .filter(|m| m.body_html.is_some() || m.body_text.is_some())
        .map(|m| MessageBody {
            message_id: m.id.clone(),
            body_html: m.body_html.clone(),
            body_text: m.body_text.clone(),
        })
        .collect();

    if bodies.is_empty() {
        return;
    }

    if let Err(e) = body_store.put_batch(bodies).await {
        log::warn!("Failed to store Graph bodies: {e}");
    }
}

// ---------------------------------------------------------------------------
// Search index helper
// ---------------------------------------------------------------------------

async fn index_messages(search: &SearchState, account_id: &str, messages: &[ParsedGraphMessage]) {
    let docs: Vec<SearchDocument> = messages
        .iter()
        .map(|m| SearchDocument {
            message_id: m.id.clone(),
            account_id: account_id.to_string(),
            thread_id: m.thread_id.clone(),
            subject: m.subject.clone(),
            from_name: m.from_name.clone(),
            from_address: m.from_address.clone(),
            to_addresses: m.to_addresses.clone(),
            body_text: m.body_text.clone(),
            snippet: Some(m.snippet.clone()),
            date: m.date / 1000, // tantivy expects seconds
            is_read: m.is_read,
            is_starred: m.is_starred,
            has_attachment: m.has_attachments,
        })
        .collect();

    if let Err(e) = search.index_messages_batch(&docs).await {
        log::warn!("Failed to index Graph messages: {e}");
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Folder sync priority for initial sync ordering.
/// Lower number = higher priority (synced first).
fn folder_priority(label_id: &str) -> u8 {
    match label_id {
        "INBOX" | "SENT" | "DRAFT" => 0,
        "archive" | "TRASH" | "SPAM" => 1,
        _ => 2,
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_progress(
    sctx: &SyncCtx<'_>,
    phase: &str,
    folder_name: &str,
    current_folder: u64,
    total_folders: u64,
    messages_processed: u64,
) {
    if let Err(err) = sctx.app_handle.emit(
        "graph-sync-progress",
        GraphSyncProgress {
            account_id: sctx.account_id.to_string(),
            phase: phase.to_string(),
            folder_name: folder_name.to_string(),
            current_folder,
            total_folders,
            messages_processed,
        },
    ) {
        log::warn!(
            "Failed to emit Graph sync progress for {}: {err}",
            sctx.account_id
        );
    }
}
