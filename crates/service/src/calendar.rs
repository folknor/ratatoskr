//! Service-side per-account calendar sync runtime.
//!
//! Phase 5 of `docs/service/phase-5-plan.md` relocates calendar sync into the
//! Service alongside email sync. `CalendarRuntime` owns the per-account map
//! of runner tasks; each runner calls `cal::sync::calendar_sync_account_impl`
//! through a panic supervisor and emits the dual notifications
//! (`CalendarRunCompleted` + `CalendarChanged`).
//!
//! ## Symmetry with `SyncRuntime` and `PushRuntime`
//!
//! Structurally symmetric with `crates/service/src/sync.rs::SyncRuntime` for
//! the lifecycle surface (per-account map, panic supervisor, start/cancel/
//! shutdown). The `closed: AtomicBool` shutdown guard mirrors
//! `crates/service/src/push.rs::PushRuntimeInner` (line 109) - `SyncRuntime`
//! itself does not have the flag. We have it here because Calendar has a
//! kick-driven entry path (the hourly tick) analogous to push's post-ready
//! iteration: any kick arriving during shutdown must be rejected.
//!
//! Diverges intentionally on:
//!
//! - **No marker-file lifecycle.** Calendar sync is idempotent against
//!   CalDAV CTags / Exchange ETags; the provider re-fetches whatever
//!   changed regardless of whether the previous run completed. No
//!   `clear_account_history_id`-equivalent needed.
//! - **No body / inline / search writer halves.** Calendar writes only
//!   to calendar tables in the main DB.
//! - **No invariant-pass entry.** Cross-store crash consistency is
//!   handled by CTag/ETag re-fetch, not the four-store cluster's
//!   marker-file dance.
//!
//! If you find yourself adding any of those, ask whether the divergence is
//! still justified - this comment is the contract for that judgement call.
//!
//! ## Drain ordering: reserved, not load-bearing today
//!
//! `CalendarRuntime::shutdown()` runs *before* `SyncRuntime::shutdown()` in
//! the consolidated drain. The order is **reserved**, not currently
//! load-bearing - the action worker is alive throughout the entire
//! consolidated drain, so calendar drains before or after sync without
//! affecting action-worker availability today.
//!
//! The order is fixed so a future change wiring calendar-cancel cleanup to
//! dispatch action plans (RSVP send is the candidate) is a one-liner instead
//! of a drain reshuffle. Don't promote this to "load-bearing today"
//! rationale unless that wiring lands - reviewers should not look for an
//! RSVP path that doesn't exist.
//!
//! ## CalendarChanged emission
//!
//! `CalendarChanged` cannot be conditioned on `result == Completed`.
//! `crates/calendar/src/sync.rs:249` upserts discovered calendars before
//! per-calendar event loops execute, and per-calendar results are applied
//! independently (line 263). A run cancelled or failed *after* a committed
//! batch has already mutated calendar rows; the UI must reload to surface
//! them.
//!
//! Phase 5 implementation: the runner emits `CalendarChanged` whenever the
//! call to `calendar_sync_account_impl` returned `Ok(())` *or* the
//! cancellation token was observed (we may have committed a batch before the
//! checkpoint fired). On non-cancelled failure the runner does NOT emit
//! `CalendarChanged` - this is a known coarsening: a partial-batch commit
//! followed by a failure observed *outside* the cancellation path leaves UI
//! state stale until the next run. Tightening this requires threading a
//! mutated flag through `cal::sync` end-to-end; that's deferred to the
//! Phase 6 calendar event-mutation relocation pass which already needs to
//! refactor those helpers.
//!
//! `CalendarRunCompleted` always fires regardless of result (the per-run_id
//! awaiter on the client side requires it for terminal correlation).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crypto_key::SecretKey;
use service_api::{
    CalendarCancelAck, CalendarChanged, CalendarRunCompleted, CalendarRunId, CalendarStartAck,
    CalendarSyncResult, Notification,
};
use service_state::WriteDbState;
use tokio::sync::{Mutex, Semaphore};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::boot_progress::NotificationSender;

