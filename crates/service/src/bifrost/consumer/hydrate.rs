use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use bifrost_sync::SyncEngine;
use bifrost_types::{
    AccountId, BlobHandle, Change, HydrationProjection, Importance, Message, ObjectChangeKind,
    ObjectId, SyncEvent,
};
use common::types::{FolderKind, LabelKind, MailProviderKind};
use db::db::queries_extra::{AttachmentInsertRow, MessageInsertRow};
use futures::StreamExt;
use mail_parser::MessageParser;
use provider_sync::consumer_support::{Rfc822Parsed, parse_rfc822, snippet_from_body};
use search::SearchDocument;
use serde::{Deserialize, Serialize};
use store::inline_image_store::InlineImage;

use super::BifrostProviderKind;

const SYNTHETIC_OBJECT_PREFIX: &str = "rtsk-synth:";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydrationOutcome {
    Succeeded,
    SucceededWithDegradedBody,
    /// A definitive hydration failure with no recoverable metadata
    /// (B3-spec 4.1.3). Skipped, recorded, does NOT block the ack.
    Failed,
    /// An object the provider reports as destroyed. This is a deletion,
    /// NOT a hydration failure, so it must not be lumped into `Failed`.
    /// The deletion write path is a B3a gap; for now the change is
    /// skipped without a message-row write and without blocking the ack.
    Deleted,
    Uncertain,
}

#[derive(Debug, Default, Clone)]
pub struct HydrateBatch {
    pub rows: Vec<ConsumerMessageRow>,
    pub failed: usize,
    pub deleted_ids: Vec<String>,
    pub uncertain: usize,
    pub blocked: bool,
}

#[derive(Debug, Clone)]
pub struct ConsumerMessageRow {
    pub message: MessageInsertRow,
    pub folders: Vec<FolderKind>,
    pub labels: Vec<LabelKind>,
    pub keywords: Vec<String>,
    pub attachments: Vec<AttachmentInsertRow>,
    pub body_html: Option<String>,
    pub body_text: Option<String>,
    pub inline_images: Vec<InlineImage>,
    pub search_document: SearchDocument,
    /// Whether this message carries the provider's "important" signal.
    /// JMAP folds the `$important` keyword into `Message::importance`
    /// (`High`), so the consumer reads importance from there rather than
    /// from the user-visible keyword set - `$important` is `$`-prefixed and
    /// is stripped by `is_user_visible_keyword`, so it never survives into
    /// `keywords`. The thread aggregate (`write.rs`) ORs this across the
    /// thread's messages to set `threads.is_important`, matching the legacy
    /// JMAP path that derived it from the IMPORTANT label.
    pub is_important: bool,
}

impl HydrateBatch {
    pub async fn from_changes(
        engine: &SyncEngine,
        account_id: &AccountId,
        provider: BifrostProviderKind,
        jmap_folder_map: &HashMap<String, FolderKind>,
        changes: &[Change],
    ) -> Result<Self, bifrost_sync::Error> {
        let mut batch = Self::default();
        for change in changes {
            match hydrate_change_to_message_insert_row(
                engine,
                account_id,
                provider,
                jmap_folder_map,
                change,
            )
            .await?
            {
                HydratedChange::Message(row, outcome) => match outcome {
                    HydrationOutcome::Succeeded | HydrationOutcome::SucceededWithDegradedBody => {
                        batch.rows.push(*row);
                    }
                    HydrationOutcome::Failed => {
                        batch.failed = batch.failed.saturating_add(1);
                    }
                    HydrationOutcome::Deleted => {
                        if let Change::ObjectChange(object) = change {
                            batch.deleted_ids.push(object.id.0.clone());
                        }
                    }
                    HydrationOutcome::Uncertain => {
                        batch.uncertain = batch.uncertain.saturating_add(1);
                        batch.blocked = true;
                    }
                },
                HydratedChange::ScopeOnly => {}
            }
        }
        Ok(batch)
    }

    #[cfg(test)]
    pub fn from_changes_offline(
        account_id: &str,
        provider: BifrostProviderKind,
        changes: &[Change],
    ) -> Self {
        let mut batch = Self::default();
        for change in changes {
            match hydrate_change_to_message_insert_row_offline(account_id, provider, change) {
                HydratedChange::Message(row, outcome) => match outcome {
                    HydrationOutcome::Succeeded | HydrationOutcome::SucceededWithDegradedBody => {
                        batch.rows.push(*row);
                    }
                    HydrationOutcome::Failed => {
                        batch.failed = batch.failed.saturating_add(1);
                    }
                    HydrationOutcome::Deleted => {
                        if let Change::ObjectChange(object) = change {
                            batch.deleted_ids.push(object.id.0.clone());
                        }
                    }
                    HydrationOutcome::Uncertain => {
                        batch.uncertain = batch.uncertain.saturating_add(1);
                        batch.blocked = true;
                    }
                },
                HydratedChange::ScopeOnly => {}
            }
        }
        batch
    }
}

