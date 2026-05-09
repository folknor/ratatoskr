use std::collections::HashMap;

use common::types::ProviderCtx;
use db::db::ReadDbState;
use db::db::queries_extra::{LabelWriteRow, upsert_labels};

use crate::client::GraphClient;
use crate::folder_mapper::FolderMap;
use crate::types::{GraphMailFolder, ODataCollection};

/// Public entry point for folder sync used by provider operations.
pub async fn sync_folders_public(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
) -> Result<FolderMap, String> {
    sync_folders(client, ctx).await
}

/// Resolve well-known folders, fetch the full tree, persist labels, and return
/// the folder map.
async fn sync_folders(client: &GraphClient, ctx: &ProviderCtx<'_>) -> Result<FolderMap, String> {
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

    let all_folders = fetch_all_folders(client, ctx.db).await?;
    let folder_map = FolderMap::build(&resolved, &all_folders);
    persist_labels(ctx, &folder_map).await?;
    Ok(folder_map)
}

async fn persist_labels(ctx: &ProviderCtx<'_>, folder_map: &FolderMap) -> Result<(), String> {
    let aid = ctx.account_id.to_string();

    let label_rows: Vec<LabelWriteRow> = folder_map
        .all_mappings()
        .map(|m| {
            LabelWriteRow {
                id: m.label_id.clone(),
                account_id: aid.clone(),
                name: m.label_name.clone(),
                label_type: m.label_type.to_string(),
                label_kind: "container".to_string(),
                color_bg: None,
                color_fg: None,
                sort_order: None,
                imap_folder_path: None,
                imap_special_use: None,
                parent_label_id: m.parent_label_id.clone(),
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
            }
        })
        .chain(std::iter::once(LabelWriteRow {
            id: "UNREAD".to_string(),
            account_id: aid.clone(),
            name: "Unread".to_string(),
            label_type: "system".to_string(),
            label_kind: "tag".to_string(),
            color_bg: None,
            color_fg: None,
            sort_order: None,
            imap_folder_path: None,
            imap_special_use: None,
            parent_label_id: None,
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
        }))
        .collect();

    ctx.db
        .with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            upsert_labels(&tx, &label_rows)?;
            tx.commit().map_err(|e| format!("commit labels: {e}"))?;
            Ok(())
        })
        .await
}

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
