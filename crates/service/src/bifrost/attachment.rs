//! Attachment byte hydration through the resident bifrost engine.

use futures_util::StreamExt;

use bifrost_types::{
    AccountId, BlobCapabilities, BlobEncoding, BlobHandle, BlobId, CloudUploadMeta,
    HostedAttachment, ObjectId, ShareScope, SyncEvent, WarningKind,
};

use super::{BifrostProviderKind, ResidentEngine};

#[derive(Debug, thiserror::Error)]
pub(crate) enum AttachmentByteError {
    #[error("resident attach failed: {0}")]
    Attach(String),
    #[error("attachment has no persisted bifrost blob id")]
    MissingBlobId,
    #[error("engine blob opener failed: {0}")]
    Engine(#[source] bifrost_sync::Error),
    #[error("attachment stream terminated: {0}")]
    Account(#[source] bifrost_types::AccountError),
    #[error("attachment is a reference, not a byte stream")]
    NotByteStream,
    #[error("IMAP MIME part extraction failed: {0}")]
    ImapPart(String),
}

#[derive(Clone)]
pub(crate) struct AttachmentByteSource {
    resident: ResidentEngine,
}

impl AttachmentByteSource {
    pub(crate) fn new(resident: ResidentEngine) -> Self {
        Self { resident }
    }

    pub(crate) async fn fetch(
        &self,
        account_id: &str,
        row: &db::db::queries_extra::AttachmentCacheInfo,
    ) -> Result<Vec<u8>, AttachmentByteError> {
        let action_account = self
            .resident
            .action_account(account_id)
            .await
            .map_err(AttachmentByteError::Attach)?;
        let account = AccountId(account_id.to_string());
        match action_account.provider {
            BifrostProviderKind::Imap => {
                let raw = action_account
                    .engine
                    .open_raw_rfc822(&account, imap_object_id(row)?)
                    .map_err(AttachmentByteError::Engine)?;
                let bytes = drain(raw).await?;
                extract_imap_part(&bytes, row)
            }
            BifrostProviderKind::Gmail | BifrostProviderKind::Graph | BifrostProviderKind::Jmap => {
                let stream = action_account
                    .engine
                    .open_blob(&account, build_blob_handle(row)?)
                    .map_err(AttachmentByteError::Engine)?;
                drain(stream).await
            }
        }
    }

    /// Fetch the whole assembled RFC822 octets for the IMAP message that owns
    /// `row`, WITHOUT extracting a part. The prefetch worker uses this to
    /// hydrate a message once and satisfy every one of its queued attachments
    /// from the single parse (see `extract_imap_part`), so an N-attachment
    /// message costs one `open_raw_rfc822` download rather than N. Errors are
    /// the same `AttachmentByteError` shape `fetch` returns, so the caller's
    /// skip-reason classification is unchanged.
    ///
    /// IMAP is the only provider that routes bytes through the whole message:
    /// bifrost IMAP `open_blob` (`BODY.PEEK[section]`) returns the part still
    /// in its content-transfer-encoding, and ratatoskr keeps no per-part CTE to
    /// decode it, whereas `open_raw_rfc822` + `extract_attachment_from_rfc822`
    /// (mail-parser) decodes exactly as the legacy fetch did. The single
    /// message parse is the batching artifact that keeps per-part reuse cheap.
    pub(crate) async fn fetch_imap_rfc822(
        &self,
        account_id: &str,
        row: &db::db::queries_extra::AttachmentCacheInfo,
    ) -> Result<Vec<u8>, AttachmentByteError> {
        let action_account = self
            .resident
            .action_account(account_id)
            .await
            .map_err(AttachmentByteError::Attach)?;
        let account = AccountId(account_id.to_string());
        let raw = action_account
            .engine
            .open_raw_rfc822(&account, imap_object_id(row)?)
            .map_err(AttachmentByteError::Engine)?;
        drain(raw).await
    }

    /// Capability-dispatched cloud hosting for a future compose caller. The
    /// resident handle attaches cold accounts before forwarding to bifrost;
    /// unsupported accounts retain their structured AccountError instead of a
    /// provider branch in ratatoskr.
    #[allow(dead_code)] // pinned compose surface; B9 intentionally adds no UI caller
    pub(crate) async fn host_large_attachment(
        &self,
        account_id: &str,
        bytes: bytes::Bytes,
        file_name: &str,
        mime_type: &str,
        scope: ShareScope,
    ) -> Result<HostedAttachment, AttachmentByteError> {
        let action_account = self
            .resident
            .action_account(account_id)
            .await
            .map_err(AttachmentByteError::Attach)?;
        let account = AccountId(account_id.to_string());
        let meta = CloudUploadMeta::new(file_name, mime_type, bytes.len() as u64, scope);
        action_account
            .engine
            .host_attachment(&account, bytes, meta)
            .map_err(AttachmentByteError::Engine)?
            .await
            .map_err(AttachmentByteError::Account)
    }
}

/// Reconstruct the bifrost `BlobHandle` for an HTTP-provider attachment from
/// its persisted verbatim `blob_id`. Only `id` is load-bearing for
/// `open_blob` (the capability flags gate `open_blob_range`, which the
/// whole-blob fetch never uses), so the flags default; a row that predates the
/// `blob_id` column (NULL) surfaces `MissingBlobId` rather than mis-fetching
/// the unwrapped `remote_attachment_id`.
fn build_blob_handle(
    row: &db::db::queries_extra::AttachmentCacheInfo,
) -> Result<BlobHandle, AttachmentByteError> {
    let blob_id = row
        .blob_id
        .as_ref()
        .ok_or(AttachmentByteError::MissingBlobId)?;
    Ok(BlobHandle {
        id: BlobId(blob_id.clone()),
        size: row.size.and_then(|size| u64::try_from(size).ok()),
        content_type: row.mime_type.clone(),
        digest: None,
        capabilities: BlobCapabilities {
            supports_range: false,
            supports_parallel: false,
            digest_available_pre_download: false,
            encoding: BlobEncoding::Raw8Bit,
        },
    })
}

/// Extract one attachment's decoded bytes from an already-fetched RFC822
/// message. Shared by the single-item `fetch` path and the prefetch worker's
/// message-batched path so both identify + decode the IMAP part identically
/// (`extract_attachment_from_rfc822` runs the mail-parser section map). A
/// `part_id` miss is a structured error, never a silent empty return.
pub(crate) fn extract_imap_part(
    raw: &[u8],
    row: &db::db::queries_extra::AttachmentCacheInfo,
) -> Result<Vec<u8>, AttachmentByteError> {
    let part_id = row
        .remote_attachment_id
        .as_deref()
        .ok_or_else(|| AttachmentByteError::ImapPart("missing persisted part id".into()))?;
    imap::client::extract_attachment_from_rfc822(raw, part_id)
        .map_err(AttachmentByteError::ImapPart)
}

fn imap_object_id(
    row: &db::db::queries_extra::AttachmentCacheInfo,
) -> Result<ObjectId, AttachmentByteError> {
    let folder = row
        .imap_folder
        .as_deref()
        .ok_or_else(|| AttachmentByteError::ImapPart("IMAP message missing folder".into()))?;
    let uid = row
        .imap_uid
        .ok_or_else(|| AttachmentByteError::ImapPart("IMAP message missing UID".into()))?;
    let uidvalidity = row
        .imap_uidvalidity
        .ok_or_else(|| AttachmentByteError::ImapPart("IMAP message missing UIDVALIDITY".into()))?;
    Ok(ObjectId(format!(
        "imap1:{}:{folder}:{uidvalidity}:{uid}",
        folder.len()
    )))
}

async fn drain(
    mut stream: bifrost_types::AccountStream<SyncEvent<bytes::Bytes>>,
) -> Result<Vec<u8>, AttachmentByteError> {
    let mut out = Vec::new();
    while let Some(event) = stream.next().await {
        match event {
            SyncEvent::Batch(batch) => {
                for bytes in batch.items {
                    out.extend_from_slice(&bytes);
                }
            }
            SyncEvent::Warning(warning)
                if matches!(warning.kind, WarningKind::BlobNotByteStream) =>
            {
                return Err(AttachmentByteError::NotByteStream);
            }
            SyncEvent::Terminated(error) => return Err(AttachmentByteError::Account(error)),
            SyncEvent::Done(_) => return Ok(out),
            SyncEvent::Progress(_) | SyncEvent::Warning(_) => {}
            _ => {}
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::db::queries_extra::AttachmentCacheInfo;

    fn row() -> AttachmentCacheInfo {
        AttachmentCacheInfo {
            id: "msg_att".into(),
            remote_attachment_id: Some("unwrapped-attachment-id".into()),
            blob_id: Some("verbatim-blob-id".into()),
            content_hash: None,
            mime_type: Some("application/pdf".into()),
            size: Some(1234),
            imap_folder: None,
            imap_uid: None,
            imap_uidvalidity: None,
            is_inline: false,
            text_indexed_at: None,
            extraction_status: None,
            blob_tombstoned_at: None,
            blob_present: false,
        }
    }

    #[test]
    fn attachment_byte_source_reconstructs_blob_handle() {
        // The handle id must be the verbatim persisted blob id, never the
        // lossy unwrapped `remote_attachment_id`.
        let handle = build_blob_handle(&row()).expect("handle");
        assert_eq!(handle.id.0, "verbatim-blob-id");
        assert_ne!(handle.id.0, "unwrapped-attachment-id");
        assert_eq!(handle.size, Some(1234));
        assert_eq!(handle.content_type.as_deref(), Some("application/pdf"));

        // A pre-column NULL blob_id fails closed rather than fetching the
        // unwrapped remote id.
        let mut missing = row();
        missing.blob_id = None;
        assert!(matches!(
            build_blob_handle(&missing),
            Err(AttachmentByteError::MissingBlobId)
        ));
    }

    #[test]
    fn imap_object_id_matches_b4a_reconstruction() {
        let mut r = row();
        r.imap_folder = Some("INBOX".into());
        r.imap_uid = Some(42);
        r.imap_uidvalidity = Some(7);
        let object_id = imap_object_id(&r).expect("object id");
        assert_eq!(object_id.0, "imap1:5:INBOX:7:42");

        // Missing IMAP coordinates surface a structured error, never a
        // silently malformed ObjectId.
        r.imap_uid = None;
        assert!(matches!(
            imap_object_id(&r),
            Err(AttachmentByteError::ImapPart(_))
        ));
    }

    fn stream(
        events: Vec<SyncEvent<bytes::Bytes>>,
    ) -> bifrost_types::AccountStream<SyncEvent<bytes::Bytes>> {
        Box::pin(futures_util::stream::iter(events))
    }

    fn byte_batch(bytes: &[u8]) -> SyncEvent<bytes::Bytes> {
        SyncEvent::Batch(bifrost_types::Batch {
            items: vec![bytes::Bytes::copy_from_slice(bytes)],
            page_boundary: bifrost_types::PageBoundary::Final,
            server_latency: std::time::Duration::ZERO,
            bytes_in: bytes.len() as u64,
            checkpoint: None,
        })
    }

    /// A Graph reference / non-byte-stream attachment emits
    /// `Warning(BlobNotByteStream)` then `Done` with NO byte batch. The
    /// drainer MUST surface a distinct `NotByteStream` error rather than
    /// returning an empty `Vec` that the caller would `PackStore::put` and
    /// ack as a cached empty file. This is the runtime half of the Graph
    /// reference-attachment gate (B9-spec finding D / § 4.1.2).
    #[tokio::test]
    async fn drain_rejects_reference_attachment_instead_of_caching_empty() {
        let events = vec![
            SyncEvent::Warning(bifrost_types::Warning::support_only(
                WarningKind::BlobNotByteStream,
                "reference attachment has no byte stream",
            )),
            SyncEvent::Done(None),
        ];
        let result = drain(stream(events)).await;
        assert!(
            matches!(result, Err(AttachmentByteError::NotByteStream)),
            "expected NotByteStream, got {result:?}"
        );
    }

    /// A legitimate zero-length attachment (a real empty `Batch` ending in
    /// `Done`) is a genuine zero-byte success, kept distinct from the
    /// warning-terminated non-byte-stream above.
    #[tokio::test]
    async fn drain_returns_bytes_and_keeps_empty_batch_distinct() {
        let full = drain(stream(vec![byte_batch(b"hello"), SyncEvent::Done(None)]))
            .await
            .expect("bytes");
        assert_eq!(full, b"hello".to_vec());

        let empty = drain(stream(vec![byte_batch(b""), SyncEvent::Done(None)]))
            .await
            .expect("empty ok");
        assert!(empty.is_empty());
    }

    /// A terminated stream surfaces the account error, not a truncated
    /// success.
    #[tokio::test]
    async fn drain_propagates_terminated_error() {
        use bifrost_types::{
            AccountErrorBuilder, AccountErrorKind, Cause, DiagnosticText, RequestCause,
            RequestErrorKind,
        };
        let err = AccountErrorBuilder::new(
            AccountErrorKind::Request(RequestErrorKind::Malformed),
            Cause::Request(RequestCause::Malformed {
                detail: DiagnosticText::support_only("blob fetch terminated"),
            }),
        )
        .try_build()
        .expect("valid account error");
        let result = drain(stream(vec![
            byte_batch(b"partial"),
            SyncEvent::Terminated(err),
        ]))
        .await;
        assert!(matches!(result, Err(AttachmentByteError::Account(_))));
    }
}
