//! Stdio corruption defense.
//!
//! Any transitive `println!`, default tracing-subscriber stdout, panic-handler
//! stdin read, or interactive printing call site that lands on the JSON-RPC
//! pipe will desynchronize framing irrecoverably. The defense:
//!
//! 1. Synchronously duplicate the real stdin/stdout to fresh FDs/HANDLEs that
//!    only this Service holds, then redirect the global slots to `/dev/null`
//!    (Linux) or `NUL` (Windows). After this returns, every other call site
//!    in the process - logger init, panic hooks, transitive prints, default
//!    tracing - writes to a sink, not the IPC pipe.
//! 2. Once a tokio runtime exists, wrap the saved FDs/HANDLEs in tokio I/O
//!    types so the dispatch loop can read/write asynchronously.
//!
//! The split exists because tokio's `pipe::Receiver::from_file_unchecked`
//! and `tokio::fs::File::from_std` require a runtime context. We do the
//! redirect *before* any other code (so logger init et al. land on the
//! sink) but the wrapping has to happen later inside `block_on`.

use std::io;

/// Saved real stdin/stdout, holding the resource that the dispatch loop will
/// later wrap into tokio I/O types. `claim_stdio` produces this synchronously;
/// `adopt_into_runtime` consumes it inside a runtime context.
pub(crate) struct SavedStdio {
    #[cfg(target_os = "linux")]
    stdin_fd: std::os::fd::OwnedFd,
    #[cfg(target_os = "linux")]
    stdout_fd: std::os::fd::OwnedFd,
    #[cfg(windows)]
    stdin_handle: std::os::windows::io::OwnedHandle,
    #[cfg(windows)]
    stdout_handle: std::os::windows::io::OwnedHandle,
}

/// Synchronously dup the real stdin/stdout aside and redirect the global
/// slots to the bit-bucket. Returns the saved descriptors for later wrapping.
/// Must run before any other Service code so transitive prints/logs/panics
/// land on the sink rather than the IPC pipe.
#[cfg(target_os = "linux")]
pub(crate) fn claim_stdio() -> io::Result<SavedStdio> {
    use std::fs::OpenOptions;
    use std::os::fd::{FromRawFd, OwnedFd};

    let stdin_raw = unsafe { libc::dup(libc::STDIN_FILENO) };
    if stdin_raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `dup` returned a fresh, owned fd; transfer ownership.
    let stdin_fd = unsafe { OwnedFd::from_raw_fd(stdin_raw) };

    let stdout_raw = unsafe { libc::dup(libc::STDOUT_FILENO) };
    if stdout_raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `dup` returned a fresh, owned fd; transfer ownership.
    let stdout_fd = unsafe { OwnedFd::from_raw_fd(stdout_raw) };

    set_nonblocking_fd(&stdin_fd)?;
    set_nonblocking_fd(&stdout_fd)?;

    let devnull = OpenOptions::new().read(true).write(true).open("/dev/null")?;
    let devnull_fd = std::os::fd::AsRawFd::as_raw_fd(&devnull);
    if unsafe { libc::dup2(devnull_fd, libc::STDIN_FILENO) } < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::dup2(devnull_fd, libc::STDOUT_FILENO) } < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(SavedStdio { stdin_fd, stdout_fd })
}

/// Wrap the saved descriptors into tokio I/O types. Must run inside a tokio
/// runtime - `pipe::Receiver::from_file_unchecked` requires it.
#[cfg(target_os = "linux")]
pub(crate) fn adopt_into_runtime(
    saved: SavedStdio,
) -> io::Result<(
    tokio::net::unix::pipe::Receiver,
    tokio::net::unix::pipe::Sender,
)> {
    use std::fs::File;

    let stdin_file = File::from(saved.stdin_fd);
    let stdout_file = File::from(saved.stdout_fd);
    let stdin = tokio::net::unix::pipe::Receiver::from_file_unchecked(stdin_file)?;
    let stdout = tokio::net::unix::pipe::Sender::from_file_unchecked(stdout_file)?;
    Ok((stdin, stdout))
}

