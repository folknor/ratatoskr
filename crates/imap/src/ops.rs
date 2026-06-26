use std::collections::HashMap;

use async_trait::async_trait;
use db::db::{ReadConn, ReadError, WriterPool};

use common::error::ProviderError;
use common::folder_roles::{imap_name_to_special_use, imap_special_use_to_label_id};
use common::ops::ProviderOps;
use common::typed_ids::FolderId;
use common::types::{
    ActionProviderCtx, FetchedAttachment, FolderKind, LabelKind, MailProviderKind, ProviderCtx,
    ProviderFolderEntry, ProviderFolderMutation, ProviderParsedAttachment, ProviderParsedMessage,
    ProviderProfile, ProviderTestResult, SendIntent,
};
use smtp;

use super::client as imap_client;
use super::connection::connect;

/// Map an IMAP folder path + special-use flag to a canonical label ID.
///
/// Mirrors the old TS `mapFolderToLabel` logic:
/// system folders get well-known IDs (INBOX, SENT, …), user folders get `folder-{path}`.
fn canonical_folder_id(path: &str, special_use: Option<&str>) -> Result<String, String> {
    let lower = path.to_lowercase();
    if let Some(id) = imap_special_use_to_label_id(special_use.unwrap_or_default())
        .or_else(|| imap_name_to_special_use(&lower).and_then(imap_special_use_to_label_id))
    {
        return Ok(id.to_string());
    }
    Ok(FolderKind::imap_user(path)?.storage_id())
}

