use crate::boot;
use crate::handlers;
use crate::lifecycle::{ServiceLifecycle, ShutdownCause};
use futures_util::FutureExt;
use serde_json::Value;
use service_api::{
    BootExitCode, BoundedLineReader, ClientNotification, FrameError, JsonRpcErrorObject,
    JsonRpcErrorResponse, JsonRpcSuccessResponse, ParsedClientMessage, RequestParams, ServiceError,
    ShutdownResponse, encode_message, parse_client_message,
};
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

const OUTBOUND_QUEUE_CAP: usize = 1024;
const MAX_IN_FLIGHT: usize = 64;
/// Hard cap on tasks the dispatch loop has spawned but not yet reaped. Sized
/// at 2x `MAX_IN_FLIGHT`: one set actively executing (holding semaphore
/// permits), one set waiting briefly for a permit to free up. Beyond this
/// the request is rejected with `ServiceError::Backpressure` synchronously,
/// so a pathological client cannot balloon Service memory by flooding stdin.
const ADMISSION_CAP: usize = 2 * MAX_IN_FLIGHT;
/// Cap on UI -> Service notification handlers running concurrently. Phase 2
/// plan scope item 11: notifications are `Drop`-class - if we already have
/// `NOTIFY_CAP` running, drop the new inbound rather than queue. A separate
/// pool from the request `JoinSet` ensures notification load cannot starve
/// request dispatch: even if the notification handlers are saturated, the
/// next `health.ping` still goes through immediately.
const NOTIFY_CAP: usize = 4;

pub async fn run_service_with_io<R, W>(reader: R, writer: W, app_data_dir: PathBuf) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    // Mirror production by passing the same app_data_dir into the lifecycle
    // (`ServiceLifecycle::new(Some(_))`). Previously this construction used
    // `None`, which made `clear_sentinel` / `drain` no-ops and meant the
    // in-process integration tests could not exercise the sentinel write/
    // clear path even though the boot sequence ran against the same dir.
    // Phase 3+ recovery tests need the in-process harness to drive the
    // sentinel-absent recovery trigger; aligning the lifecycle wiring here
    // unblocks those without a separate test entry point.
    let lifecycle = ServiceLifecycle::new(Some(app_data_dir.clone()));
    run_service_with_io_and_lifecycle(reader, writer, lifecycle, app_data_dir).await
}

