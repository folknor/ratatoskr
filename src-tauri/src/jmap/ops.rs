use async_trait::async_trait;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jmap_client::mailbox::Role;

use crate::provider::ops::ProviderOps;
use crate::provider::types::{AttachmentData, ProviderCtx, ProviderFolder, SyncResult};

use super::client::JmapClient;
use super::commands::{
    get_first_identity_id, get_mailbox_list, query_thread_email_ids, resolve_mailbox_id,
};
use super::mailbox_mapper::{find_mailbox_id_by_role, map_mailbox_to_label};

/// JMAP implementation of the provider operations trait.
pub struct JmapOps {
    pub(crate) client: JmapClient,
}

#[async_trait]
impl ProviderOps for JmapOps {
    async fn sync_initial(&self, ctx: &ProviderCtx<'_>, days_back: i64) -> Result<(), String> {
        super::sync::jmap_initial_sync(
            &self.client,
            ctx.account_id,
            days_back,
            ctx.db,
            ctx.body_store,
            ctx.search,
            ctx.app_handle,
        )
        .await
    }

    async fn sync_delta(&self, ctx: &ProviderCtx<'_>) -> Result<SyncResult, String> {
        let result = super::sync::jmap_delta_sync(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.body_store,
            ctx.search,
            ctx.app_handle,
        )
        .await?;
        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_email_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }

