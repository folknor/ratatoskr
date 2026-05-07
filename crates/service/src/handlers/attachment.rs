//! `attachment.fetch` handler (Phase 6b).
//!
//! Wire ack carries `{ content_hash, size_bytes, relative_path }`.
//! Bytes never cross the IPC (phase-1.5-plan.md backpressure
//! policy). The Service-side window between metadata check and
//! IPC ack is closed by serializing fetches against eviction via
//! `SWEEP_LOCK`. The UI-side window between IPC ack
//! receipt and `open()` remains open until lease semantics land
//! (Phase 1a's `PackStore::get_with_lease`); UI consumers should
//! treat ENOENT on open as a transient miss and re-call
//! `attachment.fetch`.
//!
//! Cache hit: lookup the row's `content_hash`, verify the file
//! exists at `attachment_cache/<content_hash>`, bump `cached_at`
//! so the LRU sweep sees this attachment as recently used, return.
//!
//! Cache miss: build a provider via the shared
//! `service::actions::provider::create_provider` (same path the
//! action service uses), call `ProviderOps::fetch_attachment` to get
//! the base64 bytes, decode + hash, stage at `<hash>.tmp` (invisible
//! to the orphan sweep because it skips `.tmp` suffixes), update the
//! attachments row, then rename `.tmp` -> final. The order ensures
//! that a sweep racing the commit either (a) sees the committed row
//! and skips the file as referenced, or (b) sees only `.tmp` and
//! skips the suffix.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use service_api::{AttachmentFetchAck, AttachmentFetchParams, ServiceError};

use crate::attachment_lock::SWEEP_LOCK;
use crate::boot::BootSharedState;

const CACHE_DIR: &str = "attachment_cache";

/// Default cache cap when no `attachment_cache_max_mb` setting is
/// stored. Phase 6b carries 5 GB; the existing
/// `attachment_cache::attachment_cache_max_bytes` defaulted to
/// 500 MB, but that was a UI-side render-cache limit. The
/// Service-side eviction sweep treats the cache as a long-tail
/// store across enterprise mailboxes (CLAUDE.md notes 150+ GB
/// uncapped) and 5 GB is the ratified post-Phase-1a cap; the flat-
/// cache transition pass before pack-aware reads also runs against
/// 5 GB.
const DEFAULT_CACHE_CAP_MB: i64 = 5 * 1024;

/// Per-kick reclaim ceiling. A 50 GB cache reduction across one
/// `SyncTick` would stall the request lane; bound the work and let
/// subsequent ticks chip away. The cache reduces incrementally over
/// at most ~250 ticks (~21 hours at the 5-min cadence) for a worst-
/// case 50 GB drop, which is acceptable on the rare cap-flip path.
const PER_KICK_RECLAIM_CAP_BYTES: i64 = 200 * 1024 * 1024;

