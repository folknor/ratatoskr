//! Concrete registry of the long-lived tasks the dispatch loop has
//! spawned. Exposes two ordered shutdown entry points:
//!
//! - [`Subsystems::drain_runtimes`] — drains the `BootSharedState`-resident
//!   subsystem runtimes (push, calendar, sync, extract, rebuild) and the
//!   search-writer task. Runs *before* the lifecycle drain so the
//!   `clean_shutdown` sentinel doesn't land while in-flight writes are
//!   still in progress.
//! - [`Subsystems::abort_tasks`] — aborts the boot task, action worker,
//!   and the four post-ready startup tasks. Runs *after* the lifecycle
//!   drain and the optional Shutdown ack so the writer task gets a
//!   chance to flush them before `out_tx` is dropped.
//!
//! Adding a new long-lived dispatch task means adding a field, spawning
//! it inside `init::init_dispatch`, and adding an `abort_and_await`
//! call inside [`Subsystems::abort_tasks`] - one place, three edits.
//! Replaces the open-coded "200 lines into `run_service_with_io`,
//! remember to abort X" pattern that grew the H1/M7/C4/L6 fix-stickers.

use crate::boot::BootSharedState;
use std::sync::Arc;
use tokio::task::JoinHandle;

pub(crate) struct Subsystems {
    /// The boot sequence task. Aborted after drain so the boot.ready
    /// IPC handler (which parks on `wait_for_ready`) doesn't deadlock
    /// the in-flight drain.
    pub boot: JoinHandle<()>,
    /// The action worker, processing the journal in a long-lived loop.
    /// Holds an `out_tx` clone directly, so it MUST be aborted before
    /// `drop(out_tx)` or the writer task's EOF wait pins forever.
    pub action_worker: JoinHandle<()>,
    /// Post-ready: builds + installs `PushRuntime` once boot.ready
    /// resolves, then spawns per-account starts. Idempotent on shutdown.
    pub push_startup: JoinHandle<()>,
    /// Post-ready: builds + installs `CalendarRuntime`. Kick-driven, so
    /// it does not iterate accounts.
    pub calendar_startup: JoinHandle<()>,
    /// Post-ready: builds + installs `ExtractRuntime` and fires the
    /// initial backfill kick. The runtime holds a `SearchWriteHandle`
    /// and `NotificationSender` clone; the drain ordering in
    /// `drain_runtimes` releases them before the search-writer await.
    pub extract_startup: JoinHandle<()>,
    /// Post-ready: dispatches a `PreserveExisting` rebuild when boot
    /// detected a `.version` mismatch. No-op in the steady state.
    pub schema_rebuild: JoinHandle<()>,
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
        drain_push(boot_state).await;
        drain_calendar(boot_state).await;
        drain_sync(boot_state).await;
        boot_state.mark_shutting_down();
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
    pub async fn abort_tasks(self) {
        abort_and_await(self.boot, "boot").await;
        abort_and_await(self.action_worker, "action_worker").await;
        abort_and_await(self.push_startup, "push_startup").await;
        abort_and_await(self.calendar_startup, "calendar_startup").await;
        abort_and_await(self.extract_startup, "extract_startup").await;
        abort_and_await(self.schema_rebuild, "schema_rebuild").await;
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