#[derive(Debug, Clone)]
pub enum HydratedChange {
    Message(Box<ConsumerMessageRow>, HydrationOutcome),
    ScopeOnly,
}

pub async fn hydrate_change_to_message_insert_row(
    engine: &SyncEngine,
    account_id: &AccountId,
    provider: BifrostProviderKind,
    jmap_folder_map: &HashMap<String, FolderKind>,
    change: &Change,
) -> Result<HydratedChange, bifrost_sync::Error> {
    match change {
        Change::ObjectChange(object) => {
            let outcome = match object.kind {
                ObjectChangeKind::Created | ObjectChangeKind::Updated => {
                    HydrationOutcome::Succeeded
                }
                ObjectChangeKind::Destroyed => HydrationOutcome::Deleted,
                // ObjectChangeKind is #[non_exhaustive]: an unknown kind is
                // an ambiguous outcome, so block the ack and let the engine
                // re-deliver rather than guess.
                _ => HydrationOutcome::Uncertain,
            };
            if let Some(synthetic) = decode_synthetic_message(&object.id.0) {
                return Ok(synthetic_to_row(
                    &account_id.0,
                    provider,
                    synthetic,
                    outcome,
                ));
            }
            if matches!(
                outcome,
                HydrationOutcome::Deleted | HydrationOutcome::Uncertain
            ) {
                return Ok(HydratedChange::Message(
                    Box::new(ConsumerMessageRow {
                        message: minimal_message_row(&account_id.0, &object.id.0),
                        folders: Vec::new(),
                        labels: Vec::new(),
                        keywords: Vec::new(),
                        attachments: Vec::new(),
                        body_html: None,
                        body_text: None,
                        inline_images: Vec::new(),
                        search_document: minimal_search_document(&account_id.0, &object.id.0),
                        is_important: false,
                    }),
                    outcome,
                ));
            }
            let message = engine
                .message_hydrate(
                    account_id,
                    ObjectId(object.id.0.clone()),
                    HydrationProjection::Full,
                )
                .await?;
            Ok(HydratedChange::Message(
                Box::new(
                    message_to_consumer_row(
                        engine,
                        account_id,
                        provider,
                        jmap_folder_map,
                        message,
                        outcome,
                    )
                    .await?,
                ),
                outcome,
            ))
        }
        Change::ScopeChange(_) => Ok(HydratedChange::ScopeOnly),
        _ => Ok(HydratedChange::ScopeOnly),
    }
}

#[cfg(test)]
pub fn hydrate_change_to_message_insert_row_offline(
    account_id: &str,
    provider: BifrostProviderKind,
    change: &Change,
) -> HydratedChange {
    match change {
        Change::ObjectChange(object) => {
            let outcome = match object.kind {
                ObjectChangeKind::Created | ObjectChangeKind::Updated => {
                    HydrationOutcome::Succeeded
                }
                ObjectChangeKind::Destroyed => HydrationOutcome::Deleted,
                _ => HydrationOutcome::Uncertain,
            };
            if let Some(synthetic) = decode_synthetic_message(&object.id.0) {
                return synthetic_to_row(account_id, provider, synthetic, outcome);
            }
            HydratedChange::Message(
                Box::new(ConsumerMessageRow {
                    message: minimal_message_row(account_id, &object.id.0),
                    folders: Vec::new(),
                    labels: Vec::new(),
                    keywords: Vec::new(),
                    attachments: Vec::new(),
                    body_html: None,
                    body_text: None,
                    inline_images: Vec::new(),
                    search_document: minimal_search_document(account_id, &object.id.0),
                    is_important: false,
                }),
                outcome,
            )
        }
        Change::ScopeChange(_) => HydratedChange::ScopeOnly,
        _ => HydratedChange::ScopeOnly,
    }
}