// Phase 7-4: SWEEP_LOCK relocated to crates/service/src/attachment_lock.rs
// so the ExtractRuntime worker can share it. Imported above.

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
    let app_data = boot_state.app_data_dir().to_path_buf();

    // 1. Cache hit: row already has a content_hash and the file
    // exists on disk. Return immediately.
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
        && let Some(ref content_hash) = info.content_hash
    {
        // Defensive: hex-only hashes (xxh3 emits 16-char hex via
        // hash_bytes). A corrupted/migrated row containing path
        // separators would escape the cache dir; rejecting here
        // avoids that even though it should not happen in practice.
        if !content_hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(ServiceError::Internal(format!(
                "rejecting non-hex content_hash: {content_hash}"
            )));
        }
        // Hold the eviction lock for the metadata check + cached_at
        // bump + ack serialization so eviction cannot unlink the
        // file underneath us before we ack. The UI-side window
        // between ack and open remains open until lease semantics
        // land - documented at the module level.
        let _hit_guard = SWEEP_LOCK.read().await;
        let file_path = app_data.join(CACHE_DIR).join(content_hash);
        if let Ok(meta) = std::fs::metadata(&file_path) {
            let bump_id = info.id.clone();
            let _ = write_db
                .with_conn(move |conn| {
                    db::db::queries_extra::bump_attachment_cached_at(conn, &bump_id)
                })
                .await;
            // Phase 7-5: cache-hit enqueue for extraction. Skip if
            // the row already has a permanent extraction status
            // (indexed or skipped:<permanent>); enqueue if null or
            // retry-eligible. The ExtractRuntime's worker pre-flight
            // does its own status-aware idempotency check too -
            // belt-and-suspenders.
            if should_enqueue_extraction(info.extraction_status.as_deref()) {
                enqueue_extraction_if_runtime_installed(
                    boot_state,
                    crate::extract::ExtractWork {
                        content_hash: content_hash.clone(),
                        account_id:   params.account_id.clone(),
                        message_id:   params.message_id.clone(),
                        attachment_id: params.attachment_id.clone(),
                    },
                );
            }
            return serde_json::to_value(AttachmentFetchAck {
                content_hash: content_hash.clone(),
                size_bytes: meta.len(),
                relative_path: format!("{CACHE_DIR}/{content_hash}"),
            })
            .map_err(|e| ServiceError::Internal(e.to_string()));
        }
    }

    // 2. Cache miss: dispatch provider.fetch_attachment, stage the
    // bytes at <hash>.tmp, update the row, then rename to final.
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
        .fetch_attachment(&provider_ctx, &params.message_id, &params.attachment_id)
        .await
        .map_err(|e| ServiceError::Internal(format!("provider fetch_attachment: {e}")))?;

    let bytes = store::attachment_cache::decode_base64(&attachment.data)
        .map_err(ServiceError::Internal)?;
    let content_hash = store::attachment_cache::hash_bytes(&bytes);
    #[allow(clippy::cast_possible_wrap)]
    let cache_size = bytes.len() as i64;
    let size_bytes = bytes.len() as u64;
    let relative_path = format!("{CACHE_DIR}/{content_hash}");

    // Hold the eviction lock for the (stage -> commit-row -> rename)
    // span so a sweep racing the commit observes either the
    // committed row (and skips as referenced) or only the `.tmp`
    // file (and skips the suffix).
    let _miss_guard = SWEEP_LOCK.read().await;

    let tmp_path =
        store::attachment_cache::write_cached_tmp(&app_data, &content_hash, &bytes)
            .map_err(ServiceError::Internal)?;

    if let Some(info) = info {
        let id = info.id;
        let local_path_for_db = relative_path.clone();
        let hash_for_db = content_hash.clone();
        write_db
            .with_conn(move |conn| {
                db::db::queries_extra::update_attachment_cache_fields(
                    conn,
                    &id,
                    &local_path_for_db,
                    cache_size,
                    &hash_for_db,
                )
            })
            .await
            .map_err(ServiceError::Internal)?;
    }

    // Rename only after the row update is committed. If the file
    // already existed (shared blob), `tmp_path` is None and there
    // is nothing to rename.
    if let Some(tmp) = tmp_path {
        store::attachment_cache::commit_cached_tmp(&app_data, &tmp, &content_hash)
            .map_err(ServiceError::Internal)?;
    }

    // Phase 7-5: cache-miss enqueue for extraction. The bytes are
    // freshly committed to the cache; ExtractRuntime worker reads
    // them via the same SWEEP_LOCK to defend against eviction.
    enqueue_extraction_if_runtime_installed(
        boot_state,
        crate::extract::ExtractWork {
            content_hash:  content_hash.clone(),
            account_id:    params.account_id.clone(),
            message_id:    params.message_id.clone(),
            attachment_id: params.attachment_id.clone(),
        },
    );

    serde_json::to_value(AttachmentFetchAck {
        content_hash,
        size_bytes,
        relative_path,
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// Phase 7-5: status-aware re-enqueue gate for the cache-hit path.
/// Returns true when extraction should be enqueued: null status (no
/// `attachment_extracted_text` row exists) or retry-eligible status
/// (`failed:transient`, `skipped:bytes_gone`, `skipped:timeout`).
/// Returns false for permanent statuses (`indexed`, `skipped:opaque`,
/// `skipped:encrypted`, `skipped:oversize`, `skipped:encoding`,
/// `skipped:empty`, `skipped:ocr`, `skipped:unknown_mime`,
/// `skipped:privacy`, `skipped:zipbomb`).
fn should_enqueue_extraction(status: Option<&str>) -> bool {
    // L1 fix: delegate to text_extract's centralised partition so all
    // three call sites (here, extract.rs::is_permanent_status, and
    // SkipReason::is_retry_eligible) share one definition.
    match status {
        None => true,
        Some(s) => crate::text_extract::is_retry_eligible_status_str(s),
    }
}

/// Phase 7-5: defensive enqueue. The ExtractRuntime is installed
/// by `spawn_post_ready_extract_startup`; if it's not yet installed
/// (boot still in flight) this is a logged no-op so `attachment.fetch`
/// still acks normally.
///
/// L5 fix: uses `try_enqueue` (non-blocking) instead of the awaiting
/// `enqueue`. attachment.fetch is on the user-facing UI critical path;
/// blocking on the worker's bounded mpsc capacity (256) when a
/// thundering-herd backfill is in flight could park the user's fetch
/// for tens of minutes. A missed enqueue self-heals on the next
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
    let hash_for_log = work.content_hash.clone();
    if let Err(e) = runtime.try_enqueue(work) {
        log::debug!("attachment.fetch try_enqueue {hash_for_log}: {e}");
    }
}

/// `attachment.eviction_kick` notification handler.
///
/// Single global sweep gated by `SWEEP_LOCK`. Two phases:
///
/// 1. **Orphan-first.** Walk `attachment_cache/`; drop any file
///    whose `content_hash` does not appear in
///    `attachments.content_hash`. These are leftovers from a prior
///    crash or partial cleanup; they reclaim disk without touching
///    user-visible state.
/// 2. **LRU eviction.** If still over cap, drop attachments rows in
///    `cached_at` order until under cap or the per-kick reclaim
///    budget (`PER_KICK_RECLAIM_CAP_BYTES`, default 200 MB) is
///    spent. A 50 GB cache reduction reduces incrementally over
///    subsequent ticks rather than stalling the request lane.
///
/// `Drop` class: missed kicks self-heal on the next `SyncTick`.
pub(crate) async fn handle_eviction_kick(
    boot_state: &Arc<BootSharedState>,
) -> Result<(), String> {
    let _guard = SWEEP_LOCK.write().await;

    let write_db = boot_state
        .write_db_state()
        .map_err(|e| format!("attachment.eviction_kick: {e}"))?;
    let read_db = write_db.to_read_state();
    let app_data = boot_state.app_data_dir().to_path_buf();

    let max_bytes = read_db
        .with_conn(|conn| {
            let raw = db::db::queries::get_setting(conn, "attachment_cache_max_mb")
                .unwrap_or(None);
            let max_mb = raw
                .as_deref()
                .and_then(|s| s.parse::<i64>().ok())
                .unwrap_or(DEFAULT_CACHE_CAP_MB);
            Ok(max_mb.saturating_mul(1024 * 1024))
        })
        .await?;

    let referenced = read_db
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT content_hash FROM attachments \
                     WHERE content_hash IS NOT NULL",
                )
                .map_err(|e| format!("eviction prepare: {e}"))?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(|e| format!("eviction query: {e}"))?;
            let hashes: std::collections::HashSet<String> =
                rows.filter_map(Result::ok).collect();
            Ok(hashes)
        })
        .await?;

    let mut reclaimed: i64 = 0;
    reclaimed += sweep_orphans(&app_data, &referenced, PER_KICK_RECLAIM_CAP_BYTES)?;

    let remaining = PER_KICK_RECLAIM_CAP_BYTES.saturating_sub(reclaimed);
    if remaining > 0 {
        reclaimed += sweep_lru(&write_db, &read_db, &app_data, max_bytes, remaining).await?;
    }

    log::info!("[attachment.eviction_kick] reclaimed {reclaimed} bytes");
    Ok(())
}

