//! Service-side boot sequence orchestrator.
//!
//! Runs concurrently with the dispatch loop so `health.ping` continues to
//! round-trip while migrations run. Implements the full Phase 1.5 sequence:
//! key load -> DB open + velo->ratatoskr rename + schema migrations ->
//! pending-ops recovery -> queued-drafts sweep -> thread-participants
//! backfill. Each step emits a corresponding `BootPhase` notification so
//! the splash can render progress.
//!
//! On fatal boot failure (missing key, migration failure, etc.) the
//! sequence does NOT call `std::process::exit` directly: it returns a
//! `BootFailure` to the caller. This is what makes the in-process test
//! harness (`run_service_with_io` over `tokio::io::duplex`) safe to use -
//! a process exit there would kill the test runner. The outer
//! `run_service_blocking` in `lib.rs` is the only caller that converts
//! the boot exit code into an actual `std::process::exit`.

use crate::boot_progress;
use crypto_key::SecretKey;
use db::db::action_journal::recover_stale_leases;
use db::db::pending_ops::db_pending_ops_recover_on_boot_sync;
use db::db::queries_extra::{
    backfill_thread_participants_for_account_sync, db_mark_queued_drafts_failed_sync,
    get_all_accounts_sync, get_all_send_identity_emails,
};
use db::db::{Connection, apply_standard_pragmas, migrations, reconcile_velo_rename};
use service_api::{BootExitCode, BootPhase, BootReadyResponse};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, mpsc};

/// Service-side boot artifacts loaded once at boot. Phase 2's `ActionContext`
/// will consume the encryption key + DB connection from here once the action
/// service moves across the boundary; until then the fields are held but
/// unused (the UI keeps its own key + DB load).
///
/// TODO(phase-2): the action service handler reads `encryption_key` and
/// `db_conn` from this struct, replacing the UI-side
/// `rtsk::load_encryption_key` and `Db::open` calls in `crates/app/src/app.rs`.
/// The `#[allow(dead_code)]` markers come off then.
pub(crate) struct BootContext {
    /// AES-256-GCM key loaded from `<app_data>/ratatoskr.key`. Held in a
    /// `SecretKey` wrapper so the bytes zeroize on drop - the Service's
    /// long-lived process otherwise risks the key lingering in freed
    /// heap pages or core dumps for the lifetime of the runtime.
    /// Phase 2's `ActionContext` consumes via `key.expose()` and copies
    /// into the cipher's internal slot.
    #[allow(dead_code)]
    pub(crate) encryption_key: SecretKey,
    /// DB connection opened during boot. Held past the boot sequence so
    /// Phase 2's relocated action service can construct its `ActionContext`
    /// from it without re-opening the file. `Arc<Mutex<Connection>>` matches
    /// the shape `ReadDbState::from_arc` expects.
    #[allow(dead_code)]
    pub(crate) db_conn: Arc<Mutex<Connection>>,
    /// Highest applied schema version after migrations completed. Echoed
    /// to the UI in `BootReadyResponse`.
    pub(crate) schema_version: u32,
    /// Number of migrations actually applied this boot. 0 on a healthy
    /// repeat boot; non-zero only on first-run or after a schema bump.
    pub(crate) migrations_applied: u32,
    /// Short human-readable labels for each recovery step that failed
    /// non-fatally. Empty on a healthy boot. Echoed to the UI in
    /// `BootReadyResponse.recovery_warnings` so the splash transition can
    /// surface "boot ok but recovery had issues" without the UI having to
    /// grep the rolling log file.
    pub(crate) recovery_warnings: Vec<String>,
}

