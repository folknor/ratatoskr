//! The dispatch loop. Watches three sources concurrently:
//!
//! - `lifecycle.notified()` - SIGTERM / parent-death.
//! - `boot_failure_rx.recv()` - a fatal boot failure from the boot task.
//! - `lines.next_line()` - the next JSON-RPC frame off stdin.
//!
//! First to fire wins. On stdin frames, `handle_line` dispatches
//! requests / notifications and (for the `Shutdown` request) records
//! the id for the shutdown drain to ack after the in-flight drain.

use crate::boot;
use crate::dispatch::config::{ADMISSION_CAP, NOTIFY_CAP};
use crate::dispatch::handlers::{
    send_handler_response, spawn_handler, spawn_notification_handler, try_send_error,
};
use crate::dispatch::state::DispatchState;
use service_api::{
    FrameError, JsonRpcErrorObject, ParsedClientMessage, RequestParams, ServiceError,
    parse_client_message,
};
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Instant;
use tokio::io::AsyncRead;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::JoinSet;

pub(crate) async fn run_dispatch_loop<R>(state: &mut DispatchState<R>)
where
    R: AsyncRead + Unpin + Send + 'static,
{
    loop {
        // Reap tasks that have completed since the last iteration so
        // `handlers_in_flight.len()` reflects truly-still-running
        // handlers when the admission gate consults it.
        reap_finished(&mut state.handlers_in_flight);
        reap_finished(&mut state.notifications_in_flight);

        tokio::select! {
            () = state.lifecycle.notified() => {
                break;
            }
            Some(code) = state.boot_failure_rx.recv() => {
                log::error!("boot sequence failed; exit code {}", code.as_i32());
                state.boot_exit_code = Some(code);
                break;
            }
            line = state.lines.next_line() => {
                match line {
                    Ok(Some(line)) => {
                        match handle_line(
                            &line,
                            &state.out_tx,
                            &state.inflight,
                            &mut state.handlers_in_flight,
                            &mut state.notifications_in_flight,
                            state.started_at,
                            &state.boot_state,
                            &state.diagnostic_drops,
                        ).await {
                            HandleOutcome::Continue => {}
                            HandleOutcome::Shutdown(id) => {
                                state.pending_shutdown_id = Some(id);
                                break;
                            }
                        }
                    }
                    Ok(None) => {
                        log::info!("service stdin closed");
                        if state.config.hang_on_stdin_eof {
                            // Test-only: simulate a wedged Service that
                            // doesn't terminate on stdin EOF. Park
                            // indefinitely so the client's Drop /
                            // wait_with_kill_watchdog escalation paths
                            // can be exercised end-to-end. 1 hour is
                            // effectively forever; the test client
                            // SIGKILLs us long before this returns.
                            log::warn!(
                                "test-hang-on-stdin-eof: ignoring stdin EOF, parking forever",
                            );
                            tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
                            break;
                        }
                        break;
                    }
                    Err(FrameError::TooLarge) => {
                        log::warn!("rejecting oversized frame");
                        try_send_error(
                            &state.out_tx,
                            None,
                            JsonRpcErrorObject::parse_error("frame too large"),
                            &state.diagnostic_drops,
                        );
                    }
                    Err(FrameError::InvalidUtf8(error)) => {
                        log::warn!("service frame had invalid utf-8: {error}");
                        try_send_error(
                            &state.out_tx,
                            None,
                            JsonRpcErrorObject::parse_error("invalid utf-8"),
                            &state.diagnostic_drops,
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
}

enum HandleOutcome {
    Continue,
    /// Shutdown request received. Caller breaks the dispatch loop and
    /// acks with the supplied id after the in-flight drain completes.
    Shutdown(u64),
}

// Eight args - several &mut Joinsets and a &AtomicU64 over and above
// the dispatch-state references. Bundling them into a struct would just
// invent a name that means "the dispatch loop's state minus its lifecycle
// fields"; the destructuring at the call site already conveys that.
#[allow(clippy::too_many_arguments)]
async fn handle_line(
    line: &str,
    out_tx: &mpsc::Sender<Vec<u8>>,
    inflight: &Arc<Semaphore>,
    handlers_in_flight: &mut JoinSet<()>,
    notifications_in_flight: &mut JoinSet<()>,
    started_at: Instant,
    boot_state: &Arc<boot::BootSharedState>,
    diagnostic_drops: &AtomicU64,
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
            // Bypass the admission gate for heartbeat-class requests
            // so a flood of slow handlers can't starve the UI's health
            // check. Non-bypass requests must fit under ADMISSION_CAP
            // - beyond that we synchronously reject with Backpressure
            // rather than spawning unbounded waiters.
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
            // Drop-class admission. If the notification pool is at
            // capacity, drop the new inbound. The UI's tick policy
            // will retry on its next firing; missing one tick is the
            // documented best-effort guarantee.
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
                diagnostic_drops,
            );
            HandleOutcome::Continue
        }
    }
}

/// Reap completed tasks without blocking. Called between dispatch-loop
/// iterations so the JoinSet's `len()` is an honest count of
/// still-running handlers when the admission gate consults it.
// TODO(tokio): collapse when JoinSet exposes len-of-running natively.
fn reap_finished(handlers: &mut JoinSet<()>) {
    while let Some(result) = handlers.try_join_next() {
        if let Err(error) = result {
            log::warn!("in-flight handler join error: {error}");
        }
    }
}
