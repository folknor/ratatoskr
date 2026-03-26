use std::collections::HashMap;

use async_trait::async_trait;
use rusqlite::Connection;

use ratatoskr_provider_utils::error::ProviderError;
use ratatoskr_provider_utils::typed_ids::{FolderId, TagId};
use ratatoskr_provider_utils::folder_roles::{imap_name_to_special_use, imap_special_use_to_label_id};
use ratatoskr_provider_utils::ops::ProviderOps;
use ratatoskr_provider_utils::types::{
    AttachmentData, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation,
    ProviderParsedAttachment, ProviderParsedMessage, ProviderProfile, ProviderTestResult,
    SyncResult,
};
use ratatoskr_smtp as smtp;

use super::client as imap_client;
use super::connection::connect;

/// Map an IMAP folder path + special-use flag to a canonical label ID.
///
/// Mirrors the old TS `mapFolderToLabel` logic:
/// system folders get well-known IDs (INBOX, SENT, …), user folders get `folder-{path}`.
fn canonical_folder_id(path: &str, special_use: Option<&str>) -> String {
    let lower = path.to_lowercase();
    imap_special_use_to_label_id(special_use.unwrap_or_default())
        .or_else(|| imap_name_to_special_use(&lower).and_then(imap_special_use_to_label_id))
        .map(str::to_string)
        .unwrap_or_else(|| format!("folder-{path}"))
}

/// Generate a short random hex string for pseudo-IDs.
fn random_hex8() -> String {
    let mut buf = [0u8; 4];
    if getrandom::getrandom(&mut buf).is_err() {
        // Fallback: use timestamp-based value (extremely unlikely to reach here)
        return format!(
            "{:08x}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.subsec_nanos())
        );
    }
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

/// IMAP implementation of the provider operations trait.
pub struct ImapOps {
    pub(crate) encryption_key: [u8; 32],
}

impl ImapOps {
    pub fn new(encryption_key: [u8; 32]) -> Self {
        Self { encryption_key }
    }

    /// Shorthand for loading the IMAP config from the database.
    async fn load_config(&self, ctx: &ProviderCtx<'_>) -> Result<super::types::ImapConfig, String> {
        crate::account_config::load_imap_config(ctx.db, ctx.account_id, &self.encryption_key)
            .await
    }

}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse an IMAP message ID (`imap-{accountId}-{folder}-{uid}`) into folder + UID.
fn parse_imap_message_id(message_id: &str, account_id: &str) -> Result<(String, u32), String> {
    let prefix = format!("imap-{account_id}-");
    if !message_id.starts_with(&prefix) {
        return Err(format!("Invalid IMAP message ID: {message_id}"));
    }
    let remainder = &message_id[prefix.len()..];
    let last_dash = remainder
        .rfind('-')
        .ok_or_else(|| format!("Invalid message ID format: {message_id}"))?;
    let folder = &remainder[..last_dash];
    let uid: u32 = remainder[last_dash + 1..]
        .parse()
        .map_err(|_| format!("Invalid UID in message ID: {message_id}"))?;
    Ok((folder.to_string(), uid))
}

/// Minimal info needed to locate a message on the IMAP server.
struct ImapMessageRef {
    folder: String,
    uid: u32,
}

/// Query messages for a thread and extract IMAP folder + UID pairs.
fn get_thread_message_refs(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<ImapMessageRef>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT imap_folder, imap_uid FROM messages \
             WHERE account_id = ?1 AND thread_id = ?2 \
             AND imap_folder IS NOT NULL AND imap_uid IS NOT NULL",
        )
        .map_err(|e| format!("prepare: {e}"))?;

    let rows = stmt
        .query_map(rusqlite::params![account_id, thread_id], |row| {
            let folder: String = row.get("imap_folder")?;
            let uid: i64 = row.get("imap_uid")?;
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            Ok(ImapMessageRef {
                folder,
                uid: uid as u32,
            })
        })
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect: {e}"))?;

    if rows.is_empty() {
        return Err(format!(
            "No IMAP messages found for thread {thread_id} in account {account_id}"
        ));
    }

    Ok(rows)
}