    async fn archive(&self, _ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let mailboxes = get_mailbox_list(&self.client).await?;
        let inbox_id =
            find_mailbox_id_by_role(&mailboxes, "inbox").ok_or("No inbox mailbox found")?;
        let archive_id = find_mailbox_id_by_role(&mailboxes, "archive");

        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            self.client
                .inner()
                .email_set_mailbox(eid, &inbox_id, false)
                .await
                .map_err(|e| format!("archive remove inbox: {e}"))?;
            if let Some(ref aid) = archive_id {
                self.client
                    .inner()
                    .email_set_mailbox(eid, aid, true)
                    .await
                    .map_err(|e| format!("archive add archive: {e}"))?;
            }
        }
        Ok(())
    }

    async fn trash(&self, _ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let mailboxes = get_mailbox_list(&self.client).await?;
        let trash_id =
            find_mailbox_id_by_role(&mailboxes, "trash").ok_or("No trash mailbox found")?;
        let inbox_id = find_mailbox_id_by_role(&mailboxes, "inbox");

        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            self.client
                .inner()
                .email_set_mailbox(eid, &trash_id, true)
                .await
                .map_err(|e| format!("trash add: {e}"))?;
            if let Some(ref iid) = inbox_id {
                self.client
                    .inner()
                    .email_set_mailbox(eid, iid, false)
                    .await
                    .map_err(|e| format!("trash remove inbox: {e}"))?;
            }
        }
        Ok(())
    }

    async fn permanent_delete(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), String> {
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            self.client
                .inner()
                .email_destroy(eid)
                .await
                .map_err(|e| format!("permanent delete: {e}"))?;
        }
        Ok(())
    }

    async fn mark_read(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), String> {
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            self.client
                .inner()
                .email_set_keyword(eid, "$seen", read)
                .await
                .map_err(|e| format!("mark read: {e}"))?;
        }
        Ok(())
    }

    async fn star(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), String> {
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            self.client
                .inner()
                .email_set_keyword(eid, "$flagged", starred)
                .await
                .map_err(|e| format!("star: {e}"))?;
        }
        Ok(())
    }

    async fn spam(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), String> {
        let mailboxes = get_mailbox_list(&self.client).await?;
        let junk_id =
            find_mailbox_id_by_role(&mailboxes, "junk").ok_or("No junk/spam mailbox found")?;
        let inbox_id =
            find_mailbox_id_by_role(&mailboxes, "inbox").ok_or("No inbox mailbox found")?;

        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            if is_spam {
                self.client
                    .inner()
                    .email_set_mailbox(eid, &junk_id, true)
                    .await
                    .map_err(|e| format!("spam add junk: {e}"))?;
                self.client
                    .inner()
                    .email_set_mailbox(eid, &inbox_id, false)
                    .await
                    .map_err(|e| format!("spam remove inbox: {e}"))?;
            } else {
                self.client
                    .inner()
                    .email_set_mailbox(eid, &inbox_id, true)
                    .await
                    .map_err(|e| format!("not-spam add inbox: {e}"))?;
                self.client
                    .inner()
                    .email_set_mailbox(eid, &junk_id, false)
                    .await
                    .map_err(|e| format!("not-spam remove junk: {e}"))?;
            }
        }
        Ok(())
    }

    async fn move_to_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        folder_id: &str,
    ) -> Result<(), String> {
        let target_id = resolve_mailbox_id(&self.client, folder_id).await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            self.client
                .inner()
                .email_set_mailboxes(eid, vec![target_id.clone()])
                .await
                .map_err(|e| format!("move to folder: {e}"))?;
        }
        Ok(())
    }

    async fn add_tag(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        let mailbox_id = resolve_mailbox_id(&self.client, tag_id).await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            self.client
                .inner()
                .email_set_mailbox(eid, &mailbox_id, true)
                .await
                .map_err(|e| format!("add tag: {e}"))?;
        }
        Ok(())
    }

    async fn remove_tag(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        let mailbox_id = resolve_mailbox_id(&self.client, tag_id).await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        for eid in &email_ids {
            self.client
                .inner()
                .email_set_mailbox(eid, &mailbox_id, false)
                .await
                .map_err(|e| format!("remove tag: {e}"))?;
        }
        Ok(())
    }

    async fn send_email(
        &self,
        _ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, String> {
        let raw_bytes = URL_SAFE_NO_PAD
            .decode(raw_base64url)
            .map_err(|e| format!("base64url decode: {e}"))?;

        let mut email = self
            .client
            .inner()
            .email_import(
                raw_bytes,
                Vec::<String>::new(),
                Some(vec!["$seen".to_string()]),
                None,
            )
            .await
            .map_err(|e| format!("Email/import: {e}"))?;

        let email_id = email.take_id();
        let identity_id = get_first_identity_id(self.client.inner()).await?;

        let _ = self
            .client
            .inner()
            .email_submission_create(&email_id, &identity_id)
            .await
            .map_err(|e| format!("EmailSubmission/set: {e}"))?;

        let _ = self
            .client
            .inner()
            .email_set_keyword(&email_id, "$draft", false)
            .await;
        let _ = self
            .client
            .inner()
            .email_set_keyword(&email_id, "$seen", true)
            .await;

        Ok(email_id)
    }

    async fn create_draft(
        &self,
        _ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, String> {
        let raw_bytes = URL_SAFE_NO_PAD
            .decode(raw_base64url)
            .map_err(|e| format!("base64url decode: {e}"))?;

        let mailboxes = get_mailbox_list(&self.client).await?;
        let drafts_id =
            find_mailbox_id_by_role(&mailboxes, "drafts").ok_or("No drafts mailbox found")?;

        let mut email = self
            .client
            .inner()
            .email_import(
                raw_bytes,
                vec![drafts_id],
                Some(vec!["$draft".to_string(), "$seen".to_string()]),
                None,
            )
            .await
            .map_err(|e| format!("Email/import draft: {e}"))?;

        Ok(email.take_id())
    }

    async fn update_draft(
        &self,
        _ctx: &ProviderCtx<'_>,
        draft_id: &str,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, String> {
        // JMAP has no draft mutation — delete old, create new
        self.client
            .inner()
            .email_destroy(draft_id)
            .await
            .map_err(|e| format!("delete old draft: {e}"))?;
        self.create_draft(_ctx, raw_base64url, _thread_id).await
    }

    async fn delete_draft(&self, _ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), String> {
        self.client
            .inner()
            .email_destroy(draft_id)
            .await
            .map_err(|e| format!("delete draft: {e}"))?;
        Ok(())
    }

    async fn fetch_attachment(
        &self,
        _ctx: &ProviderCtx<'_>,
        _message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentData, String> {
        let data = self
            .client
            .inner()
            .download(attachment_id)
            .await
            .map_err(|e| format!("Blob download: {e}"))?;

        Ok(AttachmentData {
            data: base64::engine::general_purpose::STANDARD.encode(&data),
            size: data.len(),
        })
    }

    async fn list_folders(&self, _ctx: &ProviderCtx<'_>) -> Result<Vec<ProviderFolder>, String> {
        let mailboxes = super::sync::fetch_all_mailboxes(&self.client).await?;

        let mut folders = Vec::new();
        for mb in &mailboxes {
            let Some(id) = mb.id() else { continue };
            let name = mb.name().unwrap_or("(unnamed)");
            let role = mb.role();
            let role_str = if role == Role::None {
                None
            } else {
                Some(super::sync::role_to_str(&role))
            };
            let mapping = map_mailbox_to_label(role_str, id, name);

            folders.push(ProviderFolder {
                id: mapping.label_id,
                name: mapping.label_name,
                path: name.to_string(),
                special_use: role_str.map(String::from),
            });
        }
        Ok(folders)
    }
}
