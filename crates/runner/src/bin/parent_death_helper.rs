//! Test helper for the parent-death integration test.
//!
//! Spawns the Service binary with `PR_SET_PDEATHSIG = SIGTERM` set on the
//! child via `pre_exec`, prints the resulting PID to stdout, and sleeps
//! indefinitely. The test process is expected to read the PID, then SIGKILL
//! this helper; that triggers the kernel's parent-death signal in the
//! Service, which we then poll to confirm exits within the deadline.
//!
//! Linux-only. Other platforms compile a stub `main` that exits non-zero.

#[cfg(target_os = "linux")]
fn main() -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::process::CommandExt;

    let mut args = std::env::args().skip(1);
    let service_binary = args.next().ok_or_else(|| {
        std::io::Error::other("usage: parent_death_helper <service_binary> <data_dir>")
    })?;
    let data_dir = args.next().ok_or_else(|| {
        std::io::Error::other("usage: parent_death_helper <service_binary> <data_dir>")
    })?;

    let mut command = std::process::Command::new(&service_binary);
    command.arg("--service").arg("--app-data-dir").arg(&data_dir);

    unsafe {
        command.pre_exec(|| {
            let result = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            if result != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = command.spawn()?;
    println!("{}", child.id());
    std::io::stdout().flush()?;

    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("parent_death_helper is Linux-only");
    std::process::exit(1);
}
