use async_trait::async_trait;
use chrono::{DateTime, Utc};
use jmap_client::core::response::{EmailSetResponse, EmailSubmissionSetResponse};
use jmap_client::core::set::SetObject;
use jmap_client::email_submission::{Address as SubmissionAddress, UndoStatus};
use jmap_client::mailbox::Role;
use jmap_client::Set;

use ratatoskr_provider_utils::ops::ProviderOps;
use ratatoskr_provider_utils::types::{
    AttachmentData, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation, ProviderProfile,
    ProviderTestResult, SyncResult,
};

use super::client::JmapClient;
use super::helpers::{
    get_first_identity_id, get_mailbox_list, query_thread_email_ids, resolve_mailbox_id,
};
use super::mailbox_mapper::{find_mailbox_id_by_role, map_mailbox_to_label};

/// JMAP implementation of the provider operations trait.
pub struct JmapOps {
    pub(crate) client: JmapClient,
}

impl JmapOps {
    pub fn new(client: JmapClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl ProviderOps for JmapOps {
    async fn sync_initial(
        &self,
        ctx: &ProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, String> {
        self.client.ensure_valid_token().await?;
        super::sync::jmap_initial_sync(
            &self.client,
            ctx.account_id,
            days_back,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
        )
        .await?;
        Ok(SyncResult::default())
    }

    async fn sync_delta(
        &self,
        ctx: &ProviderCtx<'_>,
        _days_back: Option<i64>,
    ) -> Result<SyncResult, String> {
        self.client.ensure_valid_token().await?;
        let result = super::sync::jmap_delta_sync(
            &self.client,
            ctx.account_id,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            ctx.progress,
        )
        .await?;
        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_email_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }

    async fn archive(&self, _ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let mailboxes = get_mailbox_list(&self.client).await?;
        let inbox_id =
            find_mailbox_id_by_role(&mailboxes, "inbox").ok_or("No inbox mailbox found")?;
        let archive_id = find_mailbox_id_by_role(&mailboxes, "archive");

        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        for eid in &email_ids {
            let update = set_req.update(eid);
            update.mailbox_id(&inbox_id, false);
            if let Some(ref aid) = archive_id {
                update.mailbox_id(aid, true);
            }
        }
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("archive: {e}"))?;
        Ok(())
    }

    async fn trash(&self, _ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let mailboxes = get_mailbox_list(&self.client).await?;
        let trash_id =
            find_mailbox_id_by_role(&mailboxes, "trash").ok_or("No trash mailbox found")?;
        let inbox_id = find_mailbox_id_by_role(&mailboxes, "inbox");

        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        for eid in &email_ids {
            let update = set_req.update(eid);
            update.mailbox_id(&trash_id, true);
            if let Some(ref iid) = inbox_id {
                update.mailbox_id(iid, false);
            }
        }
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("trash: {e}"))?;
        Ok(())
    }

    async fn permanent_delete(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        request
            .set_email()
            .destroy(email_ids.iter().map(String::as_str));
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("permanent delete: {e}"))?;
        Ok(())
    }