async fn message_to_consumer_row(
    engine: &SyncEngine,
    account_id: &AccountId,
    provider: BifrostProviderKind,
    jmap_folder_map: &HashMap<String, FolderKind>,
    message: Message,
    outcome: HydrationOutcome,
) -> Result<ConsumerMessageRow, bifrost_sync::Error> {
    let degraded = matches!(outcome, HydrationOutcome::SucceededWithDegradedBody);
    // B3a-cut-jmap 4.2: stream the verbatim RFC822 (engine I/O) and download
    // inline-image blobs (engine I/O), then hand the bytes to the pure
    // builder. Splitting the I/O from the merge lets the byte-identical
    // golden test drive the exact same merge with fixture bytes and no live
    // engine, so the production path and the gate cannot drift.
    let raw = if degraded {
        None
    } else {
        fetch_raw_rfc822(engine, account_id, &message.id).await?
    };
    let inline_hashes = hydrate_inline_images(engine, account_id, &message.attachments).await?;
    Ok(build_consumer_row(
        account_id,
        provider,
        jmap_folder_map,
        &message,
        raw.as_deref(),
        inline_hashes,
        outcome,
    ))
}

/// Pure merge of a structured bifrost `Message`, the verbatim RFC822 octets,
/// and the downloaded inline-image blobs into a `ConsumerMessageRow`. No
/// engine, no network - the production async path and the byte-identical
/// golden test both call this so their output cannot diverge (B3a-cut-jmap
/// 4.0 / 4.2).
pub(crate) fn build_consumer_row(
    account_id: &AccountId,
    provider: BifrostProviderKind,
    jmap_folder_map: &HashMap<String, FolderKind>,
    message: &Message,
    raw: Option<&[u8]>,
    inline_hashes: HashMap<String, (db::blob_hash::BlobHash, InlineImage)>,
    outcome: HydrationOutcome,
) -> ConsumerMessageRow {
    let message_id = message.id.0.clone();
    let thread_id = message
        .thread_id
        .as_ref()
        .map(|id| id.0.clone())
        .unwrap_or_else(|| message_id.clone());
    // `date` matches legacy: JMAP `sentAt || receivedAt` (bifrost computes
    // `Message::date` the same way). `internal_date` legacy = `receivedAt`,
    // a server-assigned timestamp the frozen bifrost surface does not expose
    // on `Message`/`HydratedObject`/`InventoryEntry` and that does not live
    // in the RFC822 octets either; `Message::date` is the closest available
    // value, equal to `receivedAt` whenever `sentAt` is unset.
    let date = message.date.map(system_time_ms).unwrap_or(0);

    let degraded = matches!(outcome, HydrationOutcome::SucceededWithDegradedBody);

    // B3a-cut-jmap 4.2: re-parse the verbatim RFC822 to recover the headers /
    // body / attachment detail the structured `Message` dropped. A re-parse
    // failure degrades the body lane (metadata persists, never dropped -
    // B3-spec 4.1.3) rather than failing the row.
    let raw = if degraded { None } else { raw };
    let reparsed = raw.and_then(|bytes| parse_rfc822(&MessageParser::default(), bytes).ok());
    // If the raw fetch was requested but yielded nothing parseable, the body
    // could not be recovered: treat as a degraded body (metadata-only).
    let body_degraded = degraded || (raw.is_some() && reparsed.is_none());

    let parsed = reparsed.unwrap_or_default();
    let Rfc822Parsed {
        message_id_header,
        in_reply_to_header,
        references_header,
        list_unsubscribe,
        list_unsubscribe_post,
        auth_results,
        mdn_requested,
        body_text: parsed_body_text,
        body_html: parsed_body_html,
        attachments: parsed_attachments,
    } = parsed;

    let (body_html, body_text) = if body_degraded {
        (None, None)
    } else {
        // Prefer the re-parsed body (first non-AMP part, legacy semantics);
        // fall back to the structured body if the re-parse produced nothing.
        (
            parsed_body_html.or_else(|| message.body_html.clone()),
            parsed_body_text.or_else(|| message.body_text.clone()),
        )
    };
    // Snippet: legacy used the JMAP server `email.preview()`, which bifrost
    // does not surface; derive it from the re-parsed text body as the IMAP
    // raw-MIME path does.
    let snippet = snippet_from_body(body_text.as_deref());

    let keywords = message
        .flags
        .iter()
        .filter(|keyword| common::folder_roles::is_user_visible_keyword(keyword))
        .cloned()
        .collect::<Vec<_>>();
    let mut folders = message
        .containers
        .iter()
        .filter_map(|container| {
            if provider == BifrostProviderKind::Jmap
                && let Some(folder) = jmap_folder_map.get(&container.0)
            {
                return Some(folder.clone());
            }
            FolderKind::parse(&container.0, provider.mail_provider_kind()).ok()
        })
        .collect::<Vec<_>>();
    // Legacy JMAP `get_labels_for_email` synthesized a DRAFT folder for any
    // `$draft`-keyword message that did not already resolve a Drafts
    // mailbox, so a draft without an explicit Drafts container still lands
    // in the DRAFT system folder. Preserve that to keep folder membership
    // byte-identical with the legacy path.
    if provider == BifrostProviderKind::Jmap && message.flags.contains("$draft") {
        let draft = FolderKind::System(common::types::SystemFolderId::Draft);
        if !folders.contains(&draft) {
            folders.push(draft);
        }
    }
    // Inline-image blobs were downloaded by the caller; their per-blob BLAKE3
    // hash (keyed by blob id) back-fills each attachment row's `content_hash`
    // exactly as the legacy JMAP post-store UPDATE did, so the attachment-
    // store dedup linkage stays intact.
    // Attachment rows: id / remote_attachment_id come from the bifrost blob
    // handle (the JMAP blobId); filename / content_id / is_inline come from
    // the RFC822 re-parse (the structured `BlobHandle` cannot carry the part
    // name, the Content-ID, or the inline disposition). The server returns
    // the structured attachment list and the MIME parts in the same order,
    // so the two are matched by ordinal position.
    let attachments = message
        .attachments
        .iter()
        .enumerate()
        .map(|(index, blob)| {
            let detail = parsed_attachments.get(index);
            AttachmentInsertRow {
                id: format!("{}_{}", message_id, blob.id.0),
                message_id: message_id.clone(),
                account_id: account_id.0.clone(),
                filename: detail
                    .map(|d| d.filename.clone())
                    .or_else(|| Some(blob.id.0.clone())),
                mime_type: detail
                    .map(|d| d.mime_type.clone())
                    .or_else(|| blob.content_type.clone()),
                size: detail
                    .map(|d| d.size)
                    .or_else(|| blob.size.and_then(|size| i64::try_from(size).ok())),
                remote_attachment_id: Some(blob.id.0.clone()),
                content_hash: inline_hashes.get(&blob.id.0).map(|entry| entry.0),
                content_id: detail.and_then(|d| d.content_id.clone()),
                is_inline: detail.map_or_else(
                    || {
                        blob.content_type
                            .as_deref()
                            .is_some_and(|mime| mime.starts_with("image/"))
                    },
                    |d| d.is_inline,
                ),
            }
        })
        .collect::<Vec<_>>();
    let inline_images = inline_hashes
        .into_values()
        .map(|(_, image)| image)
        .collect::<Vec<_>>();
    let raw_size = raw
        .map(|bytes| i64::try_from(bytes.len()).unwrap_or(i64::MAX))
        .or_else(|| message.size_bytes.and_then(|size| i64::try_from(size).ok()));
    let row = MessageInsertRow {
        id: message_id.clone(),
        account_id: account_id.0.clone(),
        thread_id: thread_id.clone(),
        from_address: message.from.first().map(|address| address.address.clone()),
        from_name: message
            .from
            .first()
            .and_then(|address| address.name.clone()),
        to_addresses: format_addresses(&message.to),
        cc_addresses: format_addresses(&message.cc),
        bcc_addresses: format_addresses(&message.bcc),
        reply_to: format_addresses(&message.reply_to),
        subject: message.subject.clone(),
        snippet: snippet.clone(),
        date,
        is_read: message.flags.contains("$seen"),
        is_starred: message.flags.contains("$flagged"),
        is_replied: message.flags.contains("$answered"),
        is_forwarded: message.flags.contains("$forwarded"),
        raw_size,
        internal_date: Some(date),
        list_unsubscribe,
        list_unsubscribe_post,
        auth_results,
        message_id_header,
        // Prefer the re-parsed References (all ids); fall back to the
        // structured list if the re-parse produced nothing.
        references_header: references_header.or_else(|| {
            if message.references.is_empty() {
                None
            } else {
                Some(message.references.join(" "))
            }
        }),
        // In-Reply-To: the re-parse joins ALL ids; the structured
        // `Message::in_reply_to` keeps only the first.
        in_reply_to_header: in_reply_to_header.or_else(|| message.in_reply_to.clone()),
        body_cached: body_html.is_some() || body_text.is_some(),
        mdn_requested,
        is_reaction: false,
        imap_uid: None,
        imap_folder: None,
        has_meeting_invite: parsed_attachments
            .iter()
            .map(|att| att.mime_type.as_str())
            .any(common::email_parsing::is_calendar_content_type),
        meeting_invite_method: parsed_attachments
            .iter()
            .find_map(|att| common::email_parsing::extract_imip_method(&att.mime_type)),
        meeting_invite_uid: None,
    };
    let search_document = SearchDocument {
        message_id,
        account_id: account_id.0.clone(),
        thread_id,
        subject: message.subject.clone(),
        from_name: row.from_name.clone(),
        from_address: row.from_address.clone(),
        to_addresses: row.to_addresses.clone(),
        body_text: body_text.clone(),
        snippet: Some(snippet),
        date: date / 1000,
        is_read: row.is_read,
        is_starred: row.is_starred,
        has_attachment: !attachments.is_empty(),
        attachments: Vec::new(),
    };
    ConsumerMessageRow {
        message: row,
        folders,
        labels: Vec::new(),
        keywords,
        attachments,
        body_html,
        body_text,
        inline_images,
        search_document,
        is_important: matches!(message.importance, Importance::High),
    }
}

