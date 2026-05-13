//! Service-side per-account sync runtime.
//!
//! `SyncRuntime` owns the per-account map (cancellation
//! tokens, run ids, supervisor join handles) and drives the runner
//! lifecycle:
//!
//! 1. `start_account(account_id)` either spawns a fresh runner or
//!    returns `already_in_flight: true` with the existing run's id.
//! 2. The runner writes a sync marker
//!    `<app_data>/sync_markers/<account_id>.json` with `status:
//!    "in_progress"` (atomic temp-file-then-rename) before doing any
//!    network or DB work.
//! 3. The runner calls `service::sync_dispatch::sync_for_account`,
//!    which dispatches to the provider's initial or delta sync impl.
//! 4. On exit (Ok / Err / cancelled), the runner updates the marker
//!    status (`completed | cancelled | failed`) and emits a
//!    `Notification::SyncCompleted` carrying the run id + result.
//! 5. Drain unlinks only `completed` markers; `failed` and
//!    `cancelled` (and any leftover `in_progress` from a Service
//!    panic) survive into the next boot's invariant pass.
//!
//! ## Panic supervisor
//!
//! `run_sync` is wrapped in a supervisor `tokio::spawn` that observes
//! the runner's `JoinHandle`. If the runner panicked, the supervisor
//! emits a synthetic `SyncCompleted { result: Failed("runner
//! panicked: ...") }` so any subscribers + the
//! `cancel_and_await` path are not stranded forever. The marker
//! survives in `in_progress` state - the next dirty-boot invariant
//! pass repairs by clearing the JMAP cursor.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crypto_key::SecretKey;
use db::progress::ProgressReporter;
use service_api::{Notification, SyncCancelAck, SyncCompleted, SyncResult, SyncRunId, SyncStartAck};
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};

use crate::boot_progress::NotificationSender;

/// Marker file lifecycle status. Mirrors the JSON written to
/// `sync_markers/<account_id>.json` (`status` field).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkerStatus {
    InProgress,
    Completed,
    Cancelled,
    Failed,
}

impl MarkerStatus {
    pub fn is_completed(self) -> bool {
        matches!(self, Self::Completed)
    }
}

/// On-disk marker payload. Written at the start of every sync run and
/// updated on exit. `started_at` is human-readable diagnostic only -
/// the cursor-clear decision in the invariant pass uses the
/// *existence* of a non-`completed` marker, not the timestamp.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncMarker {
    pub run_id: SyncRunId,
    pub started_at: i64,
    pub kind: String,
    pub status: MarkerStatus,
}

fn now_unix_millis() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    i64::try_from(millis).unwrap_or(i64::MAX)
}

/// Sync markers route through the shared `MarkerFile<T>` helper at
/// `crates/service/src/markers/`. Phase 6b folded the formerly-
/// inline atomic-write / unlink logic into the shared helper so
/// the account-delete marker (also Phase 6b) lands as a second
/// consumer of the same pattern instead of growing a parallel
/// implementation. The on-disk shape is unchanged:
/// `<app_data>/sync_markers/<account_id>.json` carrying a
/// `SyncMarker` payload.
const SYNC_MARKERS: crate::markers::MarkerFile<SyncMarker> =
    crate::markers::MarkerFile::new("sync_markers");

async fn write_marker_atomic(
    app_data_dir: &Path,
    account_id: &str,
    marker: &SyncMarker,
) -> Result<(), String> {
    SYNC_MARKERS.write(app_data_dir, account_id, marker).await
}

async fn unlink_marker(app_data_dir: &Path, account_id: &str) -> Result<(), String> {
    SYNC_MARKERS.unlink(app_data_dir, account_id).await
}

/// Per-account map entry. The `JoinHandle` is the supervisor's handle;
/// dropping it does NOT abort the runner (it's a detached supervisor
/// that observes panic state and emits synthetic notifications).
struct AccountSyncEntry {
    run_id: SyncRunId,
    cancellation_token: CancellationToken,
    /// Supervisor join handle. `Some` while the runner is in flight;
    /// `None` after `start_account` cleans up a finished entry.
    supervisor: Option<JoinHandle<()>>,
}

/// Per-account sync coordinator. Owns the writer halves + shared
/// services; spawns runners under panic supervisors; emits the
/// `SyncCompleted` notifications the UI's `pending_syncs` map
/// resolves against.
pub struct SyncRuntime {
    inner: Arc<SyncRuntimeInner>,
}

