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
