//! Service-side cross-store invariant pass.
//!
//! Runs at boot when the clean-shutdown sentinel is missing AND the
//! `sync_markers/` directory contains markers whose `status` is
//! anything other than `completed` (i.e., `in_progress`, `cancelled`,
//! `failed`, or unparseable). Each such marker becomes one
//! `DirtyAccount` entry; the pass:
//!
//! 1. Per dirty account: calls `clear_account_history_id`. This is the
//!    load-bearing repair: the next JMAP delta sync becomes
//!    initial-style and re-fetches the cached window from the
//!    provider, repopulating body / inline / search regardless of
//!    which leg was partial. Per-row repair was rejected because the
//!    provider cursor is the only durable authority after a partial
//!    cross-store write.
//! 2. Per dirty account: iterates the Tantivy index for that
//!    account_id, drops docs whose message_id is no longer in
//!    `messages`. Bounded by per-account scope; defense-in-depth -
//!    the cursor-clear plus next initial-style sync repopulates the
//!    index regardless.
//! 3. Per dirty account: unlinks the marker file. Subsequent boots
//!    without further sync activity see no marker and skip the pass
//!    entirely.
//! 4. Globally (gated on dirty-account presence): Phase 8-2
//!    cursor-bounded sweeps over body / inline / extracted_text
//!    stores. Each store has a cursor in `clean_shutdown_cursors`
//!    advanced on the previous graceful drain; the sweep scans only
//!    rows added since that cursor. Bounds the per-store scan to a
//!    known budget on a 200 GB mailbox.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Instant;

use search::SearchReadState;
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};

use crate::sync::{MarkerStatus, SyncMarker};
use service_api::SyncRunId;

/// One account's worth of dirty-marker state, derived from a file in
/// `<app_data>/sync_markers/`. Anything other than
/// `MarkerStatus::Completed` (and any unparseable file) is dirty.
#[derive(Debug, Clone)]
pub struct DirtyAccount {
    pub account_id: String,
    pub run_id: Option<SyncRunId>,
    pub status: DirtyStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DirtyStatus {
    /// Parsed marker; carries the on-disk status (always non-completed
    /// because `MarkerStatus::Completed` is filtered out at discovery).
    Parsed(MarkerStatus),
    /// Marker file failed to deserialize. Treated as fully-dirty per
    /// the plan's "boot-side parse-failure rule".
    Unparseable,
}

#[derive(Debug, Default)]
pub struct InvariantPassStats {
    pub history_ids_cleared: u64,
    pub body_orphans_dropped: u64,
    pub inline_orphans_dropped: u64,
    /// Phase 8-2: Tantivy docs whose `messages` row was deleted in a
    /// non-graceful exit window. Per-account scope.
    pub search_orphans_dropped: u64,
    /// Phase 8-2: `attachment_extracted_text` rows whose `content_hash`
    /// is no longer referenced by any `attachments` row.
    pub extracted_text_orphans_dropped: u64,
    pub elapsed_ms: u128,
    /// Phase 8-2: per-store elapsed times so the <5s typical / <30s
    /// 200 GB exit criterion is observable in production logs.
    pub body_scan_ms: u128,
    pub inline_scan_ms: u128,
    pub search_scan_ms: u128,
    pub extracted_text_scan_ms: u128,
}

fn marker_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("sync_markers")
}

/// Read every file in `<app_data>/sync_markers/` and build the dirty
/// account list. Files whose `status == Completed` are filtered out
/// (drain unlinks them, but a partial-write or hand-edited file could
/// still surface one). Files that fail to deserialize are surfaced as
/// `DirtyStatus::Unparseable`.
pub async fn discover_dirty_accounts(app_data_dir: &Path) -> Vec<DirtyAccount> {
    let dir = marker_dir(app_data_dir);
    let mut entries = match tokio::fs::read_dir(&dir).await {
        Ok(rd) => rd,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Vec::new(),
        Err(e) => {
            log::warn!(
                "invariant pass: failed to read sync_markers dir {}: {e}",
                dir.display()
            );
            return Vec::new();
        }
    };

    let mut dirty: Vec<DirtyAccount> = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        // Skip the temp files written during the temp-then-rename atomic
        // write so a crash mid-rename doesn't surface a half-written
        // marker. The atomic write uses `<account_id>.json.tmp`.
        if !file_name.ends_with(".json") || file_name.ends_with(".json.tmp") {
            continue;
        }
        let account_id = file_name.trim_end_matches(".json").to_string();
        if account_id.is_empty() {
            continue;
        }
        match tokio::fs::read(&path).await {
            Ok(bytes) => match serde_json::from_slice::<SyncMarker>(&bytes) {
                Ok(marker) => {
                    if matches!(marker.status, MarkerStatus::Completed) {
                        continue;
                    }
                    dirty.push(DirtyAccount {
                        account_id,
                        run_id: Some(marker.run_id),
                        status: DirtyStatus::Parsed(marker.status),
                    });
                }
                Err(e) => {
                    log::warn!(
                        "invariant pass: marker {} unparseable ({e}); treating as fully-dirty",
                        path.display()
                    );
                    dirty.push(DirtyAccount {
                        account_id,
                        run_id: None,
                        status: DirtyStatus::Unparseable,
                    });
                }
            },
            Err(e) => {
                log::warn!(
                    "invariant pass: failed to read marker {}: {e}",
                    path.display()
                );
            }
        }
    }
    dirty
}

