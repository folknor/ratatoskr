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

    /// Run the shutdown drain exactly once. Phase 2+ per-store flushes
    /// (Tantivy commit, pack-file fsync, etc.) will run regardless of
    /// `cause`; today's drain is sentinel-only and therefore short-
    /// circuits on non-graceful causes.
    pub(crate) async fn drain(&self, cause: ShutdownCause) -> bool {
        *self
            .drain_result
            .get_or_init(|| async { self.run_drain(cause).await })
            .await
    }

    async fn run_drain(&self, cause: ShutdownCause) -> bool {
        // Phase 2+: per-store flushes (Tantivy commit, pack-file fsync,
        // any other writer-owned drain step) go here. They run on every
        // exit path because the writer state is unsafe to leave
        // half-flushed regardless of WHY we're exiting. The sentinel
        // write below is the only step that's gated on `cause`.

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
    /// runs (or doesn't) when the opposite was expected, which is benign in
    /// Phase 1.5 because no recovery passes consume the sentinel yet.
    pub(crate) async fn clear_sentinel(&self) {
        let Some(app_data_dir) = self.app_data_dir.as_ref() else {
            return;
        };
        let sentinel = app_data_dir.join("clean_shutdown");
        match tokio::fs::remove_file(&sentinel).await {
            Ok(()) => log::debug!("removed clean_shutdown sentinel at boot"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => log::warn!("failed to remove clean_shutdown sentinel at boot: {error}"),
        }
    }
}
