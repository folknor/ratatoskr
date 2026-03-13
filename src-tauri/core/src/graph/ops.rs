use async_trait::async_trait;
use mail_parser::MimeHeaders;

use crate::provider::ops::ProviderOps;
use crate::provider::types::{
    AttachmentData, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation, ProviderProfile,
    ProviderTestResult, SyncResult,
};

use super::client::GraphClient;
use super::folder_mapper::FolderMap;
use super::types::{
    BatchRequest, BatchRequestItem, GraphAttachment, GraphCreateFolderRequest, GraphFlagInput,
    GraphMailFolder, GraphMessagePatch, GraphMoveRequest, GraphRenameFolderRequest,
};

/// Microsoft Graph allows max 20 requests per `/$batch` call.
const BATCH_CHUNK_SIZE: usize = 20;

/// Graph implementation of the provider operations trait.
pub struct GraphOps {
    pub(crate) client: GraphClient,
}

impl GraphOps {
    pub fn new(client: GraphClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ProviderOps for GraphOps {
    async fn sync_initial(
        &self,
        ctx: &ProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, String> {
        super::sync::graph_initial_sync(&self.client, ctx, days_back).await?;
        Ok(SyncResult::default())
    }

    async fn sync_delta(
        &self,
        ctx: &ProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<SyncResult, String> {
        super::sync::graph_delta_sync(&self.client, ctx).await
    }

    async fn archive(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let folder_map = require_folder_map(&self.client).await?;
        let archive_id = folder_map
            .resolve_folder_id("archive")
            .ok_or("No archive folder found")?
            .to_string();
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        move_messages(&self.client, ctx, &msg_ids, &archive_id).await
    }

    async fn trash(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let folder_map = require_folder_map(&self.client).await?;
        let trash_id = folder_map
            .resolve_folder_id("TRASH")
            .ok_or("No trash folder found")?
            .to_string();
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        move_messages(&self.client, ctx, &msg_ids, &trash_id).await
    }

    async fn permanent_delete(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        delete_messages(&self.client, ctx, &msg_ids).await
    }

    async fn mark_read(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), String> {
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        let patch = GraphMessagePatch {
            is_read: Some(read),
            ..Default::default()
        };
        patch_messages(&self.client, ctx, &msg_ids, &patch).await
    }

    async fn star(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), String> {
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        let status = if starred { "flagged" } else { "notFlagged" };
        let patch = GraphMessagePatch {
            flag: Some(GraphFlagInput {
                flag_status: status.to_string(),
            }),
            ..Default::default()
        };
        patch_messages(&self.client, ctx, &msg_ids, &patch).await
    }

    async fn spam(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), String> {
        let folder_map = require_folder_map(&self.client).await?;
        let target = if is_spam { "SPAM" } else { "INBOX" };
        let folder_id = folder_map
            .resolve_folder_id(target)
            .ok_or_else(|| format!("No {target} folder found"))?
            .to_string();
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        move_messages(&self.client, ctx, &msg_ids, &folder_id).await
    }

    async fn move_to_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        folder_id: &str,
    ) -> Result<(), String> {
        // folder_id could be a label_id — resolve to opaque Graph folder ID
        let folder_map = require_folder_map(&self.client).await?;
        let target = folder_map
            .resolve_folder_id(folder_id)
            .unwrap_or(folder_id)
            .to_string();
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        move_messages(&self.client, ctx, &msg_ids, &target).await
    }

    async fn add_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        // Tags map to Graph categories
        let category = tag_id.strip_prefix("cat:").unwrap_or(tag_id);
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        for msg_id in &msg_ids {
            add_category(&self.client, ctx, msg_id, category).await?;
        }
        Ok(())
    }

    async fn remove_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        let category = tag_id.strip_prefix("cat:").unwrap_or(tag_id);
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        for msg_id in &msg_ids {
            remove_category(&self.client, ctx, msg_id, category).await?;
        }
        Ok(())
    }

    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        send_via_draft(&self.client, ctx, raw_base64url, thread_id).await
    }