/// Stream the message's verbatim RFC822 octets via the engine's read-only
/// `open_raw_rfc822` passthrough. Returns `Ok(None)` when the stream
/// terminates with an error (e.g. the provider does not support raw fetch),
/// so the caller degrades the body lane rather than failing the row.
async fn fetch_raw_rfc822(
    engine: &SyncEngine,
    account_id: &AccountId,
    message_id: &ObjectId,
) -> Result<Option<Vec<u8>>, bifrost_sync::Error> {
    let mut stream = engine.open_raw_rfc822(account_id, message_id.clone())?;
    let mut bytes = Vec::new();
    let mut terminated = false;
    while let Some(event) = stream.next().await {
        match event {
            SyncEvent::Batch(batch) => {
                for chunk in batch.items {
                    bytes.extend_from_slice(&chunk);
                }
            }
            SyncEvent::Terminated(error) => {
                log::warn!(
                    "JMAP raw RFC822 fetch terminated for {}: {}",
                    message_id.0,
                    error
                );
                terminated = true;
                break;
            }
            SyncEvent::Done(_) | SyncEvent::Progress(_) | SyncEvent::Warning(_) => {}
            _ => {}
        }
    }
    if terminated || bytes.is_empty() {
        Ok(None)
    } else {
        Ok(Some(bytes))
    }
}

