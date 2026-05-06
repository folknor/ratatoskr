//! `attachment.fetch` handler (Phase 6b).
//!
//! Wire ack carries `{ content_hash, size_bytes, relative_path }`.
//! Bytes never cross the IPC (phase-1.5-plan.md backpressure
//! policy). On the flat cache, the open fd is the pin against
//! concurrent eviction - Linux `unlink` does not invalidate
//! already-open fds, so a UI process holding the cache file open
//! survives a concurrent sweep. When pack-aware reads land
//! (Phase 1a), this handler grows lease semantics + a
//! `PackStore::get_with_lease` API; that revision pass swaps the
//! wire shape and adds a `lease_id` field. Until then, no leases.
//!
//! Cache hit: lookup the row's `content_hash`, verify the file
//! exists at `attachment_cache/<content_hash>`, return immediately.
//!
//! Cache miss: build a provider via the shared
//! `service::actions::provider::create_provider` (same path the
//! action service uses), call `ProviderOps::fetch_attachment` to get
//! the base64 bytes, decode + hash + `write_cached` + update the
//! attachments row's cache columns, then return.

use std::path::Path;
use std::sync::Arc;

use serde_json::Value;
use service_api::{AttachmentFetchAck, AttachmentFetchParams, ServiceError};
use tokio::sync::Mutex;

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

/// Single global sweep lock. A slow sweep on one tick is not re-
/// entered when the next tick lands within `NOTIFY_CAP=4` queued
/// kicks; the second kick acquires the lock once the first
/// finishes (or, more commonly, is dropped at the
/// `try_lock`-equivalent if we wanted; today we use a regular
/// `Mutex` and let queued kicks await sequentially - the work is
/// idempotent so back-to-back sweeps just see the cache already
/// under cap and return immediately).
static ATTACHMENT_SWEEP_LOCK: Mutex<()> = Mutex::const_new(());

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
        let file_path = app_data.join(CACHE_DIR).join(content_hash);
        if let Ok(meta) = std::fs::metadata(&file_path) {
            return serde_json::to_value(AttachmentFetchAck {
                content_hash: content_hash.clone(),
                size_bytes: meta.len(),
                relative_path: format!("{CACHE_DIR}/{content_hash}"),
            })
            .map_err(|e| ServiceError::Internal(e.to_string()));
        }
    }

    // 2. Cache miss: dispatch provider.fetch_attachment, write the
    // bytes to the flat cache, update the row, return.
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
    let relative_path = store::attachment_cache::write_cached(&app_data, &content_hash, &bytes)
        .map_err(ServiceError::Internal)?;
    #[allow(clippy::cast_possible_wrap)]
    let cache_size = bytes.len() as i64;
    let size_bytes = bytes.len() as u64;

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

    serde_json::to_value(AttachmentFetchAck {
        content_hash,
        size_bytes,
        relative_path,
    })
    .map_err(|e| ServiceError::Internal(e.to_string()))
}

/// `attachment.eviction_kick` notification handler.
///
/// Single global sweep gated by `ATTACHMENT_SWEEP_LOCK`. Two phases:
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
    let _guard = ATTACHMENT_SWEEP_LOCK.lock().await;

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