/// Default concurrency cap on simultaneous runners. Mirrors
/// `SyncRuntime`'s shape so a Service respawn within the 5-min cadence
/// (which resets the in-memory `last_calendar_sync` gate) cannot
/// trigger an N-account thundering herd of parallel TLS sessions and
/// DB writes.
const DEFAULT_CONCURRENCY_CAP: usize = 4;

/// Per-account map entry. The `JoinHandle` is the supervisor's;
/// dropping it does NOT abort the runner.
struct AccountEntry {
    run_id: CalendarRunId,
    cancel: CancellationToken,
    supervisor: Option<JoinHandle<()>>,
}

pub(crate) struct CalendarRuntimeInner {
    accounts: Mutex<HashMap<String, AccountEntry>>,
    /// Wall-clock-ish timestamps of the most recent run completion per
    /// account (Ok or Err). In-memory only; lost on Service respawn.
    /// Open question 3 of `phase-5-plan.md`: a Service restart re-syncs
    /// every account on the next kick (idempotent by CTags/ETags), and
    /// the per-runtime semaphore bounds the parallel cost. Persisting
    /// would buy little; revisit if benchmarks surface a need.
    last_completed: Mutex<HashMap<String, Instant>>,
    pub(crate) db: WriteDbState,
    pub(crate) encryption_key: SecretKey,
    pub(crate) notification_tx: NotificationSender,
    pub(crate) service_generation: u32,
    /// Concurrency cap on simultaneous runners (see
    /// `DEFAULT_CONCURRENCY_CAP` rationale).
    pub(crate) semaphore: Arc<Semaphore>,
    /// Hard barrier against `start_account` accepting new entries after
    /// `shutdown()` has begun. Same role as `PushRuntimeInner::closed`
    /// (`crates/service/src/push.rs:109`): rejects any kick or
    /// explicit-request start that arrives during shutdown.
    closed: AtomicBool,
}

pub struct CalendarRuntime {
    inner: Arc<CalendarRuntimeInner>,
}

impl CalendarRuntime {
    pub fn new(
        db: WriteDbState,
        encryption_key: SecretKey,
        notification_tx: NotificationSender,
        service_generation: u32,
    ) -> Self {
        Self {
            inner: Arc::new(CalendarRuntimeInner {
                accounts: Mutex::new(HashMap::new()),
                last_completed: Mutex::new(HashMap::new()),
                db,
                encryption_key,
                notification_tx,
                service_generation,
                semaphore: Arc::new(Semaphore::new(DEFAULT_CONCURRENCY_CAP)),
                closed: AtomicBool::new(false),
            }),
        }
    }

