use common::types::{ProviderCtx, ProviderFolderMutation};

use super::super::client::GraphClient;
use super::super::folder_mapper::FolderMap;
use super::super::types::{
    BatchRequest, BatchRequestItem, GraphMailFolder, GraphMessagePatch, GraphMoveRequest,
};
use super::BATCH_CHUNK_SIZE;

// ── Helper functions ────────────────────────────────────────

/// Get the cached folder map or return an error if not built yet.
pub(super) async fn require_folder_map(client: &GraphClient) -> Result<FolderMap, String> {
    client
        .folder_map()
        .await
        .ok_or_else(|| "Folder map not initialized - run sync first".to_string())
}

pub(super) async fn get_folder_map(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
) -> Result<FolderMap, String> {
    if let Some(map) = client.folder_map().await {
        return Ok(map);
    }
    refresh_folder_map(client, ctx).await
}

pub(super) async fn refresh_folder_map(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
) -> Result<FolderMap, String> {
    let map = super::super::sync::sync_folders_public(client, ctx).await?;
    client.set_folder_map(map.clone()).await;
    client.set_folder_map_synced().await;
    Ok(map)
}

pub(super) async fn resolve_graph_folder_id(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    folder_id: &str,
    require_user_folder: bool,
) -> Result<String, String> {
    let folder_map = get_folder_map(client, ctx).await?;
    let graph_folder_id = folder_map
        .resolve_folder_id(folder_id)
        .unwrap_or(folder_id)
        .to_string();

    if require_user_folder
        && let Some(mapping) = folder_map.get_by_folder_id(&graph_folder_id)
        && mapping.label_type == "system"
    {
        return Err("System folders cannot be renamed or deleted for Graph accounts.".to_string());
    }

    Ok(graph_folder_id)
}

pub(super) fn graph_folder_to_mutation(folder: &GraphMailFolder) -> ProviderFolderMutation {
    ProviderFolderMutation {
        id: format!("graph-{}", folder.id),
        name: folder.display_name.clone(),
        path: folder.display_name.clone(),
        folder_type: "user".to_string(),
        special_use: None,
        delimiter: Some("/".to_string()),
        color_bg: None,
        color_fg: None,
    }
}

pub(super) async fn delete_folder_delta_token(
    ctx: &ProviderCtx<'_>,
    folder_id: &str,
) -> Result<(), String> {
    let account_id = ctx.account_id.to_string();
    let folder_id = folder_id.to_string();
    ctx.db
        .with_conn(move |conn| {
            conn.execute(
                "DELETE FROM graph_folder_delta_tokens WHERE account_id = ?1 AND folder_id = ?2",
                rusqlite::params![account_id, folder_id],
            )
            .map_err(|e| format!("delete graph folder delta token: {e}"))?;
            Ok(())
        })
        .await
}

/// Query local DB for message IDs belonging to a thread.
pub(super) async fn query_thread_message_ids(
    ctx: &ProviderCtx<'_>,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let tid = thread_id.to_string();
    let aid = ctx.account_id.to_string();
    ctx.db
        .with_conn(move |conn| db::db::lookups::get_message_ids_for_thread(conn, &aid, &tid))
        .await
}

/// Move multiple messages to a destination folder via `/$batch`.
pub(super) async fn move_messages(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_ids: &[String],
    destination_id: &str,
) -> Result<(), String> {
    let body = serde_json::to_value(GraphMoveRequest {
        destination_id: destination_id.to_string(),
    })
    .map_err(|e| format!("serialize move body: {e}"))?;

    let me = client.api_path_prefix();
    let items: Vec<BatchRequestItem> = message_ids
        .iter()
        .enumerate()
        .map(|(i, msg_id)| {
            let enc_id = urlencoding::encode(msg_id);
            BatchRequestItem {
                id: i.to_string(),
                method: "POST".to_string(),
                url: format!("{me}/messages/{enc_id}/move"),
                body: Some(body.clone()),
                headers: Some(content_type_json()),
            }
        })
        .collect();

    execute_batch(client, ctx, &items).await
}

/// PATCH multiple messages with the same patch body via `/$batch`.
pub(super) async fn patch_messages(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_ids: &[String],
    patch: &GraphMessagePatch,
) -> Result<(), String> {
    let body = serde_json::to_value(patch).map_err(|e| format!("serialize patch body: {e}"))?;

    let me = client.api_path_prefix();
    let items: Vec<BatchRequestItem> = message_ids
        .iter()
        .enumerate()
        .map(|(i, msg_id)| {
            let enc_id = urlencoding::encode(msg_id);
            BatchRequestItem {
                id: i.to_string(),
                method: "PATCH".to_string(),
                url: format!("{me}/messages/{enc_id}"),
                body: Some(body.clone()),
                headers: Some(content_type_json()),
            }
        })
        .collect();

    execute_batch(client, ctx, &items).await
}

