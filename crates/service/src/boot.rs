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
use db::db::{Connection, apply_standard_pragmas, migrations, reconcile_velo_rename};
use service_api::{BootExitCode, BootPhase};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;

/// Service-side boot artifacts loaded once at boot. Phase 2's `ActionContext`
/// will consume the encryption key + DB connection from here once the action
/// service moves across the boundary; until then the fields are held but
/// unused (the UI keeps its own key + DB load). The `allow(dead_code)`
/// resolves when Phase 2's handler reads them.
pub(crate) struct BootContext {
    #[allow(dead_code)]
    pub(crate) encryption_key: [u8; 32],
    /// DB connection opened during boot. Held past the boot sequence so
    /// Phase 2's relocated action service can construct its `ActionContext`
    /// from it without re-opening the file. `Arc<Mutex<Connection>>` matches
    /// the shape `DbState::from_arc` expects.
    #[allow(dead_code)]
    pub(crate) db_conn: Arc<Mutex<Connection>>,
    /// Highest applied schema version after migrations completed. Echoed
    /// to the UI in `BootReadyResponse`.
    #[allow(dead_code)]
    pub(crate) schema_version: u32,
    /// Number of migrations actually applied this boot. 0 on a healthy
    /// repeat boot; non-zero only on first-run or after a schema bump.
    #[allow(dead_code)]
    pub(crate) migrations_applied: u32,
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
    MigrationFailure,
}

impl BootFailure {
    pub(crate) fn as_exit_code(self) -> BootExitCode {
        match self {
            Self::KeyLoadFailure => BootExitCode::KeyLoadFailure,
            Self::MigrationFailure => BootExitCode::MigrationFailure,
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

    boot_progress::emit(&out_tx, BootPhase::OpeningDatabase, None);

    let migrate_outcome = tokio::task::spawn_blocking({
        let dir = app_data_dir.clone();
        let progress_tx = out_tx.clone();
        move || open_db_and_migrate(&dir, &progress_tx)
    })
    .await;

    let (conn, schema_version, migrations_applied) = match migrate_outcome {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            log::error!(
                "DB open / migration failed for {}: {error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::MigrationFailure);
        }
        Err(join_error) => {
            log::error!(
                "DB open / migration task panicked for {}: {join_error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::MigrationFailure);
        }
    };

    let context = BootContext {
        encryption_key: key,
        db_conn: Arc::new(Mutex::new(conn)),
        schema_version,
        migrations_applied,
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

/// Synchronous DB open + migration step. Runs inside `spawn_blocking` so
/// `rusqlite`'s blocking I/O never starves the dispatch task or the
/// notification writer. Per-step migration progress is pumped via
/// `boot_progress::emit` from the migration runner's callback (try_send is
/// safe from blocking threads since `mpsc::Sender::try_send` is non-async).
fn open_db_and_migrate(
    app_data_dir: &std::path::Path,
    out_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<(Connection, u32, u32), String> {
    reconcile_velo_rename(app_data_dir)?;
    let db_path = app_data_dir.join("ratatoskr.db");
    let conn = Connection::open(&db_path)
        .map_err(|e| format!("open db {}: {e}", db_path.display()))?;
    apply_standard_pragmas(&conn)?;

    let mut progress = |current: u32, total: u32| {
        boot_progress::emit(
            out_tx,
            BootPhase::Migrating { current, total },
            None,
        );
    };
    let migrations_applied = migrations::run_all_with_progress(&conn, &mut progress)?;
    let schema_version = migrations::current_schema_version(&conn)?;
    Ok((conn, schema_version, migrations_applied))
}