#[cfg(target_os = "linux")]
fn set_nonblocking_fd(fd: &std::os::fd::OwnedFd) -> io::Result<()> {
    use std::os::fd::AsRawFd;

    let raw = fd.as_raw_fd();
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFL, 0) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Stdio corruption defense for the Windows Service. Mirrors the Linux
/// strategy: duplicate the original handles aside, redirect the global
/// std slots to `NUL`, then later wrap the saved handles into `tokio::fs::File`.
///
/// `tokio::fs::File` dispatches every read/write through the blocking pool;
/// proper IOCP-driven async pipe handling on Windows is a deeper refactor
/// (no public tokio API today). Revisit when tokio gains a public Windows
/// pipe abstraction.
///
/// Two redirections are necessary on Windows. The Win32 std handle slots
/// (`STD_INPUT_HANDLE`, `STD_OUTPUT_HANDLE`) are what `std::io::stdout` /
/// `println!` go through and what tokio uses; redirecting those is the
/// primary defense and covers all pure-Rust I/O. The C runtime maintains a
/// SEPARATE fd table for fds 0/1 used by `printf`, `fprintf`, and any FFI
/// dependency that prints through the CRT (Phase 7 PDF/OOXML extractors are
/// the realistic future case). `SetStdHandle` does not touch the CRT fd
/// table, so we additionally `_dup2` a fresh NUL-backed CRT fd onto fds 0
/// and 1.
///
/// Partial-failure note: if the first `SetStdHandle` succeeds and the
/// second fails, this function returns `Err` and the `nul_keepalive` File
/// drops, closing the NUL handle that the successful slot still references.
/// The caller (`run_service_blocking`) exits the process on `Err`, so no
/// other code observes the stale slot. The function is therefore not
/// soundness-clean in isolation - fixing that would require a multi-phase
/// rollback of the partial assignment, which is overkill for the only
/// caller's behavior. If the call site ever changes to recover, revisit.
#[cfg(windows)]
pub(crate) fn claim_stdio() -> io::Result<SavedStdio> {
    use std::fs::OpenOptions;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::System::Console::{
        STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, SetStdHandle,
    };
    use windows_sys::Win32::System::Threading::GetCurrentProcess;

    // SAFETY: GetCurrentProcess returns a pseudo-handle that is always valid
    // for the current process; no ownership is transferred and no close is
    // required.
    let process: HANDLE = unsafe { GetCurrentProcess() };

    let stdin_handle = duplicate_std(process, STD_INPUT_HANDLE)?;
    let stdout_handle = duplicate_std(process, STD_OUTPUT_HANDLE)?;

    let nul = OpenOptions::new().read(true).write(true).open("NUL")?;
    let nul_raw = nul.as_raw_handle() as HANDLE;
    // SetStdHandle does not duplicate; the kernel records the raw handle as
    // the new STD slot value. Leak the `nul` File so the underlying handle
    // outlives this function and remains valid for any later print. Closing
    // it would invalidate the slot.
    let nul_keepalive = nul;
    if unsafe { SetStdHandle(STD_INPUT_HANDLE, nul_raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { SetStdHandle(STD_OUTPUT_HANDLE, nul_raw) } == 0 {
        return Err(io::Error::last_os_error());
    }
    std::mem::forget(nul_keepalive);

    redirect_crt_fds_to_nul()?;

    Ok(SavedStdio {
        stdin_handle,
        stdout_handle,
    })
}

/// Redirect C-runtime fds 0 and 1 to NUL via `_dup2` so `printf`-class call
/// sites in any C-FFI dependency can't bypass `SetStdHandle` and reach the
/// JSON-RPC pipe. Pure-Rust `println!` is already covered by the
/// `SetStdHandle` step above; this is forward-defense for Phase 7 extractor
/// crates and similar.
///
/// Implementation: open a dedicated NUL handle for the CRT, wrap it as a
/// CRT fd via `_open_osfhandle`, `_dup2` onto fds 0 and 1 (each `_dup2`
/// internally `DuplicateHandle`s the underlying OS handle, so the resulting
/// fds own independent OS handles), then `_close` the temporary fd. The
/// duplicates remain bound to fd 0 / fd 1 for the process lifetime.
#[cfg(windows)]
fn redirect_crt_fds_to_nul() -> io::Result<()> {
    use std::fs::OpenOptions;
    use std::os::windows::io::IntoRawHandle;

    // _O_BINARY: skip CRT text-mode CR/LF translation. We want raw NUL
    // semantics either way; the constant matters only if any caller later
    // queries the fd's mode.
    const _O_BINARY: i32 = 0x8000;

    // CRT bindings. `_open_osfhandle` takes ownership of the OS handle on
    // success; `_dup2` returns 0 on success / -1 on failure. Using extern
    // declarations rather than the `libc` crate to avoid pulling another
    // Windows dep for three calls.
    unsafe extern "C" {
        fn _open_osfhandle(osfhandle: isize, flags: i32) -> i32;
        fn _dup2(src: i32, dst: i32) -> i32;
        fn _close(fd: i32) -> i32;
    }

    let nul = OpenOptions::new().read(true).write(true).open("NUL")?;
    // _open_osfhandle takes ownership of the underlying handle on success;
    // forget the File wrapper so its drop doesn't double-close.
    let nul_handle = nul.into_raw_handle();
    let crt_fd = unsafe { _open_osfhandle(nul_handle as isize, _O_BINARY) };
    if crt_fd < 0 {
        // _open_osfhandle didn't take ownership on failure - reconstruct
        // the OwnedHandle so its drop closes the OS handle we leaked.
        // SAFETY: nul_handle came from a valid File we just dropped via
        // into_raw_handle; ownership is unchanged.
        unsafe {
            drop(std::os::windows::io::OwnedHandle::from_raw_handle(nul_handle));
        }
        return Err(io::Error::other("_open_osfhandle failed"));
    }

    let dup_in = unsafe { _dup2(crt_fd, 0) };
    let dup_out = unsafe { _dup2(crt_fd, 1) };
    // Always close the temporary CRT fd, even if a _dup2 failed - it owns
    // a duplicate of the NUL handle that we don't want to leak.
    unsafe { _close(crt_fd) };
    if dup_in < 0 || dup_out < 0 {
        return Err(io::Error::other("_dup2 onto fd 0/1 failed"));
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) fn adopt_into_runtime(
    saved: SavedStdio,
) -> io::Result<(tokio::fs::File, tokio::fs::File)> {
    use std::os::windows::io::IntoRawHandle;

    // SAFETY: `stdin_handle` / `stdout_handle` are owned handles produced by
    // DuplicateHandle; `into_raw_handle` transfers ownership into the new
    // `std::fs::File` which then owns the close.
    let stdin_file = unsafe {
        std::fs::File::from_raw_handle(saved.stdin_handle.into_raw_handle())
    };
    let stdout_file = unsafe {
        std::fs::File::from_raw_handle(saved.stdout_handle.into_raw_handle())
    };
    Ok((
        tokio::fs::File::from_std(stdin_file),
        tokio::fs::File::from_std(stdout_file),
    ))
}

#[cfg(windows)]
fn duplicate_std(
    process: windows_sys::Win32::Foundation::HANDLE,
    which: windows_sys::Win32::System::Console::STD_HANDLE,
) -> io::Result<std::os::windows::io::OwnedHandle> {
    use std::os::windows::io::{FromRawHandle, OwnedHandle, RawHandle};
    use windows_sys::Win32::Foundation::{DUPLICATE_SAME_ACCESS, DuplicateHandle, FALSE, HANDLE};
    use windows_sys::Win32::System::Console::GetStdHandle;

    // SAFETY: GetStdHandle returns the current value of the requested STD
    // slot or INVALID_HANDLE_VALUE; we check both null and -1 as invalid.
    let original = unsafe { GetStdHandle(which) };
    if original.is_null() || original as isize == -1 {
        return Err(io::Error::last_os_error());
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
        return Err(io::Error::last_os_error());
    }
    // SAFETY: DuplicateHandle gave us ownership of `duplicated`.
    Ok(unsafe { OwnedHandle::from_raw_handle(duplicated as RawHandle) })
}
