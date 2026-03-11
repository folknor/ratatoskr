use std::collections::HashMap;

use async_trait::async_trait;
use base64::Engine;
use rusqlite::Connection;

use crate::provider::ops::ProviderOps;
use crate::provider::types::{
    AttachmentData, ProviderCtx, ProviderFolderEntry, ProviderFolderMutation,
    ProviderParsedAttachment, ProviderParsedMessage, ProviderProfile, ProviderTestResult,
    SyncResult,
};
use crate::smtp;

use super::client as imap_client;
use super::connection::connect;

/// Map an IMAP folder path + special-use flag to a canonical label ID.
///
/// Mirrors the old TS `mapFolderToLabel` logic:
/// system folders get well-known IDs (INBOX, SENT, …), user folders get `folder-{path}`.
fn canonical_folder_id(path: &str, special_use: Option<&str>) -> String {
    let canonical = match special_use {
        Some("\\Inbox") => Some("INBOX"),
        Some("\\Sent") => Some("SENT"),
        Some("\\Trash") => Some("TRASH"),
        Some("\\Drafts") => Some("DRAFT"),
        Some("\\Junk") => Some("SPAM"),
        Some("\\Archive") | Some("\\All") => Some("ARCHIVE"),
        Some("\\Flagged") => Some("STARRED"),
        _ => None,
    };
    // Also detect by well-known path name as a fallback
    let by_name = match path.to_lowercase().as_str() {
        "inbox" => Some("INBOX"),
        _ => None,
    };
    canonical
        .or(by_name)
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
            let folder: String = row.get(0)?;
            let uid: i64 = row.get(1)?;
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
            "SELECT COALESCE(imap_folder_path, name) FROM labels \
             WHERE account_id = ?1 AND imap_special_use = ?2 LIMIT 1",
            rusqlite::params![account_id, special_use],
            |row| row.get(0),
        )
        .ok();

    if path.is_some() {
        return Ok(path);
    }

    // Fallback: map special-use to well-known label ID
    let label_id = match special_use {
        "\\Trash" => Some("TRASH"),
        "\\Junk" => Some("SPAM"),
        "\\Archive" => Some("ARCHIVE"),
        "\\Sent" => Some("SENT"),
        "\\Drafts" => Some("DRAFT"),
        "\\All" => Some("ALL"),
        _ => None,
    };

    if let Some(lid) = label_id {
        let fallback: Option<String> = conn
            .query_row(
                "SELECT COALESCE(imap_folder_path, name) FROM labels \
                 WHERE account_id = ?1 AND id = ?2 AND imap_folder_path IS NOT NULL LIMIT 1",
                rusqlite::params![account_id, lid],
                |row| row.get(0),
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
    ) -> Result<SyncResult, String> {
        // IMAP sync is handled by the dedicated sync module (sync_imap_initial).
        // This trait method is not the primary entry point for IMAP sync, but we
        // wire it through for consistency with the provider abstraction.
        let account_id = ctx.account_id.to_string();
        let imap_config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let result = crate::sync::imap_initial::imap_initial_sync(
            ctx.app_handle,
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
    ) -> Result<SyncResult, String> {
        let account_id = ctx.account_id.to_string();
        let imap_config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;
        let days_back = days_back.unwrap_or(365);

        let result = crate::sync::imap_delta::imap_delta_sync(
            ctx.app_handle,
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

    async fn archive(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let (refs, archive_folder) = ctx
            .db
            .with_conn(move |conn| {
                let refs = get_thread_message_refs(conn, &account_id, &tid)?;
                let archive = find_special_folder(conn, &account_id, "\\Archive")?
                    .unwrap_or_else(|| "Archive".to_string());
                Ok((refs, archive))
            })
            .await?;

        let grouped = group_by_folder(&refs);
        for (folder, uids) in grouped {
            if folder == archive_folder {
                continue;
            }
            with_session!(&config, session => {
                imap_client::move_messages(&mut session, folder, &uid_set(&uids), &archive_folder).await
            })?;
        }

        Ok(())
    }

    async fn trash(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let (refs, trash_folder) = ctx
            .db
            .with_conn(move |conn| {
                let refs = get_thread_message_refs(conn, &account_id, &tid)?;
                let trash = find_special_folder(conn, &account_id, "\\Trash")?
                    .unwrap_or_else(|| "Trash".to_string());
                Ok((refs, trash))
            })
            .await?;

        let grouped = group_by_folder(&refs);
        for (folder, uids) in grouped {
            if folder == trash_folder {
                continue;
            }
            with_session!(&config, session => {
                imap_client::move_messages(&mut session, folder, &uid_set(&uids), &trash_folder).await
            })?;
        }

        Ok(())
    }

    async fn permanent_delete(&self, ctx: &ProviderCtx<'_>, thread_id: &str) -> Result<(), String> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let refs = ctx
            .db
            .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        let grouped = group_by_folder(&refs);
        for (folder, uids) in grouped {
            with_session!(&config, session => {
                imap_client::delete_messages(&mut session, folder, &uid_set(&uids)).await
            })?;
        }

        Ok(())
    }

    async fn mark_read(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), String> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let refs = ctx
            .db
            .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        let flag_op = if read { "+FLAGS" } else { "-FLAGS" };
        let grouped = group_by_folder(&refs);
        for (folder, uids) in grouped {
            with_session!(&config, session => {
                imap_client::set_flags(&mut session, folder, &uid_set(&uids), flag_op, "(\\Seen)").await
            })?;
        }

        Ok(())
    }

    async fn star(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), String> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let refs = ctx
            .db
            .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        let flag_op = if starred { "+FLAGS" } else { "-FLAGS" };
        let grouped = group_by_folder(&refs);
        for (folder, uids) in grouped {
            with_session!(&config, session => {
                imap_client::set_flags(&mut session, folder, &uid_set(&uids), flag_op, "(\\Flagged)").await
            })?;
        }

        Ok(())
    }

    async fn spam(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), String> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let (refs, junk_folder) = ctx
            .db
            .with_conn(move |conn| {
                let refs = get_thread_message_refs(conn, &account_id, &tid)?;
                let junk = find_special_folder(conn, &account_id, "\\Junk")?
                    .unwrap_or_else(|| "Junk".to_string());
                Ok((refs, junk))
            })
            .await?;

        let destination = if is_spam { &junk_folder } else { "INBOX" };
        let grouped = group_by_folder(&refs);
        for (folder, uids) in grouped {
            if folder == destination {
                continue;
            }
            with_session!(&config, session => {
                imap_client::move_messages(&mut session, folder, &uid_set(&uids), destination).await
            })?;
        }

        Ok(())
    }

    async fn move_to_folder(
        &self,
        ctx: &ProviderCtx<'_>,
        thread_id: &str,
        folder_id: &str,
    ) -> Result<(), String> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let dest = folder_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let refs = ctx
            .db
            .with_conn(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        let grouped = group_by_folder(&refs);
        for (folder, uids) in grouped {
            if folder == dest.as_str() {
                continue;
            }
            with_session!(&config, session => {
                imap_client::move_messages(&mut session, folder, &uid_set(&uids), &dest).await
            })?;
        }

        Ok(())
    }

    async fn add_tag(
        &self,
        _ctx: &ProviderCtx<'_>,
        _thread_id: &str,
        _tag_id: &str,
    ) -> Result<(), String> {
        // IMAP doesn't have native labels/tags.
        // This is a no-op, matching the existing TS ImapSmtpProvider behavior.
        log::debug!("IMAP: add_tag is a no-op (IMAP has no native labels)");
        Ok(())
    }

    async fn remove_tag(
        &self,
        _ctx: &ProviderCtx<'_>,
        _thread_id: &str,
        _tag_id: &str,
    ) -> Result<(), String> {
        // IMAP doesn't have native labels/tags.
        log::debug!("IMAP: remove_tag is a no-op (IMAP has no native labels)");
        Ok(())
    }

    // ── Send + Drafts ───────────────────────────────────────────────────

    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, String> {
        let account_id = ctx.account_id.to_string();
        let configs = crate::imap::account_config::load_both_configs(
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

        // Send via SMTP
        let result = smtp::client::send_raw_email(&smtp_config, raw_base64url).await?;
        if !result.success {
            return Err(format!("SMTP send failed: {}", result.message));
        }

        let message_id = format!(
            "imap-sent-{}-{}",
            chrono::Utc::now().timestamp_millis(),
            random_hex8()
        );

        // Copy sent message to Sent folder (non-fatal if it fails)
        let raw_b64url = raw_base64url.to_string();
        if let Err(e) = async {
            let raw_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
                .decode(&raw_b64url)
                .map_err(|e| format!("base64url decode: {e}"))?;

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
    ) -> Result<String, String> {
        let account_id = ctx.account_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let drafts_folder = ctx
            .db
            .with_conn(move |conn| find_special_folder(conn, &account_id, "\\Drafts"))
            .await?
            .unwrap_or_else(|| "Drafts".to_string());

        let raw_bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(raw_base64url)
            .map_err(|e| format!("base64url decode: {e}"))?;

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
    ) -> Result<String, String> {
        // Delete old draft, then create a new one
        if let Err(e) = self.delete_draft(ctx, draft_id).await {
            log::warn!("Failed to delete old draft {draft_id} during update: {e}");
        }
        self.create_draft(ctx, raw_base64url, thread_id).await
    }

    async fn delete_draft(&self, ctx: &ProviderCtx<'_>, draft_id: &str) -> Result<(), String> {
        // Parse draft ID: IMAP message IDs are "imap-{accountId}-{folder}-{uid}"
        let prefix = format!("imap-{}-", ctx.account_id);
        if !draft_id.starts_with(&prefix) {
            // Generated draft IDs (imap-draft-...) can't be mapped to a server UID
            log::debug!("Draft {draft_id} has a generated ID, cannot delete from server");
            return Ok(());
        }

        let remainder = &draft_id[prefix.len()..];
        let last_dash = remainder
            .rfind('-')
            .ok_or_else(|| format!("Invalid draft ID format: {draft_id}"))?;
        let folder = &remainder[..last_dash];
        let uid: u32 = remainder[last_dash + 1..]
            .parse()
            .map_err(|_| format!("Invalid UID in draft ID: {draft_id}"))?;

        let account_id = ctx.account_id.to_string();
        let folder_owned = folder.to_string();

        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        with_session!(&config, session => {
            imap_client::delete_messages(&mut session, &folder_owned, &uid.to_string()).await
        })
    }

    // ── Attachments ─────────────────────────────────────────────────────

    async fn fetch_attachment(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
        attachment_id: &str,
    ) -> Result<AttachmentData, String> {
        let prefix = format!("imap-{}-", ctx.account_id);
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

        let account_id = ctx.account_id.to_string();
        let folder_owned = folder.to_string();
        let part_id = attachment_id.to_string();

        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let data = with_session!(&config, session => {
            imap_client::fetch_attachment(&mut session, &folder_owned, uid, &part_id).await
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
    ) -> Result<ProviderParsedMessage, String> {
        let prefix = format!("imap-{}-", ctx.account_id);
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

        let account_id = ctx.account_id.to_string();
        let folder_owned = folder.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        let message = with_session!(&config, session => {
            imap_client::fetch_message_body(&mut session, &folder_owned, uid).await
        })?;

        let mut parsed = imap_message_to_provider_message(&account_id, &folder_owned, &message);

        // Look up the thread_id stored during sync; empty string if message isn't indexed yet.
        let msg_id = message_id.to_string();
        if let Ok(thread_id) = ctx
            .db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT thread_id FROM messages WHERE id = ?1",
                    rusqlite::params![msg_id],
                    |row| row.get::<_, String>(0),
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
    ) -> Result<String, String> {
        let prefix = format!("imap-{}-", ctx.account_id);
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

        let account_id = ctx.account_id.to_string();
        let folder_owned = folder.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

        with_session!(&config, session => {
            imap_client::fetch_raw_message(&mut session, &folder_owned, uid).await
        })
    }

    // ── Folders ─────────────────────────────────────────────────────────

    async fn list_folders(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, String> {
        let account_id = ctx.account_id.to_string();
        let config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;

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
    ) -> Result<ProviderFolderMutation, String> {
        Err(
            "Creating folders is not supported for IMAP accounts via the current provider API."
                .to_string(),
        )
    }

    async fn rename_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        _folder_id: &str,
        _new_name: &str,
        _text_color: Option<&str>,
        _bg_color: Option<&str>,
    ) -> Result<ProviderFolderMutation, String> {
        Err(
            "Renaming folders is not supported for IMAP accounts via the current provider API."
                .to_string(),
        )
    }

    async fn delete_folder(&self, _ctx: &ProviderCtx<'_>, _folder_id: &str) -> Result<(), String> {
        Err(
            "Deleting folders is not supported for IMAP accounts via the current provider API."
                .to_string(),
        )
    }

    async fn test_connection(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderTestResult, String> {
        let account_id = ctx.account_id.to_string();
        let imap_config = crate::imap::account_config::load_imap_config(
            ctx.db,
            &account_id,
            &self.encryption_key,
        )
        .await?;
        let smtp_config = crate::imap::account_config::load_smtp_config(
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

    async fn get_profile(&self, ctx: &ProviderCtx<'_>) -> Result<ProviderProfile, String> {
        let account_id = ctx.account_id.to_string();
        ctx.db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT email, display_name FROM accounts WHERE id = ?1",
                    rusqlite::params![account_id],
                    |row| {
                        Ok(ProviderProfile {
                            email: row.get(0)?,
                            name: row.get(1)?,
                        })
                    },
                )
                .map_err(|e| format!("Failed to read account profile: {e}"))
            })
            .await
    }
}