/// Phase 1 of the sweep. Walks `attachment_cache/`, drops files whose
/// content_hash is not referenced by any `attachments` row. Bounded
/// by `budget` bytes reclaimed.
fn sweep_orphans(
    app_data: &Path,
    referenced: &std::collections::HashSet<String>,
    budget: i64,
) -> Result<i64, String> {
    let cache_dir = app_data.join(CACHE_DIR);
    let mut reclaimed: i64 = 0;
    let entries = match std::fs::read_dir(&cache_dir) {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("read cache dir: {e}")),
    };
    for entry in entries.flatten() {
        if reclaimed >= budget {
            break;
        }
        let path = entry.path();
        let Some(stem) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // Skip in-flight cache-miss writes staged at `<hash>.tmp` by
        // `write_cached_tmp`. The handler renames `.tmp` -> final only
        // after committing the row, but defense-in-depth: even if a
        // sweep races outside the eviction lock, the suffix marker
        // keeps the orphan walk from collecting half-written files.
        if path.extension().and_then(|s| s.to_str()) == Some("tmp") {
            continue;
        }
        if referenced.contains(stem) {
            continue;
        }
        let Ok(meta) = path.metadata() else { continue };
        #[allow(clippy::cast_possible_wrap)]
        let size = meta.len() as i64;
        match std::fs::remove_file(&path) {
            Ok(()) => reclaimed = reclaimed.saturating_add(size),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!(
                "[attachment.eviction_kick] orphan unlink {} failed: {e}",
                path.display(),
            ),
        }
    }
    Ok(reclaimed)
}