/// Per-Service-instance boot state. The boot task populates `result` (and
/// `context` on success) and fires `notify`; the `boot.ready` handler reads
/// from `result`.
///
/// Held per `run_service_with_io_and_lifecycle` invocation rather than as a
/// process-wide singleton so in-process tests can spawn multiple harnesses
/// without colliding on `OnceLock::set`. Phase 2 will introduce its own
/// per-instance state for the relocated `ActionContext`.
pub(crate) struct BootSharedState {
    notify: Notify,
    result: Mutex<Option<Result<BootReadyResponse, BootFailure>>>,
    #[allow(dead_code)]
    context: Mutex<Option<BootContext>>,
    /// Wakeup channel from the action handler to the action worker.
    /// The handler `notify_one`s this after journaling a plan; the
    /// worker (Phase 2 task 9c) parks on `notified()` and drains the
    /// journal. Using a `Notify` rather than an mpsc keeps the
    /// handler's notify-then-return path lock-free and means a missed
    /// wakeup costs at most one drain delay (the worker re-checks
    /// the journal on every wakeup, not just once per signal).
    action_worker_wakeup: Notify,
    /// Phase 1.5 carry-forward 19h / Phase 2 task 22: bounds parallel
    /// `boot.ready` handlers. Stdio is private so a malicious flood
    /// is not realistic, but a UI bug that re-issues boot.ready
    /// would balloon `JoinSet` linearly (each handler parks on
    /// `Notify` until boot completes). The first caller flips this
    /// flag to true; subsequent callers fail fast with
    /// `BootReadyInFlight` rather than queueing. If the result is
    /// already populated (boot completed but the previous ack got
    /// lost), even fail-fast callers can read the cached result -
    /// `wait_for_ready` checks `result` first so it returns
    /// immediately in that case.
    boot_ready_inflight: std::sync::atomic::AtomicBool,
    /// `<app_data>/` for this Service incarnation. Captured at
    /// `BootSharedState::new` time so handlers (specifically
    /// `action.send` for staging-vault path resolution; future
    /// surfaces likely to follow) can derive paths under
    /// `<app_data>/` without threading the directory through every
    /// dispatch signature. Immutable for the lifetime of the
    /// `BootSharedState`.
    app_data_dir: PathBuf,
    /// Per-account sync coordinator. Installed by the boot task once
    /// the writer halves (DB, body store, inline image store, search
    /// writer task) are ready, before `signal_ready` fires; from that
    /// point on every `sync.start_account` / `sync.cancel_account`
    /// handler reads it via `sync_runtime()`. Held in a `Mutex<Option<_>>`
    /// rather than `OnceLock` so the type stays consistent with the
    /// other boot-installed state on this struct (`context`, `result`).
    sync_runtime: Mutex<Option<Arc<crate::sync::SyncRuntime>>>,
}

