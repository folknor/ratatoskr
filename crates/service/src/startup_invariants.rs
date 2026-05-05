//! Service-side cross-store invariant pass.
//!
//! Runs at boot when the clean-shutdown sentinel is missing AND the
//! `sync_markers/` directory contains markers whose `status` is
//! anything other than `completed` (i.e., `in_progress`, `cancelled`,
//! `failed`, or unparseable). Each such marker becomes one
//! `DirtyAccount` entry; the pass:
//!
//! 1. Calls `clear_account_history_id` for the account. This is the
//!    load-bearing repair: the next JMAP delta sync becomes
//!    initial-style and re-fetches the cached window from the
//!    provider, repopulating body / inline / search regardless of
//!    which leg was partial. (See § "Minimal cross-store invariant
//!    pass" in `docs/service/phase-3-plan.md` for why per-row repair
//!    was rejected.)
//! 2. Drops body-store and inline-image-store orphans whose
//!    `message_id` (or `content_hash`) has no surviving row in the
//!    main DB. Cheap; redundant with the cursor-clear (next sync
//!    would clean these up too) but avoids leaving stale data
//!    visible during the gap. Tantivy orphan iteration is deferred
//!    to Phase 8 - the cursor-clear is sufficient for correctness,
//!    and the Tantivy iterator helper that the doc walk would
//!    require is non-trivial.
//! 3. Unlinks the marker file. Subsequent boots without further sync
//!    activity see no marker and skip the pass entirely.

use std::path::{Path, PathBuf};
use std::time::Instant;

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
    pub elapsed_ms: u128,
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

/// Run the invariant pass against the supplied dirty accounts. Idempotent;
/// safe to call with an empty `dirty_accounts` slice (returns immediately).
///
/// Order per account: clear `history_id`, drop body orphans, drop inline
/// orphans, unlink marker. Errors at any step are logged at warn and the
/// pass continues - per-account work is independent and a failure on one
/// account must not block the rest.
#[allow(clippy::too_many_arguments)]
pub async fn run_invariant_pass(
    db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    inline_write: &InlineImageStoreWriteState,
    _search_write: &SearchWriteHandle,
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

    for account in dirty_accounts {
        let account_id = account.account_id.clone();
        log::info!(
            "invariant pass: repairing account {account_id} (status={:?})",
            account.status
        );

        // 1. Clear JMAP cursor (load-bearing).
        let aid = account_id.clone();
        match db
            .with_conn(move |conn| ::sync::pipeline::clear_account_history_id(conn, &aid))
            .await
        {
            Ok(()) => stats.history_ids_cleared += 1,
            Err(e) => log::warn!(
                "invariant pass: clear_account_history_id({account_id}) failed: {e}"
            ),
        }

        // 2. Body-store orphan drop. Cheap defense-in-depth.
        match drop_body_orphans(db, body_write, &account_id).await {
            Ok(n) => stats.body_orphans_dropped += n,
            Err(e) => log::warn!(
                "invariant pass: body orphan drop for {account_id} failed: {e}"
            ),
        }

        // 3. Inline-image orphan drop.
        match drop_inline_orphans(db, inline_write, &account_id).await {
            Ok(n) => stats.inline_orphans_dropped += n,
            Err(e) => log::warn!(
                "invariant pass: inline orphan drop for {account_id} failed: {e}"
            ),
        }

        // 4. Tantivy orphan drop is deferred to Phase 8: the cursor-
        //    clear plus the next initial-style sync repopulates the
        //    index from scratch for this account, and a doc-walk
        //    iterator on `SearchReadState` is the missing helper. For
        //    Phase 3 the cursor-clear is sufficient for correctness.

        // 5. Unlink the marker now that repair is complete. A future
        //    crash before the next sync still leaves a clean state -
        //    history_id cleared means the next sync re-fetches.
        if let Err(e) = unlink_marker_file(app_data_dir, &account_id).await {
            log::warn!(
                "invariant pass: failed to unlink marker for {account_id}: {e}"
            );
        }
    }

    stats.elapsed_ms = started.elapsed().as_millis();
    log::info!(
        "invariant pass: done in {}ms ({} cursor(s) cleared, {} body / {} inline orphan(s) dropped)",
        stats.elapsed_ms,
        stats.history_ids_cleared,
        stats.body_orphans_dropped,
        stats.inline_orphans_dropped,
    );
    stats
}

