use crate::lifecycle::ServiceLifecycle;

#[cfg(unix)]
pub(crate) fn spawn(lifecycle: ServiceLifecycle) {
    tokio::spawn(async move {
        let signal = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate());
        match signal {
            Ok(mut signal) => {
                signal.recv().await;
                log::info!("received SIGTERM, starting shutdown drain");
                lifecycle.request_shutdown();
            }
            Err(error) => {
                log::warn!("failed to install SIGTERM handler: {error}");
            }
        }
    });
}

#[cfg(not(unix))]
pub(crate) fn spawn(_lifecycle: ServiceLifecycle) {}