pub(crate) async fn run_service_with_io_and_lifecycle<R, W>(
    reader: R,
    writer: W,
    lifecycle: ServiceLifecycle,
    app_data_dir: PathBuf,
) -> i32
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let started_at = Instant::now();

    // Clear the clean_shutdown sentinel before the boot sequence runs. Phase
    // 3 cross-store recovery uses sentinel-absent-at-boot as its trigger;
    // without this, the marker would persist across reboots and recovery
    // would never fire. Lock acquisition has already gated this call site, so
    // a contending second instance cannot race us. The return value (`true`
    // if the sentinel was present, i.e., the prior shutdown was graceful)
    // gates the Phase 3 invariant pass below.
    let had_clean_shutdown = lifecycle.clear_sentinel().await;

    let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>(OUTBOUND_QUEUE_CAP);
    let writer_handle = tokio::spawn(writer_task(writer, out_rx));
    let inflight = Arc::new(Semaphore::new(MAX_IN_FLIGHT));
    // Track every spawned handler task so a Shutdown request can drain them
    // before we ack. Without this, an in-flight Phase 2+ mutation could still
    // be running when the UI sees `flushed_ok: true` and starts terminating.
    let mut handlers_in_flight: JoinSet<()> = JoinSet::new();
    // Separate pool for UI -> Service notification handlers (Phase 2 plan
    // scope item 11). Drop-class: at-cap arrivals are dropped, never queued,
    // so a slow notification handler cannot consume a `MAX_IN_FLIGHT` slot
    // and cannot starve request dispatch.
    let mut notifications_in_flight: JoinSet<()> = JoinSet::new();
    let mut lines = BoundedLineReader::new(reader, service_api::MAX_FRAME_BYTES);
    let mut pending_shutdown_id: Option<u64> = None;

    // Per-instance boot state. The boot task signals success/failure on it;
    // the boot.ready handler awaits readiness via `wait_for_ready`. Held in
    // an Arc so tests that spawn multiple Service instances don't collide on
    // a process-wide singleton.
    let boot_state = boot::BootSharedState::new(app_data_dir.clone());
    // Phase 7-9: hand a clone of out_tx to BootSharedState so non-
    // spawn handlers (the index.rebuild handler) can mint a
    // NotificationSender for their tracked task. Cleared on drain
    // before drop(out_tx) below so the writer-handle await observes
    // EOF.
    boot_state.install_out_tx(out_tx.clone());

    // Boot sequence runs concurrently with the dispatch loop so health.ping
    // continues to round-trip while migrations / key load run. On fatal boot
    // failure the sequence posts the boot exit code via `boot_failure_tx`;
    // the dispatch loop's select! breaks out on that event so the Service
    // exits promptly with the right code.
    //
    // Channel capacity 1: one fatal failure per boot is the canonical case;
    // a `try_send`-style overflow cannot happen since the boot task only
    // emits at most one failure before completing. The send result is
    // intentionally discarded: if the dispatch loop already broke out (via
    // Shutdown or stdin EOF) the rx is dropped before the send arrives.
    // That's safe because the only paths that break out early (Shutdown,
    // EOF) want exit code 0 anyway, and `boot_exit_code` stays None.
    let (boot_failure_tx, mut boot_failure_rx) = mpsc::channel::<BootExitCode>(1);
    let boot_handle = tokio::spawn({
        let out_tx = out_tx.clone();
        let app_data_dir = app_data_dir.clone();
        let boot_state = Arc::clone(&boot_state);
        async move {
            if let Err(failure) = boot::run_boot_sequence(
                out_tx,
                app_data_dir,
                boot_state,
                had_clean_shutdown,
            )
            .await
            {
                let _ = boot_failure_tx.send(failure.as_exit_code()).await;
            }
        }
    });

    // Phase 2 task 9c: spawn the action worker alongside the boot task.
    // The worker awaits `boot_state.wait_for_ready()` internally before
    // touching the journal, so spawn order against the boot task does
    // not matter. We must abort this handle BEFORE dropping `out_tx`
    // below: the worker holds a clone of the sender, and
    // `writer_handle.await` only completes when every sender is
    // dropped. Without the abort, shutdown hangs forever waiting on
    // the worker's `out_tx` clone. Any lease the worker held stays in
    // `leased` until the next boot's `recover_stale_leases` resets it.
    let action_worker_handle = crate::actions::worker::spawn(
        Arc::clone(&boot_state),
        out_tx.clone(),
        app_data_dir.clone(),
    );

    // Phase 4 task 5: post-ready push startup. Waits for boot.ready,
    // constructs the PushRuntime, iterates JMAP accounts, and
    // tokio::spawns a per-account start. Per-account failure is
    // log-and-continue. Push startup runs *after* readiness because
    // TLS+HTTPS+OAuth-refresh latency must not block the splash
    // transition.
    let push_startup_handle = spawn_post_ready_push_startup(
        Arc::clone(&boot_state),
        out_tx.clone(),
    );

    // Phase 5 task 8: post-ready calendar startup. Constructs the
    // CalendarRuntime and installs it on BootSharedState so calendar
    // handlers (start/cancel/kick) can reach it. Calendar is kick-
    // driven, so the post-ready task does not iterate accounts -
    // calendar.kick from the UI's SyncTick triggers the actual sync.
    let calendar_startup_handle = spawn_post_ready_calendar_startup(
        Arc::clone(&boot_state),
        out_tx.clone(),
    );

    // Phase 7-4d: post-ready extract startup. Constructs ExtractRuntime
    // (which holds NotificationSender + SearchWriteHandle clones) and
    // installs it on BootSharedState. Drained in run_shutdown_drain
    // *between* sync and the search-writer await, so its handle clones
    // are released before the writer task is asked to observe EOF.
    let extract_startup_handle = spawn_post_ready_extract_startup(
        Arc::clone(&boot_state),
        out_tx.clone(),
        app_data_dir.clone(),
    );

    // Phase 7-9c: post-ready schema-rebuild dispatcher. If boot
    // detected a `.version` mismatch and marked the flag, this task
    // dispatches a Wipe rebuild and rewrites `.version` on success.
    // No-op when the flag is unset (the steady-state).
    let schema_rebuild_handle = spawn_post_ready_schema_rebuild(
        Arc::clone(&boot_state),
        app_data_dir.clone(),
    );

    let mut boot_exit_code: Option<BootExitCode> = None;

    loop {
        // Reap any tasks that have completed since the last iteration so
        // `handlers_in_flight.len()` reflects truly-still-running handlers
        // when we use it as the admission gate below.
        reap_finished(&mut handlers_in_flight);
        reap_finished(&mut notifications_in_flight);

        tokio::select! {
            () = lifecycle.notified() => {
                break;
            }
            Some(code) = boot_failure_rx.recv() => {
                log::error!("boot sequence failed; exit code {}", code.as_i32());
                boot_exit_code = Some(code);
                break;
            }
            line = lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        match handle_line(
                            &line,
                            &out_tx,
                            &inflight,
                            &mut handlers_in_flight,
                            &mut notifications_in_flight,
                            started_at,
                            &boot_state,
                        ).await {
                            HandleOutcome::Continue => {}
                            HandleOutcome::Shutdown(id) => {
                                pending_shutdown_id = Some(id);
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        log::info!("service stdin closed");
                        // Test-only: simulate a wedged Service that
                        // doesn't terminate on stdin EOF (panic-handler
                        // loop, kernel-level contention, etc.). Park
                        // indefinitely so the client's Drop /
                        // wait_with_kill_watchdog escalation paths can be
                        // exercised end-to-end. The `tokio::select!`
                        // around this arm means lifecycle.notified()
                        // could still wake us if a SIGTERM arrives, but
                        // that is also what real production deadlocks
                        // would do; the test client uses SIGKILL via
                        // start_kill which the kernel handles outside
                        // the runtime.
                        #[cfg(feature = "test-helpers")]
                        if crate::test_hang_on_stdin_eof() {
                            log::warn!(
                                "test-hang-on-stdin-eof: ignoring stdin EOF, parking forever",
                            );
                            // 1 hour is effectively forever for test
                            // purposes; the test client SIGKILLs us long
                            // before this returns.
                            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                            break;
                        }
                        break;
                    }
                    Err(FrameError::TooLarge) => {
                        log::warn!("rejecting oversized frame");
                        try_send_error(
                            &out_tx,
                            None,
                            JsonRpcErrorObject::parse_error("frame too large"),
                        );
                    }
                    Err(FrameError::InvalidUtf8(error)) => {
                        log::warn!("service frame had invalid utf-8: {error}");
                        try_send_error(
                            &out_tx,
                            None,
                            JsonRpcErrorObject::parse_error("invalid utf-8"),
                        );
                    }
                    Err(FrameError::Io(error)) => {
                        log::warn!("service frame io error: {error}");
                        break;
                    }
                }
            }
        }
    }

    // Drain in-flight handlers BEFORE running the lifecycle drain. This
    // ensures any Phase 2+ mutation actually finishes before we write the
    // sentinel and ack the Shutdown request. The dispatch loop has already
    // stopped reading new requests by the time we reach this point.
    drain_in_flight(&mut handlers_in_flight).await;
    // Notification handlers are best-effort. Phase 5 task 7: cap the
    // notification drain at 5 s aggregate so a wedged handler (the GAL
    // handler can take up to 60 s per account x N accounts in the worst
    // case, and refresh_gal_for_account performs DB writes via
    // tokio::task::spawn_blocking which itself isn't cancellable) cannot
    // stall shutdown for minutes. Past the cap we abort the remaining
    // notification tasks and log the count. The blocking work spawned
    // by an aborted GAL handler runs to completion regardless - the
    // abort releases only the outer async future. Acceptable because
    // GAL writes are bounded and idempotent.
    drain_notifications_bounded(&mut notifications_in_flight, NOTIFICATION_DRAIN_BOUND).await;

    // Reap the boot task so any boot-sequence panic surfaces in the log
    // rather than vanishing at process exit. We don't need the result -
    // the boot_failure_rx already delivered it during the dispatch loop.
    //
    // Ordering note: the abort fires AFTER `drain_in_flight` because the
    // boot.ready handler parks on `BootSharedState::wait_for_ready` and
    // would never return if we aborted the boot task before it signalled.
    // The downside is that a Shutdown arriving during a long migration
    // (the boot task is in `spawn_blocking` and cannot be aborted) waits
    // out the migration before the abort runs; under the UI's 30 s IPC
    // timeout this manifests as SIGTERM-then-SIGKILL via the standard
    // shutdown escalation path rather than a quick clean exit. Acceptable
    // per phase-1.5-plan.md scope item 18; flagged here so a future
    // refactor that swaps the ordering doesn't accidentally deadlock the
    // boot.ready handler.
    boot_handle.abort();
    let _ = boot_handle.await;

    // Determine the shutdown cause. Order matters: BootFailure dominates
    // (a boot failure that races a Shutdown request still wins because the
    // Service is exiting non-zero); otherwise a pending Shutdown id means
    // the UI asked for a graceful drain; otherwise the loop exited via
    // SIGTERM-handler / stdin-EOF / parent-death, which all collapse into
    // `Unrequested`. Only `GracefulRequest` writes the `clean_shutdown`
    // sentinel; the others leave it absent so Phase 3+ recovery fires.
    let cause = if boot_exit_code.is_some() {
        ShutdownCause::BootFailure
    } else if pending_shutdown_id.is_some() {
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
    //   action worker is alive throughout the entire consolidated drain,
    //   so calendar can drain before or after sync without affecting
    //   action-worker availability today. The order is fixed so a
    //   future change wiring calendar-cancel cleanup to dispatch action
    //   plans (RSVP send is the candidate) is a one-liner instead of a
    //   drain reshuffle. Don't promote this to "load-bearing today"
    //   rationale unless that wiring lands.
    // - Sentinel-after-everything-else fixes a pre-existing Phase 3 bug
    //   where the sentinel could land before in-flight sync writes
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
    //
    // 1. Cancel push bridges + await their supervisors (no-op if the
    //    post-ready task hasn't installed a PushRuntime yet).
    if let Some(push_runtime) = boot_state.take_push_runtime() {
        push_runtime.shutdown().await;
        drop(push_runtime);
    }
    // 2. Cancel calendar runners + await their supervisors (no-op if
    //    post-ready calendar startup hasn't installed yet).
    if let Some(calendar_runtime) = boot_state.take_calendar_runtime() {
        calendar_runtime.shutdown().await;
        drop(calendar_runtime);
    }
    // 3. Cancel sync runners + await their supervisors. Releasing this
    //    Arc drops the inner `SearchWriteHandle` clone the runtime
    //    owned, which lets the writer task observe EOF and exit.
    if let Some(runtime) = boot_state.take_sync_runtime() {
        runtime.shutdown().await;
        drop(runtime);
    }
    // 4. Phase 7-4d: cancel the extract worker + await its
    //    `JoinHandle`. ExtractRuntime holds a `NotificationSender`
    //    clone of `out_tx` AND a `SearchWriteHandle` clone, so this
    //    drain step MUST come before the search-writer JoinHandle
    //    await below. Without it, both `writer_handle.await` (search
    //    writer) and the eventual `out_tx` drop deadlock against the
    //    extract worker holding the last clones. The earlier 7-4d
    //    revert hung exactly because this step was missing.
    //
    // H1 fix: mark shutting_down BEFORE taking the runtime slot. The
    // post-ready spawn (`spawn_post_ready_extract_startup`) checks
    // this flag inside `install_extract_runtime`'s mutex; if set
    // during install, the runtime is dropped instead of installed,
    // which releases its SearchWriteHandle clone. Without the gate,
    // a spawn that finished construction after our take but before
    // our writer_handle.await would install an orphan runtime, and
    // the writer-task EOF would block forever on its clone.
    boot_state.mark_shutting_down();
    if let Some(extract_runtime) = boot_state.take_extract_runtime() {
        extract_runtime.shutdown().await;
        drop(extract_runtime);
    }
    // Phase 7-9: cancel any in-flight rebuild before clearing the
    // search_write slot. The rebuild task holds a SearchWriteHandle
    // clone and (when emitting) a NotificationSender clone; both
    // need to drop before search_writer + writer await can observe
    // EOF. The token check happens between chunks; abort handles
    // the case where the task is mid-await.
    if let Some(rebuild) = boot_state.take_rebuild_task() {
        log::info!(
            "shutdown: cancelling in-flight rebuild {}",
            rebuild.rebuild_id
        );
        rebuild.cancel.cancel();
        rebuild.handle.abort();
        let _ = rebuild.handle.await;
    }
    // Defensively clear the `search_write` slot. If shutdown raced
    // ahead of `spawn_post_ready_extract_startup`, the slot still
    // holds a `SearchWriteHandle` clone that would keep the
    // search-writer task alive across the JoinHandle await below.
    // Dropping the returned Option immediately releases the clone.
    let _ = boot_state.take_search_write();
    // Same defensive clear for the out_tx slot installed for the
    // rebuild handler's NotificationSender clone source.
    let _ = boot_state.take_out_tx();
    // 5. Await the search-writer task's JoinHandle so the consolidated
    //    drain genuinely observes termination. The task exits when its
    //    mpsc rx returns None (every SearchWriteHandle clone has been
    //    dropped); SyncRuntime::shutdown + ExtractRuntime::shutdown
    //    above released the last clones. Phase 4 review-pass fix -
    //    pre-fix the handle was discarded at construction and the
    //    "the writer task observes EOF and exits" claim relied on the
    //    run_sync flush_now() invariant being maintained, with no
    //    test that would catch a future regression of that invariant.
    if let Some(handle) = boot_state.take_search_writer_handle()
        && let Err(e) = handle.await
    {
        log::warn!("search-writer task join error during shutdown: {e}");
    }
    // 4. Sentinel write happens inside `lifecycle::drain` below, after
    //    all subsystem shutdowns. The OnceCell inside `drain` keeps the
    //    write idempotent across any future caller.
    let flushed_ok = panic_safe_drain(&lifecycle, cause).await;
    if !flushed_ok {
        log::warn!("shutdown drain completed with errors");
    }

    // If the loop exited because of a Shutdown request, ack only after the
    // drain above completes - `flushed_ok: true` means the sentinel was
    // written and every in-flight handler has returned. Skip the ack
    // entirely if boot_exit_code is set: the Service is exiting non-zero
    // because boot failed, and answering "shutdown ok, flushed_ok=true"
    // while exiting with code 71/72/73 is misleading in log triage. The
    // kernel-level exit code is what the UI observes; the missing ack is
    // benign (the UI's shutdown-request future returns ServiceCrashed,
    // which is correct for a Service that exited mid-shutdown).
    if let Some(id) = pending_shutdown_id {
        if boot_exit_code.is_some() {
            log::info!(
                "dispatch end method=shutdown id={id} outcome=skipped_boot_failed",
            );
        } else {
            // Framing-layer logging hook: same shape as spawn_handler.
            // Records the outcome (ok / internal) and elapsed-since-
            // shutdown-arrival; never the response payload.
            let outcome = if flushed_ok { "ok" } else { "internal" };
            log::info!("dispatch end method=shutdown id={id} outcome={outcome}");
            let result = serde_json::to_value(ShutdownResponse { flushed_ok })
                .map_err(|error| ServiceError::Internal(error.to_string()));
            send_handler_response(&out_tx, id, result).await;
        }
    }

    // Abort the action worker so its `out_tx` clone is dropped. Without
    // this, `writer_handle.await` below blocks until every sender on
    // the outbound channel is dropped, which never happens for an
    // unbounded loop on a long-lived task.
    //
    // The PushRuntime + SyncRuntime shutdowns ran above as part of the
    // consolidated drain (Phase 4 task 4). By this point, the search
    // writer task is the last `out_tx` clone-holder; dropping `out_tx`
    // here lets it observe EOF, commit any straggler docs, and exit.
    action_worker_handle.abort();
    let _ = action_worker_handle.await;

    // Phase 4 task 5: abort the post-ready push startup task in case
    // it was still iterating accounts when shutdown arrived. Started
    // bridges are already registered in the PushRuntime and were
    // drained by the consolidated drain above.
    push_startup_handle.abort();
    let _ = push_startup_handle.await;

    // Phase 5 task 8: same shape as push - the post-ready calendar
    // startup is bounded work (construct + install), but abort it
    // explicitly in case it was still mid-construction when shutdown
    // arrived. The runtime itself, if installed, was drained by the
    // consolidated drain above.
    calendar_startup_handle.abort();
    let _ = calendar_startup_handle.await;

    // Phase 7-4d: same pattern - abort the extract startup task in
    // case it hadn't yet finished installing the runtime when
    // shutdown arrived. If the runtime *was* installed, the
    // consolidated drain above released its NotificationSender +
    // SearchWriteHandle clones via `take_extract_runtime + shutdown`.
    extract_startup_handle.abort();
    let _ = extract_startup_handle.await;

    // Phase 7-9c: same pattern for the schema-rebuild dispatcher.
    // The actual rebuild task (if dispatched) was drained above via
    // take_rebuild_task; this just aborts the dispatcher's outer
    // wait.
    schema_rebuild_handle.abort();
    let _ = schema_rebuild_handle.await;

    drop(out_tx);
    let _ = writer_handle.await;

    match boot_exit_code {
        Some(code) => code.as_i32(),
        None => 0,
    }
}

