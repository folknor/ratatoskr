use std::collections::HashMap;

use common::types::{ImportanceLevel, ProviderCtx};
use db::db::ReadDbState;
use db::db::queries_extra::{FolderWriteRow, LabelWriteRow, insert_folders_batch, upsert_labels};
use service_state::WriteDbState;

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
    write_db: &WriteDbState,
) -> Result<FolderMap, String> {
    // Phase 1: Resolve well-known aliases to opaque IDs
    let mut resolved = HashMap::new();
    let me = client.api_path_prefix();
    for (alias, folder_id, folder_name) in FolderMap::well_known_aliases() {
        match client
            .get_json::<GraphMailFolder>(&format!("{me}/mailFolders/{alias}"), ctx.db)
            .await
        {
            Ok(folder) => {
                resolved.insert(folder.id, (folder_id, folder_name));
            }
            Err(_) => {
                log::debug!("Well-known folder '{alias}' not found, skipping");
            }
        }
    }

    // Phase 2: Fetch full folder tree
    let all_folders = fetch_all_folders(client, ctx.db).await?;

    let folder_map = FolderMap::build(&resolved, &all_folders)?;

    // Phase 3: Persist folders to DB
    persist_folders_and_importance(ctx, write_db, &folder_map).await?;

    Ok(folder_map)
}

/// Persist Graph mail folders (and the synthesised `importance:*` labels)
/// to the DB.
async fn persist_folders_and_importance(
    ctx: &ProviderCtx<'_>,
    write_db: &WriteDbState,
    folder_map: &FolderMap,
) -> Result<(), String> {
    let aid = ctx.account_id.to_string();

    let folder_rows: Vec<FolderWriteRow> = folder_map
        .all_mappings()
        .map(|m| {
            FolderWriteRow {
                id: m.folder_id.clone(),
                account_id: aid.clone(),
                name: m.folder_name.clone(),
                visible: None,
                sort_order: None,
                imap_folder_path: None,
                imap_special_use: None,
                namespace_type: None,
                parent_id: m.parent_folder_id.clone(),
                right_read: None,
                right_add: None,
                right_remove: None,
                right_set_seen: None,
                right_set_keywords: None,
                right_create_child: None,
                right_rename: None,
                right_delete: None,
                right_submit: None,
                is_subscribed: None,
                is_undeletable: m.folder_type == "system",
            }
        })
        .collect();
    let label_rows = importance_label_rows(&aid);

    write_db
        .with_write(move |conn| {
            let tx = conn
                .transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            insert_folders_batch(&tx, &folder_rows)?;
            upsert_labels(&tx, &label_rows)?;
            tx.commit().map_err(|e| format!("commit folders: {e}"))?;
            Ok(())
        })
        .await
}

fn importance_label_rows(account_id: &str) -> Vec<LabelWriteRow> {
    ImportanceLevel::ALL
    .into_iter()
    .map(|level| LabelWriteRow {
        id: level.label_id().to_string(),
        account_id: account_id.to_string(),
        name: level.display_name().to_string(),
        visible: None,
        sort_order: Some(level.sort_order()),
        server_color_bg: None,
        server_color_fg: None,
        user_color_bg: None,
        user_color_fg: None,
        is_undeletable: true,
    })
    .collect()
}

// ---------------------------------------------------------------------------
// Message fetch
// ---------------------------------------------------------------------------

/// Fetch messages from a single folder with a date filter.
pub(super) async fn fetch_folder_messages(
    client: &GraphClient,
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &'a ReadDbState,
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
