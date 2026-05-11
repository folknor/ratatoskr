//! The dispatch loop's long-lived state, passed by `&mut` between
//! `init_dispatch`, `run_dispatch_loop`, and `run_shutdown_drain`.
//!
//! Replaces the pile of `let`-bindings that used to span the first
//! ~120 lines of the old monolithic `run_service_with_io_and_lifecycle`.
//! Owning these in a struct lets each phase be a named function with
//! a single-argument signature, and makes the cross-phase dependencies
//! visible at the type level.

use crate::boot::BootSharedState;
use crate::dispatch::config::DispatchConfig;
use crate::lifecycle::ServiceLifecycle;
use service_api::{BootExitCode, BoundedLineReader};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::{Semaphore, mpsc};
use tokio::task::{JoinHandle, JoinSet};

pub(crate) struct DispatchState<R> {
    pub started_at: Instant,
    pub lifecycle: ServiceLifecycle,
    pub config: DispatchConfig,

    pub out_tx: mpsc::Sender<Vec<u8>>,
    pub writer_handle: JoinHandle<()>,

    pub inflight: Arc<Semaphore>,
    pub handlers_in_flight: JoinSet<()>,
    pub notifications_in_flight: JoinSet<()>,

    pub lines: BoundedLineReader<R>,

    pub boot_state: Arc<BootSharedState>,
    pub boot_handle: JoinHandle<()>,
    pub boot_failure_rx: mpsc::Receiver<BootExitCode>,

    pub action_worker_handle: JoinHandle<()>,
    pub push_startup_handle: JoinHandle<()>,
    pub calendar_startup_handle: JoinHandle<()>,
    pub extract_startup_handle: JoinHandle<()>,
    pub schema_rebuild_handle: JoinHandle<()>,

    /// Set to `Some(id)` by the dispatch loop when a `Shutdown` request
    /// arrives; consumed by the shutdown drain to ack after the
    /// in-flight drain completes.
    pub pending_shutdown_id: Option<u64>,

    /// Set by the dispatch loop if the boot sequence fails fatally;
    /// gates whether the Service exits non-zero and whether the
    /// Shutdown ack is suppressed.
    pub boot_exit_code: Option<BootExitCode>,
}
