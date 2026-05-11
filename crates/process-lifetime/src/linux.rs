// MANUAL TEST REQUIRED - but mostly automated.
//
// The end-to-end "parent SIGKILL kills Service within 2 s" path is covered
// by `crates/app/tests/service-harness/parent_sigkill.lua`, driven through
// `harness.spawn_parent_death_helper` in the Lua harness. Re-validate the
// fork-to-recheck window for `getppid() == 1` manually any time this
// module's race-close logic changes - the automated test only exercises
// the steady-state PR_SET_PDEATHSIG delivery.
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
    // The recheck guards the fork-to-prctl race only: if the parent
    // exited between fork and `pre_exec` calling `prctl(PR_SET_PDEATHSIG)`,
    // pdeathsig never armed, so we wouldn't get the signal. Init reparents
    // orphans to PID 1 in the typical case, so seeing `getppid() == 1`
    // here means we lost the race.
    //
    // Caveat: on systems with a `PR_SET_CHILD_SUBREAPER` ancestor (modern
    // container runtimes - runc, containerd; some session managers like
    // recent systemd-user instances), orphans reparent to the subreaper
    // rather than init. `getppid()` returns the subreaper's PID, not 1,
    // and this check no longer fires. The signal-arm path
    // (`PR_SET_PDEATHSIG`) still covers post-prctl parent death because
    // the kernel delivers the signal regardless of subreaper layout, so
    // the only window left open is "parent died between fork and prctl
    // AND we have a subreaper". Sufficiently rare to accept in v1; flag
    // for revisit if reports of zombie Service processes under specific
    // container runtimes show up.
    if parent_pid == 1 {
        log::warn!("service parent is already gone, exiting");
        std::process::exit(0);
    }
}