/// Generate a short random hex string for pseudo-IDs.
fn random_hex8() -> String {
    let mut buf = [0u8; 4];
    if getrandom::fill(&mut buf).is_err() {
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
    pub encryption_key: [u8; 32],
    writer: WriterPool,
}

impl ImapOps {
    pub fn new(encryption_key: [u8; 32], writer: WriterPool) -> Self {
        Self {
            encryption_key,
            writer,
        }
    }

    /// Shorthand for loading the IMAP config from the database.
    pub async fn load_config(
        &self,
        db: &db::db::ReadDbState,
        account_id: &str,
    ) -> Result<super::types::ImapConfig, String> {
        crate::account_config::load_imap_config(db, &self.writer, account_id, &self.encryption_key)
            .await
    }

    async fn load_smtp_config(
        &self,
        db: &db::db::ReadDbState,
        account_id: &str,
    ) -> Result<smtp::types::SmtpConfig, String> {
        crate::account_config::load_smtp_config(db, &self.writer, account_id, &self.encryption_key)
            .await
    }

    async fn load_both_configs(
        &self,
        db: &db::db::ReadDbState,
        account_id: &str,
    ) -> Result<crate::account_config::ImapAndSmtpConfig, String> {
        crate::account_config::load_both_configs(db, &self.writer, account_id, &self.encryption_key)
            .await
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse an IMAP message ID (`imap-{accountId}-{folder}-{uid}`) into folder + UID.
pub fn parse_imap_message_id(message_id: &str, account_id: &str) -> Result<(String, u32), String> {
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

async fn source_imap_location(
    ctx: &ProviderCtx<'_>,
    source_message_id: &str,
) -> Result<Option<(String, u32)>, ProviderError> {
    let account_id = ctx.account_id.to_string();
    let message_id = source_message_id.to_string();
    ctx.db
        .with_read(move |conn| {
            conn.query_row(
                "SELECT imap_folder, imap_uid FROM messages \
                 WHERE account_id = ?1 AND id = ?2 AND imap_folder IS NOT NULL AND imap_uid IS NOT NULL",
                rusqlite::params![account_id, message_id],
                |row| {
                    let folder: String = row.get(0)?;
                    let uid: i64 = row.get(1)?;
                    Ok((folder, u32::try_from(uid).unwrap_or(0)))
                },
            )
            .map(Some)
            .or_else(|e| match e {
                ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                other => Err(format!("lookup source IMAP message: {other}")),
            })
        })
        .await
        .map_err(ProviderError::Db)
}

/// Minimal info needed to locate a message on the IMAP server.
#[derive(Clone)]
struct ImapMessageRef {
    message_id: String,
    folder: String,
    uid: u32,
}

/// Local IMAP reference update after a successful server-side move.
struct MovedMessageRef {
    message_id: String,
    folder: String,
    uid: Option<u32>,
}

/// Query messages for a thread and extract IMAP folder + UID pairs.
fn get_thread_message_refs(
    conn: &ReadConn<'_>,
    account_id: &str,
    thread_id: &str,
) -> Result<Vec<ImapMessageRef>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT id, imap_folder, imap_uid FROM messages \
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
                message_id: row.get("id")?,
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

async fn update_message_refs_after_move(
    writer: &WriterPool,
    account_id: String,
    refs: Vec<MovedMessageRef>,
) -> Result<(), String> {
    if refs.is_empty() {
        return Ok(());
    }
    writer
        .with_write(move |conn| {
            for moved in refs {
                if let Some(uid) = moved.uid {
                    conn.execute(
                        "UPDATE messages SET imap_folder = ?1, imap_uid = ?2 \
                         WHERE account_id = ?3 AND id = ?4",
                        rusqlite::params![
                            moved.folder,
                            i64::from(uid),
                            account_id,
                            moved.message_id
                        ],
                    )
                    .map_err(|e| format!("update moved IMAP ref: {e}"))?;
                } else {
                    conn.execute(
                        "UPDATE messages SET imap_folder = ?1 \
                         WHERE account_id = ?2 AND id = ?3",
                        rusqlite::params![moved.folder, account_id, moved.message_id],
                    )
                    .map_err(|e| format!("update moved IMAP folder: {e}"))?;
                }
            }
            Ok(())
        })
        .await
}

/// Group message refs by folder → list of UIDs.
fn group_by_folder(refs: &[ImapMessageRef]) -> HashMap<&str, Vec<u32>> {
    let mut map: HashMap<&str, Vec<u32>> = HashMap::new();
    for r in refs {
        map.entry(&r.folder).or_default().push(r.uid);
    }
    map
}

fn group_refs_by_folder(refs: &[ImapMessageRef]) -> HashMap<String, Vec<ImapMessageRef>> {
    let mut map: HashMap<String, Vec<ImapMessageRef>> = HashMap::new();
    for r in refs {
        map.entry(r.folder.clone()).or_default().push(r.clone());
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
    folder_id: &str,
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
    let mut label_ids = vec![folder_id.to_string()];
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

/// Look up a special-use IMAP folder path from the folders table.
///
/// First checks `imap_special_use`, then falls back to well-known folder IDs
/// (e.g., "TRASH", "SPAM") when the server didn't advertise special-use flags.
fn find_special_folder(
    conn: &ReadConn<'_>,
    account_id: &str,
    special_use: &str,
) -> Result<Option<String>, String> {
    // Primary: look up by imap_special_use
    let path: Option<String> = conn
        .query_row(
            "SELECT COALESCE(imap_folder_path, name) AS folder_path FROM folders \
             WHERE account_id = ?1 AND imap_special_use = ?2 LIMIT 1",
            rusqlite::params![account_id, special_use],
            |row| row.get("folder_path"),
        )
        .map(Some)
        .or_else(|e| match e {
            ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            other => Err(format!("find IMAP special folder {special_use}: {other}")),
        })?;

    if path.is_some() {
        return Ok(path);
    }

    // Fallback: map special-use to well-known folder ID.
    let folder_id = imap_special_use_to_label_id(special_use);

    if let Some(lid) = folder_id {
        let fallback: Option<String> = conn
            .query_row(
                "SELECT COALESCE(imap_folder_path, name) AS folder_path FROM folders \
                 WHERE account_id = ?1 AND id = ?2 AND imap_folder_path IS NOT NULL LIMIT 1",
                rusqlite::params![account_id, lid],
                |row| row.get("folder_path"),
            )
            .map(Some)
            .or_else(|e| match e {
                ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                other => Err(format!("find IMAP fallback folder {lid}: {other}")),
            })?;

        if fallback.is_some() {
            return Ok(fallback);
        }
    }

    Ok(None)
}

fn resolve_folder_path(
    conn: &ReadConn<'_>,
    account_id: &str,
    folder_id: &str,
) -> Result<String, String> {
    let path: Option<String> = conn
        .query_row(
            "SELECT COALESCE(imap_folder_path, name) AS folder_path FROM folders \
             WHERE account_id = ?1 AND id = ?2 LIMIT 1",
            rusqlite::params![account_id, folder_id],
            |row| row.get("folder_path"),
        )
        .map(Some)
        .or_else(|e| match e {
            ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            other => Err(format!("resolve IMAP folder path {folder_id}: {other}")),
        })?;
    if let Some(path) = path {
        return Ok(path);
    }
    if let Ok(FolderKind::ImapUser(path)) = FolderKind::parse(folder_id, MailProviderKind::Imap) {
        return Ok(path.as_path().to_string());
    }

    Err(format!(
        "No IMAP folder path found for folder id {folder_id:?}"
    ))
}

/// Connect, run an IMAP session body, then logout - mirroring the
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
    ctx: &ActionProviderCtx<'_>,
    thread_id: &str,
    keyword: &str,
    flag_op: &str,
) -> Result<(), ProviderError> {
    let account_id = ctx.account_id.to_string();
    let tid = thread_id.to_string();

    let refs = ctx
        .db
        .with_read(move |conn| get_thread_message_refs(conn, &account_id, &tid))
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
) -> Result<Vec<MovedMessageRef>, String> {
    match action {
        FolderAction::Move(dest) => {
            let grouped = group_refs_by_folder(refs);
            let futs: Vec<_> = grouped
                .into_iter()
                .filter(|(folder, _)| folder != dest)
                .map(|(folder, refs)| {
                    let config = config.clone();
                    let dest = dest.clone();
                    async move {
                        let uids = uid_set(&refs.iter().map(|r| r.uid).collect::<Vec<_>>());
                        let copyuid = with_session!(&config, session => {
                            imap_client::move_messages(&mut session, &folder, &uids, &dest).await
                        })?;
                        let uid_map: HashMap<u32, u32> = copyuid.into_iter().collect();
                        let missing = refs
                            .iter()
                            .filter(|r| !uid_map.contains_key(&r.uid))
                            .count();
                        if missing > 0 {
                            log::warn!(
                                "IMAP: MOVE/COPY from {folder} to {dest} did not return COPYUID for {missing} message(s); local UID refs stay provisional"
                            );
                        }
                        Ok::<Vec<MovedMessageRef>, String>(
                            refs.into_iter()
                                .map(|r| MovedMessageRef {
                                    uid: uid_map.get(&r.uid).copied(),
                                    message_id: r.message_id,
                                    folder: dest.clone(),
                                })
                                .collect(),
                        )
                    }
                })
                .collect();
            let moved = futures::future::try_join_all(futs).await?;
            Ok(moved.into_iter().flatten().collect())
        }
        FolderAction::Delete => {
            let grouped = group_by_folder(refs);
            let futs: Vec<_> = grouped
                .iter()
                .map(|(folder, uids)| {
                    let config = config.clone();
                    let folder = folder.to_string();
                    let uids = uid_set(uids);
                    async move {
                        with_session!(&config, session => {
                            imap_client::delete_messages(&mut session, &folder, &uids).await
                        })
                    }
                })
                .collect();
            futures::future::try_join_all(futs).await?;
            Ok(Vec::new())
        }
    }
}

// ---------------------------------------------------------------------------
// ProviderOps implementation
// ---------------------------------------------------------------------------

#[async_trait]
impl ProviderOps for ImapOps {
    // ── Actions ─────────────────────────────────────────────────────────

    async fn archive(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;
        let query_account_id = account_id.clone();

        let (refs, archive_folder) = ctx
            .db
            .with_read(move |conn| {
                let refs = get_thread_message_refs(conn, &query_account_id, &tid)?;
                let archive = find_special_folder(conn, &query_account_id, "\\Archive")?
                    .unwrap_or_else(|| "Archive".to_string());
                Ok((refs, archive))
            })
            .await?;

        let moved =
            execute_folder_action(&config, &refs, &FolderAction::Move(archive_folder)).await?;
        update_message_refs_after_move(&self.writer, account_id, moved).await?;
        Ok(())
    }

    async fn trash(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;
        let query_account_id = account_id.clone();

        let (refs, trash_folder) = ctx
            .db
            .with_read(move |conn| {
                let refs = get_thread_message_refs(conn, &query_account_id, &tid)?;
                let trash = find_special_folder(conn, &query_account_id, "\\Trash")?
                    .unwrap_or_else(|| "Trash".to_string());
                Ok((refs, trash))
            })
            .await?;

        let moved =
            execute_folder_action(&config, &refs, &FolderAction::Move(trash_folder)).await?;
        update_message_refs_after_move(&self.writer, account_id, moved).await?;
        Ok(())
    }

    async fn permanent_delete(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        let refs = ctx
            .db
            .with_read(move |conn| get_thread_message_refs(conn, &account_id, &tid))
            .await?;

        execute_folder_action(&config, &refs, &FolderAction::Delete).await?;
        Ok(())
    }

    async fn mark_read(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        read: bool,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        let refs = ctx
            .db
            .with_read(move |conn| get_thread_message_refs(conn, &account_id, &tid))
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

    async fn mark_mdn_sent(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let mid = message_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        let lookup = ctx
            .db
            .with_read(move |conn| {
                conn.query_row(
                    "SELECT imap_folder, imap_uid FROM messages \
                     WHERE account_id = ?1 AND id = ?2 \
                       AND imap_folder IS NOT NULL AND imap_uid IS NOT NULL",
                    rusqlite::params![account_id, mid],
                    |row| {
                        let folder: String = row.get(0)?;
                        let uid: i64 = row.get(1)?;
                        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                        Ok((folder, uid as u32))
                    },
                )
                .map(Some)
                .or_else(|e| match e {
                    ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    other => Err(format!("lookup imap ref for mdn keyword: {other}")),
                })
            })
            .await?;

        let Some((folder, uid)) = lookup else {
            // No IMAP ref - nothing to keyword. Local mdn_sent already set.
            return Ok(());
        };

        with_session!(&config, session => {
            imap_client::set_keyword_if_supported(
                &mut session, &folder, uid, "+FLAGS", "$MDNSent",
            ).await
        })?;
        Ok(())
    }

    async fn star(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        starred: bool,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        let refs = ctx
            .db
            .with_read(move |conn| get_thread_message_refs(conn, &account_id, &tid))
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
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        is_spam: bool,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;
        let query_account_id = account_id.clone();

        let (refs, junk_folder) = ctx
            .db
            .with_read(move |conn| {
                let refs = get_thread_message_refs(conn, &query_account_id, &tid)?;
                let junk = find_special_folder(conn, &query_account_id, "\\Junk")?
                    .unwrap_or_else(|| "Junk".to_string());
                Ok((refs, junk))
            })
            .await?;

        let destination = if is_spam {
            junk_folder
        } else {
            "INBOX".to_string()
        };

        let moved = execute_folder_action(&config, &refs, &FolderAction::Move(destination)).await?;
        update_message_refs_after_move(&self.writer, account_id, moved).await?;
        Ok(())
    }

    async fn move_to_folder(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        folder_id: &FolderId,
    ) -> Result<(), ProviderError> {
        let account_id = ctx.account_id.to_string();
        let tid = thread_id.to_string();
        let folder_id = folder_id.as_str().to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;
        let query_account_id = account_id.clone();

        let (refs, dest) = ctx
            .db
            .with_read(move |conn| {
                let refs = get_thread_message_refs(conn, &query_account_id, &tid)?;
                let dest = resolve_folder_path(conn, &query_account_id, &folder_id)?;
                Ok((refs, dest))
            })
            .await?;

        let moved = execute_folder_action(&config, &refs, &FolderAction::Move(dest)).await?;
        update_message_refs_after_move(&self.writer, account_id, moved).await?;
        Ok(())
    }

    async fn add_label(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        label: &LabelKind,
    ) -> Result<(), ProviderError> {
        let LabelKind::ImapKeyword(keyword) = label else {
            return Err(ProviderError::Client(format!(
                "IMAP add_label received non-IMAP keyword label kind: {label:?}"
            )));
        };
        let config = self.load_config(ctx.db, ctx.account_id).await?;
        set_keyword_batched(&config, ctx, thread_id, keyword.as_str(), "+FLAGS").await
    }

    async fn remove_label(
        &self,
        ctx: &ActionProviderCtx<'_>,
        thread_id: &str,
        label: &LabelKind,
    ) -> Result<(), ProviderError> {
        let LabelKind::ImapKeyword(keyword) = label else {
            return Err(ProviderError::Client(format!(
                "IMAP remove_label received non-IMAP keyword label kind: {label:?}"
            )));
        };
        let config = self.load_config(ctx.db, ctx.account_id).await?;
        set_keyword_batched(&config, ctx, thread_id, keyword.as_str(), "-FLAGS").await
    }

    // ── Send + Drafts ───────────────────────────────────────────────────

    async fn send_email(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let configs = self.load_both_configs(ctx.db, &account_id).await?;
        let smtp_config = configs.smtp;
        let imap_config = configs.imap;

        let sent_folder = ctx
            .db
            .with_read(move |conn| find_special_folder(conn, &account_id, "\\Sent"))
            .await?
            .unwrap_or_else(|| "Sent".to_string());

        // Inject read-receipt header and send via SMTP
        log::info!(
            "[IMAP] Sending email via SMTP for account {}",
            ctx.account_id
        );
        let patched = common::headers::inject_read_receipt_header_base64url(raw_base64url)?;
        let result = smtp::client::send_raw_email(&smtp_config, &patched).await?;
        if !result.success {
            log::error!(
                "[IMAP] SMTP send failed for account {}: {}",
                ctx.account_id,
                result.message
            );
            return Err(ProviderError::Server(format!(
                "SMTP send failed: {}",
                result.message
            )));
        }
        log::info!(
            "[IMAP] Email sent successfully via SMTP for account {}",
            ctx.account_id
        );

        let message_id = format!(
            "imap-sent-{}-{}",
            chrono::Utc::now().timestamp_millis(),
            random_hex8()
        );

        // Copy sent message to Sent folder (non-fatal if it fails)
        let raw_b64url = patched;
        if let Err(e) = async {
            let raw_bytes = common::encoding::decode_base64url_nopad(&raw_b64url)?;

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

    async fn mark_send_intent(
        &self,
        ctx: &ProviderCtx<'_>,
        source_message_id: Option<&str>,
        intent: SendIntent,
    ) -> Result<(), ProviderError> {
        let Some(source_message_id) = source_message_id else {
            return Ok(());
        };
        let Some((folder, uid)) = source_imap_location(ctx, source_message_id).await? else {
            return Ok(());
        };

        let account_id = ctx.account_id.to_string();
        let config = self.load_config(ctx.db, &account_id).await?;

        match intent {
            SendIntent::New => Ok(()),
            SendIntent::Reply => {
                with_session!(&config, session => {
                    imap_client::set_flags(&mut session, &folder, &uid.to_string(), "+FLAGS", "(\\Answered)").await
                })
                .map_err(ProviderError::Server)
            }
            SendIntent::Forward => {
                with_session!(&config, session => {
                    imap_client::set_keyword_if_supported(&mut session, &folder, uid, "+FLAGS", "$Forwarded").await
                })
                .map_err(ProviderError::Server)
            }
        }
    }

    async fn create_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        raw_base64url: &str,
        _thread_id: Option<&str>,
    ) -> Result<String, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        let drafts_folder = ctx
            .db
            .with_read(move |conn| find_special_folder(conn, &account_id, "\\Drafts"))
            .await?
            .unwrap_or_else(|| "Drafts".to_string());

        let raw_bytes = common::encoding::decode_base64url_nopad(raw_base64url)?;

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

    async fn delete_draft(
        &self,
        ctx: &ProviderCtx<'_>,
        draft_id: &str,
    ) -> Result<(), ProviderError> {
        // Generated draft IDs (imap-draft-...) can't be mapped to a server UID
        let prefix = format!("imap-{}-", ctx.account_id);
        if !draft_id.starts_with(&prefix) {
            log::debug!("Draft {draft_id} has a generated ID, cannot delete from server");
            return Ok(());
        }

        let (folder, uid) = parse_imap_message_id(draft_id, ctx.account_id)?;
        let config = self.load_config(ctx.db, ctx.account_id).await?;

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
    ) -> Result<FetchedAttachment, ProviderError> {
        let (folder, uid) = parse_imap_message_id(message_id, ctx.account_id)?;
        let part_id = attachment_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        let data = with_session!(&config, session => {
            imap_client::fetch_attachment(&mut session, &folder, uid, &part_id).await
        })?;

        let size = data.len() as u64;
        Ok(FetchedAttachment { bytes: data, size })
    }

    async fn fetch_message(
        &self,
        ctx: &ProviderCtx<'_>,
        message_id: &str,
    ) -> Result<ProviderParsedMessage, ProviderError> {
        let (folder, uid) = parse_imap_message_id(message_id, ctx.account_id)?;
        let account_id = ctx.account_id.to_string();
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        let message = with_session!(&config, session => {
            imap_client::fetch_message_body(&mut session, &folder, uid).await
        })?;

        let mut parsed = imap_message_to_provider_message(&account_id, &folder, &message);

        // Look up the thread_id stored during sync; empty string if message isn't indexed yet.
        let msg_id = message_id.to_string();
        if let Ok(thread_id) = ctx
            .db
            .with_read(move |conn| {
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
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        Ok(with_session!(&config, session => {
            imap_client::fetch_raw_message(&mut session, &folder, uid).await
        })?)
    }

    // ── Folders ─────────────────────────────────────────────────────────

    async fn list_folders(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<Vec<ProviderFolderEntry>, ProviderError> {
        let config = self.load_config(ctx.db, ctx.account_id).await?;

        let folders = with_session!(&config, session => {
            imap_client::list_folders(&mut session).await
        })?;

        folders
            .into_iter()
            .map(|f| {
                let id = canonical_folder_id(&f.path, f.special_use.as_deref())
                    .map_err(ProviderError::Client)?;
                let special_use = f.special_use;
                Ok(ProviderFolderEntry {
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
                })
            })
            .collect()
    }

    async fn create_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        _name: &str,
        _parent_id: Option<&FolderId>,
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

    async fn delete_folder(
        &self,
        _ctx: &ProviderCtx<'_>,
        _folder_id: &FolderId,
    ) -> Result<(), ProviderError> {
        Err(ProviderError::Client(
            "Deleting folders is not supported for IMAP accounts via the current provider API."
                .to_string(),
        ))
    }

    async fn test_connection(
        &self,
        ctx: &ProviderCtx<'_>,
    ) -> Result<ProviderTestResult, ProviderError> {
        let account_id = ctx.account_id.to_string();
        let imap_config = self.load_config(ctx.db, ctx.account_id).await?;
        let smtp_config = self.load_smtp_config(ctx.db, &account_id).await?;

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
            .with_read(move |conn| {
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
