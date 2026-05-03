mod boot;
mod boot_progress;

/// Re-export test-helpers knobs for the in-process integration tests so
/// they can drive the artificial boot delay without `pub mod boot` leaking
/// every internal item. Compiled out of release builds.
#[cfg(feature = "test-helpers")]
pub use boot::{TEST_BOOT_DELAY_LOCK, TEST_BOOT_DELAY_MS};
mod dispatch;
mod handlers;
mod instance_lock;
mod lifecycle;
mod logging;
pub mod parent_death;
mod sigterm;
mod stdio_defense;

use service_api::BootExitCode;
use std::path::PathBuf;

pub use dispatch::run_service_with_io;

pub fn run_service_blocking() -> ! {
    // 1. Parent-death recheck - closes the fork-to-recheck window. The
    //    `pre_exec` PR_SET_PDEATHSIG ran in the child but the parent could
    //    have died before that hook fired. Run as the first synchronous
    //    statement so we exit before any allocation, file open, or runtime
    //    construction wastes work.
    parent_death::exit_if_parent_missing();

    // 2. Stdio corruption defense - synchronously dup stdin/stdout aside and
    //    redirect the real slots to /dev/null (Linux) / NUL (Windows). After
    //    this returns, every transitive `println!`, default tracing-subscriber
    //    stdout, panic-handler print, etc. lands on the sink instead of the
    //    JSON-RPC pipe. Lock the contract before the logger init, panic hook,
    //    runtime build, or any other code runs.
    let saved_stdio = match stdio_defense::claim_stdio() {
        Ok(saved) => saved,
        Err(error) => {
            // claim_stdio failed before the redirect could complete, so
            // stderr may be partially redirected. Use a direct locked
            // write to bypass any printf-shaped buffering inside
            // `eprintln!` that could land mid-byte on the JSON-RPC pipe
            // if stdout/stderr were swapped underneath us.
            let line = format!("[service] failed to claim service stdio: {error}\n");
            let _ = std::io::Write::write_all(
                &mut std::io::stderr().lock(),
                line.as_bytes(),
            );
            std::process::exit(1);
        }
    };

    // 3. Logging + panic hook. Both write only to the rolling file and
    //    stderr; never stdout. Safe after the redirect above.
    let arg_app_data_dir = app_data_dir_from_args();
    let app_data_dir = arg_app_data_dir
        .clone()
        .unwrap_or_else(default_app_data_dir);
    let _ = logging::init(&app_data_dir);
    logging::install_panic_hook();
    if arg_app_data_dir.is_none() {
        // Production launches always pass --app-data-dir from the UI; a
        // missing arg is most likely a debug-session invocation
        // (`cargo run -p app -- --service`). Log so the data dir path is
        // visible in the rolling log file - otherwise a contributor
        // chasing "why is the data dir empty?" has no signal.
        log::info!(
            "no --app-data-dir provided; falling back to {}",
            app_data_dir.display(),
        );
    }

    // 4. Single-instance lock. A second Service spawned against the same
    //    data dir gets `AnotherInstanceRunning` and exits before doing any
    //    DB work. The guard is held until process exit; the kernel releases
    //    the underlying file lock on close (clean, panic, or SIGKILL).
    //
    //    Lock acquisition fires before the writer task is alive, so no
    //    `BootProgress` notification is possible here - the distinguishable
    //    exit code is the user-visible signal.
    let _instance_lock = match instance_lock::acquire(&app_data_dir) {
        Ok(guard) => guard,
        Err(instance_lock::AcquireError::Contended) => {
            log::error!(
                "another Ratatoskr Service is already running for {}",
                app_data_dir.display()
            );
            std::process::exit(BootExitCode::AnotherInstanceRunning.as_i32());
        }
        Err(instance_lock::AcquireError::Io(error)) => {
            log::error!(
                "failed to acquire instance lock at {}: {error}",
                app_data_dir.display()
            );
            std::process::exit(BootExitCode::LockIoFailure.as_i32());
        }
    };

    // 5. Best-effort cleanup of stale `service.<pid>.log*` files older than
    //    24 h that don't belong to the current PID. Without this the per-
    //    PID log naming would let any pathological respawn loop turn the
    //    logs dir into thousands of files.
    logging::cleanup_stale_logs(&app_data_dir);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("ratatoskr-service")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            log::error!("failed to create tokio runtime: {error}");
            std::process::exit(1);
        }
    };

    let exit_code = runtime.block_on(async move {
        let lifecycle = lifecycle::ServiceLifecycle::new(Some(app_data_dir.clone()));
        sigterm::spawn(lifecycle.clone());

        // 6. Wrap the saved FDs/HANDLEs into tokio I/O types now that we
        //    have a runtime context.
        match stdio_defense::adopt_into_runtime(saved_stdio) {
            Ok((stdin, stdout)) => {
                dispatch::run_service_with_io_and_lifecycle(
                    stdin,
                    stdout,
                    lifecycle,
                    app_data_dir,
                )
                .await
            }
            Err(error) => {
                log::error!("failed to adopt service stdio into runtime: {error}");
                1
            }
        }
    });

    drop(_instance_lock);
    std::process::exit(exit_code);
}

fn app_data_dir_from_args() -> Option<PathBuf> {
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        if arg == "--app-data-dir" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

/// Test-only override for the version reported in `health.ping` responses.
/// Triggered by `--test-fake-version=N` on the Service command line. Used
/// by the version-mismatch integration test; off in production builds.
#[cfg(feature = "test-helpers")]
pub(crate) fn test_fake_version() -> Option<u32> {
    let mut args = std::env::args();
    while let Some(arg) = args.next() {
        if let Some(value) = arg.strip_prefix("--test-fake-version=") {
            return value.parse().ok();
        }
        if arg == "--test-fake-version" {
            return args.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

/// Test-only flag: when set, the dispatch loop ignores stdin EOF and
/// parks indefinitely instead of exiting. Simulates a wedged Service
/// (panic-handler that doesn't terminate, kernel-level lock contention,
/// etc.) so the client-Drop kill-escalation path can be exercised
/// end-to-end. Triggered by `--test-hang-on-stdin-eof` on the command
/// line; off in production builds.
#[cfg(feature = "test-helpers")]
pub(crate) fn test_hang_on_stdin_eof() -> bool {
    std::env::args().any(|arg| arg == "--test-hang-on-stdin-eof")
}

fn default_app_data_dir() -> PathBuf {
    if let Some(dev_dir) = workspace_dev_data_dir() {
        return dev_dir;
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("org.folknor.ratatoskr")
}

/// When the Service is invoked directly via `cargo run -p app -- --service`
/// (no `--app-data-dir` passed), point at `<workspace>/target/service-dev/`
/// instead of the production data dir. Detection: walk up from `current_exe`
/// looking for an ancestor that has both `Cargo.toml` and a `target` dir.
fn workspace_dev_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut current = exe.parent()?;
    loop {
        if current.join("Cargo.toml").is_file() && current.join("target").is_dir() {
            return Some(current.join("target").join("service-dev"));
        }
        current = current.parent()?;
    }
}
