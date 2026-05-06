//! Crash-safe account deletion using the shared marker helper.
//!
//! Phase 6b layered onto 6a-part-2's account.delete handler. Multi-
//! step recovery: each cleanup step is idempotent and the marker
//! tracks which steps have completed, so a crash mid-cleanup
//! resumes on next boot from the next un-completed step.
//!
//! Step ordering is fixed (CASCADE always last):
//!
//!   1. body store (drop blob rows for this account's message ids)
//!   2. inline image store (drop blob rows for unreferenced hashes)
//!   3. attachment file cache (unlink files for unreferenced hashes)
//!   4. search index (drop docs by message_id)
//!   5. accounts row CASCADE (the SQLite `DELETE FROM accounts ...`
//!      that fires the schema CASCADE; once this lands the external
//!      stores cannot be reverse-mapped by `account_id` anymore, so
//!      it must be the LAST step)
//!
//! Resume on boot reads the marker, identifies the next un-completed
//! step, runs forward. Each step must be idempotent (re-running on a
//! partially-finished step produces the same end state). Today's
//! external-store delete helpers are all idempotent (DELETEs that
//! match zero rows are no-ops; file unlinks tolerate `NotFound`).

use serde::{Deserialize, Serialize};
use service_state::{BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState};
use std::collections::HashSet;
use std::path::Path;

use crate::markers::MarkerFile;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AccountDeletionStep {
    Bodies,
    InlineImages,
    AttachmentCache,
    SearchIndex,
    AccountRowCascade,
}

impl AccountDeletionStep {
    /// Canonical execution order. Resume on boot walks this list and
    /// executes any step not present in the marker's `completed_steps`.
    fn ordered() -> [Self; 5] {
        [
            Self::Bodies,
            Self::InlineImages,
            Self::AttachmentCache,
            Self::SearchIndex,
            Self::AccountRowCascade,
        ]
    }
}

/// Persisted state for one in-flight account deletion. The marker
/// captures the data gathered before any cleanup runs, so a resume
/// after the CASCADE step would be impossible to rebuild from the
/// DB - the rows are gone. CASCADE is therefore always the last
/// step and the marker is unlinked once it succeeds.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct AccountDeletionMarker {
    pub account_id: String,
    pub message_ids: Vec<String>,
    /// `(local_path, content_hash)` per cached attachment. Mirrors
    /// `DbAccountDeletionData::cached_files`.
    pub cached_files: Vec<(String, String)>,
    pub inline_hashes: Vec<String>,
    pub shared_cache_hashes: Vec<String>,
    pub shared_inline_hashes: Vec<String>,
    pub completed_steps: Vec<AccountDeletionStep>,
}

/// Cleanup-report counters surfaced back to the IPC ack.
#[derive(Debug, Default)]
pub(crate) struct CleanupReport {
    pub bodies_deleted: u64,
    pub inline_images_deleted: u64,
    pub cache_files_deleted: u64,
    pub cache_file_errors: Vec<String>,
    pub search_cleaned: bool,
}

const MARKERS: MarkerFile<AccountDeletionMarker> =
    MarkerFile::new("account_delete_markers");

/// Initial entry point: gather data, write the marker, then drive
/// the cleanup steps forward. Called by the `account.delete`
/// handler after runner cancel-and-await has completed (the cancel
/// step does not need a marker because there is no DB state to
/// lose if interrupted - the next boot's runner construction is
/// fresh).
pub(crate) async fn delete_with_marker(
    write_db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    inline_write: &InlineImageStoreWriteState,
    search_write: &SearchWriteHandle,
    app_data: &Path,
    account_id: String,
) -> Result<CleanupReport, String> {
    let aid = account_id.clone();
    let plan = write_db
        .with_conn(move |conn| {
            let data = db::db::queries_extra::gather_account_deletion_data_sync(conn, &aid)?;
            let shared_cache_hashes =
                db::db::queries_extra::referenced_hashes_excluding_account_sync(
                    conn,
                    &data.cached_files,
                    &aid,
                )?;
            let shared_inline_hashes =
                db::db::queries_extra::inline_hashes_referenced_by_other_accounts_sync(
                    conn,
                    &data.inline_hashes,
                    &aid,
                )?;
            Ok((data, shared_cache_hashes, shared_inline_hashes))
        })
        .await?;
    let (data, shared_cache_hashes, shared_inline_hashes) = plan;

    let marker = AccountDeletionMarker {
        account_id: account_id.clone(),
        message_ids: data.message_ids,
        cached_files: data.cached_files,
        inline_hashes: data.inline_hashes,
        shared_cache_hashes: shared_cache_hashes.into_iter().collect(),
        shared_inline_hashes: shared_inline_hashes.into_iter().collect(),
        completed_steps: Vec::new(),
    };
    MARKERS.write(app_data, &account_id, &marker).await?;

    drive_steps(
        write_db,
        body_write,
        inline_write,
        search_write,
        app_data,
        marker,
    )
    .await
}