/// Group message refs by folder → list of UIDs.
fn group_by_folder(refs: &[ImapMessageRef]) -> HashMap<&str, Vec<u32>> {
    let mut map: HashMap<&str, Vec<u32>> = HashMap::new();
    for r in refs {
        map.entry(&r.folder).or_default().push(r.uid);
    }
    map
}

/// Build a UID set string like "1,5,10".
fn uid_set(uids: &[u32]) -> String {
    uids.iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(",")
}

fn imap_message_to_provider_message(
    account_id: &str,
    folder_label_id: &str,
    msg: &super::types::ImapMessage,
) -> ProviderParsedMessage {
    let attachments = msg
        .attachments
        .iter()
        .map(|att| ProviderParsedAttachment {
            filename: att.filename.clone(),
            mime_type: att.mime_type.clone(),
            size: att.size,
            attachment_id: att.part_id.clone(),
            content_id: att.content_id.clone(),
            is_inline: att.is_inline,
        })
        .collect::<Vec<_>>();
    let mut label_ids = vec![folder_label_id.to_string()];
    if !msg.is_read {
        label_ids.push("UNREAD".to_string());
    }
    if msg.is_starred {
        label_ids.push("STARRED".to_string());
    }
    if msg.is_draft {
        label_ids.push("DRAFT".to_string());
    }

    ProviderParsedMessage {
        id: format!("imap-{account_id}-{}-{}", msg.folder, msg.uid),
        thread_id: String::new(),
        from_address: msg.from_address.clone(),
        from_name: msg.from_name.clone(),
        to_addresses: msg.to_addresses.clone(),
        cc_addresses: msg.cc_addresses.clone(),
        bcc_addresses: msg.bcc_addresses.clone(),
        reply_to: msg.reply_to.clone(),
        subject: msg.subject.clone(),
        snippet: msg.snippet.clone().unwrap_or_else(|| {
            msg.body_text
                .clone()
                .unwrap_or_default()
                .chars()
                .take(200)
                .collect()
        }),
        date: msg.date * 1000,
        is_read: msg.is_read,
        is_starred: msg.is_starred,
        body_html: msg.body_html.clone(),
        body_text: msg.body_text.clone(),
        raw_size: msg.raw_size,
        internal_date: msg.date * 1000,
        label_ids,
        has_attachments: !attachments.is_empty(),
        attachments,
        list_unsubscribe: msg.list_unsubscribe.clone(),
        list_unsubscribe_post: msg.list_unsubscribe_post.clone(),
        auth_results: msg.auth_results.clone(),
    }
}

/// Look up a special-use IMAP folder path from the labels table.
///
/// First checks `imap_special_use`, then falls back to well-known label IDs
/// (e.g., "TRASH", "SPAM") when the server didn't advertise special-use flags.
fn find_special_folder(
    conn: &Connection,
    account_id: &str,
    special_use: &str,
) -> Result<Option<String>, String> {
    // Primary: look up by imap_special_use
    let path: Option<String> = conn
        .query_row(
            "SELECT COALESCE(imap_folder_path, name) AS folder_path FROM labels \
             WHERE account_id = ?1 AND imap_special_use = ?2 LIMIT 1",
            rusqlite::params![account_id, special_use],
            |row| row.get("folder_path"),
        )
        .ok();

    if path.is_some() {
        return Ok(path);
    }

    // Fallback: map special-use to well-known label ID
    let label_id = imap_special_use_to_label_id(special_use);

    if let Some(lid) = label_id {
        let fallback: Option<String> = conn
            .query_row(
                "SELECT COALESCE(imap_folder_path, name) AS folder_path FROM labels \
                 WHERE account_id = ?1 AND id = ?2 AND imap_folder_path IS NOT NULL LIMIT 1",
                rusqlite::params![account_id, lid],
                |row| row.get("folder_path"),
            )
            .ok();

        if fallback.is_some() {
            return Ok(fallback);
        }
    }

    Ok(None)
}