enum HandleOutcome {
    Continue,
    /// Shutdown request received. Caller should break the dispatch loop and
    /// ack with the supplied id after the in-flight drain completes.
    Shutdown(u64),
}

async fn handle_line(
    line: &str,
    out_tx: &mpsc::Sender<Vec<u8>>,
    inflight: &Arc<Semaphore>,
    handlers_in_flight: &mut JoinSet<()>,
    notifications_in_flight: &mut JoinSet<()>,
    started_at: Instant,
    boot_state: &Arc<boot::BootSharedState>,
) -> HandleOutcome {
    match parse_client_message(line) {
        Ok(ParsedClientMessage::Request { id, params })
            if matches!(*params, RequestParams::Shutdown) =>
        {
            log::info!("dispatch start method=shutdown id={id}");
            HandleOutcome::Shutdown(id)
        }
        Ok(ParsedClientMessage::Request { id, params }) => {
            let params = *params;
            // Bypass the admission gate for heartbeat-class requests so a
            // flood of slow handlers can't starve the UI's health check.
            // Non-bypass requests must fit under ADMISSION_CAP - beyond that
            // we synchronously reject with Backpressure rather than spawning
            // unbounded waiters.
            if !params.bypasses_admission() && handlers_in_flight.len() >= ADMISSION_CAP {
                send_handler_response(out_tx, id, Err(ServiceError::Backpressure)).await;
                return HandleOutcome::Continue;
            }
            spawn_handler(
                id,
                params,
                out_tx.clone(),
                Arc::clone(inflight),
                started_at,
                handlers_in_flight,
                Arc::clone(boot_state),
            );
            HandleOutcome::Continue
        }
        Ok(ParsedClientMessage::Notification(notification)) => {
            // Drop-class admission per Phase 2 plan scope item 11. If the
            // notification pool is at capacity, drop the new inbound. The
            // UI's tick policy will retry on its next firing; missing one
            // tick is the documented best-effort guarantee.
            if notifications_in_flight.len() >= NOTIFY_CAP {
                log::debug!(
                    "notification drop method={} pool_full",
                    notification.method_name(),
                );
                return HandleOutcome::Continue;
            }
            spawn_notification_handler(
                notification,
                notifications_in_flight,
                Arc::clone(boot_state),
            );
            HandleOutcome::Continue
        }
        Err(error) => {
            let response_id = error.extracted_id();
            log::warn!("request parse failed: {error}");
            try_send_error(
                out_tx,
                response_id,
                JsonRpcErrorObject::parse_error(error.to_string()),
            );
            HandleOutcome::Continue
        }
    }
}