/// Walk the canonical step list, running any not yet in
/// `completed_steps`. Each successful step is appended to
/// `completed_steps` and the marker rewritten. The marker is
/// unlinked once the final CASCADE step succeeds.
///
/// Idempotency: each step is safe to re-run. body / inline / search
/// deletes are no-ops when their rows do not exist; file unlinks
/// tolerate `NotFound`. CASCADE is `DELETE FROM accounts WHERE id
/// = ?` which is also idempotent.
async fn drive_steps(
    write_db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    inline_write: &InlineImageStoreWriteState,
    search_write: &SearchWriteHandle,
    app_data: &Path,
    mut marker: AccountDeletionMarker,
) -> Result<CleanupReport, String> {
    let mut report = CleanupReport::default();
    let completed: HashSet<AccountDeletionStep> =
        marker.completed_steps.iter().copied().collect();

    for step in AccountDeletionStep::ordered() {
        if completed.contains(&step) {
            continue;
        }
        match step {
            AccountDeletionStep::Bodies => {
                match body_write.delete(marker.message_ids.clone()).await {
                    Ok(n) => report.bodies_deleted = n,
                    Err(e) => log::error!("account.delete: body store cleanup: {e}"),
                }
            }
            AccountDeletionStep::InlineImages => {
                let shared: HashSet<&str> =
                    marker.shared_inline_hashes.iter().map(String::as_str).collect();
                let to_delete: Vec<String> = marker
                    .inline_hashes
                    .iter()
                    .filter(|h| !shared.contains(h.as_str()))
                    .cloned()
                    .collect();
                if !to_delete.is_empty() {
                    match inline_write.delete_hashes(to_delete).await {
                        Ok(n) => report.inline_images_deleted = n,
                        Err(e) => log::error!("account.delete: inline image cleanup: {e}"),
                    }
                }
            }
            AccountDeletionStep::AttachmentCache => {
                let shared: HashSet<&str> =
                    marker.shared_cache_hashes.iter().map(String::as_str).collect();
                for (path, hash) in &marker.cached_files {
                    if shared.contains(hash.as_str()) {
                        continue;
                    }
                    match store::attachment_cache::remove_cached_relative(app_data, path) {
                        Ok(()) => report.cache_files_deleted += 1,
                        Err(e) => report.cache_file_errors.push(format!("{path}: {e}")),
                    }
                }
            }
            AccountDeletionStep::SearchIndex => {
                match search_write
                    .delete_messages_batch(marker.message_ids.clone())
                    .await
                {
                    Ok(()) => report.search_cleaned = true,
                    Err(e) => log::error!("account.delete: search index cleanup: {e}"),
                }
            }
            AccountDeletionStep::AccountRowCascade => {
                let aid = marker.account_id.clone();
                write_db
                    .with_conn(move |conn| {
                        db::db::queries_extra::delete_account_row_sync(conn, &aid)
                    })
                    .await?;
            }
        }
        // Persist progress before continuing so a crash here is
        // recoverable to the next step.
        marker.completed_steps.push(step);
        if matches!(step, AccountDeletionStep::AccountRowCascade) {
            // Final step succeeded; drop the marker.
            MARKERS.unlink(app_data, &marker.account_id).await?;
        } else {
            MARKERS
                .write(app_data, &marker.account_id, &marker)
                .await?;
        }
    }

    Ok(report)
}

/// Resume any in-flight account deletions found at boot. Each
/// marker indicates a deletion that started but did not finish.
/// Drives the remaining steps forward and unlinks the marker on
/// success. Per-marker errors log and continue so one stuck
/// account does not block the boot path.
pub(crate) async fn drain_pending_deletions(
    write_db: &WriteDbState,
    body_write: &BodyStoreWriteState,
    inline_write: &InlineImageStoreWriteState,
    search_write: &SearchWriteHandle,
    app_data: &Path,
) {
    let markers = match MARKERS.list(app_data).await {
        Ok(list) => list,
        Err(e) => {
            log::warn!("drain_pending_deletions: list markers: {e}");
            return;
        }
    };
    if markers.is_empty() {
        return;
    }
    log::info!(
        "drain_pending_deletions: resuming {} in-flight account deletions",
        markers.len()
    );
    for (account_id, marker) in markers {
        match drive_steps(
            write_db,
            body_write,
            inline_write,
            search_write,
            app_data,
            marker,
        )
        .await
        {
            Ok(report) => log::info!(
                "drain_pending_deletions: account={account_id} completed; \
                 {} bodies, {} inline images, {} cache files; search_cleaned={}",
                report.bodies_deleted,
                report.inline_images_deleted,
                report.cache_files_deleted,
                report.search_cleaned,
            ),
            Err(e) => log::error!(
                "drain_pending_deletions: account={account_id} resume failed: {e}; \
                 marker will be retried on next boot",
            ),
        }
    }
}