/// Connect, run an IMAP session body, then logout — mirroring the
/// `with_imap_session!` macro from `commands.rs`.
macro_rules! with_session {
    ($config:expr, $session:ident => $body:expr) => {{
        let mut $session = connect($config).await?;
        let result = $body;
        drop($session.logout().await);
        result
    }};
}

/// What to do with messages grouped by folder.
enum FolderAction {
    /// Move messages to the given destination folder.
    Move(String),
    /// Permanently delete messages (no destination).
    Delete,
}

/// Batch keyword set/remove: groups thread messages by folder and issues
/// one IMAP session per folder with a batched UID set, matching the pattern
/// used by `mark_read` and `star`.
async fn set_keyword_batched(
    config: &super::types::ImapConfig,
    ctx: &ProviderCtx<'_>,
    thread_id: &str,
    tag_id: &str,
    flag_op: &str,
) -> Result<(), ProviderError> {
    let Some(keyword) = tag_id.strip_prefix("kw:") else {
        log::debug!("IMAP: keyword op is a no-op for non-keyword tag {tag_id}");
        return Ok(());
    };

    let account_id = ctx.account_id.to_string();
    let tid = thread_id.to_string();

    let refs = ctx
        .db
        .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
        .await?;

    let grouped = group_by_folder(&refs);
    let futs: Vec<_> = grouped
        .iter()
        .map(|(folder, uids)| {
            let config = config.clone();
            let folder = folder.to_string();
            let uids = uid_set(uids);
            let keyword = keyword.to_string();
            let flag_op = flag_op.to_string();
            async move {
                with_session!(&config, session => {
                    imap_client::set_keyword_batch_if_supported(
                        &mut session, &folder, &uids, &flag_op, &keyword,
                    ).await
                })
            }
        })
        .collect();
    futures::future::try_join_all(futs).await?;
    Ok(())
}

