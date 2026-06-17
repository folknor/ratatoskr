// MANUAL TEST REQUIRED
//
// The Windows Job Object parent-death machinery cannot be exercised from
// the Linux CI host. Three paths in this module land entirely manual until
// run on a real Windows host (tracked under "Test Harness" in `TODO.md`):
//
// 1. Job creation + KILL_ON_JOB_CLOSE flag actually applied.
// 2. AssignProcessToJobObject after spawn - the spawn->assign window is
//    accepted as residual risk; verify that under steady-state load the
//    parent dying still kills the Service.
// 3. Job inheritance for grandchildren (Phase 7 extractor subprocesses)
//    when those land.
//
// Re-run the manual checks any time this module changes.
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle, RawHandle};
use std::ptr;
use windows_sys::Win32::Foundation::{FALSE, HANDLE};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
    SetInformationJobObject,
};
use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};

/// Anonymous Job Object configured with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
/// When the last handle to this Job is released - which happens when the
/// parent process dies, holding this handle in `ServiceClient` for its
/// lifetime - the OS terminates every process in the Job. That covers the
/// Service plus any grandchildren it spawns later (PDF / OOXML extractors
/// in Phase 7), and avoids the PID-reuse race a manual lookup would have.
pub(super) struct Job {
    handle: OwnedHandle,
}

impl Job {
    pub(super) fn new() -> io::Result<Self> {
        // SAFETY: CreateJobObjectW with NULL security attrs and NULL name
        // creates an anonymous Job we own outright; on success the returned
        // HANDLE is a fresh, valid kernel handle.
        let raw = unsafe { CreateJobObjectW(ptr::null(), ptr::null()) };
        if raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a valid handle returned by CreateJobObjectW; we
        // transfer ownership to `OwnedHandle` so Drop closes it for us.
        let handle = unsafe { OwnedHandle::from_raw_handle(raw as RawHandle) };

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let info_size = u32::try_from(std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>())
            .map_err(io::Error::other)?;
        // SAFETY: `handle` is a valid Job handle; `info` is fully initialized
        // above; `info_size` matches `info`'s layout.
        let result = unsafe {
            SetInformationJobObject(
                handle.as_raw_handle() as HANDLE,
                JobObjectExtendedLimitInformation,
                std::ptr::from_ref(&info).cast(),
                info_size,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { handle })
    }

    /// Assign a freshly-spawned child to this Job. The brief window between
    /// `CreateProcess` returning and this call is closed by the fact that the
    /// parent's `tokio::process::Child` keeps an open handle to the child for
    /// the child's full lifetime; the kernel will not recycle the PID while
    /// that handle exists, so an OpenProcess(pid) lookup here is unambiguous.
    ///
    /// The Phase 1 plan originally promised "assigns the Service to it before
    /// spawning. No PID lookup, no PID-reuse race." `tokio::process::Command`
    /// cannot deliver that - it spawns the child running, with no
    /// `CREATE_SUSPENDED` knob. The fully race-free implementation would
    /// require dropping `tokio::process::Command` for raw `CreateProcessW`
    /// with `CREATE_SUSPENDED` set, then `AssignProcessToJobObject`, then
    /// `ResumeThread` on the child's main thread. That means
    /// reimplementing significant parts of tokio's process plumbing
    /// (stdio piping, exit-status reaping, kill_on_drop semantics) and
    /// owning the platform-specific bug surface that comes with it.
    /// Accepted residual risk for v1: the spawn->assign window is on the
    /// order of microseconds in normal operation, and the dominant
    /// teardown path (UI Shutdown handshake or external SIGKILL hitting
    /// after assign) doesn't touch this race. If the manual-test matrix
    /// or a real Windows host surfaces breakage, the `CreateProcessW`
    /// restructure is the documented follow-up.
    pub(super) fn assign(&self, child_pid: u32) -> io::Result<()> {
        // SAFETY: OpenProcess returns a fresh handle on success. We need
        // PROCESS_SET_QUOTA (Job assignment is a quota operation) plus
        // PROCESS_TERMINATE so the Job actually has the right to enforce
        // KILL_ON_JOB_CLOSE.
        let process =
            unsafe { OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, FALSE, child_pid) };
        if process.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: ownership is transferred; OwnedHandle's Drop closes it.
        let process = unsafe { OwnedHandle::from_raw_handle(process as RawHandle) };
        let result = unsafe {
            AssignProcessToJobObject(
                self.handle.as_raw_handle() as HANDLE,
                process.as_raw_handle() as HANDLE,
            )
        };
        if result == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }
}