    async fn mark_read(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        for eid in &email_ids {
            set_req.update(eid).keyword("$seen", read);
        }
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("mark read: {e}"))?;
        Ok(())
    }

    async fn star(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        for eid in &email_ids {
            set_req.update(eid).keyword("$flagged", starred);
        }
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("star: {e}"))?;
        Ok(())
    }

    async fn spam(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let mailboxes = get_mailbox_list(&self.client).await?;
        let junk_id =
            find_mailbox_id_by_role(&mailboxes, "junk").ok_or("No junk/spam mailbox found")?;
        let inbox_id =
            find_mailbox_id_by_role(&mailboxes, "inbox").ok_or("No inbox mailbox found")?;

        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        for eid in &email_ids {
            if is_spam {
                set_req
                    .update(eid)
                    .mailbox_id(&junk_id, true)
                    .mailbox_id(&inbox_id, false);
            } else {
                set_req
                    .update(eid)
                    .mailbox_id(&inbox_id, true)
                    .mailbox_id(&junk_id, false);
            }
        }
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("spam: {e}"))?;
        Ok(())
    }

    async fn move_to_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        folder_id: &str,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let target_id = resolve_mailbox_id(&self.client, folder_id).await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        for eid in &email_ids {
            set_req.update(eid).mailbox_ids([target_id.as_str()]);
        }
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("move to folder: {e}"))?;
        Ok(())
    }

    async fn add_tag(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let mailbox_id = resolve_mailbox_id(&self.client, tag_id).await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        for eid in &email_ids {
            set_req.update(eid).mailbox_id(&mailbox_id, true);
        }
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("add tag: {e}"))?;
        Ok(())
    }

    async fn remove_tag(
        &self,
        _ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &str,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let mailbox_id = resolve_mailbox_id(&self.client, tag_id).await?;
        let email_ids = query_thread_email_ids(&self.client, thread_id).await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        for eid in &email_ids {
            set_req.update(eid).mailbox_id(&mailbox_id, false);
        }
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("remove tag: {e}"))?;
        Ok(())
    }

    async fn apply_category(
        &self,
        _ctx: &ProviderCtx<'_>,
        message_id: &str,
        category_name: &str,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        set_req.update(message_id).keyword(category_name, true);
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("apply category: {e}"))?;
        Ok(())
    }

    async fn remove_category(
        &self,
        _ctx: &ProviderCtx<'_>,
        message_id: &str,
        category_name: &str,
    ) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let client = self.client.inner();
        let mut request = client.build();
        let set_req = request.set_email();
        set_req.update(message_id).keyword(category_name, false);
        request
            .send_single::<EmailSetResponse>()
            .await
            .map_err(|e| format!("remove category: {e}"))?;
        Ok(())
    }

    async fn send_email(
        &self,
        _ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
        _mentions: &[(String, String)],
    ) -> Result<String, String> {
        self.client.ensure_valid_token().await?;
        let raw_bytes = ratatoskr_provider_utils::encoding::decode_base64url_nopad(raw_base64url)?;
        let client = self.client.inner();

        // Step 1: Upload blob and fetch identity concurrently.
        let (mut upload_res, identity_id) = tokio::try_join!(
            async {
                client
                    .upload(None, raw_bytes, None)
                    .await
                    .map_err(|e| format!("Blob upload: {e}"))
            },
            get_first_identity_id(&client),
        )?;
        let blob_id = upload_res.take_blob_id();

        // Step 2: Batch Email/import + EmailSubmission/set into a single JMAP request.
        // The import creates the email with $seen keyword. The submission sends it,
        // and onSuccessUpdateEmail atomically clears $draft when the submission succeeds.
        let mut request = client.build();

        let import_create_id = request
            .import_email()
            .email(&blob_id)
            .mailbox_ids(Vec::<String>::new())
            .keywords(["$seen"])
            .create_id();

        let sub_request = request.set_email_submission();
        let sub_create_id = sub_request
            .create()
            .email_id(format!("#{import_create_id}"))
            .identity_id(&identity_id)
            .create_id()
            .expect("submission create_id");
        sub_request
            .arguments()
            .on_success_update_email(&sub_create_id)
            .keyword("$draft", false);

        let mut import_response = request
            .send()
            .await
            .map_err(|e| format!("JMAP batch send: {e}"))?
            .unwrap_method_responses()
            .into_iter()
            .next()
            .ok_or("No response from JMAP batch")?
            .unwrap_import_email()
            .map_err(|e| format!("Email/import response: {e}"))?;

        let email_id = import_response
            .created(&import_create_id)
            .map_err(|e| format!("Email/import: {e}"))?
            .take_id();

        Ok(email_id)
    }

    async fn create_draft(
        &self,
        _ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
        _mentions: &[(String, String)],
    ) -> Result<String, String> {
        self.client.ensure_valid_token().await?;
        let raw_bytes = ratatoskr_provider_utils::encoding::decode_base64url_nopad(raw_base64url)?;

        let mailboxes = get_mailbox_list(&self.client).await?;
        let drafts_id =
            find_mailbox_id_by_role(&mailboxes, "drafts").ok_or("No drafts mailbox found")?;

        let client = self.client.inner();
        let mut email = client
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
        self.client.ensure_valid_token().await?;
        // JMAP has no draft mutation — delete old, create new
        let client = self.client.inner();
        client
            .email_destroy(draft_id)
            .await
            .map_err(|e| format!("delete old draft: {e}"))?;
        self.create_draft(_ctx, raw_base64url, _thread_id, &[]).await
    }

    async fn delete_draft(&self, _ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let client = self.client.inner();
        client
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
        self.client.ensure_valid_token().await?;
        let client = self.client.inner();
        let data = client
            .download(attachment_id)
            .await
            .map_err(|e| format!("Blob download: {e}"))?;

        Ok(AttachmentData {
            data: ratatoskr_provider_utils::encoding::encode_base64_standard(&data),
            size: data.len(),
        })
    }

    async fn list_folders(
        &self,
        _ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, String> {
        self.client.ensure_valid_token().await?;
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

            folders.push(ProviderFolderEntry {
                id: mapping.label_id,
                name: mapping.label_name,
                path: name.to_string(),
                folder_type: mapping.label_type.to_string(),
                special_use: role_str.map(String::from),
                delimiter: Some("/".to_string()),
                message_count: None,
                unread_count: None,
                color_bg: None,
                color_fg: None,
            });
        }
        Ok(folders)
    }

    async fn create_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        name: &str,
        parent_id: Option<&str>,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, String> {
        self.client.ensure_valid_token().await?;
        let client = self.client.inner();
        let mut mb = client
            .mailbox_create(name, parent_id.map(ToOwned::to_owned), Role::None)
            .await
            .map_err(|e| format!("Mailbox/set create: {e}"))?;

        self.client.invalidate_mailbox_cache().await;
        let id = mb.take_id();
        Ok(ProviderFolderMutation {
            id: format!("jmap-{id}"),
            name: name.to_string(),
            path: name.to_string(),
            folder_type: "user".to_string(),
            special_use: None,
            delimiter: Some("/".to_string()),
            color_bg: None,
            color_fg: None,
        })
    }

    async fn rename_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        folder_id: &str,
        new_name: &str,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, String> {
        self.client.ensure_valid_token().await?;
        let mailbox_id = resolve_mailbox_id(&self.client, folder_id).await?;
        let client = self.client.inner();
        client
            .mailbox_rename(&mailbox_id, new_name)
            .await
            .map_err(|e| format!("Mailbox/set rename: {e}"))?;
        self.client.invalidate_mailbox_cache().await;

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

    async fn delete_folder(&self, _ctx: &ProviderCtx<'_>, folder_id: &str) -> Result<(), String> {
        self.client.ensure_valid_token().await?;
        let mailbox_id = resolve_mailbox_id(&self.client, folder_id).await?;
        let client = self.client.inner();
        client
            .mailbox_destroy(&mailbox_id, true)
            .await
            .map_err(|e| format!("Mailbox/set destroy: {e}"))?;
        self.client.invalidate_mailbox_cache().await;
        Ok(())
    }

    async fn test_connection(&self, _ctx: &ProviderCtx<'_>) -> Result<ProviderTestResult, String> {
        self.client.ensure_valid_token().await?;
        let session = self.client.inner().session();
        Ok(ProviderTestResult {
            success: true,
            message: format!("Connected as {}", session.username()),
        })
    }

    async fn get_profile(&self, _ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, String> {
        self.client.ensure_valid_token().await?;
        let session = self.client.inner().session();
        Ok(ProviderProfile {
            email: session.username().to_string(),
            name: None,
        })
    }
}

// ---------------------------------------------------------------------------
// JMAP FUTURERELEASE — Scheduled send via EmailSubmission
// ---------------------------------------------------------------------------
//
// RFC 4865 FUTURERELEASE uses the SMTP MAIL FROM parameter `HOLDUNTIL` to
// defer delivery.  In JMAP this is exposed through the `envelope.mailFrom
// .parameters` object on `EmailSubmission` (RFC 8621 §7).
//
// The server advertises the capability via the `urn:ietf:params:jmap:submission`
// session capability object whose `maxDelayedSend` property (seconds) declares
// the maximum deferral the server supports.  A value of 0 means scheduled send
// is not supported.

/// Schedule an email for future delivery via JMAP FUTURERELEASE.
///
/// Creates an `EmailSubmission` with an explicit envelope whose `mailFrom`
/// carries the `holduntil` parameter set to the requested UTC timestamp
/// (RFC 4865 / RFC 8621 §7).
///
/// # Arguments
///
/// * `client`       – Authenticated JMAP client.
/// * `email_id`     – ID of an already-imported `Email` object on the server.
/// * `identity_id`  – Identity to send from (see `get_first_identity_id`).
/// * `sender_email` – RFC 5321 MAIL FROM address (used in the envelope).
/// * `recipients`   – RFC 5321 RCPT TO addresses (used in the envelope).
/// * `send_at`      – Desired delivery time (UTC).  Must be in the future.
///
/// # Returns
///
/// The server-assigned `EmailSubmission` ID, which can be passed to
/// [`cancel_scheduled_send_jmap`] to abort the deferred delivery.
pub async fn schedule_send_jmap(
    client: &JmapClient,
    email_id: &str,
    identity_id: &str,
    sender_email: &str,
    recipients: &[String],
    send_at: DateTime<Utc>,
) -> Result<String, String> {
    let inner = client.inner();

    // --- Validate against maxDelayedSend -----------------------------------
    let session = inner.session();
    if let Some(sub_caps) = session.submission_capabilities() {
        let max_delay = sub_caps.max_delayed_send(); // seconds
        if max_delay == 0 {
            return Err(
                "Server does not support scheduled send (maxDelayedSend = 0)".to_string(),
            );
        }
        let delay_secs = (send_at - Utc::now()).num_seconds();
        if delay_secs <= 0 {
            return Err("send_at must be in the future".to_string());
        }
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        if (delay_secs as usize) > max_delay {
            return Err(format!(
                "Requested delay ({delay_secs}s) exceeds server maximum ({max_delay}s)"
            ));
        }
    }
    // If submission_capabilities is None the server didn't advertise the
    // submission capability at all — we still attempt the call and let the
    // server reject it if it doesn't support FUTURERELEASE.

    // --- Build the envelope with HOLDUNTIL ---------------------------------
    // RFC 4865 §4: HOLDUNTIL value is an ISO 8601 UTC timestamp.
    let holduntil_value = send_at.to_rfc3339();

    let mail_from =
        SubmissionAddress::<Set>::new(sender_email).parameter("holduntil", Some(&holduntil_value));

    let rcpt_to: Vec<SubmissionAddress<Set>> = recipients
        .iter()
        .map(|r| SubmissionAddress::<Set>::new(r.as_str()))
        .collect();

    // --- Create the EmailSubmission ----------------------------------------
    let mut request = inner.build();
    let sub_req = request.set_email_submission();
    let create_id = sub_req
        .create()
        .email_id(email_id)
        .identity_id(identity_id)
        .envelope(mail_from, rcpt_to)
        .create_id()
        .ok_or("Failed to obtain submission create ID")?;

    let mut response = request
        .send_single::<EmailSubmissionSetResponse>()
        .await
        .map_err(|e| format!("EmailSubmission/set (schedule): {e}"))?;

    let mut submission = response
        .created(&create_id)
        .map_err(|e| format!("EmailSubmission create failed: {e}"))?;

    Ok(submission.take_id())
}

/// Cancel a previously scheduled send by setting `undoStatus` to `"canceled"`.
///
/// This only works while the submission's `undoStatus` is still `"pending"`.
/// Once the server transitions it to `"final"` (i.e. the message has been
/// handed off to the MTA), cancellation is no longer possible and the server
/// will return an error.
pub async fn cancel_scheduled_send_jmap(
    client: &JmapClient,
    submission_id: &str,
) -> Result<(), String> {
    let inner = client.inner();

    let mut request = inner.build();
    request
        .set_email_submission()
        .update(submission_id)
        .undo_status(UndoStatus::Canceled);

    let mut response = request
        .send_single::<EmailSubmissionSetResponse>()
        .await
        .map_err(|e| format!("EmailSubmission/set (cancel): {e}"))?;

    response
        .updated(submission_id)
        .map_err(|e| format!("EmailSubmission cancel failed: {e}"))?;

    Ok(())
}
