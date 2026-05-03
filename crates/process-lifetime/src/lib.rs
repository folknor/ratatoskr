//! Cross-platform parent-death detection + child-lifetime guards.
//!
//! Extracted from `service::parent_death` so both `service` (the in-child
//! `exit_if_parent_missing` call) and `app` (the `ProcessGuard` held in
//! `ServiceClient`) consume the same machinery without `app -> service`
//! pulling in the entire Service crate just for this surface.

use std::io;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(windows)]
mod windows;

#[cfg(target_os = "linux")]
pub fn configure_command(command: &mut tokio::process::Command) -> io::Result<()> {
    linux::configure_command(command);
    Ok(())
}

#[cfg(not(target_os = "linux"))]
pub fn configure_command(_command: &mut tokio::process::Command) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn exit_if_parent_missing() {
    linux::exit_if_parent_missing();
}

#[cfg(not(target_os = "linux"))]
pub fn exit_if_parent_missing() {}

/// RAII handle that ties the Service's lifetime to its parent's, in a way
/// that survives the parent crashing rather than exiting cleanly.
///
/// - Linux: a no-op; `configure_command` already installed `PR_SET_PDEATHSIG`
///   at fork time, and the Service self-checks `getppid() == 1` at startup.
/// - Windows: an anonymous Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
///   The parent holds the only handle; when this struct drops (or the parent
///   process dies and the OS reclaims its handles), every process in the Job
///   is terminated. The post-spawn `assign` puts the child into the Job.
/// - macOS / other: no-op; the design is in `problem-statement.md` but
///   deferred per Phase 1's "Linux + Windows; macOS deferred to post-1.0".
pub struct ProcessGuard {
    #[cfg(windows)]
    job: windows::Job,
    #[cfg(not(windows))]
    _marker: std::marker::PhantomData<()>,
}

impl ProcessGuard {
    pub fn new() -> io::Result<Self> {
        #[cfg(windows)]
        {
            Ok(Self {
                job: windows::Job::new()?,
            })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                _marker: std::marker::PhantomData,
            })
        }
    }

    /// Place a freshly-spawned child under this guard's protection. Must be
    /// called immediately after `Command::spawn()` to keep the PID-recycle
    /// window negligible on Windows; on other platforms this is a no-op
    /// (the parent-death wiring happened at fork time via `pre_exec`).
    pub fn assign(&self, child: &tokio::process::Child) -> io::Result<()> {
        #[cfg(windows)]
        {
            let pid = child
                .id()
                .ok_or_else(|| io::Error::other("child has no pid"))?;
            self.job.assign(pid)
        }
        #[cfg(not(windows))]
        {
            let _ = child;
            Ok(())
        }
    }
}