pub(crate) struct SyncRuntimeInner {
    accounts: Mutex<HashMap<String, AccountSyncEntry>>,
    pub(crate) db: WriteDbState,
    pub(crate) body_write: BodyStoreWriteState,
    pub(crate) inline_write: InlineImageStoreWriteState,
    pub(crate) search_write: SearchWriteHandle,
    pub(crate) encryption_key: SecretKey,
    pub(crate) progress: Arc<dyn ProgressReporter>,
    pub(crate) notification_tx: NotificationSender,
    pub(crate) app_data_dir: PathBuf,
    pub(crate) service_generation: u32,
    /// Attachments roadmap Phase 4: handle back to the boot state so
    /// `run_sync` can fire a post-sync prefetch sweep without going
    /// through provider-sync (which stays prefetch-ignorant). Held as
    /// `Arc` for symmetry with `ExtractRuntime`; the reference cycle
    /// is broken at drain time when `BootSharedState::take_sync_runtime`
    /// removes the SyncRuntime from its slot.
    pub(crate) boot_state: Arc<crate::boot::BootSharedState>,
}

impl SyncRuntime {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        db: WriteDbState,
        body_write: BodyStoreWriteState,
        inline_write: InlineImageStoreWriteState,
        search_write: SearchWriteHandle,
        encryption_key: SecretKey,
        progress: Arc<dyn ProgressReporter>,
        notification_tx: NotificationSender,
        app_data_dir: PathBuf,
        service_generation: u32,
        boot_state: Arc<crate::boot::BootSharedState>,
    ) -> Self {
        Self {
            inner: Arc::new(SyncRuntimeInner {
                accounts: Mutex::new(HashMap::new()),
                db,
                body_write,
                inline_write,
                search_write,
                encryption_key,
                progress,
                notification_tx,
                app_data_dir,
                service_generation,
                boot_state,
            }),
        }
    }

    /// Spawn a runner for `account_id` if one is not already in flight.
    /// Returns the `SyncStartAck` describing the run id + whether the
    /// caller is the original kicker or a duplicate.
    ///
    /// Phase 8-5 defense-in-depth: rejects starts against an account
    /// whose `is_deleting = 1` flag has been set by `account.delete`.
    /// The UI side filters SyncTick on the same flag, but a delayed
    /// SyncTick from the iced subscription can still arrive between
    /// the flag flip and the row delete; this gate ensures the
    /// runner is never spawned against a disappearing account even if
    /// the UI filter fails.
    pub async fn start_account(&self, account_id: String) -> SyncStartAck {
        let aid = account_id.clone();
        let is_deleting: bool = self
            .inner
            .db
            .with_conn(move |conn| {
                conn.query_row(
                    "SELECT is_deleting FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |r| r.get::<_, i64>(0),
                )
                .map(|v| v != 0)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(true),
                    _ => Err(format!("read is_deleting: {e}")),
                })
            })
            .await
            .unwrap_or(false);
        if is_deleting {
            log::info!(
                "sync start_account({account_id}) rejected: account is being deleted"
            );
            return SyncStartAck {
                account_id,
                run_id: SyncRunId::new_v7(),
                already_in_flight: true,
            };
        }

        let mut map = self.inner.accounts.lock().await;

        // Opportunistic cleanup of finished entries.
        let stale_keys: Vec<String> = map
            .iter()
            .filter_map(|(k, entry)| {
                let supervisor_finished = entry
                    .supervisor
                    .as_ref()
                    .map(JoinHandle::is_finished)
                    .unwrap_or(true);
                if supervisor_finished { Some(k.clone()) } else { None }
            })
            .collect();
        for k in stale_keys {
            map.remove(&k);
        }

        if let Some(entry) = map.get(&account_id) {
            return SyncStartAck {
                account_id,
                run_id: entry.run_id,
                already_in_flight: true,
            };
        }

        let run_id = SyncRunId::new_v7();
        let cancellation_token = CancellationToken::new();

        // Write the in-progress marker BEFORE spawning so a Service
        // crash between `start_account` and the runner's first
        // checkpoint still leaves a dirty marker for the next boot to
        // observe.
        let marker = SyncMarker {
            run_id,
            started_at: now_unix_millis(),
            kind: "delta".into(),
            status: MarkerStatus::InProgress,
        };
        if let Err(e) = write_marker_atomic(&self.inner.app_data_dir, &account_id, &marker).await {
            log::warn!("Failed to write sync marker for {account_id}: {e}");
        }

        let inner = Arc::clone(&self.inner);
        let supervisor_account_id = account_id.clone();
        let supervisor_token = cancellation_token.clone();
        let supervisor = tokio::spawn(async move {
            run_sync_supervised(
                inner,
                supervisor_account_id,
                run_id,
                supervisor_token,
            )
            .await;
        });

        map.insert(
            account_id.clone(),
            AccountSyncEntry {
                run_id,
                cancellation_token,
                supervisor: Some(supervisor),
            },
        );

        SyncStartAck {
            account_id,
            run_id,
            already_in_flight: false,
        }
    }

    /// Cancel an in-flight runner for `account_id`. Returns the active
    /// `run_id` so the caller can subscribe to `SyncCompleted` and
    /// await the cancellation outcome.
    ///
    /// If the entry exists but the supervisor has already finished, the
    /// run emitted its terminal `SyncCompleted` already. Prune and
    /// return `None` so `cancel_and_await` does not park on a `run_id`
    /// that will never emit again.
    pub async fn cancel_account(&self, account_id: &str) -> SyncCancelAck {
        let mut map = self.inner.accounts.lock().await;
        let outcome = map.get(account_id).map(|entry| {
            let finished = entry
                .supervisor
                .as_ref()
                .is_none_or(JoinHandle::is_finished);
            if !finished {
                entry.cancellation_token.cancel();
            }
            (entry.run_id, finished)
        });
        match outcome {
            Some((_, true)) => {
                map.remove(account_id);
                SyncCancelAck {
                    account_id: account_id.into(),
                    run_id: None,
                    was_in_flight: false,
                    calendar_run_id: None,
                }
            }
            Some((run_id, false)) => SyncCancelAck {
                account_id: account_id.into(),
                run_id: Some(run_id),
                was_in_flight: true,
                // SyncRuntime knows nothing about calendar; the
                // handler stamps this field after piggyback.
                calendar_run_id: None,
            },
            None => SyncCancelAck {
                account_id: account_id.into(),
                run_id: None,
                was_in_flight: false,
                calendar_run_id: None,
            },
        }
    }

    /// Clone the body-store write half. Used by the
    /// `account.delete` handler so external-store cleanup can run
    /// Service-side after the DB delete commits.
    pub fn body_write(&self) -> service_state::BodyStoreWriteState {
        self.inner.body_write.clone()
    }

    /// Clone the inline-image-store write half.
    pub fn inline_write(&self) -> service_state::InlineImageStoreWriteState {
        self.inner.inline_write.clone()
    }

    /// Clone the search write handle. Cheap (it's an mpsc Sender);
    /// cloning does not duplicate the underlying writer task.
    pub fn search_write(&self) -> service_state::SearchWriteHandle {
        self.inner.search_write.clone()
    }

    /// Cancel the runner for `account_id` and await its supervisor
    /// `JoinHandle`. Used by Phase 6a-part-2's `account.delete`
    /// handler so the runner-quiescence invariant (no sync writer
    /// holds a connection during the DB delete) closes Service-side.
    /// Returns once the supervisor has exited - either because it
    /// observed the cancellation token at a checkpoint, ran to
    /// completion, or panicked (the supervisor body converts panics
    /// into `SyncCompleted { Failed }`).
    ///
    /// `Ok(())` on success or "no runner registered." `Err` only if
    /// joining the supervisor surfaces an error.
    pub async fn cancel_account_and_await(&self, account_id: &str) -> Result<(), String> {
        let supervisor = {
            let mut map = self.inner.accounts.lock().await;
            match map.get_mut(account_id) {
                Some(entry) => {
                    entry.cancellation_token.cancel();
                    entry.supervisor.take()
                }
                None => None,
            }
        };
        if let Some(sup) = supervisor {
            sup.await.map_err(|e| format!("sync supervisor join: {e}"))?;
        }
        // Drop the entry so a re-insert after delete starts fresh.
        self.inner.accounts.lock().await.remove(account_id);
        Ok(())
    }

    /// Drain step: cancel every runner + await all supervisor
    /// `JoinHandle`s. Phase 3 task 13 calls this from
    /// `lifecycle::run_drain` before the search-writer flush and the
    /// sentinel write.
    pub async fn shutdown(&self) {
        let supervisors: Vec<JoinHandle<()>> = {
            let mut map = self.inner.accounts.lock().await;
            map.values_mut()
                .filter_map(|entry| {
                    entry.cancellation_token.cancel();
                    entry.supervisor.take()
                })
                .collect()
        };
        for sup in supervisors {
            // Best-effort: a panic in a supervisor is logged below.
            if let Err(e) = sup.await {
                log::warn!("supervisor join error during shutdown: {e}");
            }
        }
    }

    /// Internal accessor used by `service::handlers::sync` to surface
    /// state to the in-process integration tests. Phase 3 task 9 wires
    /// the handler.
    #[allow(dead_code)]
    pub(crate) fn inner(&self) -> &Arc<SyncRuntimeInner> {
        &self.inner
    }
}