    /// Spawn a runner for `account_id` if one is not already in flight.
    /// Returns the existing or freshly-generated `CalendarStartAck`.
    ///
    /// Same shutdown-guard pattern as PushRuntime - check `closed`
    /// before the slow path, re-acquire the lock for the insert, and
    /// re-check the guard. Mirrors
    /// `crates/service/src/push.rs::PushRuntime::start_account`.
    /// Diverging is a refactor smell. Returns
    /// `Result<CalendarStartAck, String>` so post-shutdown calls produce
    /// a testable `Err`, not a silently-dropped start.
    pub async fn start_account(
        &self,
        account_id: String,
    ) -> Result<CalendarStartAck, String> {
        if self.inner.closed.load(Ordering::Acquire) {
            return Err("CalendarRuntime is shutting down".into());
        }

        let mut map = self.inner.accounts.lock().await;

        // Re-check after acquiring the lock to close the race against a
        // concurrent shutdown.
        if self.inner.closed.load(Ordering::Acquire) {
            return Err("CalendarRuntime is shutting down".into());
        }

        // Opportunistic cleanup of finished entries.
        let stale_keys: Vec<String> = map
            .iter()
            .filter_map(|(k, entry)| {
                let finished = entry
                    .supervisor
                    .as_ref()
                    .map(JoinHandle::is_finished)
                    .unwrap_or(true);
                if finished { Some(k.clone()) } else { None }
            })
            .collect();
        for k in stale_keys {
            map.remove(&k);
        }

        if let Some(entry) = map.get(&account_id) {
            return Ok(CalendarStartAck {
                account_id,
                run_id: entry.run_id,
                already_in_flight: true,
            });
        }

        let run_id = CalendarRunId::new_v7();
        let cancellation_token = CancellationToken::new();

        let inner = Arc::clone(&self.inner);
        let supervisor_account_id = account_id.clone();
        let supervisor_token = cancellation_token.clone();
        let supervisor = tokio::spawn(async move {
            run_calendar_supervised(inner, supervisor_account_id, run_id, supervisor_token).await;
        });

        map.insert(
            account_id.clone(),
            AccountEntry {
                run_id,
                cancel: cancellation_token,
                supervisor: Some(supervisor),
            },
        );

        Ok(CalendarStartAck {
            account_id,
            run_id,
            already_in_flight: false,
        })
    }

    /// Cancel an in-flight runner for `account_id`. Returns the active
    /// `run_id` so the caller can subscribe to `CalendarRunCompleted` and
    /// await the cancellation outcome (mirrors `SyncCancelAck`).
    ///
    /// If the entry exists but the supervisor has already finished, the
    /// run already emitted its terminal notification and there is nothing
    /// to await. Prune and return `None` so `cancel_and_await` does not
    /// subscribe to a `run_id` that will never emit again. Drops the
    /// in-memory `last_completed` entry too so a future re-create with the
    /// same account id starts with a clean slate.
    pub async fn cancel_account(&self, account_id: &str) -> CalendarCancelAck {
        let mut map = self.inner.accounts.lock().await;
        let outcome = map.get(account_id).map(|entry| {
            let finished = entry
                .supervisor
                .as_ref()
                .is_none_or(JoinHandle::is_finished);
            if !finished {
                entry.cancel.cancel();
            }
            (entry.run_id, finished)
        });
        match outcome {
            Some((_, true)) => {
                map.remove(account_id);
                drop(map);
                self.inner.last_completed.lock().await.remove(account_id);
                CalendarCancelAck {
                    account_id: account_id.into(),
                    run_id: None,
                    was_in_flight: false,
                }
            }
            Some((run_id, false)) => CalendarCancelAck {
                account_id: account_id.into(),
                run_id: Some(run_id),
                was_in_flight: true,
            },
            None => {
                drop(map);
                self.inner.last_completed.lock().await.remove(account_id);
                CalendarCancelAck {
                    account_id: account_id.into(),
                    run_id: None,
                    was_in_flight: false,
                }
            }
        }
    }

    /// Filter `account_ids` to those whose last completion was more
    /// than `staleness` ago (or that have no recorded completion).
    /// Used by `handle_calendar_kick` to gate the kick-driven path
    /// without re-syncing accounts that just finished.
    ///
    /// Open question 3 in `phase-5-plan.md` is closed in favour of
    /// in-memory tracking + the per-runtime semaphore: a Service
    /// respawn re-syncs every account on the next kick, but the
    /// concurrency cap bounds the cost and CTags/ETags make the work
    /// idempotent on the wire.
    pub async fn accounts_due_for_sync(
        &self,
        account_ids: Vec<String>,
        staleness: Duration,
    ) -> Vec<String> {
        let now = Instant::now();
        let map = self.inner.last_completed.lock().await;
        account_ids
            .into_iter()
            .filter(|aid| {
                map.get(aid)
                    .is_none_or(|last| now.duration_since(*last) >= staleness)
            })
            .collect()
    }