/// Dispatch a UI -> Service notification on the dedicated notification
/// task pool. No response is sent (notifications are id-less by
/// definition); the handler runs to completion or is dropped on shutdown.
fn spawn_notification_handler(
    notification: ClientNotification,
    notifications_in_flight: &mut JoinSet<()>,
    boot_state: Arc<boot::BootSharedState>,
) {
    let method = notification.method_name();
    log::info!("notification dispatch method={method}");
    notifications_in_flight.spawn(async move {
        let entered = Instant::now();
        let result = AssertUnwindSafe(handlers::dispatch_notification(notification, boot_state))
            .catch_unwind()
            .await;
        let elapsed_ms = entered.elapsed().as_millis();
        match result {
            Ok(Ok(())) => {
                log::info!(
                    "notification dispatch end method={method} elapsed_ms={elapsed_ms} outcome=ok",
                );
            }
            Ok(Err(error)) => {
                log::warn!(
                    "notification dispatch end method={method} elapsed_ms={elapsed_ms} outcome=err: {error}",
                );
            }
            Err(panic) => {
                log::error!(
                    "notification handler panicked method={method}: {}",
                    panic_message(panic.as_ref()),
                );
            }
        }
    });
}

/// Reap completed tasks without blocking. Called between dispatch-loop
/// iterations so the JoinSet's `len()` is an honest count of still-running
/// handlers when the admission gate consults it.
fn reap_finished(handlers: &mut JoinSet<()>) {
    while let Some(result) = handlers.try_join_next() {
        if let Err(error) = result {
            log::warn!("in-flight handler join error: {error}");
        }
    }
}

