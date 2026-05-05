use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, OnceCell};

/// Why the Service is shutting down. Determines whether the dispatch tail
/// writes the `clean_shutdown` sentinel: only `GracefulRequest` does, so
/// every other exit path triggers the Phase 3+ recovery scan on the next
/// boot. This matches the user-visible exit-path matrix in
/// `docs/service/problem-statement.md` § "Cross-store crash consistency"
/// modulo the simplification that we collapse parent-death, external
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
    /// # Phase 4 drain ordering
    ///
    /// As of Phase 4 task 4, the sentinel write is no longer the
    /// entirety of the drain. The orchestrating helper in
    /// `dispatch::run_shutdown_drain` calls subsystem shutdowns
    /// *before* this method:
    ///
    /// 1. `PushRuntime::shutdown()` (Phase 4) - cancel push bridges
    ///    so a late `StateChange` can't call
    ///    `SyncRuntime::start_account` after step 2.
    /// 2. `SyncRuntime::shutdown()` (Phase 3) - cancel + await runners.
    /// 3. Drop `Arc<SyncRuntime>` so `SearchWriteHandle` releases.
    /// 4. Await search-writer `JoinHandle` (via `out_tx` drop +
    ///    `writer_handle.await` in dispatch).
    /// 5. **Then** this method: write the `clean_shutdown` sentinel.
    ///
    /// Calling this *before* steps 1-4 (the pre-Phase-4 layout) was
    /// racy: a sync runner mid-write could land bytes after the
    /// sentinel claimed clean state, leaving the next boot's invariant
    /// pass unable to detect the gap. The drain consolidation in
    /// `dispatch.rs` is what closes the race.
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
