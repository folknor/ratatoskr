#[cfg(unix)]
pub(crate) fn claim_stdio() -> std::io::Result<(
    tokio::net::unix::pipe::Receiver,
    tokio::net::unix::pipe::Sender,
)> {
    use std::fs::OpenOptions;
    use std::os::fd::FromRawFd;

    let stdin_fd = unsafe { libc::dup(libc::STDIN_FILENO) };
    if stdin_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    let stdout_fd = unsafe { libc::dup(libc::STDOUT_FILENO) };
    if stdout_fd < 0 {
        return Err(std::io::Error::last_os_error());
    }

    set_nonblocking(stdin_fd)?;
    set_nonblocking(stdout_fd)?;

    let devnull = OpenOptions::new().read(true).write(true).open("/dev/null")?;
    let devnull_fd = std::os::fd::AsRawFd::as_raw_fd(&devnull);
    if unsafe { libc::dup2(devnull_fd, libc::STDIN_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::dup2(devnull_fd, libc::STDOUT_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let stdin_file = unsafe { std::fs::File::from_raw_fd(stdin_fd) };
    let stdout_file = unsafe { std::fs::File::from_raw_fd(stdout_fd) };
    let stdin = tokio::net::unix::pipe::Receiver::from_file_unchecked(stdin_file)?;
    let stdout = tokio::net::unix::pipe::Sender::from_file_unchecked(stdout_file)?;
    Ok((stdin, stdout))
}

#[cfg(unix)]
fn set_nonblocking(fd: libc::c_int) -> std::io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Stdio corruption defense for the Windows Service. The C runtime, the
/// default tracing-subscriber, transitive `println!` sites, and panic
/// handlers all eventually write to the global stdout HANDLE. Without this
/// shuffle they would write to the JSON-RPC pipe and desync the framing.
///
/// Strategy mirrors the unix dup-and-replace:
///   1. Duplicate the original stdin / stdout HANDLEs so we keep working
///      copies that point at the real pipes.
///   2. Open `NUL` (Windows's bit-bucket).
///   3. `SetStdHandle(STD_INPUT_HANDLE / STD_OUTPUT_HANDLE, NUL)` so any
///      callsite that goes through the global handles writes to NUL.
///   4. Wrap the saved pipe handles in `tokio::fs::File` for use by the
///      JSON-RPC dispatch loop.
///
/// `tokio::fs::File` dispatches every read / write to the blocking pool;
/// proper IOCP-driven async pipe handling on Windows is a deeper refactor
/// (no public tokio API today). This is the same trade-off the unix side
/// had before the recent `tokio::net::unix::pipe` swap; revisit when tokio
/// gains a public Windows pipe abstraction.
#[cfg(windows)]
pub(crate) fn claim_stdio() -> std::io::Result<(tokio::fs::File, tokio::fs::File)> {
    use std::fs::OpenOptions;
    use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle};
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Console::{
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: GetCurrentProcess returns a pseudo-handle that is always valid
    // for the current process; no ownership is transferred and no close is
    // required.
    let process: HANDLE = unsafe { GetCurrentProcess() };

    let saved_stdin = duplicate_std(process, STD_INPUT_HANDLE)?;
    let saved_stdout = duplicate_std(process, STD_OUTPUT_HANDLE)?;

    let nul = OpenOptions::new().read(true).write(true).open("NUL")?;
    let nul_handle = nul.as_raw_handle() as HANDLE;
    // SetStdHandle does not duplicate; the kernel records the raw handle as
    // the new STD slot value. Leak the `nul` File so the underlying handle
    // outlives this function and remains valid for any later print. Closing
    // it would invalidate the slot.
    let nul_keepalive = nul;
    if unsafe { SetStdHandle(STD_INPUT_HANDLE, nul_handle) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { SetStdHandle(STD_OUTPUT_HANDLE, nul_handle) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    std::mem::forget(nul_keepalive);

    // SAFETY: `saved_stdin` / `saved_stdout` are owned handles produced by
    // DuplicateHandle; `into_raw_handle` transfers ownership into the new
    // `std::fs::File` which then owns the close.
    let stdin_file = unsafe { std::fs::File::from_raw_handle(saved_stdin.into_raw_handle()) };
    let stdout_file = unsafe { std::fs::File::from_raw_handle(saved_stdout.into_raw_handle()) };
    Ok((
        tokio::fs::File::from_std(stdin_file),
        tokio::fs::File::from_std(stdout_file),
    ))
}

#[cfg(windows)]
fn duplicate_std(
    process: windows_sys::Win32::Foundation::HANDLE,
    which: windows_sys::Win32::System::Console::STD_HANDLE,
) -> std::io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
    use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::Console::GetStdHandle;

    // SAFETY: GetStdHandle returns the current value of the requested STD
    // slot or INVALID_HANDLE_VALUE; we check both null and -1 as invalid.
    let original = unsafe { GetStdHandle(which) };
    if original.is_null() || original as isize == -1 {
        return Err(std::io::Error::last_os_error());
    }
    let mut duplicated: HANDLE = std::ptr::null_mut();
    // SAFETY: source and target processes are the same (current); `original`
    // is a valid handle from GetStdHandle; `duplicated` receives ownership.
    let result = unsafe {
        DuplicateHandle(
            process,
            original,
            process,
            &mut duplicated,
            0,
            FALSE,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if result == 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: DuplicateHandle gave us ownership of `duplicated`.
    Ok(unsafe { OwnedHandle::from_raw_handle(duplicated as RawHandle) })
}