/// Run the invariant pass against the supplied dirty accounts.
/// Idempotent; safe to call with an empty `dirty_accounts` slice
/// (returns immediately).
///
/// Order:
/// 1. Per dirty account: clear `history_id`, drop Tantivy orphans
///    scoped to that account, unlink marker.
/// 2. Globally (gated on dirty-account presence): cursor-bounded
///    sweeps over body / inline / extracted_text stores. Each store
///    has a cursor in `clean_shutdown_cursors`; the sweep scans only
///    rows added since that cursor. Bounds the per-store scan to a
///    known budget on a 200 GB mailbox.
///
/// Errors at any step are logged at warn and the pass continues -
/// the work is independent and a failure in one slice must not
/// block the rest.
#[allow(clippy::too_many_arguments)]
pub async fn run_invariant_pass(
    db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    inline_write: &InlineImageStoreWriteState,
    search_write: &SearchWriteHandle,
    search_read: Option<&SearchReadState>,
    app_data_dir: &Path,
    dirty_accounts: &[DirtyAccount],
) -> InvariantPassStats {
    let started = Instant::now();
    let mut stats = InvariantPassStats::default();

    if dirty_accounts.is_empty() {
        stats.elapsed_ms = started.elapsed().as_millis();
        return stats;
    }

    log::info!(
        "invariant pass: processing {} dirty account(s)",
        dirty_accounts.len()
    );

    // Read all cursors once. Failure to read falls back to 0 (scan
    // everything) - the cursor is an optimization hint, not a
    // correctness gate.
    let body_cursor = db
        .with_write(|c| ::db::db::queries_extra::get_clean_shutdown_cursor(c, "body"))
        .await
        .unwrap_or(0);
    let inline_cursor = db
        .with_write(|c| ::db::db::queries_extra::get_clean_shutdown_cursor(c, "inline"))
        .await
        .unwrap_or(0);
    let extract_cursor = db
        .with_write(|c| ::db::db::queries_extra::get_clean_shutdown_cursor(c, "extract"))
        .await
        .unwrap_or(0);
    log::debug!(
        "invariant pass: cursors body={body_cursor} inline={inline_cursor} extract={extract_cursor}"
    );

    // ── Per-account work ─────────────────────────────────────
    let search_started = Instant::now();
    for account in dirty_accounts {
        let account_id = account.account_id.clone();
        log::info!(
            "invariant pass: repairing account {account_id} (run_id={:?}, status={:?})",
            account.run_id,
            account.status,
        );

        // Clear JMAP cursor (load-bearing).
        let aid = account_id.clone();
        match db
            .with_write(move |conn| ::sync::pipeline::clear_account_history_id(conn, &aid))
            .await
        {
            Ok(()) => stats.history_ids_cleared += 1,
            Err(e) => {
                log::warn!("invariant pass: clear_account_history_id({account_id}) failed: {e}");
            }
        }

        // Tantivy orphan iteration scoped to this account. Skipped
        // if the SearchReadState was unavailable at boot - the
        // history_id-clear plus next initial-style sync still
        // repopulates the index.
        if let Some(search_read) = search_read {
            match drop_search_orphans(db, search_read, search_write, &account_id).await {
                Ok(n) => stats.search_orphans_dropped += n,
                Err(e) => {
                    log::warn!("invariant pass: search orphan drop for {account_id} failed: {e}");
                }
            }
        }

        // Unlink the marker now that per-account repair is complete.
        if let Err(e) = unlink_marker_file(app_data_dir, &account_id).await {
            log::warn!("invariant pass: failed to unlink marker for {account_id}: {e}");
        }
    }
    stats.search_scan_ms = search_started.elapsed().as_millis();

    // ── Global cursor-bounded sweeps ─────────────────────────
    let body_started = Instant::now();
    match drop_body_orphans(db, body_write, body_cursor).await {
        Ok(n) => stats.body_orphans_dropped = n,
        Err(e) => log::warn!("invariant pass: body orphan sweep failed: {e}"),
    }
    stats.body_scan_ms = body_started.elapsed().as_millis();

    let inline_started = Instant::now();
    match drop_inline_orphans(db, inline_write, inline_cursor).await {
        Ok(n) => stats.inline_orphans_dropped = n,
        Err(e) => log::warn!("invariant pass: inline orphan sweep failed: {e}"),
    }
    stats.inline_scan_ms = inline_started.elapsed().as_millis();

    let extract_started = Instant::now();
    match db
        .with_write(move |c| {
            ::db::db::queries_extra::delete_extracted_text_orphans_since(c, extract_cursor)
        })
        .await
    {
        Ok(n) => stats.extracted_text_orphans_dropped = n,
        Err(e) => log::warn!("invariant pass: extracted_text orphan sweep failed: {e}"),
    }
    stats.extracted_text_scan_ms = extract_started.elapsed().as_millis();

    stats.elapsed_ms = started.elapsed().as_millis();
    log::info!(
        "invariant pass: done in {}ms (history={}, body={}/{}ms, inline={}/{}ms, search={}/{}ms, extract={}/{}ms)",
        stats.elapsed_ms,
        stats.history_ids_cleared,
        stats.body_orphans_dropped,
        stats.body_scan_ms,
        stats.inline_orphans_dropped,
        stats.inline_scan_ms,
        stats.search_orphans_dropped,
        stats.search_scan_ms,
        stats.extracted_text_orphans_dropped,
        stats.extracted_text_scan_ms,
    );
    stats
}

