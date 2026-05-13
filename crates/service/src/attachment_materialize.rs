//! Per-fetch transient extraction from `PackStore` to a tmp file.
//!
//! Phase 3 of the attachments roadmap. The UI reads attachment bytes
//! by re-opening the file at the path returned by `materialize_blob`.
//! Bytes do not cross the IPC. The open fd is the pin against
//! eviction: tombstoning a blob in `attachment_blobs` and eventually
//! GC'ing its pack frame is independent of any in-flight UI read,
//! because the read is against the tmp file, not the pack.
//!
//! Tmp files live under `<app_data>/attachment_fetch_tmp/<hash>-<uuid>`.
//! `AttachmentTmpCleanupKick` reaps entries older than 10 minutes on
//! the 5-min `SyncTick` cadence.

use std::path::PathBuf;
use std::sync::Arc;

use db::blob_hash::BlobHash;
use service_api::ServiceError;

use crate::boot::BootSharedState;

const TMP_DIR: &str = "attachment_fetch_tmp";

pub(crate) struct MaterializedBlob {
    /// Absolute path of the materialized blob on disk.
    pub path: PathBuf,
    /// `<app_data>`-relative form. Carried in `AttachmentFetchAck`.
    pub relative_path: String,
    pub size_bytes: u64,
}

/// Copy the blob backing `content_hash` out of the pack store to a
/// fresh tmp file and return its location. The caller (UI handler or
/// extract worker) opens the file positionally; the file lives until
/// the periodic cleanup kick unlinks it.
pub(crate) async fn materialize_blob(
    boot_state: &Arc<BootSharedState>,
    content_hash: &BlobHash,
) -> Result<MaterializedBlob, ServiceError> {
    let pack_store = boot_state.pack_store().ok_or_else(|| {
        ServiceError::Internal(
            "pack store not installed; UI must wait for boot.ready before \
             attachment.fetch"
                .into(),
        )
    })?;

    let bytes = pack_store
        .get(content_hash)
        .await
        .map_err(|e| ServiceError::Internal(format!("PackStore::get: {e}")))?
        .ok_or_else(|| {
            ServiceError::Internal(format!(
                "blob {content_hash} indexed in attachments but absent from pack store"
            ))
        })?;

    write_bytes_to_tmp(boot_state, content_hash, bytes).await
}

/// Write a pre-fetched byte buffer to the same tmp-file layout
/// `materialize_blob` uses. Inline-image attachments take this path
/// (bytes come from `inline_images.db`, not PackStore) so the wire
/// ack stays uniform.
pub(crate) async fn write_bytes_to_tmp(
    boot_state: &Arc<BootSharedState>,
    content_hash: &BlobHash,
    bytes: Vec<u8>,
) -> Result<MaterializedBlob, ServiceError> {
    let size_bytes = bytes.len() as u64;
    let app_data = boot_state.app_data_dir().to_path_buf();
    let tmp_dir = app_data.join(TMP_DIR);
    let hash_hex = content_hash.to_hex();
    let request_id = uuid::Uuid::new_v4();
    let filename = format!("{hash_hex}-{request_id}");
    let final_path = tmp_dir.join(&filename);
    let part_path = tmp_dir.join(format!("{filename}.part"));
    let final_for_blocking = final_path.clone();
    let part_for_blocking = part_path.clone();
    let tmp_dir_for_blocking = tmp_dir.clone();

    tokio::task::spawn_blocking(move || -> std::io::Result<()> {
        std::fs::create_dir_all(&tmp_dir_for_blocking)?;
        let mut file = std::fs::File::create(&part_for_blocking)?;
        std::io::Write::write_all(&mut file, &bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&part_for_blocking, &final_for_blocking)
    })
    .await
    .map_err(|e| ServiceError::Internal(format!("spawn_blocking materialize: {e}")))?
    .map_err(|e| ServiceError::Internal(format!("materialize tmp write: {e}")))?;

    let relative_path = format!("{TMP_DIR}/{filename}");
    Ok(MaterializedBlob {
        path: final_path,
        relative_path,
        size_bytes,
    })
}

/// Walk `<app_data>/attachment_fetch_tmp/` and unlink entries whose
/// mtime is older than `max_age_secs`. Returns the count unlinked.
/// Idempotent; runs from `AttachmentTmpCleanupKick`.
pub(crate) async fn reap_stale_tmp_files(
    boot_state: &Arc<BootSharedState>,
    max_age_secs: u64,
) -> Result<u32, String> {
    let app_data = boot_state.app_data_dir().to_path_buf();
    tokio::task::spawn_blocking(move || -> Result<u32, String> {
        let tmp_dir = app_data.join(TMP_DIR);
        let entries = match std::fs::read_dir(&tmp_dir) {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
            Err(e) => return Err(format!("read tmp dir {}: {e}", tmp_dir.display())),
        };
        let now = std::time::SystemTime::now();
        let mut count = 0u32;
        for entry in entries.flatten() {
            let Ok(meta) = entry.metadata() else { continue };
            if !meta.is_file() {
                continue;
            }
            let Ok(modified) = meta.modified() else { continue };
            let Ok(age) = now.duration_since(modified) else { continue };
            if age.as_secs() < max_age_secs {
                continue;
            }
            if let Err(e) = std::fs::remove_file(entry.path()) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn!(
                        "attachment_fetch_tmp reap: unlink {} failed: {e}",
                        entry.path().display(),
                    );
                }
                continue;
            }
            count = count.saturating_add(1);
        }
        Ok(count)
    })
    .await
    .map_err(|e| format!("spawn_blocking reap: {e}"))?
}