/// Find body rows whose `message_id` has no surviving row in the main
/// `messages` table for `account_id`, then delete them from the body
/// store. Bounded SQL: pulls the account's message ids into memory
/// (capped) and the body store's message ids similarly.
async fn drop_body_orphans(
    db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    account_id: &str,
) -> Result<u64, String> {
    use std::collections::HashSet;

    // Live message ids for the account.
    let aid = account_id.to_string();
    let live: HashSet<String> = db
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM messages WHERE account_id = ?1")
                .map_err(|e| format!("prepare live msgs: {e}"))?;
            let rows = stmt
                .query_map(rusqlite::params![aid], |r| r.get::<_, String>(0))
                .map_err(|e| format!("query live msgs: {e}"))?;
            let mut out = HashSet::new();
            for row in rows {
                out.insert(row.map_err(|e| format!("collect live msg: {e}"))?);
            }
            Ok(out)
        })
        .await?;

    // We can't filter the body store by account_id (no such column), so the
    // strategy is: walk the live set, find which body rows exist, and delete
    // bodies for messages that were live but the body store row is for a
    // message_id that's no longer in the main DB. Concretely: enumerate body
    // store ids, intersect with this-account ids that are gone. The body
    // store has no account scoping so we cannot tell which account a stray
    // row belonged to without a join; instead we pick orphans whose
    // message_id is not in *any* account's messages table.

    let body_ids = list_body_message_ids(body_write).await?;
    if body_ids.is_empty() {
        return Ok(0);
    }
    // Refetch the global live set so the orphan check is against all
    // accounts, not just the dirty one (a body row whose message_id
    // belongs to a different account is NOT an orphan).
    let global_live: HashSet<String> = db
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT id FROM messages")
                .map_err(|e| format!("prepare all msg ids: {e}"))?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| format!("query all msg ids: {e}"))?;
            let mut out = HashSet::new();
            for row in rows {
                out.insert(row.map_err(|e| format!("collect msg id: {e}"))?);
            }
            Ok(out)
        })
        .await?;
    let _ = live; // keep the per-account fetch as a sanity warm-up

    let orphans: Vec<String> = body_ids
        .into_iter()
        .filter(|id| !global_live.contains(id))
        .collect();
    if orphans.is_empty() {
        return Ok(0);
    }

    let count = body_write.delete(orphans).await?;
    Ok(count)
}

async fn list_body_message_ids(body_write: &BodyStoreWriteState) -> Result<Vec<String>, String> {
    body_write
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT message_id FROM bodies")
                .map_err(|e| format!("prepare body ids: {e}"))?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| format!("query body ids: {e}"))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| format!("collect body id: {e}"))?);
            }
            Ok(out)
        })
        .await
}

/// Drop inline-image rows whose `content_hash` no longer has a referencing
/// `attachments` row. Reuses `find_unreferenced_hashes` semantics but
/// operates over the entire inline store; the dirty-account gate decides
/// *whether* to run, not *what* to scan.
async fn drop_inline_orphans(
    db: &WriteDbState,
    inline_write: &InlineImageStoreWriteState,
    _account_id: &str,
) -> Result<u64, String> {
    let hashes = list_inline_hashes(inline_write).await?;
    if hashes.is_empty() {
        return Ok(0);
    }

    let orphans: Vec<String> = db
        .with_conn(move |conn| {
            ::store::inline_image_store::find_unreferenced_hashes(conn, &hashes)
        })
        .await?;
    if orphans.is_empty() {
        return Ok(0);
    }

    let n = inline_write.delete_hashes(orphans).await?;
    Ok(n)
}

async fn list_inline_hashes(
    inline_write: &InlineImageStoreWriteState,
) -> Result<Vec<String>, String> {
    inline_write
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT content_hash FROM inline_images")
                .map_err(|e| format!("prepare hash list: {e}"))?;
            let rows = stmt
                .query_map([], |r| r.get::<_, String>(0))
                .map_err(|e| format!("query hash list: {e}"))?;
            let mut out = Vec::new();
            for row in rows {
                out.push(row.map_err(|e| format!("collect hash: {e}"))?);
            }
            Ok(out)
        })
        .await
}

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
            vec!["acc-a".to_string(), "acc-b".to_string(), "acc-c".to_string()]
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

