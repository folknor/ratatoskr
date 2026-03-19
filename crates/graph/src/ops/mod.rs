mod helpers;
mod send;

use async_trait::async_trait;

use ratatoskr_db::db::DbState;
use ratatoskr_provider_utils::error::ProviderError;
use ratatoskr_provider_utils::ops::ProviderOps;
use ratatoskr_provider_utils::types::{
    AttachmentData, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation, ProviderProfile,
    ProviderTestResult, SyncResult,
};

use super::client::GraphClient;
use super::types::{
    GraphAttachment, GraphCreateFolderRequest, GraphFlagInput,
    GraphMailFolder, GraphMessagePatch, GraphRenameFolderRequest,
    SingleValueExtendedProperty,
};

use self::helpers::{
    batch_get_categories, batch_set_categories, delete_folder_delta_token,
    graph_folder_to_mutation, move_messages, patch_messages, query_thread_message_ids,
    refresh_folder_map, require_folder_map, resolve_graph_folder_id,
};
use self::send::{create_draft_impl, send_via_draft};

/// Microsoft Graph allows max 20 requests per `/$batch` call.
const BATCH_CHUNK_SIZE: usize = 20;

/// MAPI property tag for `PidTagDeferredSendTime` — tells Exchange to hold
/// the message server-side until the specified UTC time before sending.
const PID_TAG_DEFERRED_SEND_TIME: &str = "SystemTime 0x3FEF";

/// Graph implementation of the provider operations trait.
pub struct GraphOps {
    pub(crate) client: GraphClient,
}

impl GraphOps {
    pub fn new(client: GraphClient) -> Self {
        Self { client }
    }

    /// Dynamic API path prefix: `/me` for primary, `/users/{id}` for shared.
    fn me(&self) -> String {
        self.client.api_path_prefix()
    }
}

#[async_trait]
impl ProviderOps for GraphOps {
    async fn sync_initial(
        &self,
        ctx: &ProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, ProviderError> {
        super::sync::graph_initial_sync(&self.client, ctx, days_back).await?;
        Ok(SyncResult::default())
    }

    async fn sync_delta(
        &self,
        ctx: &ProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<SyncResult, ProviderError> {
        Ok(super::sync::graph_delta_sync(&self.client, ctx).await?)
    }

    async fn archive(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        let folder_map = require_folder_map(&self.client).await?;
        let archive_id = folder_map
            .resolve_folder_id("archive")
            .ok_or_else(|| ProviderError::NotFound("No archive folder found".to_string()))?
            .to_string();
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        Ok(move_messages(&self.client, ctx, &msg_ids, &archive_id).await?)
    }

    async fn trash(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        let folder_map = require_folder_map(&self.client).await?;
        let trash_id = folder_map
            .resolve_folder_id("TRASH")
            .ok_or_else(|| ProviderError::NotFound("No trash folder found".to_string()))?
            .to_string();
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        Ok(move_messages(&self.client, ctx, &msg_ids, &trash_id).await?)
    }

    async fn permanent_delete(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        Ok(helpers::delete_messages(&self.client, ctx, &msg_ids).await?)
    }

    async fn mark_read(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), ProviderError> {
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        let patch = GraphMessagePatch {
            is_read: Some(read),
            ..Default::default()
        };
        Ok(patch_messages(&self.client, ctx, &msg_ids, &patch).await?)
    }

    async fn star(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), ProviderError> {
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        let status = if starred { "flagged" } else { "notFlagged" };
        let patch = GraphMessagePatch {
            flag: Some(GraphFlagInput {
                flag_status: status.to_string(),
            }),
            ..Default::default()
        };
        Ok(patch_messages(&self.client, ctx, &msg_ids, &patch).await?)
    }

    async fn spam(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), ProviderError> {
        let folder_map = require_folder_map(&self.client).await?;
        let target = if is_spam { "SPAM" } else { "INBOX" };
        let folder_id = folder_map
            .resolve_folder_id(target)
            .ok_or_else(|| ProviderError::NotFound(format!("No {target} folder found")))?
            .to_string();
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        Ok(move_messages(&self.client, ctx, &msg_ids, &folder_id).await?)
    }

    async fn move_to_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        folder_id: &str,
    ) -> Result<(), ProviderError> {
        // folder_id could be a label_id — resolve to opaque Graph folder ID
        let folder_map = require_folder_map(&self.client).await?;
        let target = folder_map
            .resolve_folder_id(folder_id)
            .unwrap_or(folder_id)
            .to_string();
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        Ok(move_messages(&self.client, ctx, &msg_ids, &target).await?)
    }

    async fn add_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), ProviderError> {
        let category = tag_id.strip_prefix("cat:").unwrap_or(tag_id);
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        // Hold category lock for the entire read-modify-write to prevent clobber
        let _guard = self.client.lock_categories().await;
        let current = batch_get_categories(&self.client, ctx, &msg_ids).await?;
        let mut patches = Vec::new();
        for (msg_id, mut cats) in current {
            if !cats.iter().any(|c| c == category) {
                cats.push(category.to_string());
                patches.push((msg_id, cats));
            }
        }
        Ok(batch_set_categories(&self.client, ctx, &patches).await?)
    }