async fn drain_in_flight(handlers: &mut JoinSet<()>) {
    while let Some(result) = handlers.join_next().await {
        if let Err(error) = result {
            // A handler task panic surfaced as a JoinError. The per-handler
            // catch_unwind already converted handler panics into
            // ServiceError::Panic, so reaching here implies the wrapper
            // itself panicked or the task was cancelled. Log and continue
            // so a single bad handler can't hold up shutdown.
            log::warn!("in-flight handler join failed during drain: {error}");
        }
    }
}

/// Phase 5 task 7: aggregate cap on the notification drain. A wedged
/// notification handler must not stall shutdown indefinitely.
///
/// **Caveat for handlers that wrap blocking work in `spawn_blocking`:**
/// aborting the outer async task does NOT stop a running blocking
/// closure. The GAL handler is the live example -
/// `crates/core/src/contacts/gal.rs:212` writes via `spawn_blocking`,
/// so a drain-timeout abort releases the async wrapper but the
/// blocking write runs to completion. Acceptable because GAL writes
/// are bounded and idempotent. Any handler added later that doesn't
/// satisfy that contract must either keep its blocking work
/// cancellation-aware or accept the same drain-timeout semantics.
const NOTIFICATION_DRAIN_BOUND: std::time::Duration = std::time::Duration::from_secs(5);