/// Phase 8-2: drop body rows added since `cursor` whose `message_id`
/// is not in the main DB's `messages` table. Cursor-bounded so on a
/// 200 GB mailbox the scan only inspects rows written since the last
/// graceful drain.
async fn drop_body_orphans(
    db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    cursor: i64,
) -> Result<u64, String> {
    let candidates = list_body_message_ids_since(body_write, cursor).await?;
    if candidates.is_empty() {
        return Ok(0);
    }
    let orphans: Vec<String> = db
        .with_write(move |conn| {
            ::db::db::queries_extra::find_unreferenced_message_ids(conn, &candidates)
        })
        .await?;
    if orphans.is_empty() {
        return Ok(0);
    }
    let count = body_write.delete(orphans).await?;
    Ok(count)
}

async fn list_body_message_ids_since(
    body_write: &BodyStoreWriteState,
    cursor: i64,
) -> Result<Vec<String>, String> {
    body_write
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT message_id FROM bodies WHERE inserted_at > ?1")
                .map_err(|e| format!("prepare body ids: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![cursor], |r| r.get::<_, String>(0))
                .map_err(|e| format!("query body ids: {e}"))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| format!("collect body id: {e}"))?);
            }
            Ok(out)
        })
        .await
}

/// Phase 8-2: drop inline-image rows added since `cursor` whose
/// `content_hash` no longer has a referencing `attachments` row.
async fn drop_inline_orphans(
    db: &WriteDbState,
    inline_write: &InlineImageStoreWriteState,
    cursor: i64,
) -> Result<u64, String> {
    let hashes = list_inline_hashes_since(inline_write, cursor).await?;
    if hashes.is_empty() {
        return Ok(0);
    }
    let orphans: Vec<String> = db
        .with_write(move |conn| {
            ::store::inline_image_store::find_unreferenced_hashes(conn, &hashes)
        })
        .await?;
    if orphans.is_empty() {
        return Ok(0);
    }
    let n = inline_write.delete_hashes(orphans).await?;
    Ok(n)
}

async fn list_inline_hashes_since(
    inline_write: &InlineImageStoreWriteState,
    cursor: i64,
) -> Result<Vec<String>, String> {
    inline_write
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT content_hash FROM inline_images WHERE created_at > ?1")
                .map_err(|e| format!("prepare hash list: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![cursor], |r| r.get::<_, String>(0))
                .map_err(|e| format!("query hash list: {e}"))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| format!("collect hash: {e}"))?);
            }
            Ok(out)
        })
        .await
}

