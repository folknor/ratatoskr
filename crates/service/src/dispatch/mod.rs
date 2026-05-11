//! Service dispatch loop.
//!
//! Three named phases run in sequence:
//!
//! 1. [`init::init_dispatch`] - build [`state::DispatchState`]: spawn
//!    the writer task, boot task, action worker, and four post-ready
//!    startup tasks; clone `out_tx` into each holder; install the
//!    parsed [`config::DispatchConfig`] on [`crate::boot::BootSharedState`].
//! 2. [`loop_body::run_dispatch_loop`] - `tokio::select!` over
//!    `lifecycle.notified()`, the boot-failure channel, and stdin
//!    frames. Exits on graceful shutdown, fatal boot failure, EOF, or
//!    a fatal frame I/O error.
//! 3. [`shutdown::run_shutdown_drain`] - drain in-flight handlers and
//!    delegate to [`crate::subsystems::Subsystems`] for the ordered
//!    teardown, write the `clean_shutdown` sentinel, ack any pending
//!    Shutdown request, drop `out_tx`, await the writer task.
//!
//! Adding a new long-lived dispatch task:
//!
//! 1. Add a field to [`crate::subsystems::Subsystems`].
//! 2. Spawn it inside [`init::init_dispatch`] and populate the field
//!    in the `Subsystems { ... }` literal.
//! 3. Add an `abort_and_await` call inside
//!    [`crate::subsystems::Subsystems::abort_tasks`].
//!
//! If the task holds a `BootSharedState`-resident runtime (push,
//! calendar, sync, extract, rebuild), add a `drain_*` step inside
//! [`crate::subsystems::Subsystems::drain_runtimes`] in the
//! load-bearing order documented there. If the task performs
//! long-running or multi-stage work, also have it `select!` on
//! `boot_state.shutdown_token().cancelled()` so it exits promptly
//! instead of waiting out per-iteration timeouts.

mod config;
mod handlers;
mod init;
mod loop_body;
mod post_ready;
mod shutdown;
mod state;

pub use config::DispatchConfig;

use crate::lifecycle::ServiceLifecycle;
use std::path::PathBuf;
use tokio::io::{AsyncRead, AsyncWrite};

/// Top-level Service entry point used by `run_service_blocking`.
///
/// Parses the test-only knobs from argv via
/// [`DispatchConfig::from_cli_args`] and delegates to
/// [`run_service_with_io_and_lifecycle`] with a fresh lifecycle.
pub async fn run_service_with_io<R, W>(reader: R, writer: W, app_data_dir: PathBuf) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let lifecycle = ServiceLifecycle::new(Some(app_data_dir.clone()));
    let config = DispatchConfig::from_cli_args();
    run_service_with_io_and_lifecycle(reader, writer, lifecycle, config, app_data_dir).await
}

/// Run the three-phase dispatch sequence against a caller-supplied
/// lifecycle and config. Used directly by `run_service_blocking` (to
/// share the lifecycle with the SIGTERM handler) and by the in-process
/// integration tests (which inject duplex pipes and a synthetic
/// lifecycle).
pub(crate) async fn run_service_with_io_and_lifecycle<R, W>(
    reader: R,
    writer: W,
    lifecycle: ServiceLifecycle,
    config: DispatchConfig,
    app_data_dir: PathBuf,
) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let mut state = init::init_dispatch(reader, writer, lifecycle, config, app_data_dir).await;
    loop_body::run_dispatch_loop(&mut state).await;
    shutdown::run_shutdown_drain(state).await
}
