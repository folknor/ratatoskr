fn main() -> app::RunResult {
    // The parent-death test helper is a hidden mode of this same binary
    // rather than a separate sibling executable. Folding it in means the
    // single `ratatoskr` binary the harness build produces always carries
    // the capability - there is no second binary that can be left unbuilt
    // in the harness bin dir (which is exactly what silently broke the
    // `parent_sigkill` gate when the harness build emitted only `ratatoskr`
    // and no `parent_death_helper`). The flag is never passed in
    // production; only the service-harness spawns it.
    if let Some((service_binary, data_dir)) = parent_death_helper_args() {
        run_parent_death_helper(&service_binary, &data_dir);
    }

    if std::env::args().any(|arg| arg == "--service") {
        // `run_service_blocking` is divergent (`-> !`): it runs the
        // service event loop until shutdown and calls
        // `std::process::exit` itself. The `!` return type is what
        // protects us against fall-through to the test-harness branch
        // below; if a future refactor relaxes the signature, the
        // compiler will start complaining at this call site (because
        // `main` needs a `RunResult`, not `()`) and force an explicit
        // exit decision rather than silently torn-down service.
        service::run_service_blocking()
    }

    if let Some(script) = test_harness_script_arg() {
        return match app::harness::run(script) {
            Ok(()) => Ok(()),
            Err(error) => {
                eprintln!("[harness] {error}");
                std::process::exit(1);
            }
        };
    }

    app::run_app_blocking()
}

fn test_harness_script_arg() -> Option<std::path::PathBuf> {
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        if arg == "--test-harness" {
            return args.next().map(std::path::PathBuf::from);
        }
    }
    None
}

/// Parse `--parent-death-helper <service_binary> <data_dir>`. Returns the two
/// positional operands when the flag is present, `None` otherwise.
fn parent_death_helper_args() -> Option<(std::ffi::OsString, std::ffi::OsString)> {
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        if arg == "--parent-death-helper" {
            let service_binary = args.next()?;
            let data_dir = args.next()?;
            return Some((service_binary, data_dir));
        }
    }
    None
}

/// Test helper for the parent-death integration test (`parent_sigkill.lua`).
///
/// Spawns the Service binary with `PR_SET_PDEATHSIG = SIGTERM` set on the
/// child via `pre_exec`, prints the resulting Service PID to stdout, and
/// sleeps indefinitely. The test process reads the PID, then SIGKILLs this
/// process; that fires the kernel's parent-death signal in the Service, which
/// the test then polls to confirm exits within the deadline. Diverges: it
/// either loops forever (Linux) or exits non-zero (other platforms), so it
/// never falls through to the normal runner dispatch.
#[cfg(target_os = "linux")]
fn run_parent_death_helper(service_binary: &std::ffi::OsStr, data_dir: &std::ffi::OsStr) -> ! {
    use std::io::Write;
    use std::os::unix::process::CommandExt;

    let mut command = std::process::Command::new(service_binary);
    command.arg("--service").arg("--app-data-dir").arg(data_dir);

    unsafe {
        command.pre_exec(|| {
            let result = libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
            if result != 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    let child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            eprintln!("parent_death_helper: failed to spawn service: {error}");
            std::process::exit(1);
        }
    };

    println!("{}", child.id());
    if std::io::stdout().flush().is_err() {
        std::process::exit(1);
    }

    loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
    }
}

#[cfg(not(target_os = "linux"))]
fn run_parent_death_helper(_service_binary: &std::ffi::OsStr, _data_dir: &std::ffi::OsStr) -> ! {
    eprintln!("parent_death_helper is Linux-only");
    std::process::exit(1);
}

#[cfg(test)]
mod tests {
    #[test]
    fn production_runner_does_not_enable_dev_seed_by_default() {
        let manifest = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"),
        )
        .expect("read runner Cargo.toml");

        assert!(
            manifest.contains(r#"app = { path = "../app", default-features = false }"#),
            "runner must disable app default features so production builds do not pull dev-seed",
        );
        assert!(
            manifest.contains(r#"service = { path = "../service", default-features = false }"#),
            "runner must keep service defaults disabled too",
        );
        assert!(
            manifest.contains("default = []"),
            "runner default feature set must stay empty",
        );
        assert!(
            manifest.contains(r#"dev-seed = ["app/dev-seed"]"#),
            "dev-seed must be an explicit runner feature, not a production default",
        );
    }
}