/// Supervisor body. Wraps `run_sync` in a `tokio::spawn`, observes
/// the inner `JoinHandle`, and converts panics into a synthetic
/// `SyncCompleted { result: Failed(...) }` so subscribers and
/// `cancel_and_await` cannot strand.
async fn run_sync_supervised(
    inner: Arc<SyncRuntimeInner>,
    account_id: String,
    run_id: SyncRunId,
    cancellation_token: CancellationToken,
) {
    let inner_for_runner = Arc::clone(&inner);
    let runner_account = account_id.clone();
    let runner_token = cancellation_token.clone();
    let runner = tokio::spawn(async move {
        run_sync(inner_for_runner, runner_account, run_id, runner_token).await;
    });

    match runner.await {
        Ok(()) => {
            // Normal completion: run_sync already updated the marker
            // and emitted the SyncCompleted notification.
        }
        Err(join_err) if join_err.is_panic() => {
            log::error!("sync runner for {account_id} panicked: {join_err:?}");
            emit_completed(
                &inner,
                &account_id,
                run_id,
                SyncResult::Failed(format!("runner panicked: {join_err}")),
            )
            .await;
            update_marker_status(&inner.app_data_dir, &account_id, MarkerStatus::Failed).await;
        }
        Err(join_err) => {
            log::warn!("sync runner for {account_id} aborted: {join_err:?}");
            emit_completed(
                &inner,
                &account_id,
                run_id,
                SyncResult::Failed(format!("runner aborted: {join_err}")),
            )
            .await;
            update_marker_status(&inner.app_data_dir, &account_id, MarkerStatus::Failed).await;
        }
    }
}

