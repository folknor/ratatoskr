use std::path::PathBuf;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, OnceCell};

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

    pub(crate) async fn drain(&self) -> bool {
        *self
            .drain_result
            .get_or_init(|| async { self.run_drain().await })
            .await
    }

    async fn run_drain(&self) -> bool {
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