impl BootSharedState {
    pub(crate) fn new(app_data_dir: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            notify: Notify::new(),
            result: Mutex::new(None),
            context: Mutex::new(None),
            action_worker_wakeup: Notify::new(),
            boot_ready_inflight: std::sync::atomic::AtomicBool::new(false),
            app_data_dir,
            sync_runtime: Mutex::new(None),
        })
    }

    /// Install the `SyncRuntime` once the boot task has constructed it
    /// (Phase 3 task 12 wires this from `run_boot_sequence_inner`).
    /// Must be called exactly once; a second call is a programming
    /// error and is logged at warn (the first install wins).
    #[allow(dead_code)] // wired up in Phase 3 task 12
    pub(crate) fn install_sync_runtime(&self, runtime: Arc<crate::sync::SyncRuntime>) {
        let mut guard = self
            .sync_runtime
            .lock()
            .expect("sync_runtime mutex poisoned");
        if guard.is_some() {
            log::warn!(
                "BootSharedState::install_sync_runtime called twice; second install ignored",
            );
            return;
        }
        *guard = Some(runtime);
    }

    /// Snapshot the active `SyncRuntime` if boot has installed one.
    /// Returns `None` if boot has not reached the sync-runtime
    /// construction step yet (or if a non-sync code path reaches here
    /// before boot completes).
    pub(crate) fn sync_runtime(&self) -> Option<Arc<crate::sync::SyncRuntime>> {
        self.sync_runtime
            .lock()
            .expect("sync_runtime mutex poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    /// Path to `<app_data>/` for this Service incarnation. Used by
    /// handlers that need to derive paths under app-data without
    /// piping the directory through dispatch signatures (e.g.
    /// `action.send` resolves staging / vault dirs from this).
    pub(crate) fn app_data_dir(&self) -> &std::path::Path {
        &self.app_data_dir
    }

    /// Try to claim the boot.ready in-flight slot. Returns a guard on
    /// success that releases the slot on drop; returns `None` if a
    /// previous caller is still parked. The handler in
    /// `crates/service/src/handlers/boot.rs` consults this before
    /// awaiting `wait_for_ready` so a UI bug (or future surface) that
    /// fires multiple boot.ready calls cannot balloon the dispatch
    /// `JoinSet` with parked tasks.
    pub(crate) fn try_claim_boot_ready_slot(self: &Arc<Self>) -> Option<BootReadyGuard> {
        if self
            .boot_ready_inflight
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            None
        } else {
            Some(BootReadyGuard {
                state: Arc::clone(self),
            })
        }
    }

    /// Inspect the cached `result`, used by the boot.ready handler when
    /// it loses the in-flight slot race: if the result has already
    /// landed, a "second caller" can satisfy the request from the
    /// cache instead of failing.
    pub(crate) fn cached_result(&self) -> Option<Result<BootReadyResponse, BootFailure>> {
        self.result.lock().expect("boot result poisoned").clone()
    }

    /// Wake up the action worker so it drains the journal. Called by
    /// the `action.execute_plan` handler after journaling a plan
    /// (Phase 2 task 9b). The worker (task 9c) parks on
    /// `await_action_worker_wakeup()` and re-scans the journal on
    /// every wake. Idempotent under the hood (Notify's
    /// `notify_one`-without-waiter semantics still wake the next
    /// `notified().await`).
    pub(crate) fn notify_action_worker(&self) {
        self.action_worker_wakeup.notify_one();
    }

    #[allow(dead_code)] // consumed by the worker in task 9c
    pub(crate) fn await_action_worker_wakeup(&self) -> tokio::sync::futures::Notified<'_> {
        self.action_worker_wakeup.notified()
    }

    /// Park until `signal_ready` fires. The boot.ready handler calls this.
    ///
    /// The outer `loop` is defense-in-depth: today `signal_ready` populates
    /// `result` exactly once and `notify.notify_waiters()` wakes every
    /// parked task, so the second iteration always finds `Some(result)`.
    /// The loop survives a future refactor that might use `notify_one` or
    /// add a spurious-wakeup path; keeping it costs one extra mutex check
    /// in the unreachable case.
    /// Get a clone of the DB connection arc once boot has populated
    /// `context`. Returns `None` if boot is still in flight or has not
    /// run (which is a contract violation for any caller other than the
    /// boot task itself - by the time `boot.ready` returns, `context`
    /// is populated and stays so for the lifetime of the Service).
    ///
    /// Used by handlers (`action.job_status` today; the action service
    /// handler+worker in task 9) that need to query the journal after
    /// boot. Cloning the `Arc` lets the handler drive its own
    /// `spawn_blocking` against the connection without holding the
    /// `BootSharedState` mutex across the blocking work.
    pub(crate) fn db_conn(&self) -> Option<Arc<Mutex<Connection>>> {
        let guard = self.context.lock().expect("boot context poisoned");
        guard.as_ref().map(|ctx| Arc::clone(&ctx.db_conn))
    }

    /// Snapshot the encryption key out of `BootContext` once boot has
    /// populated it. Returns the raw 32 bytes by copy because the
    /// action service consumers (`ActionContext::encryption_key`,
    /// SMTP credential decrypt) take `[u8; 32]`. Returns `None` if
    /// boot is still in flight or has not run.
    pub(crate) fn encryption_key(&self) -> Option<[u8; 32]> {
        let guard = self.context.lock().expect("boot context poisoned");
        guard.as_ref().map(|ctx| *ctx.encryption_key.expose())
    }

    pub(crate) async fn wait_for_ready(&self) -> Result<BootReadyResponse, BootFailure> {
        loop {
            if let Some(result) = self.result.lock().expect("boot result poisoned").clone() {
                return result;
            }
            // Build the waiter BEFORE re-checking `result` to avoid losing a
            // `notify_waiters()` that fires between the check and the await.
            let waiter = self.notify.notified();
            tokio::pin!(waiter);
            if let Some(result) = self.result.lock().expect("boot result poisoned").clone() {
                return result;
            }
            waiter.as_mut().await;
        }
    }

    fn signal_ready(
        &self,
        result: Result<BootReadyResponse, BootFailure>,
        context: Option<BootContext>,
    ) {
        {
            let mut guard = self.result.lock().expect("boot result poisoned");
            if guard.is_some() {
                // Double-signal is a "should never happen" - the boot task
                // is the sole producer. A Phase 2 retry path that calls
                // `signal_ready` twice with different results would
                // silently lose the second one; warn so the situation is
                // visible in the rolling log without forcing a debug
                // build.
                log::warn!(
                    "BootSharedState::signal_ready called twice; second call ignored - \
                     this is a programming error, the boot task should signal exactly once",
                );
            } else {
                *guard = Some(result);
            }
        }
        if let Some(ctx) = context {
            let mut guard = self.context.lock().expect("boot context poisoned");
            if guard.is_none() {
                *guard = Some(ctx);
            }
        }
        self.notify.notify_waiters();
    }
}

