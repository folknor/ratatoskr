//! Last-libtest holdout in the service-subprocess cohort.
//!
//! Every other test in this file has been ported to Lua scripts under
//! `crates/app/tests/service-harness/`. This one survives because it
//! tests the kernel's `PR_SET_PDEATHSIG` behaviour by SIGKILL-ing the
//! parent process that spawned the Service - a setup the Lua harness
//! can't drive without a sibling-binary spawn primitive (the harness
//! binary IS the parent in every other test). When the harness gains
//! that primitive, this test moves to Lua too and the file goes away.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::process::Command;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

/// RAII handle for the per-test data directory. Removes the dir when dropped
/// (panic-on-test-failure included) so smoke runs don't accumulate stray
/// `target/service-smoke-*` directories. Writes a dummy `ratatoskr.key`
/// so the Service's boot-time key load succeeds.
struct DataDirGuard {
    path: PathBuf,
}

impl DataDirGuard {
    fn new(suffix: &str) -> std::io::Result<Self> {
        let path = std::env::current_dir()?
            .join("target")
            .join(format!("service-smoke-{}-{}", std::process::id(), suffix));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path)?;
        write_dummy_key(&path)?;
        Ok(Self { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

fn write_dummy_key(dir: &Path) -> std::io::Result<()> {
    use base64::{Engine, engine::general_purpose::STANDARD};
    // Non-zero key: crypto-key's `LoadError::AllZero` rejects 32 zero
    // bytes in every build profile, so test fixtures use a constant
    // non-zero pattern. Matches the dev-seed fixture byte for the same
    // reason.
    let key_bytes = [0xA5u8; 32];
    let encoded = STANDARD.encode(key_bytes);
    std::fs::write(dir.join("ratatoskr.key"), encoded)
}

impl Drop for DataDirGuard {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

/// Resolve the path to the `app` binary the test should spawn.
///
/// Preferred source: the `BROKKR_TEST_BIN_DIR` env var that brokkr's test
/// harness sets to the directory containing the just-rebuilt
/// `build_packages` artefacts. Reading `cfg!(debug_assertions)` is
/// unreliable because `[profile.test]` overrides can flip
/// `debug-assertions` in the test binary even though the rebuilt binary
/// lives under `debug/`.
///
/// Fallback: `CARGO_BIN_EXE_app`, the path cargo wires in at compile
/// time of the test crate. This is correct under plain `cargo test` (no
/// brokkr) and under `brokkr check` / `brokkr test` when `build_packages`
/// is unset, since both point at the same binary in those cases.
fn binary_path() -> Result<std::path::PathBuf, std::io::Error> {
    if let Ok(dir) = std::env::var("BROKKR_TEST_BIN_DIR") {
        let candidate = std::path::PathBuf::from(dir).join("app");
        if candidate.exists() {
            return Ok(candidate);
        }
        // BROKKR_TEST_BIN_DIR was set but the binary isn't there. Fall
        // through to CARGO_BIN_EXE_app rather than erroring; the env var
        // can outlive a stale cargo target dir, and the compile-time
        // fallback is still correct in that case.
    }
    option_env!("CARGO_BIN_EXE_app")
        .map(std::path::PathBuf::from)
        .ok_or_else(|| {
            std::io::Error::other(
                "neither BROKKR_TEST_BIN_DIR nor CARGO_BIN_EXE_app is set",
            )
        })
}

/// SIGKILL the helper that spawned the Service; the kernel's
/// PR_SET_PDEATHSIG (set on the child via `pre_exec`) must fire promptly
/// and the Service must exit within ~2 s. Linux-only - macOS is deferred,
/// Windows uses Job Object KILL_ON_JOB_CLOSE which can only be exercised
/// on a real Windows host.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn linux_parent_sigkill_terminates_service_within_two_seconds() -> TestResult {
    use tokio::io::AsyncBufReadExt;

    let service_binary = binary_path()?;
    let helper_binary = option_env!("CARGO_BIN_EXE_parent_death_helper").ok_or_else(|| {
        std::io::Error::other("CARGO_BIN_EXE_parent_death_helper not set")
    })?;
    let data_dir = DataDirGuard::new("parent_sigkill")?;

    let mut helper = Command::new(helper_binary)
        .arg(service_binary)
        .arg(data_dir.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .kill_on_drop(true)
        .spawn()?;

    let stdout = helper
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("helper has no stdout"))?;
    let mut reader = tokio::io::BufReader::new(stdout);
    let mut line = String::new();
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        reader.read_line(&mut line),
    )
    .await
    .map_err(|_| std::io::Error::other("helper did not print pid in time"))??;
    let service_pid: u32 = line
        .trim()
        .parse()
        .map_err(|e| std::io::Error::other(format!("parse pid {line:?}: {e}")))?;

    let helper_pid = helper
        .id()
        .ok_or_else(|| std::io::Error::other("helper has no pid"))?;
    let helper_pid = i32::try_from(helper_pid).map_err(std::io::Error::other)?;
    // SAFETY: SIGKILL on a known PID we just spawned. Holding the
    // `kill_on_drop(true)` Child handle keeps the PID stable.
    let kill_result = unsafe { libc::kill(helper_pid, libc::SIGKILL) };
    if kill_result != 0 {
        return Err(std::io::Error::last_os_error().into());
    }

    let started = std::time::Instant::now();
    while started.elapsed() < std::time::Duration::from_secs(3) {
        if !pid_is_alive(service_pid)? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    Err(std::io::Error::other(format!(
        "Service pid {service_pid} still alive {:?} after parent SIGKILL",
        started.elapsed()
    ))
    .into())
}

#[cfg(unix)]
fn pid_is_alive(pid: u32) -> std::io::Result<bool> {
    let pid = i32::try_from(pid).map_err(std::io::Error::other)?;
    // SAFETY: kill(pid, 0) only checks reachability + permission; no signal
    // is delivered. The libc ABI is straightforward.
    let result = unsafe { libc::kill(pid, 0) };
    if result == 0 {
        return Ok(true);
    }
    let err = std::io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::ESRCH) => Ok(false),
        // EPERM means the process exists but we can't signal it - still alive.
        Some(libc::EPERM) => Ok(true),
        _ => Err(err),
    }
}
