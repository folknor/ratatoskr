//! Service-side boot sequence orchestrator.
//!
//! Runs concurrently with the dispatch loop so `health.ping` continues to
//! round-trip while migrations run. The current implementation covers the
//! key-load step; future Phase 1.5 commits add the remaining phases (DB
//! open + migrations, pending-ops recovery, queued-drafts sweep, thread-
//! participants backfill).
//!
//! On fatal boot failure (missing key, migration failure, etc.) the
//! sequence does NOT call `std::process::exit` directly: it returns a
//! `BootFailure` to the caller. This is what makes the in-process test
//! harness (`run_service_with_io` over `tokio::io::duplex`) safe to use -
//! a process exit there would kill the test runner. The outer
//! `run_service_blocking` in `lib.rs` is the only caller that converts
//! the boot exit code into an actual `std::process::exit`.

use crate::boot_progress;
use crate::key_load;
use service_api::{BootExitCode, BootPhase};
use std::path::PathBuf;
use std::sync::OnceLock;
use tokio::sync::mpsc;

/// Service-side boot artifacts loaded once at boot. Phase 2's `ActionContext`
/// will consume the encryption key from here once the action service moves
/// across the boundary; until then the field is held but unused (the UI
/// keeps its own key load for its existing `ActionContext`). The
/// `allow(dead_code)` on the field resolves when Phase 2's handler reads it.
pub(crate) struct BootContext {
    #[allow(dead_code)]
    pub(crate) encryption_key: [u8; 32],
}

/// Process-wide singleton populated by `run_boot_sequence` on success. Phase
/// 2 consumes it from the action handler; Phase 1.5 just stashes it for that
/// future use. `OnceLock` semantics rule out double-population if a future
/// commit accidentally invokes the boot sequence twice in the same process.
pub(crate) static BOOT_CONTEXT: OnceLock<BootContext> = OnceLock::new();

/// Discriminant of why the boot sequence failed. The caller maps this to a
/// `BootExitCode` via `as_exit_code()`.
#[derive(Debug, Clone, Copy)]
pub(crate) enum BootFailure {
    KeyLoadFailure,
}

impl BootFailure {
    pub(crate) fn as_exit_code(self) -> BootExitCode {
        match self {
            Self::KeyLoadFailure => BootExitCode::KeyLoadFailure,
        }
    }
}

/// Run the Service boot sequence.
///
/// Emits `BootPhase::*` notifications via `out_tx` so the UI splash can
/// render progress. Synchronous DB / filesystem work runs in
/// `tokio::task::spawn_blocking` so the dispatch task and the writer task
/// (which pumps notifications) never starve waiting on `rusqlite` /
/// `std::fs::read_to_string`.
///
/// On success, populates `BOOT_CONTEXT` and returns `Ok(())`. On fatal
/// failure, returns `Err(BootFailure)`; the caller drives the actual
/// process exit.
pub(crate) async fn run_boot_sequence(
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: PathBuf,
) -> Result<(), BootFailure> {
    boot_progress::emit(&out_tx, BootPhase::LoadingKey, None);

    let key = match tokio::task::spawn_blocking({
        let dir = app_data_dir.clone();
        move || key_load::load_encryption_key(&dir)
    })
    .await
    {
        Ok(Ok(key)) => key,
        Ok(Err(error)) => {
            log::error!(
                "encryption key load failed for {}: {error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::KeyLoadFailure);
        }
        Err(join_error) => {
            log::error!(
                "encryption key load task panicked for {}: {join_error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::KeyLoadFailure);
        }
    };

    let context = BootContext {
        encryption_key: key,
    };
    if BOOT_CONTEXT.set(context).is_err() {
        // OnceLock::set returns Err if already set. The boot sequence runs
        // exactly once per process, so reaching this arm means a future
        // commit accidentally invoked it twice. Log loudly but do not fail
        // - the existing populated context is still correct.
        log::warn!("BOOT_CONTEXT already populated; ignoring duplicate set");
    }

    Ok(())
}
