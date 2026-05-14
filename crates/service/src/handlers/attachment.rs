//! `attachment.fetch` handler.
//!
//! Wire ack carries `{ content_hash, size_bytes, relative_path }`.
//! Bytes never cross the IPC. The Service materializes the blob from
//! `PackStore` to `<app_data>/attachment_fetch_tmp/<hash>-<uuid>` and
//! returns the relative path; the UI re-opens the file positionally
//! and the open fd is the pin against eviction (Linux `unlink` is
//! fd-safe). An idle cleanup pass reaps tmp entries older than 10
//! minutes (`attachment.tmp_cleanup_kick`).
//!
//! On cache miss the handler runs:
//! provider `fetch_attachment` -> `BlobHash::hash` -> `PackStore::put`
//! -> `update_attachment_cache_fields` -> `materialize_blob` -> ack.
//! On cache hit the handler skips the provider call and goes straight
//! to `materialize_blob`.

use std::sync::Arc;

use serde_json::Value;
use service_api::{
    AttachmentCacheSizeAck, AttachmentClearCacheAck, AttachmentFetchAck, AttachmentFetchParams,
    ServiceError,
};

use crate::attachment_materialize::{self, MaterializedBlob};
use crate::boot::BootSharedState;

/// Inline-image fallback for the cache-hit path. Provider sync writes
/// small CID images to `inline_images.db` and never calls
/// `PackStore::put`, so the cache-hit branch has to consult that
/// store first for `is_inline = 1` rows. Returns `None` if the inline
/// store has no row for this hash (the caller falls through to
/// PackStore, which will surface the absence as an error).
async fn try_inline_image_materialize(
    boot_state: &Arc<BootSharedState>,
    content_hash: &db::blob_hash::BlobHash,
) -> Result<Option<MaterializedBlob>, ServiceError> {
    let read = boot_state.inline_image_read().ok_or_else(|| {
        ServiceError::Internal(
            "inline image store not installed; UI must wait for boot.ready before \
             attachment.fetch"
                .into(),
        )
    })?;
    let hit = read
        .get(content_hash.to_hex())
        .await
        .map_err(|e| ServiceError::Internal(format!("inline image get: {e}")))?;
    let Some((bytes, _mime)) = hit else { return Ok(None) };
    let materialized =
        attachment_materialize::write_bytes_to_tmp(boot_state, content_hash, bytes).await?;
    Ok(Some(materialized))
}

