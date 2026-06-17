//! Build `DispatchState` from the inputs the caller hands to
//! `run_service_with_io_and_lifecycle`. Spawns the writer task, boot
//! task, action worker, and the four post-ready startup tasks; wires
//! every `out_tx` clone into the right place so the shutdown drain
//! can release them in order.

use crate::boot;
use crate::dispatch::config::{DispatchConfig, MAX_IN_FLIGHT, OUTBOUND_QUEUE_CAP};
use crate::dispatch::handlers::writer_task;
use crate::dispatch::post_ready::{
    spawn_post_ready_calendar_startup, spawn_post_ready_extract_startup,
    spawn_post_ready_prefetch_startup, spawn_post_ready_push_startup,
    spawn_post_ready_schema_rebuild,
};
use crate::dispatch::state::DispatchState;
use crate::lifecycle::ServiceLifecycle;
use crate::subsystems::Subsystems;
use service_api::{BootExitCode, BoundedLineReader};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

pub(crate) async fn init_dispatch<R, W>(
    reader: R,
    writer: W,
    lifecycle: ServiceLifecycle,
    config: DispatchConfig,
    app_data_dir: PathBuf,
) -> DispatchState<R>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let started_at = Instant::now();

    // Clear the clean_shutdown sentinel before the boot sequence runs.
    // Phase 3 cross-store recovery uses sentinel-absent-at-boot as its
    // trigger; without this, the marker would persist across reboots
    // and recovery would never fire. The return value (`true` if the
    // sentinel was present, i.e., the prior shutdown was graceful)
    // gates the Phase 3 invariant pass inside the boot sequence.
    let had_clean_shutdown = lifecycle.clear_sentinel().await;

    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE_CAP);
    let writer_handle = tokio::spawn(writer_task(writer, out_rx));
    let inflight = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    let handlers_in_flight: JoinSet<()> = JoinSet::new();
    let notifications_in_flight: JoinSet<()> = JoinSet::new();
    let lines = BoundedLineReader::new(reader, service_api::MAX_FRAME_BYTES);

    // Per-instance boot state. Held in an Arc so tests that spawn
    // multiple Service instances don't collide on a process-wide
    // singleton. Carries the parsed `DispatchConfig` so handlers
    // (`health.ping`, the boot sequence) can read test-only knobs
    // without re-parsing argv.
    let boot_state = boot::BootSharedState::new(app_data_dir.clone(), config.clone());

    // Phase 7-9: hand a clone of out_tx to BootSharedState so non-spawn
    // handlers (the `index.rebuild` handler) can mint a
    // NotificationSender for their tracked task. Cleared on drain
    // before drop(out_tx) so the writer-handle await observes EOF.
    boot_state.install_out_tx(out_tx.clone());

    // Boot sequence runs concurrently with the dispatch loop so
    // `health.ping` continues to round-trip while migrations / key
    // load run. On fatal boot failure the sequence posts the boot
    // exit code via `boot_failure_tx`; the dispatch loop's select!
    // breaks out on that event so the Service exits promptly with
    // the right code.
    let (boot_failure_tx, boot_failure_rx) = mpsc::channel::<BootExitCode>(1);
    let boot_handle = tokio::spawn({
        let out_tx = out_tx.clone();
        let app_data_dir = app_data_dir.clone();
        let boot_state = Arc::clone(&boot_state);
        async move {
            if let Err(failure) =
                boot::run_boot_sequence(out_tx, app_data_dir, boot_state, had_clean_shutdown).await
            {
                let _ = boot_failure_tx.send(failure.as_exit_code()).await;
            }
        }
    });

    // Phase 2 task 9c: the action worker awaits `boot_state.wait_for_ready()`
    // internally before touching the journal, so spawn order against the
    // boot task does not matter. The worker holds a clone of out_tx; the
    // shutdown drain aborts it BEFORE dropping out_tx so the writer
    // handle's EOF wait is not pinned.
    let action_worker_handle = crate::actions::worker::spawn(
        Arc::clone(&boot_state),
        out_tx.clone(),
        app_data_dir.clone(),
    );

    let push_startup_handle =
        spawn_post_ready_push_startup(Arc::clone(&boot_state), out_tx.clone());
    let calendar_startup_handle =
        spawn_post_ready_calendar_startup(Arc::clone(&boot_state), out_tx.clone());
    let extract_startup_handle = spawn_post_ready_extract_startup(
        Arc::clone(&boot_state),
        out_tx.clone(),
        app_data_dir.clone(),
    );
    let prefetch_startup_handle =
        spawn_post_ready_prefetch_startup(Arc::clone(&boot_state), out_tx.clone());
    let schema_rebuild_handle =
        spawn_post_ready_schema_rebuild(Arc::clone(&boot_state), app_data_dir.clone());

    DispatchState {
        started_at,
        lifecycle,
        config,
        out_tx,
        writer_handle,
        inflight,
        handlers_in_flight,
        notifications_in_flight,
        lines,
        boot_state,
        boot_failure_rx,
        diagnostic_drops: Arc::new(AtomicU64::new(0)),
        subsystems: Subsystems {
            boot: Some(boot_handle),
            action_worker: action_worker_handle,
            push_startup: Some(push_startup_handle),
            calendar_startup: Some(calendar_startup_handle),
            extract_startup: Some(extract_startup_handle),
            prefetch_startup: Some(prefetch_startup_handle),
            schema_rebuild: Some(schema_rebuild_handle),
        },
        pending_shutdown_id: None,
        boot_exit_code: None,
    }
}