    async fn remove_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), ProviderError> {
        let category = tag_id.strip_prefix("cat:").unwrap_or(tag_id);
        let msg_ids = query_thread_message_ids(ctx, thread_id).await?;
        let _guard = self.client.lock_categories().await;
        let current = batch_get_categories(&self.client, ctx, &msg_ids).await?;
        let mut patches = Vec::new();
        for (msg_id, mut cats) in current {
            let before_len = cats.len();
            cats.retain(|c| c != category);
            if cats.len() != before_len {
                patches.push((msg_id, cats));
            }
        }
        Ok(batch_set_categories(&self.client, ctx, &patches).await?)
    }

    async fn apply_category(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        category_name: &str,
    ) -> Result<(), ProviderError> {
        let _guard = self.client.lock_categories().await;
        let current = batch_get_categories(&self.client, ctx, &[message_id.to_string()]).await?;
        let (_, mut cats) = current.into_iter().next()
            .ok_or_else(|| ProviderError::NotFound("No category data returned".to_string()))?;
        if !cats.iter().any(|c| c == category_name) {
            cats.push(category_name.to_string());
            batch_set_categories(&self.client, ctx, &[(message_id.to_string(), cats)]).await?;
        }
        Ok(())
    }

    async fn remove_category(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        category_name: &str,
    ) -> Result<(), ProviderError> {
        let _guard = self.client.lock_categories().await;
        let current = batch_get_categories(&self.client, ctx, &[message_id.to_string()]).await?;
        let (_, mut cats) = current.into_iter().next()
            .ok_or_else(|| ProviderError::NotFound("No category data returned".to_string()))?;
        let before_len = cats.len();
        cats.retain(|c| c != category_name);
        if cats.len() != before_len {
            batch_set_categories(&self.client, ctx, &[(message_id.to_string(), cats)]).await?;
        }
        Ok(())
    }

    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
        mentions: &[(String, String)],
    ) -> Result<String, ProviderError> {
        Ok(send_via_draft(&self.client, ctx, raw_base64url, thread_id, mentions).await?)
    }

    async fn create_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
        mentions: &[(String, String)],
    ) -> Result<String, ProviderError> {
        Ok(create_draft_impl(&self.client, ctx, raw_base64url, thread_id, mentions).await?)
    }

    async fn update_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, ProviderError> {
        // Graph has no draft mutation — delete and recreate
        let enc_id = urlencoding::encode(draft_id);
        let me = self.me();
        self.client
            .delete(&format!("{me}/messages/{enc_id}"), ctx.db)
            .await?;
        Ok(create_draft_impl(&self.client, ctx, raw_base64url, thread_id, &[]).await?)
    }

    async fn delete_draft(&self, ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), ProviderError> {
        let enc_id = urlencoding::encode(draft_id);
        let me = self.me();
        self.client
            .delete(&format!("{me}/messages/{enc_id}"), ctx.db)
            .await?;
        Ok(())
    }

    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentData, ProviderError> {
        let enc_msg_id = urlencoding::encode(message_id);
        let enc_att_id = urlencoding::encode(attachment_id);
        let me = self.me();
        let attachment: GraphAttachment = self
            .client
            .get_json(
                &format!("{me}/messages/{enc_msg_id}/attachments/{enc_att_id}"),
                ctx.db,
            )
            .await?;

        let data = if let Some(ref content_bytes) = attachment.content_bytes {
            ratatoskr_provider_utils::encoding::decode_base64_standard(content_bytes)
                .map_err(|e| ProviderError::Client(format!("Failed to decode attachment: {e}")))?
        } else {
            let raw = self
                .client
                .get_bytes(
                    &format!("{me}/messages/{enc_msg_id}/attachments/{enc_att_id}/$value"),
                    ctx.db,
                )
                .await?;
            if raw.is_empty() {
                return Err(ProviderError::NotFound(format!("Attachment {attachment_id} has no content")));
            }
            raw
        };

        let size = data.len();
        Ok(AttachmentData {
            data: ratatoskr_provider_utils::encoding::encode_base64_standard(&data),
            size,
        })
    }

    async fn list_folders(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, ProviderError> {
        // Use cached folder map if it was synced less than 60 seconds ago
        let use_cache = if let Some(age) = self.client.folder_map_age().await {
            age < std::time::Duration::from_secs(60) && self.client.folder_map().await.is_some()
        } else {
            false
        };

        let folder_map = if use_cache {
            self.client
                .folder_map()
                .await
                .ok_or_else(|| ProviderError::Client("Folder map vanished".to_string()))?
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
    ) -> Result<ProviderFolderMutation, ProviderError> {
        let parent_graph_id = match parent_id {
            Some(parent_id) => {
                Some(resolve_graph_folder_id(&self.client, ctx, parent_id, false).await?)
            }
            None => None,
        };
        let body = GraphCreateFolderRequest {
            display_name: name.to_string(),
        };
        let me = self.me();
        let created: GraphMailFolder = if let Some(parent_graph_id) = parent_graph_id {
            let enc_parent_id = urlencoding::encode(&parent_graph_id);
            self.client
                .post(
                    &format!("{me}/mailFolders/{enc_parent_id}/childFolders"),
                    &body,
                    ctx.db,
                )
                .await?
        } else {
            self.client
                .post(&format!("{me}/mailFolders"), &body, ctx.db)
                .await?
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
    ) -> Result<ProviderFolderMutation, ProviderError> {
        let graph_folder_id = resolve_graph_folder_id(&self.client, ctx, folder_id, true).await?;
        let enc_folder_id = urlencoding::encode(&graph_folder_id);
        let body = GraphRenameFolderRequest {
            display_name: new_name.to_string(),
        };
        let me = self.me();
        self.client
            .patch(&format!("{me}/mailFolders/{enc_folder_id}"), &body, ctx.db)
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

    async fn delete_folder(&self, ctx: &ProviderCtx<'_>, folder_id: &str) -> Result<(), ProviderError> {
        let graph_folder_id = resolve_graph_folder_id(&self.client, ctx, folder_id, true).await?;
        let enc_folder_id = urlencoding::encode(&graph_folder_id);
        let me = self.me();
        self.client
            .delete(&format!("{me}/mailFolders/{enc_folder_id}"), ctx.db)
            .await?;
        delete_folder_delta_token(ctx, &graph_folder_id).await?;
        refresh_folder_map(&self.client, ctx).await?;
        Ok(())
    }

    async fn test_connection(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderTestResult, ProviderError> {
        let me = self.me();
        let profile: super::types::GraphProfile = self
            .client
            .get_json(
                &format!("{me}?$select=displayName,mail,userPrincipalName"),
                ctx.db,
            )
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

    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, ProviderError> {
        let me = self.me();
        let profile: super::types::GraphProfile = self
            .client
            .get_json(
                &format!("{me}?$select=displayName,mail,userPrincipalName"),
                ctx.db,
            )
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

// ── Deferred delivery (Exchange scheduled send) ─────────────

impl GraphOps {
    /// Schedule a deferred send via Exchange's `PidTagDeferredSendTime`.
    ///
    /// Creates a draft from raw MIME, sets the deferred send time as a
    /// `singleValueLegacyExtendedProperty`, and then sends it. Exchange
    /// holds the message server-side until `send_at_utc` before delivering.
    ///
    /// Returns the draft/message ID.
    pub async fn schedule_send(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        thread_id: Option<&str>,
        send_at_utc: &str,
    ) -> Result<String, String> {
        let draft_id =
            send::create_draft_with_deferred_time(&self.client, ctx, raw_base64url, thread_id, send_at_utc)
                .await?;

        // Send the draft — Exchange will hold it until the deferred time
        let enc_draft_id = urlencoding::encode(&draft_id);
        let me = self.me();
        self.client
            .post_no_content::<()>(&format!("{me}/messages/{enc_draft_id}/send"), None, ctx.db)
            .await?;

        log::info!("Scheduled deferred send for {send_at_utc}, draft_id={draft_id}");
        Ok(draft_id)
    }

    /// Cancel a deferred send by deleting the message before its scheduled time.
    ///
    /// The message must still be in the Drafts/Outbox folder (not yet sent).
    /// After the deferred send time has passed, the message is already delivered
    /// and cannot be cancelled.
    pub async fn cancel_scheduled_send(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
    ) -> Result<(), String> {
        let enc_id = urlencoding::encode(message_id);
        let me = self.me();
        self.client
            .delete(&format!("{me}/messages/{enc_id}"), ctx.db)
            .await?;
        log::info!("Cancelled deferred send for message_id={message_id}");
        Ok(())
    }

    /// Reschedule a deferred send by updating the `PidTagDeferredSendTime`
    /// extended property on the message to a new UTC time.
    ///
    /// The message must still be pending (not yet delivered by Exchange).
    pub async fn reschedule_send(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        new_send_at_utc: &str,
    ) -> Result<(), String> {
        let enc_id = urlencoding::encode(message_id);
        let patch = GraphMessagePatch {
            single_value_extended_properties: Some(vec![SingleValueExtendedProperty {
                id: PID_TAG_DEFERRED_SEND_TIME.to_string(),
                value: new_send_at_utc.to_string(),
            }]),
            ..Default::default()
        };
        let me = self.me();
        self.client
            .patch(&format!("{me}/messages/{enc_id}"), &patch, ctx.db)
            .await?;
        log::info!(
            "Rescheduled deferred send to {new_send_at_utc} for message_id={message_id}"
        );
        Ok(())
    }
}

// ── Shared mailbox send ─────────────────────────────────────

impl GraphOps {
    /// Send as a shared mailbox ("Send As" permission).
    ///
    /// Sets `from` to the shared mailbox address and sends via
    /// `POST /users/{shared_mailbox}/messages/{id}/send`. The delegate's
    /// identity is invisible to the recipient — the message appears to come
    /// directly from the shared mailbox.
    ///
    /// Requires `Mail.Send.Shared` OAuth scope and "Send As" permission
    /// on the shared mailbox in Exchange.
    pub async fn send_as_shared_mailbox(
        &self,
        ctx: &ProviderCtx<'_>,
        shared_mailbox_email: &str,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, String> {
        use super::types::{GraphEmailAddress, GraphRecipient};

        let raw_bytes = ratatoskr_provider_utils::encoding::decode_base64url_nopad(raw_base64url)
            .map_err(|e| format!("Failed to decode base64url: {e}"))?;

        let parsed = mail_parser::MessageParser::default()
            .parse(&raw_bytes)
            .ok_or("Failed to parse MIME message")?;

        let mut create_msg = send::mime_to_graph_message(&parsed)?;

        // Override `from` to the shared mailbox
        create_msg.from = Some(GraphRecipient {
            email_address: GraphEmailAddress {
                name: None,
                address: shared_mailbox_email.to_string(),
            },
        });

        // Create draft in the shared mailbox's folder
        let enc_user = urlencoding::encode(shared_mailbox_email);
        let draft: serde_json::Value = self
            .client
            .post(
                &format!("/users/{enc_user}/messages"),
                &create_msg,
                ctx.db,
            )
            .await?;

        let draft_id = draft
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Draft response missing id")?
            .to_string();

        // Upload attachments to the shared mailbox draft
        send::upload_attachments_to_user_mailbox(
            &self.client,
            ctx,
            &enc_user,
            &draft_id,
            &parsed,
        )
        .await?;

        // Send the draft from the shared mailbox
        let enc_draft_id = urlencoding::encode(&draft_id);
        self.client
            .post_no_content::<()>(
                &format!("/users/{enc_user}/messages/{enc_draft_id}/send"),
                None,
                ctx.db,
            )
            .await?;

        log::info!(
            "Sent as shared mailbox {shared_mailbox_email}, draft_id={draft_id}"
        );
        Ok(draft_id)
    }

    /// Send on behalf of a shared mailbox ("Send on Behalf Of" permission).
    ///
    /// Sets `from` to the shared mailbox and `sender` to the delegate.
    /// The recipient sees "Alice on behalf of SharedMailbox".
    ///
    /// Requires `Mail.Send.Shared` OAuth scope and "Send on Behalf Of"
    /// permission on the shared mailbox in Exchange.
    pub async fn send_on_behalf_of(
        &self,
        ctx: &ProviderCtx<'_>,
        shared_mailbox_email: &str,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, String> {
        use super::types::{GraphEmailAddress, GraphRecipient};

        let raw_bytes = ratatoskr_provider_utils::encoding::decode_base64url_nopad(raw_base64url)
            .map_err(|e| format!("Failed to decode base64url: {e}"))?;

        let parsed = mail_parser::MessageParser::default()
            .parse(&raw_bytes)
            .ok_or("Failed to parse MIME message")?;

        let mut create_msg = send::mime_to_graph_message(&parsed)?;

        // Get delegate email from account
        let delegate_email = {
            let aid = ctx.account_id.to_string();
            ctx.db
                .with_conn(move |conn| {
                    conn.query_row(
                        "SELECT email FROM accounts WHERE id = ?1",
                        rusqlite::params![aid],
                        |row| row.get::<_, String>(0),
                    )
                    .map_err(|e| format!("lookup delegate email: {e}"))
                })
                .await?
        };

        // Set `from` to the shared mailbox, `sender` to the delegate
        create_msg.from = Some(GraphRecipient {
            email_address: GraphEmailAddress {
                name: None,
                address: shared_mailbox_email.to_string(),
            },
        });
        create_msg.sender = Some(GraphRecipient {
            email_address: GraphEmailAddress {
                name: None,
                address: delegate_email.clone(),
            },
        });

        // Create draft in the shared mailbox
        let enc_user = urlencoding::encode(shared_mailbox_email);
        let draft: serde_json::Value = self
            .client
            .post(
                &format!("/users/{enc_user}/messages"),
                &create_msg,
                ctx.db,
            )
            .await?;

        let draft_id = draft
            .get("id")
            .and_then(|v| v.as_str())
            .ok_or("Draft response missing id")?
            .to_string();

        send::upload_attachments_to_user_mailbox(
            &self.client,
            ctx,
            &enc_user,
            &draft_id,
            &parsed,
        )
        .await?;

        let enc_draft_id = urlencoding::encode(&draft_id);
        self.client
            .post_no_content::<()>(
                &format!("/users/{enc_user}/messages/{enc_draft_id}/send"),
                None,
                ctx.db,
            )
            .await?;

        log::info!(
            "Sent on behalf of {shared_mailbox_email} (delegate: {delegate_email}), draft_id={draft_id}"
        );
        Ok(draft_id)
    }
}

/// Create a reference attachment (cloud file link) on a draft message.
///
/// Uses the Graph **beta** endpoint because `referenceAttachment` is not
/// available on v1.0.
pub async fn create_reference_attachment(
    client: &GraphClient,
    message_id: &str,
    source_url: &str,
    file_name: &str,
    file_size: Option<i64>,
    provider_type: &str,
    db: &DbState,
) -> Result<(), String> {
    let enc_id = urlencoding::encode(message_id);
    let me = client.api_path_prefix();
    let path = format!("{me}/messages/{enc_id}/attachments");

    let mut body = serde_json::json!({
        "@odata.type": "#microsoft.graph.referenceAttachment",
        "name": file_name,
        "sourceUrl": source_url,
        "providerType": provider_type,
        "permission": "view",
        "isFolder": false,
    });

    if let Some(size) = file_size {
        body.as_object_mut()
            .expect("body is an object")
            .insert("size".to_string(), serde_json::json!(size));
    }

    let _response: serde_json::Value = client.post_beta(&path, &body, db).await?;

    log::info!(
        "Created reference attachment '{file_name}' on message {message_id}"
    );
    Ok(())
}