/// Phase 8-2: enumerate Tantivy docs for `account_id` and drop those
/// whose `message_id` is no longer in `messages`. Per-account scope
/// bounds the scan natively; the cursor-clear plus the next
/// initial-style sync repopulates the index regardless, so this is
/// defense-in-depth.
async fn drop_search_orphans(
    db: &WriteDbState,
    search_read: &SearchReadState,
    search_write: &SearchWriteHandle,
    account_id: &str,
) -> Result<u64, String> {
    let aid = account_id.to_string();
    let live: HashSet<String> = db
        .with_write(move |conn| ::db::db::queries_extra::list_message_ids_for_account(conn, &aid))
        .await?;
    let aid2 = account_id.to_string();
    let read_clone = search_read.clone();
    let orphans = tokio::task::spawn_blocking(move || {
        read_clone.find_orphan_message_ids_for_account(&aid2, &live)
    })
    .await
    .map_err(|e| format!("orphan iter join: {e}"))??;
    if orphans.is_empty() {
        return Ok(0);
    }
    let n = orphans.len() as u64;
    search_write
        .delete_messages_batch(orphans)
        .await
        .map_err(|e| format!("orphan delete: {e}"))?;
    Ok(n)
}

// Phase 3 of the attachments roadmap retired `reconcile_attachment_cache`
// along with the flat cache. PackStore's open-time recovery walks the
// open pack; orphan detection at the pack-blob level lands with Phase 8.

async fn unlink_marker_file(app_data_dir: &Path, account_id: &str) -> Result<(), String> {
    let path = marker_dir(app_data_dir).join(format!("{account_id}.json"));
    match tokio::fs::remove_file(&path).await {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!("remove marker {}: {e}", path.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use service_api::SyncRunId;
    use tempfile::tempdir;

    async fn write_marker(dir: &Path, account_id: &str, status: MarkerStatus) {
        let marker = SyncMarker {
            run_id: SyncRunId::new_v7(),
            started_at: 1234,
            kind: "delta".into(),
            status,
        };
        let markers = dir.join("sync_markers");
        tokio::fs::create_dir_all(&markers).await.expect("create");
        let bytes = serde_json::to_vec_pretty(&marker).expect("serialize");
        tokio::fs::write(markers.join(format!("{account_id}.json")), bytes)
            .await
            .expect("write");
    }

    #[tokio::test]
    async fn discover_returns_empty_when_dir_missing() {
        let dir = tempdir().expect("tempdir");
        let dirty = discover_dirty_accounts(dir.path()).await;
        assert!(dirty.is_empty());
    }

    #[tokio::test]
    async fn discover_skips_completed_markers() {
        let dir = tempdir().expect("tempdir");
        write_marker(dir.path(), "acc-clean", MarkerStatus::Completed).await;
        let dirty = discover_dirty_accounts(dir.path()).await;
        assert!(
            dirty.is_empty(),
            "completed markers must not surface as dirty"
        );
    }

    #[tokio::test]
    async fn discover_surfaces_in_progress_failed_cancelled_markers() {
        let dir = tempdir().expect("tempdir");
        write_marker(dir.path(), "acc-a", MarkerStatus::InProgress).await;
        write_marker(dir.path(), "acc-b", MarkerStatus::Failed).await;
        write_marker(dir.path(), "acc-c", MarkerStatus::Cancelled).await;
        write_marker(dir.path(), "acc-clean", MarkerStatus::Completed).await;
        let mut dirty: Vec<_> = discover_dirty_accounts(dir.path())
            .await
            .into_iter()
            .map(|d| d.account_id)
            .collect();
        dirty.sort();
        assert_eq!(
            dirty,
            vec![
                "acc-a".to_string(),
                "acc-b".to_string(),
                "acc-c".to_string()
            ]
        );
    }

    #[tokio::test]
    async fn discover_surfaces_unparseable_marker_as_fully_dirty() {
        let dir = tempdir().expect("tempdir");
        let markers = dir.path().join("sync_markers");
        tokio::fs::create_dir_all(&markers).await.expect("create");
        tokio::fs::write(markers.join("garbage.json"), b"not json")
            .await
            .expect("write");
        let dirty = discover_dirty_accounts(dir.path()).await;
        assert_eq!(dirty.len(), 1);
        assert_eq!(dirty[0].account_id, "garbage");
        assert!(matches!(dirty[0].status, DirtyStatus::Unparseable));
    }

    #[tokio::test]
    async fn discover_skips_tmp_files() {
        let dir = tempdir().expect("tempdir");
        let markers = dir.path().join("sync_markers");
        tokio::fs::create_dir_all(&markers).await.expect("create");
        tokio::fs::write(markers.join("foo.json.tmp"), b"")
            .await
            .expect("write");
        let dirty = discover_dirty_accounts(dir.path()).await;
        assert!(dirty.is_empty(), "in-progress temp files must be ignored");
    }
}
