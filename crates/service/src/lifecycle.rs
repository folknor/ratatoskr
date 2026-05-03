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
}