/// Download each image blob once and return, keyed by blob id, the BLAKE3
/// hash plus the `InlineImage` ready for the store. The hash is surfaced so
/// the caller can back-fill `attachments.content_hash` (legacy parity); a
/// blob is downloaded at most once per message so duplicate attachment rows
/// pointing at the same blob share a single download and the same hash.
async fn hydrate_inline_images(
    engine: &SyncEngine,
    account_id: &AccountId,
    blobs: &[BlobHandle],
) -> Result<HashMap<String, (db::blob_hash::BlobHash, InlineImage)>, bifrost_sync::Error> {
    let mut images: HashMap<String, (db::blob_hash::BlobHash, InlineImage)> = HashMap::new();
    for blob in blobs {
        let Some(mime_type) = blob.content_type.as_ref() else {
            continue;
        };
        if !mime_type.starts_with("image/") {
            continue;
        }
        if images.contains_key(&blob.id.0) {
            continue;
        }
        let mut stream = engine.open_blob(account_id, blob.clone())?;
        let mut bytes = Vec::new();
        while let Some(event) = stream.next().await {
            match event {
                SyncEvent::Batch(batch) => {
                    for chunk in batch.items {
                        bytes.extend_from_slice(&chunk);
                    }
                }
                SyncEvent::Terminated(error) => {
                    log::warn!(
                        "JMAP inline blob {} hydration terminated: {}",
                        blob.id.0,
                        error
                    );
                    bytes.clear();
                    break;
                }
                SyncEvent::Done(_) | SyncEvent::Progress(_) | SyncEvent::Warning(_) => {}
                _ => {}
            }
        }
        if !bytes.is_empty() && bytes.len() <= store::inline_image_store::MAX_INLINE_SIZE {
            let hash = db::blob_hash::BlobHash::hash(&bytes);
            images.insert(
                blob.id.0.clone(),
                (
                    hash,
                    InlineImage {
                        content_hash: hash.to_hex(),
                        data: bytes,
                        mime_type: mime_type.clone(),
                    },
                ),
            );
        }
    }
    Ok(images)
}

fn system_time_ms(time: SystemTime) -> i64 {
    let millis = time
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    i64::try_from(millis).unwrap_or(i64::MAX)
}

