// MANUAL TEST REQUIRED - but mostly automated.
//
// The end-to-end "parent SIGKILL kills Service within 2 s" path is covered
// by `crates/app/tests/service_subprocess.rs::linux_parent_sigkill_terminates_service_within_two_seconds`.
// Re-run the cross-platform smoke checks in `docs/service/manual-test-matrix.md`
// any time this module's race-close logic changes - the automated test only
// exercises the steady-state PR_SET_PDEATHSIG delivery, not the fork-to-recheck
// window for `getppid() == 1`.
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