async fn drain_notifications_bounded(
    handlers: &mut JoinSet<()>,
    bound: std::time::Duration,
) {
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
            // Drain JoinErrors so the JoinSet is empty before we return.
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
            Err(_) => {
                // Loop iteration to fall into the deadline-passed branch
                // above (which logs + aborts). Don't open-code the abort
                // here so the log line is one place.
                continue;
            }
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

fn spawn_handler(
    id: u64,
    params: RequestParams,
    out_tx: mpsc::Sender<Vec<u8>>,
    inflight: Arc<Semaphore>,
    started_at: Instant,
    handlers_in_flight: &mut JoinSet<()>,
    boot_state: Arc<boot::BootSharedState>,
) {
    let method = params.method_name();
    // Framing-layer logging hook: record method + id only at dispatch entry.
    // Never the params payload - once Phase 2+ methods carry user content
    // (message bodies, search queries, OAuth codes) in params, payload-level
    // logging would silently bypass the RedactedString net.
    log::info!("dispatch start method={method} id={id}");
    handlers_in_flight.spawn(async move {
        let entered = Instant::now();
        // Acquire the in-flight permit *inside* the spawned task - never
        // in the dispatch loop. Acquiring upfront would stall the dispatch
        // loop's stdin read whenever MAX_IN_FLIGHT slow handlers are
        // running, queuing fast methods behind slow ones. The dispatch
        // loop's ADMISSION_CAP gate keeps the number of waiters bounded.
        let _permit = if params.bypasses_admission() {
            None
        } else {
            match inflight.acquire_owned().await {
                Ok(permit) => Some(permit),
                Err(error) => {
                    log::warn!(
                        "dispatch end method={method} id={id} elapsed_ms={} outcome=internal",
                        entered.elapsed().as_millis(),
                    );
                    send_handler_response(
                        &out_tx,
                        id,
                        Err(ServiceError::Internal(error.to_string())),
                    )
                    .await;
                    return;
                }
            }
        };
        let result = dispatch_with_panic_safety(params, started_at, boot_state).await;
        // Framing-layer logging hook: record outcome discriminant only.
        // The error variant name lands; the error message does not, since
        // ServiceError::Panic and ServiceError::Internal can carry caller-
        // -supplied content.
        let elapsed_ms = entered.elapsed().as_millis();
        let outcome = match &result {
            Ok(_) => "ok",
            Err(error) => error_outcome_kind(error),
        };
        log::info!("dispatch end method={method} id={id} elapsed_ms={elapsed_ms} outcome={outcome}");
        send_handler_response(&out_tx, id, result).await;
    });
}

/// Discriminant-only outcome string for dispatch-end logging. Returns the
/// variant name with no payload content - the error message itself is held
/// back from the rolling log file by sensitive-value policy.
fn error_outcome_kind(error: &ServiceError) -> &'static str {
    match error {
        ServiceError::Panic { .. } => "panic",
        ServiceError::InvalidParams { .. } => "invalid_params",
        ServiceError::UnknownMethod(_) => "unknown_method",
        ServiceError::Internal(_) => "internal",
        ServiceError::Backpressure => "backpressure",
        ServiceError::BootFailure { .. } => "boot_failure",
    }
}

async fn dispatch_with_panic_safety(
    params: RequestParams,
    started_at: Instant,
    boot_state: Arc<boot::BootSharedState>,
) -> Result<Value, ServiceError> {
    let method = params.method_name();
    let result = AssertUnwindSafe(handlers::dispatch(params, started_at, boot_state))
        .catch_unwind()
        .await;
    match result {
        Ok(result) => result,
        Err(panic) => Err(ServiceError::Panic {
            method: method.to_string(),
            message: panic_message(panic.as_ref()),
        }),
    }
}

fn panic_message(panic: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = panic.downcast_ref::<&str>() {
        return (*message).to_string();
    }
    if let Some(message) = panic.downcast_ref::<String>() {
        return message.clone();
    }
    "unknown panic payload".to_string()
}

async fn send_handler_response(
    out_tx: &mpsc::Sender<Vec<u8>>,
    id: u64,
    result: Result<Value, ServiceError>,
) {
    match result {
        Ok(value) => send_success(out_tx, id, value).await,
        Err(error) => send_error(out_tx, Some(id), JsonRpcErrorObject::from(error)).await,
    }
}

async fn send_success(out_tx: &mpsc::Sender<Vec<u8>>, id: u64, value: Value) {
    let response = JsonRpcSuccessResponse::new(id, value);
    match encode_message(&response) {
        Ok(bytes) => {
            let _ = out_tx.send(bytes).await;
        }
        Err(error) => {
            log::error!("failed to encode success response: {error}");
        }
    }
}

async fn send_error(
    out_tx: &mpsc::Sender<Vec<u8>>,
    id: Option<u64>,
    error: JsonRpcErrorObject,
) {
    let response = JsonRpcErrorResponse::new(id, error);
    match encode_message(&response) {
        Ok(bytes) => {
            let _ = out_tx.send(bytes).await;
        }
        Err(error) => {
            log::error!("failed to encode error response: {error}");
        }
    }
}

/// Non-blocking error send used from the dispatch loop. Awaiting `out_tx.send`
/// here would stall stdin reads when the outbound queue is full; for parse
/// errors and frame errors that's the wrong trade - drop the diagnostic and
/// keep reading.
fn try_send_error(
    out_tx: &mpsc::Sender<Vec<u8>>,
    id: Option<u64>,
    error: JsonRpcErrorObject,
) {
    let response = JsonRpcErrorResponse::new(id, error);
    match encode_message(&response) {
        Ok(bytes) => {
            if let Err(send_err) = out_tx.try_send(bytes) {
                log::warn!("dropped diagnostic error response: {send_err}");
            }
        }
        Err(error) => {
            log::error!("failed to encode error response: {error}");
        }
    }
}

async fn writer_task<W>(mut writer: W, mut out_rx: mpsc::Receiver<Vec<u8>>)
where
    W: AsyncWrite + Unpin,
{
    while let Some(bytes) = out_rx.recv().await {
        if let Err(error) = writer.write_all(&bytes).await {
            log::warn!("service stdout write failed: {error}");
            break;
        }
        if let Err(error) = writer.flush().await {
            log::warn!("service stdout flush failed: {error}");
            break;
        }
    }
}