fn format_addresses(addresses: &[bifrost_types::Address]) -> Option<String> {
    if addresses.is_empty() {
        return None;
    }
    common::email_parsing::format_address_list(
        addresses
            .iter()
            .map(|address| (address.name.clone(), address.address.clone())),
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyntheticMessage {
    pub id: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub from_addr: String,
    #[serde(default)]
    pub to_addrs: Vec<String>,
    #[serde(default)]
    pub folder_ids: Vec<String>,
    #[serde(default)]
    pub label_ids: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default)]
    pub raw_body: Vec<u8>,
    #[serde(default)]
    pub degraded_body: bool,
    #[serde(default)]
    pub forced_outcome: Option<SyntheticOutcome>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyntheticOutcome {
    Succeeded,
    DegradedBody,
    Failed,
    Uncertain,
}

pub fn encode_synthetic_message(message: &SyntheticMessage) -> Result<String, String> {
    serde_json::to_string(message)
        .map(|json| format!("{SYNTHETIC_OBJECT_PREFIX}{json}"))
        .map_err(|error| format!("encode synthetic bifrost message: {error}"))
}

fn decode_synthetic_message(id: &str) -> Option<SyntheticMessage> {
    let json = id.strip_prefix(SYNTHETIC_OBJECT_PREFIX)?;
    serde_json::from_str(json).ok()
}

fn synthetic_to_row(
    account_id: &str,
    provider: BifrostProviderKind,
    synthetic: SyntheticMessage,
    mut outcome: HydrationOutcome,
) -> HydratedChange {
    if let Some(forced) = &synthetic.forced_outcome {
        outcome = match forced {
            SyntheticOutcome::Succeeded => HydrationOutcome::Succeeded,
            SyntheticOutcome::DegradedBody => HydrationOutcome::SucceededWithDegradedBody,
            SyntheticOutcome::Failed => HydrationOutcome::Failed,
            SyntheticOutcome::Uncertain => HydrationOutcome::Uncertain,
        };
    } else if synthetic.degraded_body {
        outcome = HydrationOutcome::SucceededWithDegradedBody;
    }
    // The degraded-body lane suppresses body hydration regardless of whether
    // it was requested via the `degraded_body` field or a forced
    // `DegradedBody` outcome - both must yield the same metadata-only row so
    // `body_cached` reflects the failure (spec 4.1.3 degraded-body lane).
    let degraded = matches!(outcome, HydrationOutcome::SucceededWithDegradedBody);
    let body_text = if degraded || synthetic.raw_body.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&synthetic.raw_body).into_owned())
    };
    let thread_id = synthetic
        .thread_id
        .clone()
        .unwrap_or_else(|| synthetic.id.clone());
    let snippet = body_text
        .as_deref()
        .unwrap_or("")
        .chars()
        .take(160)
        .collect::<String>();
    let message = MessageInsertRow {
        id: synthetic.id.clone(),
        account_id: account_id.to_string(),
        thread_id: thread_id.clone(),
        from_address: Some(synthetic.from_addr.clone()),
        from_name: None,
        to_addresses: if synthetic.to_addrs.is_empty() {
            None
        } else {
            Some(synthetic.to_addrs.join(", "))
        },
        cc_addresses: None,
        bcc_addresses: None,
        reply_to: None,
        subject: Some(synthetic.subject.clone()),
        snippet: snippet.clone(),
        date: 1_700_000_000_000,
        is_read: false,
        is_starred: synthetic
            .keywords
            .iter()
            .any(|keyword| keyword == "$flagged"),
        is_replied: false,
        is_forwarded: false,
        raw_size: i64::try_from(synthetic.raw_body.len()).ok(),
        internal_date: Some(1_700_000_000_000),
        list_unsubscribe: None,
        list_unsubscribe_post: None,
        auth_results: None,
        message_id_header: Some(format!("<{}@synthetic.ratatoskr>", synthetic.id)),
        references_header: None,
        in_reply_to_header: None,
        body_cached: body_text.is_some(),
        mdn_requested: false,
        is_reaction: false,
        imap_uid: None,
        imap_folder: None,
        has_meeting_invite: false,
        meeting_invite_method: None,
        meeting_invite_uid: None,
    };
    let folders = synthetic
        .folder_ids
        .iter()
        .filter_map(|id| FolderKind::parse(id, provider.mail_provider_kind()).ok())
        .collect::<Vec<_>>();
    let labels = synthetic
        .label_ids
        .iter()
        .filter_map(|id| LabelKind::parse(id, provider.mail_provider_kind()).ok())
        .collect::<Vec<_>>();
    let inline_images = if degraded || synthetic.raw_body.is_empty() {
        Vec::new()
    } else {
        vec![InlineImage {
            content_hash: format!("bifrost-synth-{}", synthetic.id),
            data: synthetic.raw_body.clone(),
            mime_type: "text/plain".to_string(),
        }]
    };
    let search_document = SearchDocument {
        message_id: synthetic.id.clone(),
        account_id: account_id.to_string(),
        thread_id,
        subject: Some(synthetic.subject),
        from_name: None,
        from_address: Some(synthetic.from_addr),
        to_addresses: if synthetic.to_addrs.is_empty() {
            None
        } else {
            Some(synthetic.to_addrs.join(", "))
        },
        body_text: body_text.clone(),
        snippet: Some(snippet),
        date: 1_700_000_000,
        is_read: false,
        is_starred: synthetic
            .keywords
            .iter()
            .any(|keyword| keyword == "$flagged"),
        has_attachment: false,
        attachments: Vec::new(),
    };
    let is_important = synthetic
        .keywords
        .iter()
        .any(|keyword| keyword == "$important");
    HydratedChange::Message(
        Box::new(ConsumerMessageRow {
            message,
            folders,
            labels,
            keywords: synthetic.keywords,
            attachments: Vec::new(),
            body_html: None,
            body_text,
            inline_images,
            search_document,
            is_important,
        }),
        outcome,
    )
}