/// RAII guard returned by `BootSharedState::try_claim_boot_ready_slot`.
/// Releases the in-flight flag on drop so a future legitimate retry
/// (e.g. after a respawn) can re-claim. Tied to the `BootSharedState`
/// the slot belongs to via an `Arc` clone.
pub(crate) struct BootReadyGuard {
    state: Arc<BootSharedState>,
}

impl Drop for BootReadyGuard {
    fn drop(&mut self) {
        self.state
            .boot_ready_inflight
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Discriminant of why the boot sequence failed. The caller maps this to a
/// `BootExitCode` via `as_exit_code()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BootFailure {
    KeyLoadFailure,
    MigrationFailure,
}

impl BootFailure {
    pub(crate) fn as_exit_code(self) -> BootExitCode {
        match self {
            Self::KeyLoadFailure => BootExitCode::KeyLoadFailure,
            Self::MigrationFailure => BootExitCode::MigrationFailure,
        }
    }
}

/// Run the Service boot sequence.
///
/// Emits `BootPhase::*` notifications via `out_tx` so the UI splash can
/// render progress. Synchronous DB / filesystem work runs in
/// `tokio::task::spawn_blocking` so the dispatch task and the writer task
/// (which pumps notifications) never starve waiting on `rusqlite` /
/// `std::fs::read_to_string`.
///
/// On success, populates `state.context` and signals
/// `state.result(Ok(BootReadyResponse))`. On failure, signals
/// `state.result(Err(BootFailure))` and returns the failure to the caller;
/// the dispatch loop drives the actual process exit.
pub(crate) async fn run_boot_sequence(
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: PathBuf,
    state: Arc<BootSharedState>,
) -> Result<(), BootFailure> {
    let inner = run_boot_sequence_inner(out_tx, app_data_dir).await;
    let (result, context) = match inner {
        Ok(ctx) => (
            Ok(BootReadyResponse {
                ready: true,
                schema_version: ctx.schema_version,
                migrations_applied: ctx.migrations_applied,
                recovery_warnings: ctx.recovery_warnings.clone(),
            }),
            Some(ctx),
        ),
        Err(failure) => (Err(failure), None),
    };
    let outcome = match &result {
        Ok(_) => Ok(()),
        Err(failure) => Err(*failure),
    };
    state.signal_ready(result, context);
    outcome
}

/// Test-only artificial delay inserted at the start of the boot sequence,
/// before the LoadingKey phase emits. Used by the in-process integration
/// tests to verify that `boot.ready` actually parks on `BootSharedState`
/// rather than racing past via a fast-DB no-delay path. The delay is
/// process-wide; tests that drive it must serialize on
/// `crate::boot::TEST_BOOT_DELAY_LOCK` so they don't race each other.
/// Always returns 0 in release builds where the test-helpers feature is
/// not compiled in.
#[cfg(feature = "test-helpers")]
fn test_boot_delay_ms() -> u64 {
    use std::sync::atomic::Ordering;
    TEST_BOOT_DELAY_MS.load(Ordering::SeqCst)
}

#[cfg(not(feature = "test-helpers"))]
fn test_boot_delay_ms() -> u64 {
    0
}

/// Test-only knob: set the artificial boot delay (in milliseconds) inserted
/// at the start of `run_boot_sequence_inner`. Process-wide; serialize via
/// `TEST_BOOT_DELAY_LOCK` from tests that need exclusive control over it.
#[cfg(feature = "test-helpers")]
pub static TEST_BOOT_DELAY_MS: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(0);

/// Test-only mutex used to serialize tests that set `TEST_BOOT_DELAY_MS`.
/// Tests acquire this guard, set the atomic, run the boot, then reset.
#[cfg(feature = "test-helpers")]
pub static TEST_BOOT_DELAY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

async fn run_boot_sequence_inner(
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: PathBuf,
) -> Result<BootContext, BootFailure> {
    let delay = test_boot_delay_ms();
    if delay > 0 {
        log::debug!("test-helpers: artificial boot delay {delay}ms before LoadingKey");
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }

    boot_progress::emit(&out_tx, BootPhase::LoadingKey, None);

    let key = match tokio::task::spawn_blocking({
        let dir = app_data_dir.clone();
        move || crypto_key::load_encryption_key(&dir)
    })
    .await
    {
        Ok(Ok(key)) => key,
        Ok(Err(error)) => {
            log::error!(
                "encryption key load failed for {}: {error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::KeyLoadFailure);
        }
        Err(join_error) => {
            log::error!(
                "encryption key load task panicked for {}: {join_error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::KeyLoadFailure);
        }
    };

    boot_progress::emit(&out_tx, BootPhase::OpeningDatabase, None);

    let migrate_outcome = tokio::task::spawn_blocking({
        let dir = app_data_dir.clone();
        let progress_tx = out_tx.clone();
        move || open_db_and_migrate(&dir, &progress_tx)
    })
    .await;

    let MigrateOutcome {
        conn,
        schema_version,
        migrations_applied,
    } = match migrate_outcome {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            log::error!(
                "DB open / migration failed for {}: {error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::MigrationFailure);
        }
        Err(join_error) => {
            log::error!(
                "DB open / migration task panicked for {}: {join_error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::MigrationFailure);
        }
    };

    let conn = Arc::new(Mutex::new(conn));
    let mut recovery_warnings: Vec<String> = Vec::new();

    // RecoveringPendingOps: state-repair on the pending_operations table
    // (resets stranded `status='executing'` rows to 'pending') AND on
    // local_drafts (resurfaces stranded `sync_status='sending'` rows as
    // 'failed'). The two repairs share a phase because both target rows
    // that were mid-mutation when the previous Service died, and both run
    // off the same connection. Distinct from SweepingQueuedDrafts below,
    // which targets a different sync_status value ('queued') and a
    // different failure mode (drafts that never made it into 'sending').
    boot_progress::emit(&out_tx, BootPhase::RecoveringPendingOps, None);
    if let Err(error) = run_boot_recovery(&conn, "pending-ops recovery", |c| {
        db_pending_ops_recover_on_boot_sync(c)
    })
    .await
    {
        log::warn!("pending-ops boot recovery failed: {error}");
        recovery_warnings.push("pending-ops recovery".to_string());
    }

    // Action-journal stale-lease reset. Any `action_jobs` row in `leased` /
    // `executing` and any `action_job_ops` row in `leased` / `executing`
    // belongs to a previous Service incarnation that's already gone; the
    // worker UUID in lease_owner cannot be the current one. Reset to
    // `queued` / `pending` so the worker re-leases on its first scheduling
    // pass. Runs before the worker spawns; without this, a SIGKILL during
    // batch_execute strands the in-flight op forever (lease_next_ready_op
    // filters strictly on status='pending' and ignores lease_expires_at,
    // and replay_unemitted requires outcome IS NOT NULL which the crashed
    // op never wrote).
    if let Err(error) = run_boot_recovery(&conn, "action-journal stale leases", |c| {
        let (jobs_reset, ops_reset) = recover_stale_leases(c)?;
        if jobs_reset > 0 || ops_reset > 0 {
            log::info!(
                "[action-journal] Reset {jobs_reset} stale jobs + {ops_reset} stale ops back to queued/pending"
            );
        }
        Ok(())
    })
    .await
    {
        log::warn!("action-journal stale-lease recovery failed: {error}");
        recovery_warnings.push("action-journal stale leases".to_string());
    }

    // Send-vault orphan cleanup (Phase 2 task 5 of compose-send
    // relocation). Walks `<app_data>/send_vault/` and removes any
    // subdirectory whose name does not parse as a UUIDv7 OR whose
    // parsed PlanId is not in the set of "live" send jobs (kind =
    // 'send' and status NOT IN ('completed', 'failed')). Three crash
    // scenarios this catches:
    //
    // - Handler died mid-transfer: bytes in vault, no journal row -
    //   orphan, removed.
    // - Worker died after finalize but before cleanup_vault_dir:
    //   journal terminal, vault still on disk - orphan, removed.
    // - Worker died mid-execution: journal queued/leased/executing
    //   (the `live` set); vault preserved - the next worker pass
    //   replays the SMTP submit.
    //
    // Folded into the same recovery block as pending-ops because both
    // are state-repair passes that run before the boot handshake
    // signals readiness; no separate BootPhase notification (the
    // pass is fast - filesystem walk, no user-visible progress).
    if let Err(error) = reconcile_send_vault(&conn, &app_data_dir).await {
        log::warn!("send-vault orphan cleanup failed: {error}");
        recovery_warnings.push("send-vault cleanup".to_string());
    }

    // SweepingQueuedDrafts: marks `local_drafts.sync_status='queued'` rows
    // as 'failed' so the user sees them surfaced in the drafts view rather
    // than stuck in a queue that the previous Service never drained. Phase
    // 1.5 has no live action service, so any 'queued' row is by definition
    // orphaned. Distinct from the 'sending' resurfacing above (different
    // sync_status, different lifecycle stage).
    boot_progress::emit(&out_tx, BootPhase::SweepingQueuedDrafts, None);
    if let Err(error) = run_boot_recovery(&conn, "queued-drafts sweep", |c| {
        let count = db_mark_queued_drafts_failed_sync(c)?;
        if count > 0 {
            log::info!("[drafts] Resurfaced {count} orphaned 'queued' drafts as 'failed'");
        }
        Ok(())
    })
    .await
    {
        log::warn!("queued-drafts sweep failed: {error}");
        recovery_warnings.push("queued-drafts sweep".to_string());
    }

    boot_progress::emit(&out_tx, BootPhase::BackfillingThreadParticipants, None);
    if let Err(error) = run_boot_recovery(&conn, "thread-participants backfill", |c| {
        run_backfill_for_all_accounts(c)
    })
    .await
    {
        log::warn!("thread-participants backfill failed: {error}");
        recovery_warnings.push("thread-participants backfill".to_string());
    }

    Ok(BootContext {
        encryption_key: key,
        db_conn: conn,
        schema_version,
        migrations_applied,
        recovery_warnings,
    })
}

/// Run a synchronous recovery step inside `spawn_blocking` against a shared
/// `Arc<Mutex<Connection>>`. The dispatch task and the writer task continue
/// to run on the async side; only the rusqlite work blocks. Errors from
/// recovery steps are logged at warn (the boot sequence proceeds) - per
/// scope items 4, 5, and 5a of `phase-1.5-plan.md`, these steps are state
/// repair, not correctness gates: a failure leaves the DB in the same state
/// the previous boot left it in, which is no worse than skipping recovery.
async fn run_boot_recovery<F>(
    conn: &Arc<Mutex<Connection>>,
    label: &'static str,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(&Connection) -> Result<(), String> + Send + 'static,
{
    let conn = Arc::clone(conn);
    tokio::task::spawn_blocking(move || {
        let conn = conn
            .lock()
            .map_err(|e| format!("db lock poisoned during {label}: {e}"))?;
        f(&conn)
    })
    .await
    .map_err(|e| format!("{label} task panicked: {e}"))?
}

/// Boot-time send-vault orphan cleanup (Phase 2 task 5).
///
/// Reads the set of live send job IDs from the journal, then walks
/// `<app_data>/send_vault/` and removes every subdirectory whose
/// PlanId is not in that set. Each step lives behind its own
/// spawn_blocking (the journal query holds the DB lock; the
/// filesystem walk holds neither lock and can be relatively slow on
/// a populated vault tree).
async fn reconcile_send_vault(
    conn: &Arc<Mutex<Connection>>,
    app_data_dir: &std::path::Path,
) -> Result<(), String> {
    let conn = Arc::clone(conn);
    let live_ids: std::collections::HashSet<service_api::PlanId> =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let conn = conn
                .lock()
                .map_err(|e| format!("db lock poisoned: {e}"))?;
            let ids = db::db::action_journal::live_send_job_ids(&conn)?;
            Ok(ids
                .into_iter()
                .map(|bytes| service_api::PlanId(uuid::Uuid::from_bytes(bytes)))
                .collect())
        })
        .await
        .map_err(|e| format!("live_send_job_ids task: {e}"))??;

    let app_data = app_data_dir.to_path_buf();
    let removed = tokio::task::spawn_blocking(move || {
        crate::send_vault::cleanup_orphan_vaults(&app_data, &live_ids)
            .map_err(|e| format!("cleanup_orphan_vaults: {e}"))
    })
    .await
    .map_err(|e| format!("cleanup_orphan_vaults task: {e}"))??;

    if removed > 0 {
        log::info!("[send-vault] removed {removed} orphan vault dir(s) at boot");
    }
    Ok(())
}

/// Per-account thread-participants backfill. Reads accounts + cross-account
/// send-identity emails synchronously, then runs `backfill_thread_participants
/// _for_account_sync` for each account. The helper itself is idempotent (it
/// scans only threads that have no participants row), so this is safe to
/// call on every boot.
///
/// Returns Err if any per-account backfill fails. The outer caller turns
/// that into a `recovery_warnings` entry on `BootReadyResponse`; per-
/// account detail stays in the rolling log file. Without this propagation
/// the UI would see `BootReady` with no warning even though a subset of
/// accounts had partial state repair.
fn run_backfill_for_all_accounts(conn: &Connection) -> Result<(), String> {
    let accounts = get_all_accounts_sync(conn)?;
    if accounts.is_empty() {
        return Ok(());
    }
    let mut user_emails: Vec<String> = accounts
        .iter()
        .map(|a| a.email.to_lowercase())
        .collect();
    for email in get_all_send_identity_emails(conn)? {
        let lower = email.to_lowercase();
        if !user_emails.contains(&lower) {
            user_emails.push(lower);
        }
    }
    let total_accounts = accounts.len();
    let mut failed_accounts: Vec<String> = Vec::new();
    for account in accounts {
        match backfill_thread_participants_for_account_sync(conn, &account.id, &user_emails) {
            Ok(0) => {}
            Ok(count) => log::info!(
                "[chat] thread_participants backfill rebuilt {count} threads for {}",
                account.id
            ),
            Err(error) => {
                log::warn!(
                    "[chat] thread_participants backfill for {} failed: {error}",
                    account.id
                );
                failed_accounts.push(account.id.clone());
            }
        }
    }
    if failed_accounts.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {total_accounts} account(s) failed: {}",
            failed_accounts.len(),
            failed_accounts.join(", "),
        ))
    }
}

