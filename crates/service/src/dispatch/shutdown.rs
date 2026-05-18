//! The shutdown drain. Eleven named steps; the per-subsystem teardown
//! lives in [`crate::subsystems::Subsystems`].
//!
//! Order matters and is documented inline; if you add a new long-lived
//! task to the dispatch lifecycle, register it in `Subsystems` rather
//! than open-coding `take_X()` + `await` here. That keeps the drain
//! ordering invariants in one named place instead of growing
//! fix-stickers (H1, M7, C4, L6) inside this function.

use crate::boot::BootSharedState;
use crate::dispatch::config::NOTIFICATION_DRAIN_BOUND;
use crate::dispatch::handlers::{panic_message, send_handler_response};
use crate::dispatch::state::DispatchState;
use crate::lifecycle::{ServiceLifecycle, ShutdownCause};
use crate::subsystems::Subsystems;
use futures_util::FutureExt;
use service_api::{ServiceError, ShutdownResponse};
use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use tokio::io::AsyncRead;
use tokio::task::JoinSet;

pub(crate) async fn run_shutdown_drain<R>(mut state: DispatchState<R>) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
{
    // 1. Drain request handlers before firing cooperative
    //    cancellation. Request handlers can enqueue journal work and
    //    wake the action worker; cancelling first would let the worker
    //    exit before seeing the last wakeup.
    drain_in_flight(&mut state.handlers_in_flight).await;

    // 2. Fire cooperative cancellation before draining in-flight
    //    notifications. Notification handlers like GAL check this
    //    token between long-running account iterations, so it must be
    //    visible before the notification drain waits on them.
    state.boot_state.shutdown_token().cancel();

    // 3. Drain in-flight notifications before writing any sentinel.
    //    The boot task cannot be safely aborted until after the
    //    request drain above because the `boot.ready` handler parks on
    //    `wait_for_ready`.
    drain_notifications_bounded(&mut state.notifications_in_flight, NOTIFICATION_DRAIN_BOUND).await;

    // 4. Resolve the boot task before any clean-shutdown sentinel can
    //    be written. For an explicit Shutdown request, let boot finish:
    //    some in-process clients never call boot.ready but still expect
    //    Shutdown to drain cleanly once boot has actually completed. For
    //    unrequested exits, abort boot so post-ready startup tasks cannot
    //    race runtime installs after the runtime drain.
    if state.pending_shutdown_id.is_some() && state.boot_exit_code.is_none() {
        state.subsystems.join_boot().await;
        while let Ok(code) = state.boot_failure_rx.try_recv() {
            state.boot_exit_code = Some(code);
        }
    }
    state.subsystems.abort_boot().await;

    // Stop post-ready startup tasks before draining runtime slots. If
    // boot just completed due to the join above, these tasks may have
    // woken but not yet installed their runtimes; aborting them here
    // prevents a late install after the matching drain step has passed.
    state.subsystems.abort_startup_tasks().await;

    // 5. Decide the shutdown cause from the dispatch loop's exit
    //    reason. Order: BootFailure > GracefulRequest > Unrequested.
    let cause = decide_shutdown_cause(&state);

    // 6. Drain the BootSharedState-resident subsystem runtimes (push,
    //    calendar, sync, extract, rebuild) and await the search-writer
    //    task. This releases every `NotificationSender` and
    //    `SearchWriteHandle` clone the runtimes own.
    Subsystems::drain_runtimes(&state.boot_state).await;

    // 7. Lifecycle drain (sentinel + Phase-3 hooks). Idempotent via the
    //    `OnceCell` inside `ServiceLifecycle`; safe under panic.
    let flushed_ok = panic_safe_drain(&state.lifecycle, cause).await;
    if !flushed_ok {
        log::warn!("shutdown drain completed with errors");
    }

    // 8. Ack the pending Shutdown request, if any. Suppressed on boot
    //    failure - answering "shutdown ok" while exiting with code
    //    71/72/73 is misleading in log triage; the UI's
    //    shutdown-request future returns `ServiceCrashed` which is
    //    correct for a Service exiting mid-shutdown.
    maybe_send_shutdown_ack(&state, cause, flushed_ok).await;

    // 9. Abort the remaining long-lived task handles (action worker, four
    //    post-ready startup tasks). The action worker holds an
    //    `out_tx` clone directly; the post-ready tasks hold clones
    //    through their constructed runtimes (which step 6 drained).
    state.subsystems.abort_tasks().await;

    // 10. Phase 8-2: advance the per-store `clean_shutdown_cursors` rows
    //    so the next dirty boot's invariant pass can bound its scans.
    //    Skipped on non-graceful exits.
    maybe_advance_cursors(&state.boot_state, cause).await;

    // 11. Drop the last out_tx clone so the writer task observes EOF,
    //    then await its termination.
    drop(state.out_tx);
    let _ = state.writer_handle.await;

    let drops = state
        .diagnostic_drops
        .load(std::sync::atomic::Ordering::Relaxed);
    if drops > 0 {
        log::warn!(
            "dispatch shutdown: {drops} diagnostic error response(s) dropped (outbound queue full)",
        );
    }

    state
        .boot_exit_code
        .map(service_api::BootExitCode::as_i32)
        .unwrap_or(0)
}