impl BifrostProviderKind {
    fn mail_provider_kind(self) -> MailProviderKind {
        match self {
            Self::Gmail => MailProviderKind::Gmail,
            Self::Graph => MailProviderKind::Graph,
            Self::Imap => MailProviderKind::Imap,
            Self::Jmap => MailProviderKind::Jmap,
        }
    }
}

fn minimal_message_row(account_id: &str, id: &str) -> MessageInsertRow {
    MessageInsertRow {
        id: id.to_string(),
        account_id: account_id.to_string(),
        thread_id: id.to_string(),
        from_address: None,
        from_name: None,
        to_addresses: None,
        cc_addresses: None,
        bcc_addresses: None,
        reply_to: None,
        subject: None,
        snippet: String::new(),
        date: 0,
        is_read: true,
        is_starred: false,
        is_replied: false,
        is_forwarded: false,
        raw_size: None,
        internal_date: None,
        list_unsubscribe: None,
        list_unsubscribe_post: None,
        auth_results: None,
        message_id_header: None,
        references_header: None,
        in_reply_to_header: None,
        body_cached: false,
        mdn_requested: false,
        is_reaction: false,
        imap_uid: None,
        imap_folder: None,
        has_meeting_invite: false,
        meeting_invite_method: None,
        meeting_invite_uid: None,
    }
}

