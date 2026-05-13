use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, OnceCell};

/// Why the Service is shutting down. Determines whether the dispatch tail
/// writes the `clean_shutdown` sentinel: only `GracefulRequest` does, so
/// every other exit path triggers the recovery scan on the next
/// boot. This matches the cross-store crash-consistency contract in
/// `docs/architecture.md` modulo the simplification that we collapse
/// parent-death, external
/// SIGTERM, and plain stdin EOF into one `Unrequested` arm - the
/// distinction between them does not change Phase 1.5 behavior, and
/// recovery scans are idempotent so the cost of "scan when we could have
/// trusted the sentinel" is bounded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownCause {
    /// UI sent a `Shutdown` JSON-RPC request and we are acking after the
    /// drain completes. This is the only path that writes the
    /// `clean_shutdown` sentinel.
    GracefulRequest,
    /// Boot sequence reported a fatal failure (`KeyLoadFailure`,
    /// `MigrationFailure`, etc.). Service is exiting non-zero. No
    /// sentinel - the previous run did not complete its work, so the
    /// next boot's recovery scan must fire.
    BootFailure,
    /// Loop exit triggered by anything other than a `Shutdown` request:
    /// parent-death PR_SET_PDEATHSIG-induced SIGTERM, an external
    /// operator `kill -TERM <pid>`, stdin EOF without a Shutdown
    /// handshake, or a SIGTERM-handler call to `request_shutdown` from
    /// any other source. None of these are clean shutdowns from the
    /// UI's perspective, so the sentinel stays absent.
    Unrequested,
}

#[derive(Clone)]
pub(crate) struct ServiceLifecycle {
    notify: Arc<Notify>,
    requested: Arc<AtomicBool>,
    drain_result: Arc<OnceCell<bool>>,
    app_data_dir: Option<Arc<PathBuf>>,
}

impl ServiceLifecycle {
    pub(crate) fn new(app_data_dir: Option<PathBuf>) -> Self {
        Self {
            notify: Arc::new(Notify::new()),
            requested: Arc::new(AtomicBool::new(false)),
            drain_result: Arc::new(OnceCell::new()),
            app_data_dir: app_data_dir.map(Arc::new),
        }
    }

    pub(crate) fn request_shutdown(&self) {
        self.requested.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub(crate) fn is_requested(&self) -> bool {
        self.requested.load(Ordering::SeqCst)
    }

    pub(crate) async fn notified(&self) {
        if self.is_requested() {
            return;
        }
        self.notify.notified().await;
    }

    /// Write the `clean_shutdown` sentinel exactly once, gated on
    /// `cause`.
    ///
    /// # Drain ordering (Phase 4 + later additions)
    ///
    /// L7 fix: refreshed to reflect Phase 5 (Calendar between Push
    /// and Sync) and Phase 7 (Extract before search-writer + Rebuild
    /// after Extract).
    ///
    /// The sentinel write is no longer the entirety of the drain. The
    /// orchestrating helper in `dispatch::run_shutdown_drain` calls
    /// subsystem shutdowns *before* this method:
    ///
    /// 1. `PushRuntime::shutdown()` (Phase 4) - cancel push bridges
    ///    so a late `StateChange` can't call
    ///    `SyncRuntime::start_account` after step 2.
    /// 2. `CalendarRuntime::shutdown()` (Phase 5) - cancel + await
    ///    calendar runners.
    /// 3. `SyncRuntime::shutdown()` (Phase 3) - cancel + await sync
    ///    runners.
    /// 4. Mark BootSharedState shutting_down (Phase 7 H1) +
    ///    `take_extract_runtime` -> `ExtractRuntime::shutdown()`
    ///    (Phase 7) - cancel + await worker + drain per-item JoinSet.
    /// 5. `take_rebuild_task` -> cancel + abort + await (Phase 7).
    /// 6. Drop `Arc<SyncRuntime>` and clear single-use `search_write`
    ///    slot so `SearchWriteHandle` clones release.
    /// 7. Await search-writer `JoinHandle` (via `out_tx` drop +
    ///    `writer_handle.await` in dispatch).
    /// 8. **Then** this method: write the `clean_shutdown` sentinel.
    ///
    /// Calling this *before* steps 1-7 (the pre-Phase-4 layout) was
    /// racy: a sync runner mid-write could land bytes after the
    /// sentinel claimed clean state, leaving the next boot's invariant
    /// pass unable to detect the gap. The drain consolidation in
    /// `dispatch.rs` is what closes the race.
    ///
    /// PackStore is intentionally absent from the step list. The
    /// attachments roadmap Phase 3 boot lifecycle sketched an explicit
    /// `PackStore::flush()` call here, but `PackStore::put` already
    /// fsyncs every frame + commits the matching index row inside a
    /// single SQLite transaction, so any blob whose `put` returned has
    /// already landed durably. By the time this sentinel runs steps 1-7
    /// have stopped every subsystem that could call `put`, so there is
    /// nothing to flush. The on-disk state is consistent without any
    /// extra ordering.
    pub(crate) async fn drain(&self, cause: ShutdownCause) -> bool {
        *self
            .drain_result
            .get_or_init(|| async { self.run_drain(cause).await })
            .await
    }

    async fn run_drain(&self, cause: ShutdownCause) -> bool {
        if !matches!(cause, ShutdownCause::GracefulRequest) {
            // Non-graceful exit (boot failure, parent-death, SIGTERM,
            // stdin EOF). Leave the sentinel absent so the next boot's
            // Phase 3+ recovery scan fires. The drain "succeeded" in the
            // only sense that matters for `flushed_ok` reporting:
            // nothing was supposed to land on disk on this path.
            return true;
        }
        let Some(app_data_dir) = self.app_data_dir.as_ref() else {
            return true;
        };
        let sentinel = app_data_dir.join("clean_shutdown");
        match tokio::fs::write(&sentinel, b"clean\n").await {
            Ok(()) => true,
            Err(error) => {
                log::warn!("failed to write clean shutdown sentinel: {error}");
                false
            }
        }
    }

    /// Remove the `clean_shutdown` sentinel at boot. Must run after the
    /// instance lock has been acquired (so a contending second instance can't
    /// remove the live one's sentinel). The sentinel is a "last shutdown was
    /// clean" marker; absence at boot tells Phase 3+ recovery passes that the
    /// previous run did not finish its drain. Without this clear-at-boot, the
    /// sentinel would persist across reboots and recovery would never fire.
    ///
    /// Errors are logged at warn and ignored - the worst case is that recovery
    /// runs (or doesn't) when the opposite was expected.
    ///
    /// Returns `true` if the sentinel was present and successfully removed
    /// (i.e., the previous shutdown was graceful), `false` otherwise. Phase 3
    /// uses the return value to gate the cross-store invariant pass: a `true`
    /// return means the previous Service drained cleanly and skipped the
    /// pass; a `false` return triggers it.
    pub(crate) async fn clear_sentinel(&self) -> bool {
        let Some(app_data_dir) = self.app_data_dir.as_ref() else {
            return false;
        };
        let sentinel = app_data_dir.join("clean_shutdown");
        match tokio::fs::remove_file(&sentinel).await {
            Ok(()) => {
                log::debug!("removed clean_shutdown sentinel at boot");
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => {
                log::warn!("failed to remove clean_shutdown sentinel at boot: {error}");
                false
            }
        }
    }
}