/// Inner runner. Calls `core::sync_dispatch::sync_for_account`,
/// then emits the terminal notification + updates the marker.
async fn run_sync(
    inner: Arc<SyncRuntimeInner>,
    account_id: String,
    run_id: SyncRunId,
    cancellation_token: CancellationToken,
) {
    let mut encryption_key_bytes = [0u8; 32];
    encryption_key_bytes.copy_from_slice(inner.encryption_key.expose().as_slice());

    let result = crate::sync_dispatch::sync_for_account(
        &inner.db,
        &account_id,
        encryption_key_bytes,
        &inner.body_write,
        &inner.inline_write,
        &inner.search_write,
        inner.progress.as_ref(),
        &cancellation_token,
    )
    .await;

    // Force a tantivy commit so any docs queued during this run are
    // observable by the time the UI handles `SyncCompleted`. The
    // writer task acks once the commit lands; the post-commit
    // `IndexCommitted` notification rides the same notification queue
    // and lands ahead of `SyncCompleted` in queue order.
    if let Err(e) = inner.search_write.flush_now().await {
        log::warn!("flush_now after sync for {account_id} failed: {e}");
    }

    let (sync_result, marker_status) = match result {
        Ok(()) => (SyncResult::Completed, MarkerStatus::Completed),
        Err(_) if cancellation_token.is_cancelled() => {
            (SyncResult::Cancelled, MarkerStatus::Cancelled)
        }
        Err(e) => (SyncResult::Failed(e), MarkerStatus::Failed),
    };

    // Attachments roadmap Phase 4: post-sync prefetch sweep. Fires on
    // Ok only; cancelled and failed syncs leave NULL-hash rows for the
    // next backfill kick (or for re-sync) to pick up. Errors here are
    // logged but do not affect the sync result the UI sees.
    //
    // Phase 6: skip the sweep entirely if `cache_attachments_enabled`
    // is 0. The PrefetchRuntime's worker would skip the items anyway
    // via `SkipReason::AccountDisabled`, but bailing here avoids
    // enqueueing them just to drop them.
    let account_caching_on = {
        let aid = account_id.clone();
        inner
            .db
            .with_conn(move |conn| {
                let v: i64 = conn
                    .query_row(
                        "SELECT COALESCE(cache_attachments_enabled, 1) \
                         FROM accounts WHERE id = ?1",
                        rusqlite::params![aid],
                        |r| r.get(0),
                    )
                    .unwrap_or(1);
                Ok(v != 0)
            })
            .await
            .unwrap_or(true)
    };
    if matches!(sync_result, SyncResult::Completed)
        && account_caching_on
        && let Some(prefetch) = inner.boot_state.prefetch_runtime()
    {
        let window_start_unix = match inner.db.with_conn(|conn| {
            Ok(sync::config::get_sync_period_days(conn))
        }).await {
            Ok(days) => chrono::Utc::now().timestamp() - days.saturating_mul(86_400),
            Err(e) => {
                log::debug!("post-sync prefetch sweep: sync_period_days read failed: {e}");
                0
            }
        };
        if let Err(e) = prefetch.enqueue_window_for_account(
            &account_id,
            window_start_unix,
            crate::prefetch::PrefetchPriority::Sync,
            Some(crate::prefetch::SYNC_SWEEP_LIMIT),
        ).await {
            log::debug!("post-sync prefetch sweep {account_id}: {e}");
        }
    }

    update_marker_status(&inner.app_data_dir, &account_id, marker_status).await;
    emit_completed(&inner, &account_id, run_id, sync_result).await;
}