/// Phase 2 of the sweep. Evicts cached `attachments` rows in
/// `cached_at` order until total cache size is under `max_bytes`
/// or `budget` reclaim has been spent (whichever first).
async fn sweep_lru(
    write_db: &service_state::WriteDbState,
    read_db: &db::db::ReadDbState,
    app_data: &Path,
    max_bytes: i64,
    budget: i64,
) -> Result<i64, String> {
    let total = read_db
        .with_conn(|conn| {
            db::db::queries_extra::get_total_cached_attachment_size(conn)
        })
        .await?;
    if total <= max_bytes {
        return Ok(0);
    }
    let target = (total - max_bytes).min(budget);

    let candidates = read_db
        .with_conn(db::db::queries_extra::get_cached_attachments_oldest_first)
        .await?;

    let mut freed: i64 = 0;
    let mut ids_to_clear: Vec<String> = Vec::new();
    let mut paths_with_hashes: Vec<(String, Option<String>)> = Vec::new();
    for row in candidates {
        if freed >= target {
            break;
        }
        freed = freed.saturating_add(row.cache_size);
        ids_to_clear.push(row.attachment_id);
        paths_with_hashes.push((row.local_path, row.content_hash));
    }

    if ids_to_clear.is_empty() {
        return Ok(0);
    }

    let clear_ids = ids_to_clear.clone();
    write_db
        .with_conn(move |conn| {
            db::db::queries_extra::clear_attachment_cache_fields_batch(conn, &clear_ids)
        })
        .await?;

    for (local_path, content_hash) in paths_with_hashes {
        let still_referenced = if let Some(hash) = content_hash {
            let h = hash.clone();
            read_db
                .with_conn(move |conn| {
                    db::db::queries_extra::count_cached_attachment_refs(conn, &h)
                })
                .await
                .map(|n| n > 0)
                .unwrap_or(false)
        } else {
            false
        };
        if !still_referenced
            && let Err(e) = store::attachment_cache::remove_cached_relative(app_data, &local_path)
        {
            log::warn!(
                "[attachment.eviction_kick] LRU unlink {local_path} failed: {e}",
            );
        }
    }

    Ok(freed)
}
