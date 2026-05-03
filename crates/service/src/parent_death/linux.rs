pub(super) fn configure_command(command: &mut tokio::process::Command) {
    unsafe {
        command.pre_exec(|| {
            let result = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            if result != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

pub(super) fn exit_if_parent_missing() {
    let parent_pid = unsafe { libc::getppid() };
    if parent_pid == 1 {
        log::warn!("service parent is already gone, exiting");
        std::process::exit(0);
    }
}