/// Group thread message refs by folder and execute move/delete in parallel sessions.
async fn execute_folder_action(
    config: &super::types::ImapConfig,
    refs: &[ImapMessageRef],
    action: &FolderAction,
) -> Result<(), String> {
    let grouped = group_by_folder(refs);
    let futs: Vec<_> = grouped
        .iter()
        .filter(|(folder, _)| match action {
            FolderAction::Move(dest) => **folder != dest,
            FolderAction::Delete => true,
        })
        .map(|(folder, uids)| {
            let config = config.clone();
            let folder = folder.to_string();
            let uids = uid_set(uids);
            let action_dest = match action {
                FolderAction::Move(dest) => Some(dest.clone()),
                FolderAction::Delete => None,
            };
            async move {
                with_session!(&config, session => {
                    match action_dest {
                        Some(dest) => imap_client::move_messages(&mut session, &folder, &uids, &dest).await,
                        None => imap_client::delete_messages(&mut session, &folder, &uids).await,
                    }
                })
            }
        })
        .collect();
    futures::future::try_join_all(futs).await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// ProviderOps implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ProviderOps for ImapOps {
    // ── Sync (delegated to existing Rust IMAP sync engine) ──────────────

    async fn sync_initial(
        &self,
        ctx: &ProviderCtx<'_>,
        days_back: i64,
    ) -> Result<SyncResult, ProviderError> {
        // IMAP sync is handled by the dedicated sync module (sync_imap_initial).
        // This trait method is not the primary entry point for IMAP sync, but we
        // wire it through for consistency with the provider abstraction.
        let account_id = ctx.account_id.to_string();
        let imap_config = self.load_config(ctx).await?;

        let result = super::imap_initial::imap_initial_sync(
            ctx.progress,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            &account_id,
            &imap_config,
            days_back,
        )
        .await?;

        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_message_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }

    async fn sync_delta(
        &self,
        ctx: &ProviderCtx<'_>,
        days_back: Option<i64>,
    ) -> Result<SyncResult, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let imap_config = self.load_config(ctx).await?;
        let days_back = days_back.unwrap_or(365);

        let result = super::imap_delta::imap_delta_sync(
            ctx.progress,
            ctx.db,
            ctx.body_store,
            ctx.inline_images,
            ctx.search,
            &account_id,
            &imap_config,
            days_back,
        )
        .await?;

        Ok(SyncResult {
            new_inbox_message_ids: result.new_inbox_message_ids,
            affected_thread_ids: result.affected_thread_ids,
        })
    }

    // ── Actions ─────────────────────────────────────────────────────────

    async fn archive(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx).await?;

        let (refs, archive_folder) = ctx
            .db
            .with_conn(move |conn| {
                let refs = get_thread_message_refs(conn, &account_id, &tid)?;
                let archive = find_special_folder(conn, &account_id, "\\Archive")?
                    .unwrap_or_else(|| "Archive".to_string());
                Ok((refs, archive))
            })
            .await?;

        execute_folder_action(&config, &refs, &FolderAction::Move(archive_folder)).await?;
        Ok(())
    }

    async fn trash(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx).await?;

        let (refs, trash_folder) = ctx
            .db
            .with_conn(move |conn| {
                let refs = get_thread_message_refs(conn, &account_id, &tid)?;
                let trash = find_special_folder(conn, &account_id, "\\Trash")?
                    .unwrap_or_else(|| "Trash".to_string());
                Ok((refs, trash))
            })
            .await?;

        execute_folder_action(&config, &refs, &FolderAction::Move(trash_folder)).await?;
        Ok(())
    }

    async fn permanent_delete(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx).await?;

        let refs = ctx
            .db
            .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        execute_folder_action(&config, &refs, &FolderAction::Delete).await?;
        Ok(())
    }

    async fn mark_read(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx).await?;

        let refs = ctx
            .db
            .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        let flag_op = if read { "+FLAGS" } else { "-FLAGS" };
        let grouped = group_by_folder(&refs);
        let futs: Vec<_> = grouped
            .iter()
            .map(|(folder, uids)| {
                let config = config.clone();
                let folder = folder.to_string();
                let uids = uid_set(uids);
                async move {
                    with_session!(&config, session => {
                        imap_client::set_flags(&mut session, &folder, &uids, flag_op, "(\\Seen)").await
                    })
                }
            })
            .collect();
        futures::future::try_join_all(futs).await?;

        Ok(())
    }

    async fn star(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx).await?;

        let refs = ctx
            .db
            .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        let flag_op = if starred { "+FLAGS" } else { "-FLAGS" };
        let grouped = group_by_folder(&refs);
        let futs: Vec<_> = grouped
            .iter()
            .map(|(folder, uids)| {
                let config = config.clone();
                let folder = folder.to_string();
                let uids = uid_set(uids);
                async move {
                    with_session!(&config, session => {
                        imap_client::set_flags(&mut session, &folder, &uids, flag_op, "(\\Flagged)").await
                    })
                }
            })
            .collect();
        futures::future::try_join_all(futs).await?;

        Ok(())
    }

    async fn spam(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx).await?;

        let (refs, junk_folder) = ctx
            .db
            .with_conn(move |conn| {
                let refs = get_thread_message_refs(conn, &account_id, &tid)?;
                let junk = find_special_folder(conn, &account_id, "\\Junk")?
                    .unwrap_or_else(|| "Junk".to_string());
                Ok((refs, junk))
            })
            .await?;

        let destination = if is_spam {
            junk_folder
        } else {
            "INBOX".to_string()
        };

        execute_folder_action(&config, &refs, &FolderAction::Move(destination)).await?;
        Ok(())
    }

    async fn move_to_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        folder_id: &FolderId,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let dest = folder_id.as_str().to_string();
        let config = self.load_config(ctx).await?;

        let refs = ctx
            .db
            .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        execute_folder_action(&config, &refs, &FolderAction::Move(dest)).await?;
        Ok(())
    }

    async fn add_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &TagId,
    ) -> Result<(), ProviderError> {
        let config = self.load_config(ctx).await?;
        set_keyword_batched(&config, ctx, thread_id, tag_id.as_str(), "+FLAGS").await
    }

    async fn remove_tag(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        tag_id: &TagId,
    ) -> Result<(), ProviderError> {
        let config = self.load_config(ctx).await?;
        set_keyword_batched(&config, ctx, thread_id, tag_id.as_str(), "-FLAGS").await
    }

    // ── Send + Drafts ───────────────────────────────────────────────────

    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let configs = crate::account_config::load_both_configs(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;
        let smtp_config = configs.smtp;
        let imap_config = configs.imap;

        let sent_folder = ctx
            .db
            .with_conn(move |conn| find_special_folder(conn, &account_id, "\\Sent"))
            .await?
            .unwrap_or_else(|| "Sent".to_string());

        // Inject read-receipt header and send via SMTP
        log::info!("[IMAP] Sending email via SMTP for account {}", ctx.account_id);
        let patched = ratatoskr_provider_utils::headers::inject_read_receipt_header_base64url(raw_base64url)?;
        let result = smtp::client::send_raw_email(&smtp_config, &patched).await?;
        if !result.success {
            log::error!("[IMAP] SMTP send failed for account {}: {}", ctx.account_id, result.message);
            return Err(ProviderError::Server(format!("SMTP send failed: {}", result.message)));
        }
        log::info!("[IMAP] Email sent successfully via SMTP for account {}", ctx.account_id);

        let message_id = format!(
            "imap-sent-{}-{}",
            chrono::Utc::now().timestamp_millis(),
            random_hex8()
        );

        // Copy sent message to Sent folder (non-fatal if it fails)
        let raw_b64url = patched;
        if let Err(e) = async {
            let raw_bytes = ratatoskr_provider_utils::encoding::decode_base64url_nopad(&raw_b64url)?;

            with_session!(&imap_config, session => {
                imap_client::append_message(&mut session, &sent_folder, Some("(\\Seen)"), &raw_bytes).await
            })
        }
        .await
        {
            log::error!("Failed to copy sent message to Sent folder: {e}");
        }

        Ok(message_id)
    }

    async fn create_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let config = self.load_config(ctx).await?;

        let drafts_folder = ctx
            .db
            .with_conn(move |conn| find_special_folder(conn, &account_id, "\\Drafts"))
            .await?
            .unwrap_or_else(|| "Drafts".to_string());

        let raw_bytes = ratatoskr_provider_utils::encoding::decode_base64url_nopad(raw_base64url)?;

        with_session!(&config, session => {
            imap_client::append_message(&mut session, &drafts_folder, Some("(\\Draft)"), &raw_bytes).await
        })?;

        let draft_id = format!(
            "imap-draft-{}-{}",
            chrono::Utc::now().timestamp_millis(),
            random_hex8()
        );
        Ok(draft_id)
    }

    async fn update_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
        raw_base64url: &str,
        thread_id: Option<&str>,
    ) -> Result<String, ProviderError> {
        // Delete old draft, then create a new one
        if let Err(e) = self.delete_draft(ctx, draft_id).await {
            log::warn!("Failed to delete old draft {draft_id} during update: {e}");
        }
        self.create_draft(ctx, raw_base64url, thread_id).await
    }

    async fn delete_draft(&self, ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), ProviderError> {
        // Generated draft IDs (imap-draft-...) can't be mapped to a server UID
        let prefix = format!("imap-{}-", ctx.account_id);
        if !draft_id.starts_with(&prefix) {
            log::debug!("Draft {draft_id} has a generated ID, cannot delete from server");
            return Ok(());
        }

        let (folder, uid) = parse_imap_message_id(draft_id, ctx.account_id)?;
        let config = self.load_config(ctx).await?;

        with_session!(&config, session => {
            imap_client::delete_messages(&mut session, &folder, &uid.to_string()).await
        })?;
        Ok(())
    }

    // ── Attachments ─────────────────────────────────────────────────────

    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentData, ProviderError> {
        let (folder, uid) = parse_imap_message_id(message_id, ctx.account_id)?;
        let part_id = attachment_id.to_string();
        let config = self.load_config(ctx).await?;

        let data = with_session!(&config, session => {
            imap_client::fetch_attachment(&mut session, &folder, uid, &part_id).await
        })?;

        // data is base64-encoded; compute actual byte size
        let padding = if data.ends_with("==") {
            2
        } else if data.ends_with('=') {
            1
        } else {
            0
        };
        let size = (data.len() * 3 / 4).saturating_sub(padding);

        Ok(AttachmentData { data, size })
    }

    async fn fetch_message(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
    ) -> Result<ProviderParsedMessage, ProviderError> {
        let (folder, uid) = parse_imap_message_id(message_id, ctx.account_id)?;
        let account_id = ctx.account_id.to_string();
        let config = self.load_config(ctx).await?;

        let message = with_session!(&config, session => {
            imap_client::fetch_message_body(&mut session, &folder, uid).await
        })?;

        let mut parsed = imap_message_to_provider_message(&account_id, &folder, &message);

        // Look up the thread_id stored during sync; empty string if message isn't indexed yet.
        let msg_id = message_id.to_string();
        if let Ok(thread_id) = ctx
            .db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT thread_id FROM messages WHERE id = ?1",
                    rusqlite::params![msg_id],
                    |row| row.get::<_, String>("thread_id"),
                )
                .map_err(|e| format!("thread_id lookup: {e}"))
            })
            .await
        {
            parsed.thread_id = thread_id;
        }

        Ok(parsed)
    }

    async fn fetch_raw_message(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
    ) -> Result<String, ProviderError> {
        let (folder, uid) = parse_imap_message_id(message_id, ctx.account_id)?;
        let config = self.load_config(ctx).await?;

        Ok(with_session!(&config, session => {
            imap_client::fetch_raw_message(&mut session, &folder, uid).await
        })?)
    }

    // ── Folders ─────────────────────────────────────────────────────────

    async fn list_folders(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, ProviderError> {
        let config = self.load_config(ctx).await?;

        let folders = with_session!(&config, session => {
            imap_client::list_folders(&mut session).await
        })?;

        Ok(folders
            .into_iter()
            .map(|f| {
                let id = canonical_folder_id(&f.path, f.special_use.as_deref());
                let special_use = f.special_use;
                ProviderFolderEntry {
                    id,
                    name: f.name,
                    path: f.path,
                    folder_type: if special_use.is_some() {
                        "system".to_string()
                    } else {
                        "user".to_string()
                    },
                    special_use,
                    delimiter: Some(f.delimiter),
                    message_count: Some(f.exists),
                    unread_count: Some(f.unseen),
                    color_bg: None,
                    color_fg: None,
                }
            })
            .collect())
    }

    async fn create_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        _name: &str,
        _parent_id: Option<&str>,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, ProviderError> {
        Err(ProviderError::Client(
            "Creating folders is not supported for IMAP accounts via the current provider API."
                .to_string(),
        ))
    }

    async fn rename_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        _folder_id: &FolderId,
        _new_name: &str,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, ProviderError> {
        Err(ProviderError::Client(
            "Renaming folders is not supported for IMAP accounts via the current provider API."
                .to_string(),
        ))
    }

    async fn delete_folder(&self, _ctx: &ProviderCtx<'_>, _folder_id: &FolderId) -> Result<(), ProviderError> {
        Err(ProviderError::Client(
            "Deleting folders is not supported for IMAP accounts via the current provider API."
                .to_string(),
        ))
    }

    async fn test_connection(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderTestResult, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let imap_config = self.load_config(ctx).await?;
        let smtp_config = crate::account_config::load_smtp_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let imap_result = imap_client::test_connection(&imap_config).await?;
        let smtp_result = smtp::client::test_connection(&smtp_config).await?;
        if !smtp_result.success {
            return Ok(ProviderTestResult {
                success: false,
                message: format!("IMAP OK, but SMTP failed: {}", smtp_result.message),
            });
        }

        Ok(ProviderTestResult {
            success: true,
            message: format!("Connected: {imap_result}"),
        })
    }

    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, ProviderError> {
        let account_id = ctx.account_id.to_string();
        ctx.db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT email, display_name FROM accounts WHERE id = ?1",
                    rusqlite::params![account_id],
                    |row| {
                        Ok(ProviderProfile {
                            email: row.get("email")?,
                            name: row.get("display_name")?,
                        })
                    },
                )
                .map_err(|e| format!("Failed to read account profile: {e}"))
            })
            .await
            .map_err(ProviderError::from)
    }
}
