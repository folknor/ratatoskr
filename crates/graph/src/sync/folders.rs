use std::collections::HashMap;

use ratatoskr_db::db::DbState;
use ratatoskr_provider_utils::types::ProviderCtx;

use super::super::client::GraphClient;
use super::super::folder_mapper::FolderMap;
use super::super::parse::{ParsedGraphMessage, parse_graph_message};
use super::super::types::{
    GraphMailFolder, GraphMessage, MESSAGE_SELECT, ODataCollection, REACTIONS_EXPAND,
};
use super::BATCH_SIZE;

// ---------------------------------------------------------------------------
// Folder sync
// ---------------------------------------------------------------------------

/// Resolve well-known folders, fetch full tree, persist labels, return FolderMap.
pub(super) async fn sync_folders(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
) -> Result<FolderMap, String> {
    // Phase 1: Resolve well-known aliases to opaque IDs
    let mut resolved = HashMap::new();
    let me = client.api_path_prefix();
    for (alias, label_id, label_name) in FolderMap::well_known_aliases() {
        match client
            .get_json::<GraphMailFolder>(&format!("{me}/mailFolders/{alias}"), ctx.db)
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
pub(super) async fn fetch_folder_messages(
    client: &GraphClient,
    db: &DbState,
    folder_id: &str,
    since_iso: &str,
    folder_map: &FolderMap,
) -> Result<Vec<ParsedGraphMessage>, String> {
    let mut messages = Vec::new();
    let enc_folder_id = urlencoding::encode(folder_id);
    let me = client.api_path_prefix();
    let initial_url = format!(
        "{me}/mailFolders/{enc_folder_id}/messages\
         ?$filter=receivedDateTime ge {since_iso}\
         &$select={MESSAGE_SELECT}\
         &$expand=attachments($select=id,name,contentType,size,isInline,contentId,contentBytes),{REACTIONS_EXPAND}\
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
            let me = client.api_path_prefix();
            client
                .get_json(&format!("{me}/mailFolders?$top=250"), db)
                .await?
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
        let me = client.api_path_prefix();
        let initial_url = format!("{me}/mailFolders/{enc_parent_id}/childFolders?$top=250");
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
