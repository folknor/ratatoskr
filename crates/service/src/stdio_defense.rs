#[cfg(unix)]
pub(crate) fn claim_stdio() -> std::io::Result<(tokio::fs::File, tokio::fs::File)> {
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

    let devnull = OpenOptions::new().read(true).write(true).open("/dev/null")?;
    let devnull_fd = std::os::fd::AsRawFd::as_raw_fd(&devnull);
    if unsafe { libc::dup2(devnull_fd, libc::STDIN_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error());
    }
    if unsafe { libc::dup2(devnull_fd, libc::STDOUT_FILENO) } < 0 {
        return Err(std::io::Error::last_os_error());
    }

    let stdin = unsafe { std::fs::File::from_raw_fd(stdin_fd) };
    let stdout = unsafe { std::fs::File::from_raw_fd(stdout_fd) };
    Ok((tokio::fs::File::from_std(stdin), tokio::fs::File::from_std(stdout)))
}