    async fn create_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        create_draft_impl(&self.client, ctx, raw_base64url, thread_id).await
    }

    async fn update_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, String> {
        // Graph has no draft mutation — delete and recreate
        let enc_id = urlencoding::encode(draft_id);
        self.client
            .delete(&format!("/me/messages/{enc_id}"), ctx.db)
            .await?;
        create_draft_impl(&self.client, ctx, raw_base64url, thread_id).await
    }

    async fn delete_draft(&self, ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), String> {
        let enc_id = urlencoding::encode(draft_id);
        self.client
            .delete(&format!("/me/messages/{enc_id}"), ctx.db)
            .await
    }

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentData, String> {
        let enc_msg_id = urlencoding::encode(message_id);
        let enc_att_id = urlencoding::encode(attachment_id);
        let attachment: GraphAttachment = self
            .client
            .get_json(
                &format!("/me/messages/{enc_msg_id}/attachments/{enc_att_id}"),
                ctx.db,
            )
            .await?;

        let data = if let Some(ref content_bytes) = attachment.content_bytes {
            crate::provider::encoding::decode_base64_standard(content_bytes)
                .map_err(|e| format!("Failed to decode attachment: {e}"))?
        } else {
            let raw = self
                .client
                .get_bytes(
                    &format!("/me/messages/{enc_msg_id}/attachments/{enc_att_id}/$value"),
                    ctx.db,
                )
                .await?;
            if raw.is_empty() {
                return Err(format!("Attachment {attachment_id} has no content"));
            }
            raw
        };

        let size = data.len();
        Ok(AttachmentData {
            data: crate::provider::encoding::encode_base64_standard(&data),
            size,
        })
    }

    async fn list_folders(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, String> {
        // Use cached folder map if it was synced less than 60 seconds ago
        let use_cache = if let Some(age) = self.client.folder_map_age().await {
            age < std::time::Duration::from_secs(60) && self.client.folder_map().await.is_some()
        } else {
            false
        };

        let folder_map = if use_cache {
            // Safe to unwrap: we just checked is_some() above
            self.client
                .folder_map()
                .await
                .ok_or("Folder map vanished")?
        } else {
            let map = super::sync::sync_folders_public(&self.client, ctx).await?;
            self.client.set_folder_map(map.clone()).await;
            self.client.set_folder_map_synced().await;
            map
        };

        let folders = folder_map
            .all_mappings()
            .map(|m| ProviderFolderEntry {
                id: m.label_id.clone(),
                name: m.label_name.clone(),
                path: m.label_name.clone(),
                folder_type: m.label_type.to_string(),
                special_use: if m.label_type == "system" {
                    Some(m.label_id.clone())
                } else {
                    None
                },
                delimiter: Some("/".to_string()),
                message_count: None,
                unread_count: None,
                color_bg: None,
                color_fg: None,
            })
            .collect();
        Ok(folders)
    }

    async fn create_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        name: &str,
        parent_id: Option<&str>,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, String> {
        let parent_graph_id = match parent_id {
            Some(parent_id) => {
                Some(resolve_graph_folder_id(&self.client, ctx, parent_id, false).await?)
            }
            None => None,
        };
        let body = GraphCreateFolderRequest {
            display_name: name.to_string(),
        };
        let created: GraphMailFolder = if let Some(parent_graph_id) = parent_graph_id {
            let enc_parent_id = urlencoding::encode(&parent_graph_id);
            self.client
                .post(
                    &format!("/me/mailFolders/{enc_parent_id}/childFolders"),
                    &body,
                    ctx.db,
                )
                .await?
        } else {
            self.client.post("/me/mailFolders", &body, ctx.db).await?
        };

        refresh_folder_map(&self.client, ctx).await?;
        Ok(graph_folder_to_mutation(&created))
    }

    async fn rename_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        folder_id: &str,
        new_name: &str,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, String> {
        let graph_folder_id = resolve_graph_folder_id(&self.client, ctx, folder_id, true).await?;
        let enc_folder_id = urlencoding::encode(&graph_folder_id);
        let body = GraphRenameFolderRequest {
            display_name: new_name.to_string(),
        };
        self.client
            .patch(&format!("/me/mailFolders/{enc_folder_id}"), &body, ctx.db)
            .await?;

        refresh_folder_map(&self.client, ctx).await?;
        Ok(ProviderFolderMutation {
            id: folder_id.to_string(),
            name: new_name.to_string(),
            path: new_name.to_string(),
            folder_type: "user".to_string(),
            special_use: None,
            delimiter: Some("/".to_string()),
            color_bg: None,
            color_fg: None,
        })
    }

    async fn delete_folder(&self, ctx: &ProviderCtx<'_>, folder_id: &str) -> Result<(), String> {
        let graph_folder_id = resolve_graph_folder_id(&self.client, ctx, folder_id, true).await?;
        let enc_folder_id = urlencoding::encode(&graph_folder_id);
        self.client
            .delete(&format!("/me/mailFolders/{enc_folder_id}"), ctx.db)
            .await?;
        delete_folder_delta_token(ctx, &graph_folder_id).await?;
        refresh_folder_map(&self.client, ctx).await?;
        Ok(())
    }

    async fn test_connection(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderTestResult, String> {
        let profile: super::types::GraphProfile = self
            .client
            .get_json("/me?$select=displayName,mail,userPrincipalName", ctx.db)
            .await?;
        let display = profile
            .mail
            .clone()
            .or(profile.user_principal_name.clone())
            .unwrap_or_else(|| "Unknown".to_string());

        Ok(ProviderTestResult {
            success: true,
            message: format!("Connected as {display}"),
        })
    }

    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, String> {
        let profile: super::types::GraphProfile = self
            .client
            .get_json("/me?$select=displayName,mail,userPrincipalName", ctx.db)
            .await?;

        Ok(ProviderProfile {
            email: profile
                .mail
                .or(profile.user_principal_name)
                .unwrap_or_default(),
            name: profile.display_name,
        })
    }
}

// ── Helper functions ────────────────────────────────────────

/// Get the cached folder map or return an error if not built yet.
async fn require_folder_map(client: &GraphClient) -> Result<FolderMap, String> {
    client
        .folder_map()
        .await
        .ok_or_else(|| "Folder map not initialized — run sync first".to_string())
}

async fn get_folder_map(client: &GraphClient, ctx: &ProviderCtx<'_>) -> Result<FolderMap, String> {
    if let Some(map) = client.folder_map().await {
        return Ok(map);
    }
    refresh_folder_map(client, ctx).await
}

async fn refresh_folder_map(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
) -> Result<FolderMap, String> {
    let map = super::sync::sync_folders_public(client, ctx).await?;
    client.set_folder_map(map.clone()).await;
    client.set_folder_map_synced().await;
    Ok(map)
}

async fn resolve_graph_folder_id(
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

    if require_user_folder {
        if let Some(mapping) = folder_map.get_by_folder_id(&graph_folder_id) {
            if mapping.label_type == "system" {
                return Err(
                    "System folders cannot be renamed or deleted for Graph accounts.".to_string(),
                );
            }
        }
    }

    Ok(graph_folder_id)
}

fn graph_folder_to_mutation(folder: &GraphMailFolder) -> ProviderFolderMutation {
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

async fn delete_folder_delta_token(ctx: &ProviderCtx<'_>, folder_id: &str) -> Result<(), String> {
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
async fn query_thread_message_ids(
    ctx: &ProviderCtx<'_>,
    thread_id: &str,
) -> Result<Vec<String>, String> {
    let tid = thread_id.to_string();
    let aid = ctx.account_id.to_string();
    ctx.db
        .with_conn(move |conn| crate::db::lookups::get_message_ids_for_thread(conn, &aid, &tid))
        .await
}

/// Move multiple messages to a destination folder via `/$batch`.
async fn move_messages(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_ids: &[String],
    destination_id: &str,
) -> Result<(), String> {
    let body = serde_json::to_value(GraphMoveRequest {
        destination_id: destination_id.to_string(),
    })
    .map_err(|e| format!("serialize move body: {e}"))?;

    let items: Vec<BatchRequestItem> = message_ids
        .iter()
        .enumerate()
        .map(|(i, msg_id)| {
            let enc_id = urlencoding::encode(msg_id);
            BatchRequestItem {
                id: i.to_string(),
                method: "POST".to_string(),
                url: format!("/me/messages/{enc_id}/move"),
                body: Some(body.clone()),
                headers: Some(content_type_json()),
            }
        })
        .collect();

    execute_batch(client, ctx, &items).await
}

/// PATCH multiple messages with the same patch body via `/$batch`.
async fn patch_messages(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_ids: &[String],
    patch: &GraphMessagePatch,
) -> Result<(), String> {
    let body = serde_json::to_value(patch).map_err(|e| format!("serialize patch body: {e}"))?;

    let items: Vec<BatchRequestItem> = message_ids
        .iter()
        .enumerate()
        .map(|(i, msg_id)| {
            let enc_id = urlencoding::encode(msg_id);
            BatchRequestItem {
                id: i.to_string(),
                method: "PATCH".to_string(),
                url: format!("/me/messages/{enc_id}"),
                body: Some(body.clone()),
                headers: Some(content_type_json()),
            }
        })
        .collect();

    execute_batch(client, ctx, &items).await
}

/// Delete multiple messages via `/$batch`.
async fn delete_messages(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_ids: &[String],
) -> Result<(), String> {
    let items: Vec<BatchRequestItem> = message_ids
        .iter()
        .enumerate()
        .map(|(i, msg_id)| {
            let enc_id = urlencoding::encode(msg_id);
            BatchRequestItem {
                id: i.to_string(),
                method: "DELETE".to_string(),
                url: format!("/me/messages/{enc_id}"),
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
async fn execute_batch(
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
                    .map(|b| b.to_string())
                    .unwrap_or_default();
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

/// Add a category to a single message.
async fn add_category(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_id: &str,
    category: &str,
) -> Result<(), String> {
    // Fetch current categories, add the new one
    let enc_id = urlencoding::encode(message_id);
    let msg: serde_json::Value = client
        .get_json(&format!("/me/messages/{enc_id}?$select=categories"), ctx.db)
        .await?;
    let mut cats: Vec<String> = msg
        .get("categories")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    if !cats.iter().any(|c| c == category) {
        cats.push(category.to_string());
        let patch = GraphMessagePatch {
            categories: Some(cats),
            ..Default::default()
        };
        client
            .patch(&format!("/me/messages/{enc_id}"), &patch, ctx.db)
            .await?;
    }
    Ok(())
}

/// Remove a category from a single message.
async fn remove_category(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    message_id: &str,
    category: &str,
) -> Result<(), String> {
    let enc_id = urlencoding::encode(message_id);
    let msg: serde_json::Value = client
        .get_json(&format!("/me/messages/{enc_id}?$select=categories"), ctx.db)
        .await?;
    let mut cats: Vec<String> = msg
        .get("categories")
        .and_then(|v| serde_json::from_value(v.clone()).ok())
        .unwrap_or_default();
    let before_len = cats.len();
    cats.retain(|c| c != category);
    if cats.len() != before_len {
        let patch = GraphMessagePatch {
            categories: Some(cats),
            ..Default::default()
        };
        client
            .patch(&format!("/me/messages/{enc_id}"), &patch, ctx.db)
            .await?;
    }
    Ok(())
}

// ── Send via create-draft-then-send ─────────────────────────

/// Send an email by creating a draft from raw MIME, then sending it.
async fn send_via_draft(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    raw_base64url: &str,
    thread_id: Option<&str>,
) -> Result<String, String> {
    let draft_id = create_draft_impl(client, ctx, raw_base64url, thread_id).await?;
    // Send the draft — no response body (202 Accepted)
    let enc_draft_id = urlencoding::encode(&draft_id);
    client
        .post_no_content::<()>(&format!("/me/messages/{enc_draft_id}/send"), None, ctx.db)
        .await?;
    Ok(draft_id)
}

/// Create a draft message from raw MIME (base64url-encoded).
#[allow(clippy::too_many_lines)]
async fn create_draft_impl(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    raw_base64url: &str,
    _thread_id: Option<&str>,
) -> Result<String, String> {
    // Decode base64url → raw MIME bytes
    let raw_bytes = crate::provider::encoding::decode_base64url_nopad(raw_base64url)
        .map_err(|e| format!("Failed to decode base64url: {e}"))?;

    // Parse MIME using mail-parser
    let parsed = mail_parser::MessageParser::default()
        .parse(&raw_bytes)
        .ok_or("Failed to parse MIME message")?;

    // Build Graph message JSON from parsed MIME
    let create_msg = mime_to_graph_message(&parsed)?;

    // Create draft via POST /me/messages
    let draft: serde_json::Value = client.post("/me/messages", &create_msg, ctx.db).await?;

    let draft_id = draft
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("Draft response missing id")?
        .to_string();

    // Upload attachments if any
    upload_attachments_from_mime(client, ctx, &draft_id, &parsed).await?;

    Ok(draft_id)
}

/// Convert a parsed MIME message to a Graph create-message request.
fn mime_to_graph_message(
    parsed: &mail_parser::Message<'_>,
) -> Result<super::types::GraphCreateMessage, String> {
    use super::types::{GraphBodyInput, GraphCreateMessage, GraphEmailAddress, GraphRecipient};

    let subject = parsed.subject().map(String::from);

    // Prefer HTML body, fall back to text
    let body = if let Some(html) = parsed.body_html(0) {
        Some(GraphBodyInput {
            content_type: "html".to_string(),
            content: html.to_string(),
        })
    } else {
        parsed.body_text(0).map(|text| GraphBodyInput {
            content_type: "text".to_string(),
            content: text.to_string(),
        })
    };

    let to = addr_to_recipients(parsed.to());
    let cc = addr_to_recipients(parsed.cc());
    let bcc = addr_to_recipients(parsed.bcc());
    let reply_to = addr_to_recipients(parsed.reply_to());

    fn addr_to_recipients(addr: Option<&mail_parser::Address<'_>>) -> Option<Vec<GraphRecipient>> {
        let addr = addr?;
        let recips: Vec<GraphRecipient> = addr
            .iter()
            .filter_map(|group| {
                group.address.as_ref().map(|email| GraphRecipient {
                    email_address: GraphEmailAddress {
                        name: group.name.as_ref().map(std::string::ToString::to_string),
                        address: email.to_string(),
                    },
                })
            })
            .collect();
        if recips.is_empty() {
            None
        } else {
            Some(recips)
        }
    }

    let message_id = parsed.message_id().map(String::from);

    Ok(GraphCreateMessage {
        subject,
        body,
        to_recipients: to,
        cc_recipients: cc,
        bcc_recipients: bcc,
        reply_to,
        importance: None,
        internet_message_id: message_id,
    })
}

/// Upload attachments from a parsed MIME message to a Graph draft.
async fn upload_attachments_from_mime(
    client: &GraphClient,
    ctx: &ProviderCtx<'_>,
    draft_id: &str,
    parsed: &mail_parser::Message<'_>,
) -> Result<(), String> {
    use super::types::GraphAttachmentInput;

    let enc_draft_id = urlencoding::encode(draft_id);
    for attachment in parsed.attachments() {
        let name = attachment
            .attachment_name()
            .unwrap_or("attachment")
            .to_string();
        let content_type = attachment
            .content_type()
            .map(|ct| {
                if let Some(st) = ct.subtype() {
                    format!("{}/{st}", ct.ctype())
                } else {
                    ct.ctype().to_string()
                }
            })
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let content_bytes =
            crate::provider::encoding::encode_base64_standard(attachment.contents());
        let is_inline = attachment
            .content_disposition()
            .is_some_and(|d| d.ctype() == "inline");
        let content_id = attachment
            .content_id()
            .map(|id| id.trim_matches(&['<', '>'] as &[char]).to_string());

        let input = GraphAttachmentInput {
            odata_type: "#microsoft.graph.fileAttachment".to_string(),
            name,
            content_type,
            content_bytes,
            is_inline: if is_inline { Some(true) } else { None },
            content_id,
        };

        let _: serde_json::Value = client
            .post(
                &format!("/me/messages/{enc_draft_id}/attachments"),
                &input,
                ctx.db,
            )
            .await?;
    }

    Ok(())
}