fn decide_shutdown_cause<R>(state: &DispatchState<R>) -> ShutdownCause {
    if state.boot_exit_code.is_some() {
        ShutdownCause::BootFailure
    } else if state.pending_shutdown_id.is_some() && state.boot_state.boot_succeeded() {
        ShutdownCause::GracefulRequest
    } else {
        ShutdownCause::Unrequested
    }
}

async fn maybe_send_shutdown_ack<R>(
    state: &DispatchState<R>,
    cause: ShutdownCause,
    flushed_ok: bool,
) {
    let Some(id) = state.pending_shutdown_id else {
        return;
    };
    if state.boot_exit_code.is_some() {
        log::info!("dispatch end method=shutdown id={id} outcome=skipped_boot_failed");
        return;
    }
    let flushed_ok = flushed_ok && matches!(cause, ShutdownCause::GracefulRequest);
    let outcome = if flushed_ok { "ok" } else { "internal" };
    log::info!("dispatch end method=shutdown id={id} outcome={outcome}");
    let result = serde_json::to_value(ShutdownResponse { flushed_ok })
        .map_err(|error| ServiceError::Internal(error.to_string()));
    send_handler_response(&state.out_tx, id, result).await;
}

/// Phase 8-2: advance the per-store `clean_shutdown_cursors` rows so
/// the next dirty boot's invariant pass can bound its scans. Skipped
/// on non-graceful exits (cursor-as-of-now would mis-advertise
/// unflushed work as "clean").
///
/// Uses a fresh `Connection::open` rather than the shared writer mutex.
/// The shared `WriteDbState::with_conn` path hung when an
/// aborted-but-still-running `spawn_blocking` from the action worker
/// held the rust-level `Mutex<Connection>`. SQLite WAL handles
/// multi-connection write contention via its busy timeout.
async fn maybe_advance_cursors(boot_state: &Arc<BootSharedState>, cause: ShutdownCause) {
    if !matches!(cause, ShutdownCause::GracefulRequest) {
        return;
    }
    let app_data_dir = boot_state.app_data_dir().to_path_buf();
    let result = tokio::task::spawn_blocking(move || {
        let pool = ::db::db::open_writer_pool(&app_data_dir)
            .map_err(|e| format!("open writer pool for cursor write: {e}"))?;
        pool.with_write_sync(|conn| {
            ::db::db::queries_extra::update_clean_shutdown_cursors(
                conn,
                &["body", "inline", "extract"],
            )
        })
    })
    .await;
    match result {
        Ok(Ok(())) => log::debug!("invariant pass: clean_shutdown_cursors advanced"),
        Ok(Err(e)) => log::warn!(
            "invariant pass: clean_shutdown_cursors update failed: {e}; \
             next dirty boot will scan more rows than ideal"
        ),
        Err(e) => log::warn!("invariant pass: cursor join failed: {e}"),
    }
}

async fn drain_in_flight(handlers: &mut JoinSet<()>) {
    while let Some(result) = handlers.join_next().await {
        if let Err(error) = result {
            // The per-handler catch_unwind already converted handler
            // panics into ServiceError::Panic, so reaching here implies
            // the wrapper itself panicked or the task was cancelled.
            // Log and continue so a single bad handler can't hold up
            // shutdown.
            log::warn!("in-flight handler join failed during drain: {error}");
        }
    }
}

async fn drain_notifications_bounded(handlers: &mut JoinSet<()>, bound: std::time::Duration) {
    let deadline = tokio::time::Instant::now() + bound;
    loop {
        if handlers.is_empty() {
            return;
        }
        let now = tokio::time::Instant::now();
        if now >= deadline {
            let remaining = handlers.len();
            log::warn!(
                "notification drain timed out after {} s, aborting {} task(s)",
                bound.as_secs(),
                remaining,
            );
            handlers.abort_all();
            while handlers.join_next().await.is_some() {}
            return;
        }
        match tokio::time::timeout_at(deadline, handlers.join_next()).await {
            Ok(Some(result)) => {
                if let Err(error) = result {
                    log::warn!("notification join failed during drain: {error}");
                }
            }
            Ok(None) => return,
            Err(_) => continue,
        }
    }
}

async fn panic_safe_drain(lifecycle: &ServiceLifecycle, cause: ShutdownCause) -> bool {
    match AssertUnwindSafe(lifecycle.drain(cause)).catch_unwind().await {
        Ok(value) => value,
        Err(panic) => {
            log::error!("shutdown drain panicked: {}", panic_message(panic.as_ref()));
            false
        }
    }
}
