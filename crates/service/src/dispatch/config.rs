//! Dispatch-loop tunables and the test-only configuration parsed from
//! the Service command line.
//!
//! The four boolean / numeric knobs in `DispatchConfig` are the only
//! test-shaped behaviour the Service exposes. They are parsed once at
//! Service launch, packaged into a `DispatchConfig`, and read from a
//! field on `BootSharedState` thereafter - no globals, no atomics, no
//! ad-hoc argv re-scans inside handler code.

use std::time::Duration;

/// Outbound mpsc capacity. Each dispatched request sends exactly one
/// response on this channel; cap sized to absorb a burst from a
/// saturated dispatch loop without blocking the writer task.
pub(crate) const OUTBOUND_QUEUE_CAP: usize = 1024;

/// Max request handlers holding an in-flight permit. Above this,
/// non-bypass requests park on the semaphore until a slot frees.
pub(crate) const MAX_IN_FLIGHT: usize = 64;

/// Hard cap on tasks the dispatch loop has spawned but not yet reaped.
/// Sized at 2x `MAX_IN_FLIGHT`: one set actively executing (holding
/// semaphore permits), one set waiting briefly for a permit to free up.
/// Beyond this the request is rejected with `ServiceError::Backpressure`
/// synchronously, so a pathological client cannot balloon Service memory
/// by flooding stdin.
pub(crate) const ADMISSION_CAP: usize = 2 * MAX_IN_FLIGHT;

/// Cap on UI -> Service notification handlers running concurrently.
/// Notifications are `Drop`-class: at-cap arrivals are dropped, never
/// queued, so a slow notification handler cannot consume a
/// `MAX_IN_FLIGHT` slot and cannot starve request dispatch. The UI's
/// tick policy retries on the next firing.
pub(crate) const NOTIFY_CAP: usize = 4;

/// Aggregate cap on the notification drain at shutdown. A wedged
/// notification handler must not stall shutdown indefinitely; past this
/// budget the remaining tasks are aborted. The async wrapper is what
/// gets cancelled - any `spawn_blocking` closure inside runs to
/// completion regardless. Handlers that don't satisfy that contract
/// must keep their blocking work cancellation-aware (see Phase 3 of
/// the bulletproofing refactor).
pub(crate) const NOTIFICATION_DRAIN_BOUND: Duration = Duration::from_secs(5);

/// Test-only Service knobs parsed once from the command line. None
/// of these have any effect in production - they only matter if the
/// corresponding `--test-*` flag appears on argv.
#[derive(Debug, Clone, Default)]
pub struct DispatchConfig {
    /// `--test-hang-on-stdin-eof`: ignore stdin EOF in the dispatch
    /// loop and park instead of exiting. Simulates a wedged Service
    /// (panic-loop, kernel-level contention) so the client-Drop
    /// kill-escalation path can be exercised end-to-end.
    pub hang_on_stdin_eof: bool,
    /// `--test-fake-version=N`: override the protocol version reported
    /// in `health.ping` responses. Drives the version-mismatch
    /// handshake test.
    pub fake_protocol_version: Option<u32>,
    /// `--test-fake-schema=N`: override the schema version reported in
    /// `boot.ready`. Drives the respawn-on-binary-swap test.
    pub fake_schema_version: Option<u32>,
    /// `--test-boot-delay-ms=N`: artificial sleep at the start of the
    /// boot sequence so the harness can observe boot-phase
    /// notifications. None or Some(0) means no delay.
    pub boot_delay_ms: Option<u64>,
}

impl DispatchConfig {
    /// Parse the command-line knobs from `std::env::args()`. Cheap to
    /// call but should only be invoked once at Service launch - the
    /// resulting struct threads through `BootSharedState` for handler
    /// reads.
    pub fn from_cli_args() -> Self {
        let args: Vec<String> = std::env::args().collect();
        Self {
            hang_on_stdin_eof: args.iter().any(|a| a == "--test-hang-on-stdin-eof"),
            fake_protocol_version: parse_flag(&args, "--test-fake-version"),
            fake_schema_version: parse_flag(&args, "--test-fake-schema"),
            boot_delay_ms: parse_flag(&args, "--test-boot-delay-ms"),
        }
    }
}

fn parse_flag<T: std::str::FromStr>(args: &[String], flag: &str) -> Option<T> {
    let eq_prefix = format!("{flag}=");
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        if let Some(value) = arg.strip_prefix(&eq_prefix) {
            return value.parse().ok();
        }
        if arg == flag {
            return iter.next().and_then(|v| v.parse().ok());
        }
    }
    None
}