/// Delete multiple messages via `/$batch`.
pub(super) async fn delete_messages(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_ids: &[String],
) -> Result<(), String> {
    let me = client.api_path_prefix();
    let items: Vec<BatchRequestItem> = message_ids
        .iter()
        .enumerate()
        .map(|(i, msg_id)| {
            let enc_id = urlencoding::encode(msg_id);
            BatchRequestItem {
                id: i.to_string(),
                method: "DELETE".to_string(),
                url: format!("{me}/messages/{enc_id}"),
                body: None,
                headers: None,
            }
        })
        .collect();

    execute_batch(client, ctx, &items).await
}

/// Execute batch request items in chunks of `BATCH_CHUNK_SIZE` (20).
///
/// Collects per-item errors and returns the first failure if any.
pub(super) async fn execute_batch(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    items: &[BatchRequestItem],
) -> Result<(), String> {
    for chunk in items.chunks(BATCH_CHUNK_SIZE) {
        let batch = BatchRequest {
            requests: chunk.to_vec(),
        };
        let response = client.post_batch(&batch, ctx.db).await?;

        for resp in &response.responses {
            if resp.status >= 400 {
                let detail = resp
                    .body
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                log::error!(
                    "[Graph] Batch request {} failed with status {}: {detail}",
                    resp.id,
                    resp.status
                );
                return Err(format!(
                    "Batch request {} failed with status {}: {detail}",
                    resp.id, resp.status
                ));
            }
        }
    }
    Ok(())
}

fn content_type_json() -> std::collections::HashMap<String, String> {
    let mut m = std::collections::HashMap::new();
    m.insert("Content-Type".to_string(), "application/json".to_string());
    m
}

/// Fetch current categories for multiple messages.
///
/// Returns `(message_id, categories)` pairs. Uses `/$batch` when there are
/// multiple messages, falls back to a single GET for one message.
pub(super) async fn batch_get_categories(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_ids: &[String],
) -> Result<Vec<(String, Vec<String>)>, String> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }

    let me = client.api_path_prefix();

    // Single message: skip batch overhead
    if message_ids.len() == 1 {
        let enc_id = urlencoding::encode(&message_ids[0]);
        let msg: serde_json::Value = client
            .get_json(
                &format!("{me}/messages/{enc_id}?$select=categories"),
                ctx.db,
            )
            .await?;
        let cats: Vec<String> = msg
            .get("categories")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        return Ok(vec![(message_ids[0].clone(), cats)]);
    }

    let mut results = Vec::with_capacity(message_ids.len());

    for chunk in message_ids.chunks(BATCH_CHUNK_SIZE) {
        let items: Vec<BatchRequestItem> = chunk
            .iter()
            .enumerate()
            .map(|(i, msg_id)| {
                let enc_id = urlencoding::encode(msg_id);
                BatchRequestItem {
                    id: i.to_string(),
                    method: "GET".to_string(),
                    url: format!("{me}/messages/{enc_id}?$select=categories"),
                    body: None,
                    headers: None,
                }
            })
            .collect();

        let batch = BatchRequest { requests: items };
        let response = client.post_batch(&batch, ctx.db).await?;

        for resp in &response.responses {
            let idx: usize = resp
                .id
                .parse()
                .map_err(|_| format!("Invalid batch response id: {}", resp.id))?;
            if resp.status >= 400 {
                let detail = resp
                    .body
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                return Err(format!(
                    "Batch GET categories for message {} failed: {detail}",
                    chunk[idx]
                ));
            }
            let cats: Vec<String> = resp
                .body
                .as_ref()
                .and_then(|b| b.get("categories"))
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .unwrap_or_default();
            results.push((chunk[idx].clone(), cats));
        }
    }

    Ok(results)
}

/// PATCH categories on multiple messages via `/$batch`.
pub(super) async fn batch_set_categories(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    patches: &[(String, Vec<String>)],
) -> Result<(), String> {
    if patches.is_empty() {
        return Ok(());
    }

    let me = client.api_path_prefix();
    let items: Vec<BatchRequestItem> = patches
        .iter()
        .enumerate()
        .map(|(i, (msg_id, cats))| {
            let enc_id = urlencoding::encode(msg_id);
            let patch = GraphMessagePatch {
                categories: Some(cats.clone()),
                ..Default::default()
            };
            BatchRequestItem {
                id: i.to_string(),
                method: "PATCH".to_string(),
                url: format!("{me}/messages/{enc_id}"),
                body: serde_json::to_value(&patch).ok(),
                headers: Some(content_type_json()),
            }
        })
        .collect();

    execute_batch(client, ctx, &items).await
}