async fn update_marker_status(app_data_dir: &Path, account_id: &str, status: MarkerStatus) {
    // Read existing marker to preserve `started_at` / `kind` / `run_id`.
    let existing = SYNC_MARKERS
        .read(app_data_dir, account_id)
        .await
        .ok()
        .flatten();
    let updated = match existing {
        Some(prev) => SyncMarker {
            run_id: prev.run_id,
            started_at: prev.started_at,
            kind: prev.kind,
            status,
        },
        None => SyncMarker {
            run_id: SyncRunId::new_v7(),
            started_at: now_unix_millis(),
            kind: "delta".into(),
            status,
        },
    };
    if status.is_completed() {
        // Phase 3 task 13's drain unlinks completed markers; we can
        // shortcut that here for cleanly-completed runs.
        if let Err(e) = unlink_marker(app_data_dir, account_id).await {
            log::warn!("unlink completed marker for {account_id}: {e}");
        }
    } else if let Err(e) = write_marker_atomic(app_data_dir, account_id, &updated).await {
        log::warn!("update marker for {account_id}: {e}");
    }
}

async fn emit_completed(
    inner: &SyncRuntimeInner,
    account_id: &str,
    run_id: SyncRunId,
    result: SyncResult,
) {
    let notif = Notification::SyncCompleted(SyncCompleted {
        account_id: account_id.into(),
        run_id,
        result,
        service_generation: inner.service_generation,
    });
    if let Err(e) = inner.notification_tx.send(notif).await {
        log::warn!("emit SyncCompleted for {account_id}: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[tokio::test(flavor = "multi_thread")]
    async fn write_then_read_marker_round_trips() {
        let dir = tempdir().expect("tempdir");
        let marker = SyncMarker {
            run_id: SyncRunId::new_v7(),
            started_at: 1234,
            kind: "delta".into(),
            status: MarkerStatus::InProgress,
        };
        write_marker_atomic(dir.path(), "acc-1", &marker)
            .await
            .expect("write");
        let parsed = SYNC_MARKERS
            .read(dir.path(), "acc-1")
            .await
            .expect("read")
            .expect("present");
        assert_eq!(parsed.run_id, marker.run_id);
        assert_eq!(parsed.status, MarkerStatus::InProgress);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unlink_is_idempotent() {
        let dir = tempdir().expect("tempdir");
        // Unlink with no marker present: Ok.
        unlink_marker(dir.path(), "missing").await.expect("ok");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn marker_status_completed_unlinks() {
        let dir = tempdir().expect("tempdir");
        let marker = SyncMarker {
            run_id: SyncRunId::new_v7(),
            started_at: 1234,
            kind: "delta".into(),
            status: MarkerStatus::InProgress,
        };
        write_marker_atomic(dir.path(), "acc-2", &marker)
            .await
            .expect("write");
        update_marker_status(dir.path(), "acc-2", MarkerStatus::Completed).await;
        let recovered = SYNC_MARKERS
            .read(dir.path(), "acc-2")
            .await
            .expect("read");
        assert!(
            recovered.is_none(),
            "completed status should unlink marker; got {recovered:?}",
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn marker_status_failed_persists() {
        let dir = tempdir().expect("tempdir");
        let marker = SyncMarker {
            run_id: SyncRunId::new_v7(),
            started_at: 1234,
            kind: "delta".into(),
            status: MarkerStatus::InProgress,
        };
        write_marker_atomic(dir.path(), "acc-3", &marker)
            .await
            .expect("write");
        update_marker_status(dir.path(), "acc-3", MarkerStatus::Failed).await;
        let parsed = SYNC_MARKERS
            .read(dir.path(), "acc-3")
            .await
            .expect("read")
            .expect("present");
        assert_eq!(parsed.status, MarkerStatus::Failed);
    }
}
