//! Concrete registry of the long-lived tasks the dispatch loop has
//! spawned. Exposes two ordered shutdown entry points:
//!
//! - [`Subsystems::drain_runtimes`] - drains the `BootSharedState`-resident
//!   subsystem runtimes (push, calendar, sync, extract, rebuild) and the
//!   search-writer task. Runs *before* the lifecycle drain so the
//!   `clean_shutdown` sentinel doesn't land while in-flight writes are
//!   still in progress.
//! - [`Subsystems::join_boot`] / [`Subsystems::abort_boot`] - complete
//!   or abort the boot task after in-flight request handlers are
//!   drained, before any clean-shutdown sentinel can be written.
//! - [`Subsystems::abort_startup_tasks`] - aborts post-ready startup
//!   tasks before runtime slots are drained, so they cannot install a
//!   runtime after its slot has already been checked.
//! - [`Subsystems::abort_tasks`] - aborts any remaining long-lived task
//!   handles after the lifecycle drain and optional Shutdown ack so the
//!   writer task gets a chance to flush them before `out_tx` is dropped.
//!
//! Adding a new long-lived dispatch task means adding a field, spawning
//! it inside `init::init_dispatch`, and adding it to the matching
//! [`Subsystems`] join/drain/abort method - one place, three edits.
//! Replaces the open-coded "200 lines into `run_service_with_io`,
//! remember to abort X" pattern that grew the H1/M7/C4/L6 fix-stickers.

use crate::boot::BootSharedState;
use std::sync::Arc;
use tokio::task::JoinHandle;

pub(crate) struct Subsystems {
    /// The boot sequence task. Aborted after drain so the boot.ready
    /// IPC handler (which parks on `wait_for_ready`) doesn't deadlock
    /// the in-flight drain.
    pub boot: Option<JoinHandle<()>>,
    /// The action worker, processing the journal in a long-lived loop.
    /// Holds an `out_tx` clone directly, so it MUST be aborted before
    /// `drop(out_tx)` or the writer task's EOF wait pins forever.
    pub action_worker: JoinHandle<()>,
    /// Post-ready: builds + installs `PushRuntime` once boot.ready
    /// resolves, then spawns per-account starts. Idempotent on shutdown.
    pub push_startup: Option<JoinHandle<()>>,
    /// Post-ready: builds + installs `CalendarRuntime`. Kick-driven, so
    /// it does not iterate accounts.
    pub calendar_startup: Option<JoinHandle<()>>,
    /// Post-ready: builds + installs `ExtractRuntime` and fires the
    /// initial backfill kick. The runtime holds a `SearchWriteHandle`
    /// and `NotificationSender` clone; the drain ordering in
    /// `drain_runtimes` releases them before the search-writer await.
    pub extract_startup: Option<JoinHandle<()>>,
    /// Post-ready: builds + installs `PrefetchRuntime` (attachments
    /// roadmap Phase 4) and fires the initial boot-recovery backfill.
    /// The runtime holds a `NotificationSender` clone; drain ordering
    /// in `drain_runtimes` releases it between sync (which feeds
    /// prefetch) and extract (whose backfill consumes the same rows).
    pub prefetch_startup: Option<JoinHandle<()>>,
    /// Post-ready: dispatches a `PreserveExisting` rebuild when boot
    /// detected a `.version` mismatch. No-op in the steady state.
    pub schema_rebuild: Option<JoinHandle<()>>,
}

impl Subsystems {
    /// Drain every `BootSharedState`-resident runtime in load-bearing
    /// order, then await the search-writer task. After this returns,
    /// every clone of `out_tx` and `SearchWriteHandle` held by a
    /// runtime, rebuild task, or the search-writer task has been
    /// released.
    ///
    /// Order matters:
    ///
    /// - **Push -> Sync** is load-bearing: a `StateChange` mid-shutdown
    ///   would otherwise call `SyncRuntime::start_account` after sync
    ///   has begun draining, leaking a runner past the drain.
    /// - **Calendar -> Sync** is reserved (not load-bearing today), so
    ///   a future change wiring calendar-cancel cleanup to dispatch
    ///   action plans (RSVP send is the candidate) is a one-liner
    ///   instead of a drain reshuffle.
    /// - **Extract -> search-writer await** is load-bearing: the
    ///   extract worker holds a `SearchWriteHandle` clone, and the
    ///   writer task only exits when every clone has been dropped.
    /// - **mark_shutting_down before take_extract_runtime**: the H1
    ///   fix - the post-ready spawn checks the flag inside
    ///   `install_extract_runtime`'s mutex; if set, the runtime is
    ///   dropped instead of installed, which releases its handle clone.
    pub async fn drain_runtimes(boot_state: &Arc<BootSharedState>) {
        // The shutdown drain normally fires this before draining
        // in-flight notifications so GAL can observe it. Keep the call
        // here as an idempotent backstop for direct/future call sites.
        boot_state.shutdown_token().cancel();

        boot_state.mark_shutting_down();
        drain_push(boot_state).await;
        drain_calendar(boot_state).await;
        drain_sync(boot_state).await;
        drain_prefetch(boot_state).await;
        drain_extract(boot_state).await;
        drain_rebuild(boot_state).await;
        // Defensive: clear the slots that handlers may have populated
        // without a runtime ever taking ownership. The takes return
        // Option<...>; dropping them releases the clones.
        let _ = boot_state.take_search_write();
        let _ = boot_state.take_out_tx();
        // Search-writer task observes EOF when every SearchWriteHandle
        // clone is dropped; the drains above released the last ones.
        if let Some(handle) = boot_state.take_search_writer_handle()
            && let Err(e) = handle.await
        {
            log::warn!("search-writer task join error during shutdown: {e}");
        }
        // Attachments roadmap Phase 3: fsync the open pack before the
        // clean-shutdown sentinel write. PackStore's `put` already
        // fsyncs per frame, so this only catches frames that landed in
        // the open pack between the last put and shutdown - effectively
        // a no-op on the durability side, but it pairs the sentinel
        // write with one final flush for symmetry with the body /
        // search drains above.
        if let Some(pack_store) = boot_state.take_pack_store()
            && let Err(e) = pack_store.flush().await
        {
            log::warn!("PackStore flush during shutdown failed: {e}");
        }
    }