fn minimal_search_document(account_id: &str, id: &str) -> SearchDocument {
    SearchDocument {
        message_id: id.to_string(),
        account_id: account_id.to_string(),
        thread_id: id.to_string(),
        subject: None,
        from_name: None,
        from_address: None,
        to_addresses: None,
        body_text: None,
        snippet: None,
        date: 0,
        is_read: true,
        is_starred: false,
        has_attachment: false,
        attachments: Vec::new(),
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{HydratedChange, HydrationOutcome, hydrate_change_to_message_insert_row_offline};
    use crate::bifrost::consumer::BifrostProviderKind;
    use bifrost_types::{Change, ObjectChange, ObjectChangeKind, ObjectId, ScopeChange};

    #[test]
    fn hydrate_change_to_message_insert_row_offline_maps_object_create() {
        let change = Change::ObjectChange(ObjectChange {
            id: ObjectId("m1".to_string()),
            kind: ObjectChangeKind::Created,
        });
        match hydrate_change_to_message_insert_row_offline(
            "acc",
            BifrostProviderKind::Jmap,
            &change,
        ) {
            HydratedChange::Message(row, HydrationOutcome::Succeeded) => {
                assert_eq!(row.message.id, "m1");
                assert_eq!(row.message.account_id, "acc");
                assert_eq!(row.message.thread_id, "m1");
            }
            other => panic!("unexpected hydration result: {other:?}"),
        }
    }

    #[test]
    fn hydrate_change_to_message_insert_row_offline_classifies_destroyed_as_deleted() {
        let change = Change::ObjectChange(ObjectChange {
            id: ObjectId("m1".to_string()),
            kind: ObjectChangeKind::Destroyed,
        });
        match hydrate_change_to_message_insert_row_offline(
            "acc",
            BifrostProviderKind::Jmap,
            &change,
        ) {
            HydratedChange::Message(_, HydrationOutcome::Deleted) => {}
            other => panic!("expected Deleted outcome, got {other:?}"),
        }
    }

    #[test]
    fn hydrate_batch_taxonomy_drops_deleted_and_does_not_block_ack() {
        let changes = vec![
            Change::ObjectChange(ObjectChange {
                id: ObjectId("keep".to_string()),
                kind: ObjectChangeKind::Created,
            }),
            Change::ObjectChange(ObjectChange {
                id: ObjectId("gone".to_string()),
                kind: ObjectChangeKind::Destroyed,
            }),
        ];
        let batch =
            super::HydrateBatch::from_changes_offline("acc", BifrostProviderKind::Jmap, &changes);
        assert_eq!(batch.rows.len(), 1, "deleted item must not persist a row");
        assert_eq!(batch.rows[0].message.id, "keep");
        assert_eq!(batch.deleted_ids.len(), 1);
        assert!(!batch.blocked, "a deletion must not block the ack");
    }

    fn synthetic_change(id: &str, forced: super::SyntheticOutcome) -> Change {
        let synthetic = super::SyntheticMessage {
            id: id.to_string(),
            thread_id: None,
            subject: format!("subject {id}"),
            from_addr: "peer@example.com".to_string(),
            to_addrs: vec!["me@example.com".to_string()],
            folder_ids: vec!["INBOX".to_string()],
            label_ids: Vec::new(),
            keywords: Vec::new(),
            raw_body: b"body".to_vec(),
            degraded_body: false,
            forced_outcome: Some(forced),
        };
        let encoded = super::encode_synthetic_message(&synthetic).unwrap();
        Change::ObjectChange(ObjectChange {
            id: ObjectId(encoded),
            kind: ObjectChangeKind::Created,
        })
    }

    /// Spec 4.1.3 / 6.1: the per-item hydration taxonomy. A degraded-body
    /// item persists its metadata row (NOT dropped) with body hydration off;
    /// a Failed item is skipped and does NOT block the ack; an Uncertain
    /// item leaves siblings persisted but sets `blocked` so the ack is held.
    #[test]
    fn hydrate_change_to_message_insert_row_offline_taxonomy_lanes() {
        use super::SyntheticOutcome;

        // Degraded body: metadata row persists, body hydration off.
        match hydrate_change_to_message_insert_row_offline(
            "acc",
            BifrostProviderKind::Jmap,
            &synthetic_change("deg", SyntheticOutcome::DegradedBody),
        ) {
            HydratedChange::Message(row, HydrationOutcome::SucceededWithDegradedBody) => {
                assert_eq!(row.message.id, "deg");
                assert!(!row.message.body_cached, "degraded body is not cached");
                assert!(row.body_text.is_none(), "degraded body text dropped");
            }
            other => panic!("expected degraded-body lane, got {other:?}"),
        }

        // A Failed item is skipped in the batch and does NOT block the ack.
        let changes = vec![
            synthetic_change("ok", SyntheticOutcome::Succeeded),
            synthetic_change("bad", SyntheticOutcome::Failed),
        ];
        let batch =
            super::HydrateBatch::from_changes_offline("acc", BifrostProviderKind::Jmap, &changes);
        assert_eq!(batch.rows.len(), 1, "Failed item is skipped");
        assert_eq!(batch.rows[0].message.id, "ok");
        assert_eq!(batch.failed, 1);
        assert!(!batch.blocked, "a Failed item must not block the ack");

        // An Uncertain item persists its Succeeded siblings but blocks the ack.
        let changes = vec![
            synthetic_change("sib", SyntheticOutcome::Succeeded),
            synthetic_change("maybe", SyntheticOutcome::Uncertain),
        ];
        let batch =
            super::HydrateBatch::from_changes_offline("acc", BifrostProviderKind::Jmap, &changes);
        assert_eq!(batch.rows.len(), 1, "siblings persist, Uncertain does not");
        assert_eq!(batch.uncertain, 1);
        assert!(batch.blocked, "an Uncertain item must block the ack");
    }

    #[test]
    fn hydrate_scope_change_is_membership_only() {
        let change = Change::ScopeChange(ScopeChange {
            id: ObjectId("m1".to_string()),
            membership: bifrost_types::MembershipScope::Folder(bifrost_types::FolderId(
                "INBOX".to_string(),
            )),
            kind: bifrost_types::ScopeChangeKind::Added,
        });
        assert!(matches!(
            hydrate_change_to_message_insert_row_offline("acc", BifrostProviderKind::Jmap, &change),
            HydratedChange::ScopeOnly
        ));
    }
}