    /// Drain step. Cancel every runner, await every supervisor.
    /// Phase 5 task 7 calls this from the consolidated drain helper
    /// before `SyncRuntime::shutdown()`.
    ///
    /// Order: flip `closed` BEFORE snapshotting the map. Any
    /// `start_account` arriving after this point sees the flag and
    /// returns Err. Without this ordering a late `start_account` could
    /// insert a fresh entry between the snapshot and the supervisor
    /// await, leaking a runner that outlives the drain.
    pub async fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::Release);
        let supervisors: Vec<JoinHandle<()>> = {
            let mut map = self.inner.accounts.lock().await;
            map.values_mut()
                .filter_map(|entry| {
                    entry.cancel.cancel();
                    entry.supervisor.take()
                })
                .collect()
        };
        for sup in supervisors {
            if let Err(e) = sup.await {
                log::warn!("calendar supervisor join error during shutdown: {e}");
            }
        }
    }
}

/// Supervisor body. Wraps `run_calendar` in a spawn so the runner's
/// `JoinHandle` is observable. On panic / abort, emit a synthetic
/// `CalendarRunCompleted { Failed }` so per-`run_id` awaiters cannot
/// strand. Mirrors `SyncRuntime::run_sync_supervised`.
async fn run_calendar_supervised(
    inner: Arc<CalendarRuntimeInner>,
    account_id: String,
    run_id: CalendarRunId,
    cancellation_token: CancellationToken,
) {
    let inner_for_runner = Arc::clone(&inner);
    let runner_account = account_id.clone();
    let runner_token = cancellation_token.clone();
    let runner = tokio::spawn(async move {
        run_calendar(inner_for_runner, runner_account, run_id, runner_token).await;
    });

    match runner.await {
        Ok(()) => {
            // Normal completion: run_calendar already emitted notifications.
        }
        Err(join_err) if join_err.is_panic() => {
            log::error!("calendar runner for {account_id} panicked: {join_err:?}");
            emit_run_completed(
                &inner,
                &account_id,
                run_id,
                CalendarSyncResult::Failed(format!("runner panicked: {join_err}")),
                false,
            )
            .await;
        }
        Err(join_err) => {
            log::warn!("calendar runner for {account_id} aborted: {join_err:?}");
            emit_run_completed(
                &inner,
                &account_id,
                run_id,
                CalendarSyncResult::Failed(format!("runner aborted: {join_err}")),
                false,
            )
            .await;
        }
    }
}

/// Inner runner. Acquires a semaphore permit (concurrency cap), calls
/// `cal::sync::calendar_sync_account_impl`, then emits the terminal
/// notifications.
async fn run_calendar(
    inner: Arc<CalendarRuntimeInner>,
    account_id: String,
    run_id: CalendarRunId,
    cancellation_token: CancellationToken,
) {
    let _permit = match Arc::clone(&inner.semaphore).acquire_owned().await {
        Ok(p) => p,
        Err(e) => {
            log::warn!("calendar semaphore closed for {account_id}: {e}");
            emit_run_completed(
                &inner,
                &account_id,
                run_id,
                CalendarSyncResult::Failed(format!("semaphore closed: {e}")),
                false,
            )
            .await;
            return;
        }
    };

    let mut encryption_key_bytes = [0u8; 32];
    encryption_key_bytes.copy_from_slice(inner.encryption_key.expose().as_slice());
    let gmail = gmail::client::new_gmail_state(encryption_key_bytes);
    let graph = graph::client::new_graph_state(encryption_key_bytes);

    let result = cal::sync::calendar_sync_account_impl(
        &account_id,
        &inner.db,
        &gmail,
        &graph,
        &cancellation_token,
    )
    .await;

    // Phase 5 emission rule:
    //   - CalendarRunCompleted always fires (per-run_id awaiter contract).
    //   - CalendarChanged fires whenever `mutated == true`. Without
    //     end-to-end mutated tracking through cal::sync (deferred to the
    //     Phase 6 cleanup pass alongside calendar event-mutation
    //     relocation), we conservatively treat any non-failure run as
    //     mutated: `Completed` always changed something (the discover
    //     upsert at minimum), and `Cancelled` may have committed a batch
    //     before the checkpoint observed the token. Pure failures
    //     (provider config error, "no calendar provider") do NOT emit
    //     CalendarChanged.
    let (sync_result, mutated) = match result {
        Ok(()) => (CalendarSyncResult::Completed, true),
        Err(_) if cancellation_token.is_cancelled() => (CalendarSyncResult::Cancelled, true),
        Err(e) => (CalendarSyncResult::Failed(e), false),
    };

    if mutated {
        emit_calendar_changed(&inner, &account_id).await;
    }
    // Stamp last_completed regardless of result. The kick handler reads
    // this map to gate the staleness check (1h default) and a failed
    // attempt still counts as "we just tried" - hammering a flaky
    // provider every 5 minutes wouldn't help.
    {
        let mut map = inner.last_completed.lock().await;
        map.insert(account_id.clone(), Instant::now());
    }
    emit_run_completed(&inner, &account_id, run_id, sync_result, mutated).await;
}