/// Outcome of the synchronous DB open + migration step. A struct rather
/// than a magic 3-tuple so future additions (e.g. a "DB was reopened from
/// the post-rename path" flag) can land as named fields.
struct MigrateOutcome {
    conn: Connection,
    schema_version: u32,
    migrations_applied: u32,
}

/// Synchronous DB open + migration step. Runs inside `spawn_blocking` so
/// `rusqlite`'s blocking I/O never starves the dispatch task or the
/// notification writer. Per-step migration progress is pumped via
/// `boot_progress::emit` from the migration runner's callback (try_send is
/// safe from blocking threads since `mpsc::Sender::try_send` is non-async).
fn open_db_and_migrate(
    app_data_dir: &std::path::Path,
    out_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<MigrateOutcome, String> {
    reconcile_velo_rename(app_data_dir)?;
    let db_path = app_data_dir.join("ratatoskr.db");
    let conn = Connection::open(&db_path)
        .map_err(|e| format!("open db {}: {e}", db_path.display()))?;
    apply_standard_pragmas(&conn)?;

    let mut progress = |current: u32, total: u32| {
        // Populate the human-readable message so the splash always has
        // text to render even on a fresh-DB single-migration run where the
        // before-COMMIT and after-COMMIT frames flicker quickly. The UI
        // also derives "Migration N of M" from the structured `current` /
        // `total`; the message is the wire-side label the splash uses
        // when no localised override is wired in.
        let message = if current == 0 {
            format!("Starting migration 1 of {total}")
        } else {
            format!("Applied migration {current} of {total}")
        };
        boot_progress::emit(
            out_tx,
            BootPhase::Migrating { current, total },
            Some(message),
        );
    };
    let migrations_applied = migrations::run_all_with_progress(&conn, &mut progress)?;
    let schema_version = migrations::current_schema_version(&conn)?;
    Ok(MigrateOutcome {
        conn,
        schema_version,
        migrations_applied,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test that constructs a `BootContext` and reads every field.
    /// The struct is `#[allow(dead_code)]` on the encryption key + DB
    /// connection until Phase 2's action service starts consuming them; if
    /// a future PR drops a field thinking it was unused, the dead_code
    /// allow would silence the unused-field warning AND this test would
    /// fail to compile, surfacing the loss before it lands. Locks the
    /// scaffold-for-Phase-2 contract.
    #[test]
    fn boot_context_constructs_with_every_field_readable() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        let ctx = BootContext {
            encryption_key: SecretKey::from_bytes([9u8; 32]),
            db_conn: Arc::new(Mutex::new(conn)),
            schema_version: 100,
            migrations_applied: 1,
            recovery_warnings: vec!["pending-ops recovery".to_string()],
        };
        // Read every field; if Phase 2 drops one of these, the test stops
        // compiling.
        assert_eq!(ctx.encryption_key.expose()[0], 9);
        assert_eq!(ctx.encryption_key.expose().len(), 32);
        assert_eq!(ctx.schema_version, 100);
        assert_eq!(ctx.migrations_applied, 1);
        assert_eq!(ctx.recovery_warnings.len(), 1);
        assert_eq!(ctx.recovery_warnings[0], "pending-ops recovery");
        // The DB connection is held under Arc<Mutex<>>; verify we can
        // acquire it (the shape Phase 2's ActionContext expects).
        let _guard = ctx.db_conn.lock().expect("db_conn mutex");
    }

    /// `signal_ready` symmetry: the success path must populate both
    /// `result` (Some(Ok)) and `context` (Some(_)); the failure path
    /// must populate `result` (Some(Err)) and leave `context` empty.
    /// A future refactor that swaps the args, drops the `if let Some(ctx)`
    /// guard, or otherwise breaks the pairing would only show up in a
    /// downstream Phase 2 test today; this catches it at the boot crate.
    #[test]
    fn signal_ready_symmetry_success_populates_context() {
        let state = BootSharedState::new(PathBuf::new());
        let conn = Connection::open_in_memory().expect("open in-memory db");
        let ctx = BootContext {
            encryption_key: SecretKey::from_bytes([1u8; 32]),
            db_conn: Arc::new(Mutex::new(conn)),
            schema_version: 100,
            migrations_applied: 0,
            recovery_warnings: Vec::new(),
        };
        let response = BootReadyResponse {
            ready: true,
            schema_version: 100,
            migrations_applied: 0,
            recovery_warnings: Vec::new(),
        };
        state.signal_ready(Ok(response.clone()), Some(ctx));
        let got_result = state
            .result
            .lock()
            .expect("result mutex")
            .clone();
        assert!(matches!(got_result, Some(Ok(_))));
        let context_present = state
            .context
            .lock()
            .expect("context mutex")
            .is_some();
        assert!(context_present, "success must populate context");
    }

    #[test]
    fn signal_ready_symmetry_failure_leaves_context_empty() {
        let state = BootSharedState::new(PathBuf::new());
        state.signal_ready(Err(BootFailure::KeyLoadFailure), None);
        let got_result = state
            .result
            .lock()
            .expect("result mutex")
            .clone();
        assert!(matches!(got_result, Some(Err(BootFailure::KeyLoadFailure))));
        let context_present = state
            .context
            .lock()
            .expect("context mutex")
            .is_some();
        assert!(!context_present, "failure must NOT populate context");
    }

    /// Second `signal_ready` is a no-op: the first result wins, the
    /// second is logged at warn (see implementation) and dropped.
    /// Locks the contract that prevents a Phase 2 retry path from
    /// silently flipping a Service from "ready" to "failed" (or vice
    /// versa) after the first signal.
    #[test]
    fn signal_ready_second_call_is_no_op() {
        let state = BootSharedState::new(PathBuf::new());
        let response = BootReadyResponse {
            ready: true,
            schema_version: 100,
            migrations_applied: 0,
            recovery_warnings: Vec::new(),
        };
        state.signal_ready(Ok(response.clone()), None);
        // Second call with a different result: must not overwrite.
        state.signal_ready(Err(BootFailure::MigrationFailure), None);
        let got_result = state
            .result
            .lock()
            .expect("result mutex")
            .clone();
        assert!(matches!(got_result, Some(Ok(_))));
    }
}