/// Phase 4 task 5: post-ready push startup task.
///
/// Spawn a task that waits for `boot.ready`, then constructs a
/// `PushRuntime` and starts a bridge per JMAP account. Per-account
/// starts are themselves `tokio::spawn`'d inside `PushRuntime::start_account`,
/// so a slow initial connect (TLS+HTTPS+OAuth refresh) for one account
/// does not delay the others.
///
/// Push startup explicitly runs *after* `boot.ready` rather than as a
/// boot phase: readiness must not depend on push setup work, and a
/// missing JMAP server (network down at boot) must not block the
/// splash transition. Per-account failure is log-and-continue.
fn spawn_post_ready_push_startup(
    boot_state: Arc<boot::BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        // Park until boot.ready resolves (or the boot task is aborted).
        if boot_state.wait_for_ready().await.is_err() {
            log::debug!("post-ready push startup: boot failed, skipping push setup");
            return;
        }

        // SyncRuntime is installed by run_boot_sequence_inner before
        // signal_ready fires, so by this point it MUST be present. A
        // missing SyncRuntime here is a programming error.
        let Some(sync_runtime) = boot_state.sync_runtime() else {
            log::error!(
                "post-ready push startup: SyncRuntime missing after boot.ready - programming error",
            );
            return;
        };

        let Some(db_conn) = boot_state.db_conn() else {
            log::error!(
                "post-ready push startup: db_conn missing after boot.ready - programming error",
            );
            return;
        };
        let Some(key_bytes) = boot_state.encryption_key() else {
            log::error!(
                "post-ready push startup: encryption key missing after boot.ready - programming error",
            );
            return;
        };

        let db_state = service_state::WriteDbState::from_arc(db_conn);
        let encryption_key = crypto_key::SecretKey::from_bytes(key_bytes);
        let notification_tx = crate::boot_progress::NotificationSender::new(out_tx);

        // service_generation is overwritten by the UI's reader task at
        // enqueue time; emit 0 here per the WithGeneration trait
        // contract documented on `Notification::service_generation()`.
        let push_runtime = Arc::new(crate::push::PushRuntime::new(
            db_state.clone(),
            encryption_key,
            sync_runtime,
            notification_tx,
            0,
        ));
        boot_state.install_push_runtime(Arc::clone(&push_runtime));

        // Iterate JMAP accounts. Per-account failure is logged and the
        // iteration continues - a misconfigured / network-unreachable
        // account must not block push setup for healthy ones.
        let jmap_account_ids: Result<Vec<String>, String> = db_state
            .with_conn(|conn| {
                let mut stmt = conn
                    .prepare("SELECT id FROM accounts WHERE provider = 'jmap'")
                    .map_err(|e| format!("prepare jmap accounts query: {e}"))?;
                let ids = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| format!("query jmap accounts: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect jmap account ids: {e}"))?;
                Ok(ids)
            })
            .await;

        let account_ids = match jmap_account_ids {
            Ok(ids) => ids,
            Err(e) => {
                log::warn!(
                    "post-ready push startup: failed to enumerate JMAP accounts: {e}",
                );
                return;
            }
        };

        log::info!(
            "post-ready push startup: starting bridges for {} JMAP account(s)",
            account_ids.len()
        );
        for account_id in account_ids {
            let push_runtime = Arc::clone(&push_runtime);
            // Spawn per-account so a slow TLS handshake for one account
            // doesn't sequence the others.
            tokio::spawn(async move {
                if let Err(e) = push_runtime.start_account(account_id.clone()).await {
                    log::warn!("[push] start_account({account_id}) failed: {e}");
                }
            });
        }
    })
}

/// Phase 5 task 8: post-ready calendar startup.
///
/// Parks until `boot.ready`, constructs the `CalendarRuntime`, and
/// installs it on `BootSharedState` so calendar handlers can reach it.
/// Unlike push startup, this does NOT iterate accounts - calendar is
/// kick-driven (`calendar.kick` notification from the UI's `SyncTick`),
/// and the kick handler enumerates accounts itself.
fn spawn_post_ready_calendar_startup(
    boot_state: Arc<boot::BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if boot_state.wait_for_ready().await.is_err() {
            log::debug!("post-ready calendar startup: boot failed, skipping");
            return;
        }

        let Some(db_conn) = boot_state.db_conn() else {
            log::error!(
                "post-ready calendar startup: db_conn missing after boot.ready - programming error",
            );
            return;
        };
        let Some(key_bytes) = boot_state.encryption_key() else {
            log::error!(
                "post-ready calendar startup: encryption key missing after boot.ready - programming error",
            );
            return;
        };

        let db_state = service_state::WriteDbState::from_arc(db_conn);
        let encryption_key = crypto_key::SecretKey::from_bytes(key_bytes);
        let notification_tx = crate::boot_progress::NotificationSender::new(out_tx);

        // service_generation is overwritten by the UI's reader task at
        // enqueue time; emit 0 here per the WithGeneration trait
        // contract documented on `Notification::service_generation()`.
        let calendar_runtime = Arc::new(crate::calendar::CalendarRuntime::new(
            db_state,
            &encryption_key,
            notification_tx,
            0,
        ));
        boot_state.install_calendar_runtime(Arc::clone(&calendar_runtime));

        log::info!("post-ready calendar startup: CalendarRuntime installed");
    })
}