pub(crate) async fn handle_fetch(
    boot_state: &Arc<BootSharedState>,
    params: AttachmentFetchParams,
) -> Result<Value, ServiceError> {
    let key = boot_state.encryption_key().ok_or_else(|| {
        ServiceError::Internal(
            "encryption key not loaded; UI must wait for boot.ready before calling \
             attachment.fetch"
                .into(),
        )
    })?;
    let write_db = boot_state.write_db_state()?;
    let read_db = write_db.to_read_state();

    // 1. Look up the attachment row to find out whether we already
    // have a content_hash for it (cache hit) or need to fetch from
    // the provider (cache miss).
    let lookup_account = params.account_id.clone();
    let lookup_message = params.message_id.clone();
    let lookup_attachment = params.attachment_id.clone();
    let info = read_db
        .with_conn(move |conn| {
            db::db::queries_extra::find_attachment_cache_info(
                conn,
                &lookup_account,
                &lookup_message,
                &lookup_attachment,
            )
        })
        .await
        .map_err(ServiceError::Internal)?;

    if let Some(ref info) = info
        && let Some(content_hash) = info.content_hash
    {
        // Cache hit: bytes already live either in PackStore (the
        // common case) or in `inline_images.db` (small CID images;
        // provider sync writes them straight to that store without
        // ever consulting PackStore). The inline image store is a
        // sibling tier, not a PackStore impl, so we check it first
        // for is_inline rows before going to the pack.
        let materialized = if info.is_inline {
            match try_inline_image_materialize(boot_state, &content_hash).await? {
                Some(m) => m,
                None => {
                    // is_inline row but no row in inline_images -
                    // could be a stale sync that never persisted.
                    // Fall through to PackStore which will surface
                    // the absence as ServiceError::Internal.
                    attachment_materialize::materialize_blob(boot_state, &content_hash).await?
                }
            }
        } else {
            attachment_materialize::materialize_blob(boot_state, &content_hash).await?
        };
        let MaterializedBlob {
            path: _,
            relative_path,
            size_bytes,
        } = materialized;

        if should_enqueue_extraction(info.extraction_status.as_deref()) {
            enqueue_extraction_if_runtime_installed(
                boot_state,
                crate::extract::ExtractWork {
                    content_hash,
                    account_id:   params.account_id.clone(),
                    message_id:   params.message_id.clone(),
                    attachment_id: info.id.clone(),
                },
            );
        }
        return serde_json::to_value(AttachmentFetchAck {
            content_hash: content_hash.to_hex(),
            size_bytes,
            relative_path,
        })
        .map_err(|e| ServiceError::Internal(e.to_string()));
    }

    // 2. Cache miss: provider fetch, hash, PackStore::put, update the
    // attachments row, then materialize.
    let provider_attachment_id = info
        .as_ref()
        .and_then(|info| info.remote_attachment_id.as_deref())
        .unwrap_or(&params.attachment_id)
        .to_string();
    let local_attachment_id = info
        .as_ref()
        .map_or_else(|| params.attachment_id.clone(), |info| info.id.clone());

    let provider =
        crate::actions::provider::create_provider(&read_db, &params.account_id, key)
            .await
            .map_err(ServiceError::Internal)?;

    let provider_ctx = common::types::ProviderCtx {
        account_id: &params.account_id,
        db: &read_db,
        progress: &db::progress::NoopProgressReporter,
    };
    let attachment = provider
        .fetch_attachment(&provider_ctx, &params.message_id, &provider_attachment_id)
        .await
        .map_err(|e| ServiceError::Internal(format!("provider fetch_attachment: {e}")))?;
    let bytes = attachment.bytes;

    let pack_store = boot_state.pack_store().ok_or_else(|| {
        ServiceError::Internal(
            "pack store not installed; UI must wait for boot.ready before calling \
             attachment.fetch"
                .into(),
        )
    })?;
    let content_hash = pack_store
        .put(bytes)
        .await
        .map_err(|e| ServiceError::Internal(format!("PackStore::put: {e}")))?;

    if let Some(info) = info {
        let id = info.id;
        write_db
            .with_conn(move |conn| {
                db::db::queries_extra::update_attachment_cache_fields(
                    conn,
                    &id,
                    &content_hash,
                )
            })
            .await
            .map_err(ServiceError::Internal)?;
    }

    let MaterializedBlob {
        path: _,
        relative_path,
        size_bytes,
    } = attachment_materialize::materialize_blob(boot_state, &content_hash).await?;

    enqueue_extraction_if_runtime_installed(
        boot_state,
        crate::extract::ExtractWork {
            content_hash,
            account_id:    params.account_id.clone(),
            message_id:    params.message_id.clone(),
            attachment_id: local_attachment_id,
        },
    );

    serde_json::to_value(AttachmentFetchAck {
        content_hash: content_hash.to_hex(),
        size_bytes,
        relative_path,
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Status-aware re-enqueue gate for the cache-hit path. Returns true
/// when extraction should be enqueued: null status (no
/// `attachment_extracted_text` row exists) or retry-eligible status
/// (`failed:transient`, `skipped:bytes_gone`, `skipped:timeout`).
/// Returns false for permanent statuses (`indexed`, `skipped:opaque`,
/// `skipped:encrypted`, `skipped:oversize`, `skipped:encoding`,
/// `skipped:empty`, `skipped:ocr`, `skipped:unknown_mime`,
/// `skipped:privacy`, `skipped:zipbomb`).
fn should_enqueue_extraction(status: Option<&str>) -> bool {
    match status {
        None => true,
        Some(s) => crate::text_extract::is_retry_eligible_status_str(s),
    }
}

/// Defensive enqueue. The ExtractRuntime is installed by
/// `spawn_post_ready_extract_startup`; if it's not yet installed
/// (boot still in flight) this is a logged no-op so `attachment.fetch`
/// still acks normally. Uses `try_enqueue` (non-blocking) instead of
/// the awaiting `enqueue`: attachment.fetch is on the user-facing UI
/// critical path; blocking on the worker's bounded mpsc capacity (256)
/// when a thundering-herd backfill is in flight could park the user's
/// fetch for tens of minutes. A missed enqueue self-heals on the next
/// hourly backfill kick.
fn enqueue_extraction_if_runtime_installed(
    boot_state: &Arc<crate::boot::BootSharedState>,
    work: crate::extract::ExtractWork,
) {
    let Some(runtime) = boot_state.extract_runtime() else {
        log::debug!(
            "attachment.fetch: ExtractRuntime not installed; skipping enqueue \
             for hash {} (boot in flight or shutting down)",
            work.content_hash,
        );
        return;
    };
    let hash_for_log = work.content_hash;
    if let Err(e) = runtime.try_enqueue(work) {
        log::debug!("attachment.fetch try_enqueue {hash_for_log}: {e}");
    }
}

/// `attachment.eviction_kick` notification handler.
///
/// Retained as a no-op for wire compatibility. The flat-cache LRU
/// sweep retired in attachments roadmap Phase 3 along with the
/// `local_path` / `cached_at` / `cache_size` columns. Phase 8 reuses
/// the same notification variant for date-windowed tombstoning on
/// PackStore.
/// `attachment.cache_size` request handler. Single SQL aggregate over
/// `attachment_blobs.length`, partitioned by tombstone state.
///
/// Snapshot semantics: the values reflect the SQLite index at the
/// instant of the query. Concurrent `PackStore::put` or `tombstone`
/// calls move the truth out from under the response - acceptable
/// since the settings-UI readout is informational.
pub(crate) async fn handle_cache_size(
    boot_state: &Arc<BootSharedState>,
) -> Result<Value, ServiceError> {
    let pack_store = boot_state
        .pack_store()
        .ok_or_else(|| ServiceError::Internal("PackStore not installed".into()))?;
    let (live_bytes, tombstoned_bytes) = pack_store
        .size_breakdown()
        .await
        .map_err(|e| ServiceError::Internal(format!("attachment.cache_size: {e}")))?;
    let ack = AttachmentCacheSizeAck { live_bytes, tombstoned_bytes };
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Attachments roadmap Phase 8c: bulk-tombstone every live blob and
/// chain a GC pass to physically reclaim the bytes. Surface for the
/// settings-UI "Clear cache now" button. Blocks on completion so the
/// ack carries accurate post-action numbers.
pub(crate) async fn handle_clear_cache(
    boot_state: &Arc<BootSharedState>,
) -> Result<Value, ServiceError> {
    let pack_store = boot_state
        .pack_store()
        .ok_or_else(|| ServiceError::Internal("PackStore not installed".into()))?;
    let blobs_tombstoned = pack_store
        .tombstone_all_live()
        .await
        .map_err(|e| ServiceError::Internal(format!("attachment.clear_cache tombstone: {e}")))?;
    // Snapshot tombstoned bytes pre-GC; gc_stats.bytes_reclaimed
    // tells us how many of those bytes the compaction physically
    // dropped. Both are reported to the UI; consumers can choose
    // which one to surface.
    let notification_tx = boot_state
        .notification_sender()
        .ok_or_else(|| {
            ServiceError::Internal("notification sender not installed".into())
        })?;
    let gc_stats = crate::gc::run_gc_pass(
        pack_store,
        notification_tx,
        0,
        crate::gc::GcTrigger::PostEviction,
        crate::gc::DEFAULT_DENSITY_THRESHOLD,
    ).await;
    let ack = AttachmentClearCacheAck {
        blobs_tombstoned,
        bytes_reclaimed: gc_stats.bytes_reclaimed,
    };
    serde_json::to_value(ack).map_err(|e| ServiceError::Internal(e.to_string()))
}

pub(crate) async fn handle_eviction_kick(
    _boot_state: &Arc<BootSharedState>,
) -> Result<(), String> {
    log::debug!(
        "attachment.eviction_kick: no-op (Phase 3 retired the LRU sweep; Phase 8 reuses)"
    );
    Ok(())
}

/// `attachment.tmp_cleanup_kick` notification handler.
///
/// Walks `<app_data>/attachment_fetch_tmp/` and unlinks entries whose
/// mtime is older than 10 minutes. `Drop` class: a missed kick
/// self-heals on the next `SyncTick`.
pub(crate) async fn handle_tmp_cleanup_kick(
    boot_state: &Arc<BootSharedState>,
) -> Result<(), String> {
    const MAX_AGE_SECS: u64 = 10 * 60;
    let reaped =
        crate::attachment_materialize::reap_stale_tmp_files(boot_state, MAX_AGE_SECS).await?;
    if reaped > 0 {
        log::info!("[attachment.tmp_cleanup_kick] unlinked {reaped} stale tmp files");
    }
    Ok(())
}
