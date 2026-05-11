//! The shutdown drain. Consumes the `DispatchState` produced by
//! `init_dispatch` after the dispatch loop has broken out, drains every
//! in-flight handler and every subsystem in the load-bearing order, runs
//! the lifecycle sentinel write, optionally acks a pending Shutdown
//! request, and finally lets the writer task observe EOF on `out_tx`.
//!
//! Phase 1 of the bulletproofing refactor keeps the body of this drain
//! verbatim - it moved from the old monolithic function but did not
//! restructure. Phase 2 splits it into a `Subsystems` registry that
//! collapses the per-subsystem drain steps into one named place.

use crate::dispatch::config::NOTIFICATION_DRAIN_BOUND;
use crate::dispatch::handlers::{panic_message, send_handler_response};
use crate::dispatch::state::DispatchState;
use crate::lifecycle::{ServiceLifecycle, ShutdownCause};
use futures_util::FutureExt;
use service_api::{ServiceError, ShutdownResponse};
use std::panic::AssertUnwindSafe;
use tokio::io::AsyncRead;
use tokio::task::JoinSet;

pub(crate) async fn run_shutdown_drain<R>(mut state: DispatchState<R>) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
{
    // Drain in-flight handlers BEFORE running the lifecycle drain. This
    // ensures any Phase 2+ mutation actually finishes before we write
    // the sentinel and ack the Shutdown request. The dispatch loop has
    // already stopped reading new requests by the time we reach this
    // point.
    drain_in_flight(&mut state.handlers_in_flight).await;
    // Notification handlers are best-effort. Phase 5 task 7: cap the
    // notification drain at 5 s aggregate so a wedged handler (the GAL
    // handler can take up to 60 s per account x N accounts in the worst
    // case, and `refresh_gal_for_account` performs DB writes via
    // tokio::task::spawn_blocking which itself isn't cancellable)
    // cannot stall shutdown for minutes. Past the cap we abort the
    // remaining notification tasks and log the count. The blocking
    // work spawned by an aborted GAL handler runs to completion
    // regardless - the abort releases only the outer async future.
    // Acceptable because GAL writes are bounded and idempotent.
    drain_notifications_bounded(&mut state.notifications_in_flight, NOTIFICATION_DRAIN_BOUND).await;

    // Reap the boot task so any boot-sequence panic surfaces in the
    // log rather than vanishing at process exit.
    //
    // Ordering note: the abort fires AFTER `drain_in_flight` because
    // the boot.ready handler parks on `BootSharedState::wait_for_ready`
    // and would never return if we aborted the boot task before it
    // signalled. The downside is that a Shutdown arriving during a
    // long migration (the boot task is in `spawn_blocking` and cannot
    // be aborted) waits out the migration before the abort runs;
    // under the UI's 30 s IPC timeout this manifests as
    // SIGTERM-then-SIGKILL via the standard shutdown escalation path
    // rather than a quick clean exit. Acceptable per
    // phase-1.5-plan.md scope item 18.
    state.boot_handle.abort();
    let _ = state.boot_handle.await;

    // Determine the shutdown cause. Order matters: BootFailure
    // dominates (a boot failure that races a Shutdown request still
    // wins because the Service is exiting non-zero); otherwise a
    // pending Shutdown id means the UI asked for a graceful drain;
    // otherwise the loop exited via SIGTERM-handler / stdin-EOF /
    // parent-death, which all collapse into `Unrequested`. Only
    // `GracefulRequest` writes the `clean_shutdown` sentinel; the
    // others leave it absent so Phase 3+ recovery fires.
    let cause = if state.boot_exit_code.is_some() {
        ShutdownCause::BootFailure
    } else if state.pending_shutdown_id.is_some() {
        ShutdownCause::GracefulRequest
    } else {
        ShutdownCause::Unrequested
    };

    // Phase 4 task 4 + Phase 5 task 7: consolidated drain. Order is
    // critical -
    //
    // - **Push -> Sync** is load-bearing: a `StateChange` mid-shutdown
    //   would otherwise call `SyncRuntime::start_account` after sync
    //   has begun draining, leaking a runner past the drain.
    // - **Calendar -> Sync** is reserved, NOT load-bearing today: the
    //   action worker is alive throughout the entire consolidated
    //   drain, so calendar can drain before or after sync without
    //   affecting action-worker availability today. The order is fixed
    //   so a future change wiring calendar-cancel cleanup to dispatch
    //   action plans (RSVP send is the candidate) is a one-liner
    //   instead of a drain reshuffle.
    // - Sentinel-after-everything-else fixes a pre-existing Phase 3
    //   bug where the sentinel could land before in-flight sync writes
    //   completed.
    //
    // See `service::lifecycle::ServiceLifecycle::drain` and
    // `crates/service/src/calendar.rs` for the rationale matrix.
    //
    // Each runtime holds a NotificationSender clone of `out_tx`;
    // dropping the runtime Arc here is what eventually lets
    // `drop(out_tx)` below close the writer's input channel and the
    // writer task exit. Skipping any of these (e.g. forgetting to
    // shutdown CalendarRuntime when it's installed) hangs
    // `writer_handle.await` indefinitely.

    // 1. Cancel push bridges + await their supervisors.
    if let Some(push_runtime) = state.boot_state.take_push_runtime() {
        push_runtime.shutdown().await;
        drop(push_runtime);
    }
    // 2. Cancel calendar runners + await their supervisors.
    if let Some(calendar_runtime) = state.boot_state.take_calendar_runtime() {
        calendar_runtime.shutdown().await;
        drop(calendar_runtime);
    }
    // 3. Cancel sync runners + await their supervisors. Releasing this
    //    Arc drops the inner `SearchWriteHandle` clone the runtime
    //    owned, which lets the writer task observe EOF and exit.
    if let Some(runtime) = state.boot_state.take_sync_runtime() {
        runtime.shutdown().await;
        drop(runtime);
    }
    // 4. Cancel the extract worker + await its JoinHandle.
    // ExtractRuntime holds a `NotificationSender` clone AND a
    // `SearchWriteHandle` clone, so this drain step MUST come before
    // the search-writer JoinHandle await below.
    //
    // H1 fix: mark shutting_down BEFORE taking the runtime slot. The
    // post-ready spawn checks this flag inside
    // `install_extract_runtime`'s mutex; if set during install, the
    // runtime is dropped instead of installed, which releases its
    // SearchWriteHandle clone. Without the gate, a spawn that
    // finished construction after our take but before our
    // writer_handle.await would install an orphan runtime, and the
    // writer-task EOF would block forever on its clone.
    state.boot_state.mark_shutting_down();
    if let Some(extract_runtime) = state.boot_state.take_extract_runtime() {
        extract_runtime.shutdown().await;
        drop(extract_runtime);
    }
    // 5. Cancel any in-flight rebuild before clearing the search_write
    // slot. The rebuild task holds a SearchWriteHandle clone and (when
    // emitting) a NotificationSender clone.
    if let Some(rebuild) = state.boot_state.take_rebuild_task() {
        log::info!("shutdown: cancelling in-flight rebuild {}", rebuild.rebuild_id);
        rebuild.cancel.cancel();
        rebuild.handle.abort();
        let _ = rebuild.handle.await;
    }
    // Defensively clear the `search_write` slot. If shutdown raced
    // ahead of `spawn_post_ready_extract_startup`, the slot still
    // holds a `SearchWriteHandle` clone that would keep the
    // search-writer task alive across the JoinHandle await below.
    let _ = state.boot_state.take_search_write();
    // Same defensive clear for the out_tx slot installed for the
    // rebuild handler's NotificationSender clone source.
    let _ = state.boot_state.take_out_tx();
    // 6. Await the search-writer task's JoinHandle so the consolidated
    //    drain genuinely observes termination. The task exits when its
    //    mpsc rx returns None (every SearchWriteHandle clone has been
    //    dropped); SyncRuntime::shutdown + ExtractRuntime::shutdown
    //    above released the last clones.
    if let Some(handle) = state.boot_state.take_search_writer_handle()
        && let Err(e) = handle.await
    {
        log::warn!("search-writer task join error during shutdown: {e}");
    }

    // Sentinel write happens inside `lifecycle::drain`, after all
    // subsystem shutdowns. The OnceCell inside `drain` keeps the write
    // idempotent across any future caller.
    let flushed_ok = panic_safe_drain(&state.lifecycle, cause).await;
    if !flushed_ok {
        log::warn!("shutdown drain completed with errors");
    }

    // If the loop exited because of a Shutdown request, ack only after
    // the drain above completes - `flushed_ok: true` means the sentinel
    // was written and every in-flight handler has returned. Skip the
    // ack entirely if boot_exit_code is set: the Service is exiting
    // non-zero because boot failed, and answering "shutdown ok,
    // flushed_ok=true" while exiting with code 71/72/73 is misleading
    // in log triage.
    if let Some(id) = state.pending_shutdown_id {
        if state.boot_exit_code.is_some() {
            log::info!("dispatch end method=shutdown id={id} outcome=skipped_boot_failed");
        } else {
            let outcome = if flushed_ok { "ok" } else { "internal" };
            log::info!("dispatch end method=shutdown id={id} outcome={outcome}");
            let result = serde_json::to_value(ShutdownResponse { flushed_ok })
                .map_err(|error| ServiceError::Internal(error.to_string()));
            send_handler_response(&state.out_tx, id, result).await;
        }
    }

    // Abort the action worker so its `out_tx` clone is dropped.
    // Without this, `writer_handle.await` below blocks until every
    // sender on the outbound channel is dropped, which never happens
    // for an unbounded loop on a long-lived task.
    state.action_worker_handle.abort();
    let _ = state.action_worker_handle.await;

    // Phase 8-2: advance the per-store `clean_shutdown_cursors` rows
    // so the next dirty boot's invariant pass can bound its scans to
    // rows added since this drain. Skipped on non-graceful exits.
    //
    // Uses a fresh `Connection::open` rather than the shared writer
    // mutex. The shared `WriteDbState::with_conn` path hung when an
    // aborted-but-still-running `spawn_blocking` from the action
    // worker held the rust-level `Mutex<Connection>`. SQLite WAL
    // handles multi-connection write contention via its busy timeout.
    if matches!(cause, ShutdownCause::GracefulRequest) {
        let app_data_dir = state.boot_state.app_data_dir().to_path_buf();
        let result = tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(app_data_dir.join("ratatoskr.db"))
                .map_err(|e| format!("open ratatoskr.db for cursor write: {e}"))?;
            conn.busy_timeout(std::time::Duration::from_secs(5))
                .map_err(|e| format!("busy_timeout: {e}"))?;
            ::db::db::queries_extra::update_clean_shutdown_cursors(
                &conn,
                &["body", "inline", "extract"],
            )
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

    // Abort the post-ready startup tasks in case they were still
    // mid-construction when shutdown arrived. Their runtimes, if
    // installed, were drained above.
    state.push_startup_handle.abort();
    let _ = state.push_startup_handle.await;
    state.calendar_startup_handle.abort();
    let _ = state.calendar_startup_handle.await;
    state.extract_startup_handle.abort();
    let _ = state.extract_startup_handle.await;
    state.schema_rebuild_handle.abort();
    let _ = state.schema_rebuild_handle.await;

    drop(state.out_tx);
    let _ = state.writer_handle.await;

    match state.boot_exit_code {
        Some(code) => code.as_i32(),
        None => 0,
    }
}

async fn drain_in_flight(handlers: &mut JoinSet<()>) {
    while let Some(result) = handlers.join_next().await {
        if let Err(error) = result {
            // A handler task panic surfaced as a JoinError. The
            // per-handler catch_unwind already converted handler
            // panics into ServiceError::Panic, so reaching here
            // implies the wrapper itself panicked or the task was
            // cancelled. Log and continue so a single bad handler
            // can't hold up shutdown.
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