/// Phase 7-4d: post-ready extract startup. Mirrors
/// `spawn_post_ready_calendar_startup` - waits for boot.ready,
/// snapshots the search-writer + body-store + db handles, constructs
/// `ExtractRuntime`, installs it on `BootSharedState`. Extract is
/// kick-driven (`extract.backfill_kick` and per-`attachment.fetch`
/// enqueues), so the post-ready task does not iterate accounts.
///
/// **The earlier 7-4d revert.** A previous version of this spawn
/// caused `boot_ready_blocks_until_sequence_completes` to hang at the
/// 20 s test ceiling. Root cause: `ExtractRuntime` held a
/// `NotificationSender` (clone of the outbound `out_tx`) and a
/// `SearchWriteHandle` clone, but the consolidated drain in
/// `run_shutdown_drain` had no step to drain it. The writer task's
/// JoinHandle.await blocked forever waiting for the last `out_tx`
/// clone to drop, which couldn't happen while the extract worker
/// kept the runtime Arc alive. Fixed by:
///
/// 1. Adding async `ExtractRuntime::shutdown()` driven by a
///    `CancellationToken` + stored worker `JoinHandle`.
/// 2. Adding the drain step in `run_shutdown_drain` between the sync
///    drain and the search-writer await.
fn spawn_post_ready_extract_startup(
    boot_state: Arc<boot::BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: std::path::PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if boot_state.wait_for_ready().await.is_err() {
            log::debug!("post-ready extract startup: boot failed, skipping");
            return;
        }

        let Some(db_conn) = boot_state.db_conn() else {
            log::error!(
                "post-ready extract startup: db_conn missing after boot.ready - programming error",
            );
            return;
        };
        let Some(search_write) = boot_state.take_search_write() else {
            // H1 fix: take_search_write (consume), not search_write
            // (clone). The slot is single-use as the plan promised:
            // either the post-ready spawn consumes it on success, or
            // run_shutdown_drain's defensive take_search_write drains
            // it before awaiting the writer task. Cloning left a
            // SearchWriteHandle in the slot that drain would correctly
            // take, but ALSO held a separate clone in this spawn's
            // local that drain couldn't see - if drain raced ahead of
            // install_extract_runtime, the writer-task await blocked
            // forever on the orphan clone here.
            log::debug!(
                "post-ready extract startup: search_write slot empty (shutdown raced)",
            );
            return;
        };
        let body_read = match store::body_store::BodyStoreReadState::init(&app_data_dir) {
            Ok(b) => b,
            Err(e) => {
                log::error!("post-ready extract startup: body_store init failed: {e}");
                return;
            }
        };

        let db_state = service_state::WriteDbState::from_arc(db_conn);
        let notification_tx = crate::boot_progress::NotificationSender::new(out_tx);

        let extract_runtime = crate::extract::ExtractRuntime::new(
            db_state,
            app_data_dir,
            search_write,
            body_read,
            notification_tx,
            0,
        );
        boot_state.install_extract_runtime(extract_runtime);

        log::info!("post-ready extract startup: ExtractRuntime installed");
    })
}

/// Phase 7-9c: post-ready schema-version rebuild dispatcher.
///
/// If `check_schema_version_and_dispatch` marked a pending rebuild
/// during boot (the persisted `.version` differs from
/// `INDEX_SCHEMA_VERSION`), this task dispatches a Wipe rebuild via
/// the in-process IPC handler and rewrites `.version` once the
/// rebuild emits `IndexRebuildCompleted`. The task awaits the
/// rebuild's completion notification by polling
/// `boot_state.rebuild_in_flight_id` because subscribing to the
/// outbound stream from inside the dispatch process is overkill for
/// this one-shot path.
///
/// On no-flag: the task immediately exits (steady-state boot).
fn spawn_post_ready_schema_rebuild(
    boot_state: Arc<boot::BootSharedState>,
    app_data_dir: std::path::PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if boot_state.wait_for_ready().await.is_err() {
            return;
        }
        if !boot_state.take_pending_schema_rebuild() {
            return;
        }
        log::info!(
            "post-ready schema rebuild: dispatching Wipe rebuild for INDEX_SCHEMA_VERSION change",
        );

        // Dispatch the rebuild via the same in-process handler the
        // palette command uses. This installs a RebuildTaskState on
        // BootSharedState; we then watch for it to clear (= rebuild
        // completed or got cancelled by drain).
        let params = service_api::IndexRebuildParams {
            policy: service_api::RebuildPolicy::Wipe,
            force:  false,
        };
        let rebuild_id = match crate::handlers::extract::handle_rebuild(&boot_state, params).await
        {
            Ok(value) => match value
                .get("rebuild_id")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
            {
                Some(id) => id,
                None => {
                    log::warn!(
                        "post-ready schema rebuild: handle_rebuild ack missing rebuild_id; \
                         skipping .version bookkeeping",
                    );
                    return;
                }
            },
            Err(e) => {
                log::warn!("post-ready schema rebuild: dispatch failed: {e:?}");
                return;
            }
        };

        // Poll the slot for completion. The rebuild task itself
        // calls `take_rebuild_task` on graceful exit; on shutdown
        // drain the slot is also taken. Either way, slot becoming
        // None signals the rebuild ended - but the slot can't
        // distinguish success from cancellation, so we cross-check
        // `last_completed_rebuild_id` (set by `run_wipe_rebuild_inner`
        // only on Ok exit) before writing `.version`.
        let poll_interval = std::time::Duration::from_millis(500);
        loop {
            if boot_state.rebuild_in_flight_id().is_none() {
                break;
            }
            tokio::time::sleep(poll_interval).await;
        }

        // C4 fix: gate the `.version` write to "this specific rebuild
        // ran to clean completion." Cancellation, drain abort, and
        // run_wipe_rebuild_inner errors all leave
        // last_completed_rebuild_id unchanged from a prior rebuild
        // (or unset on first boot), so the rebuild_id check fails and
        // the OLD `.version` stays on disk. Next boot reads the old
        // value, sees a mismatch, and re-fires the rebuild - the
        // sentinel-write ordering the plan promised.
        let completed = boot_state.last_completed_rebuild_id();
        if completed.as_deref() != Some(rebuild_id.as_str()) {
            log::warn!(
                "post-ready schema rebuild {rebuild_id}: did not complete cleanly \
                 (last completed rebuild_id: {completed:?}); leaving .version unchanged \
                 so next boot re-fires",
            );
            return;
        }
        if let Err(e) = boot::write_current_search_index_version(&app_data_dir) {
            log::warn!("post-ready schema rebuild: .version write failed: {e}");
            return;
        }
        log::info!(
            "post-ready schema rebuild {rebuild_id}: .version updated to {}",
            search::INDEX_SCHEMA_VERSION,
        );
    })
}