    /// Abort the long-lived task handles owned by this registry. Runs
    /// after the lifecycle drain so the Shutdown ack has a chance to
    /// reach the writer task while it still has `out_tx` clones.
    ///
    /// Ordering note: the boot task aborts AFTER the in-flight drain
    /// (which the shutdown drain runs before calling `abort_tasks`)
    /// because the `boot.ready` IPC handler parks on
    /// `BootSharedState::wait_for_ready` and would never return if we
    /// aborted boot first.
    pub async fn join_boot(&mut self) {
        if let Some(handle) = self.boot.take()
            && let Err(e) = handle.await
            && !e.is_cancelled()
        {
            log::warn!("boot task join error during shutdown: {e}");
        }
    }

    pub async fn abort_boot(&mut self) {
        if let Some(handle) = self.boot.take() {
            abort_and_await(handle, "boot").await;
        }
    }

    pub async fn abort_startup_tasks(&mut self) {
        if let Some(handle) = self.push_startup.take() {
            abort_and_await(handle, "push_startup").await;
        }
        if let Some(handle) = self.calendar_startup.take() {
            abort_and_await(handle, "calendar_startup").await;
        }
        if let Some(handle) = self.extract_startup.take() {
            abort_and_await(handle, "extract_startup").await;
        }
        if let Some(handle) = self.prefetch_startup.take() {
            abort_and_await(handle, "prefetch_startup").await;
        }
        if let Some(handle) = self.schema_rebuild.take() {
            abort_and_await(handle, "schema_rebuild").await;
        }
    }

    pub async fn abort_tasks(mut self) {
        self.abort_boot().await;
        self.abort_startup_tasks().await;
        abort_and_await(self.action_worker, "action_worker").await;
    }
}

async fn drain_push(boot_state: &Arc<BootSharedState>) {
    if let Some(runtime) = boot_state.take_push_runtime() {
        runtime.shutdown().await;
        drop(runtime);
    }
}

async fn drain_calendar(boot_state: &Arc<BootSharedState>) {
    if let Some(runtime) = boot_state.take_calendar_runtime() {
        runtime.shutdown().await;
        drop(runtime);
    }
}

async fn drain_sync(boot_state: &Arc<BootSharedState>) {
    if let Some(runtime) = boot_state.take_sync_runtime() {
        runtime.shutdown().await;
        drop(runtime);
    }
}

async fn drain_extract(boot_state: &Arc<BootSharedState>) {
    if let Some(runtime) = boot_state.take_extract_runtime() {
        runtime.shutdown().await;
        drop(runtime);
    }
}

async fn drain_prefetch(boot_state: &Arc<BootSharedState>) {
    if let Some(runtime) = boot_state.take_prefetch_runtime() {
        runtime.shutdown().await;
        drop(runtime);
    }
}

async fn drain_rebuild(boot_state: &Arc<BootSharedState>) {
    if let Some(rebuild) = boot_state.take_rebuild_task() {
        log::info!(
            "shutdown: cancelling in-flight rebuild {}",
            rebuild.rebuild_id,
        );
        rebuild.cancel.cancel();
        rebuild.handle.abort();
        let _ = rebuild.handle.await;
    }
}

async fn abort_and_await(handle: JoinHandle<()>, name: &'static str) {
    handle.abort();
    if let Err(e) = handle.await
        && !e.is_cancelled()
    {
        log::warn!("{name} task join error during shutdown: {e}");
    }
}