async fn emit_run_completed(
    inner: &CalendarRuntimeInner,
    account_id: &str,
    run_id: CalendarRunId,
    result: CalendarSyncResult,
    mutated: bool,
) {
    let notif = Notification::CalendarRunCompleted(CalendarRunCompleted {
        account_id: account_id.into(),
        run_id,
        result,
        mutated,
        service_generation: inner.service_generation,
    });
    if let Err(e) = inner.notification_tx.send(notif).await {
        log::warn!("emit CalendarRunCompleted for {account_id}: {e}");
    }
}

async fn emit_calendar_changed(inner: &CalendarRuntimeInner, account_id: &str) {
    let notif = Notification::CalendarChanged(CalendarChanged {
        account_id: account_id.into(),
        service_generation: inner.service_generation,
    });
    if let Err(e) = inner.notification_tx.send(notif).await {
        log::warn!("emit CalendarChanged for {account_id}: {e}");
    }
}

#[cfg(test)]
mod tests {
    // Lifecycle-only unit tests. Real runner behaviour against a stub
    // calendar provider needs either a fake CalDAV fixture or
    // `test_dummy` constructors on writer-state types - same caveat as
    // SyncRuntime's tests, deferred to Phase 8 alongside the rest of
    // Phase 5's integration cohort.

    use super::*;
    use crypto_key::SecretKey;
    use rusqlite::Connection;
    use std::sync::{Arc as StdArc, Mutex as StdMutex};
    use tokio::sync::mpsc;

    fn fresh_runtime() -> (CalendarRuntime, mpsc::Receiver<Vec<u8>>) {
        let conn = StdArc::new(StdMutex::new(
            Connection::open_in_memory().expect("open in-memory db"),
        ));
        let db = WriteDbState::from_arc(conn);
        let key = SecretKey::from_bytes([0u8; 32]);
        let (tx, rx) = mpsc::channel::<Vec<u8>>(16);
        let notification_tx = NotificationSender::new(tx);
        let runtime = CalendarRuntime::new(db, key, notification_tx, 1);
        (runtime, rx)
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn cancel_account_returns_none_when_no_entry() {
        let (runtime, _rx) = fresh_runtime();
        let ack = runtime.cancel_account("missing").await;
        assert!(ack.run_id.is_none());
        assert!(!ack.was_in_flight);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn shutdown_is_safe_on_empty_runtime() {
        let (runtime, _rx) = fresh_runtime();
        runtime.shutdown().await;
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn start_account_returns_err_after_shutdown() {
        let (runtime, _rx) = fresh_runtime();
        runtime.shutdown().await;
        let result = runtime.start_account("acc-1".into()).await;
        assert!(result.is_err(), "expected Err after shutdown, got {result:?}");
    }
}
