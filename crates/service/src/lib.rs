mod dispatch;
mod handlers;
mod lifecycle;
mod logging;
pub mod parent_death;
mod sigterm;
mod stdio_defense;

use std::path::PathBuf;

pub use dispatch::run_service_with_io;

pub fn run_service_blocking() -> ! {
    let app_data_dir = app_data_dir_from_args().unwrap_or_else(default_app_data_dir);
    let _ = logging::init(&app_data_dir);
    logging::install_panic_hook();

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("ratatoskr-service")
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("[service] failed to create tokio runtime: {error}");
            std::process::exit(1);
        }
    };

    let exit_code = runtime.block_on(async move {
        parent_death::exit_if_parent_missing();
        let lifecycle = lifecycle::ServiceLifecycle::new(Some(app_data_dir));
        sigterm::spawn(lifecycle.clone());

        #[cfg(unix)]
        {
            match stdio_defense::claim_stdio() {
                Ok((stdin, stdout)) => {
                    dispatch::run_service_with_io_and_lifecycle(stdin, stdout, lifecycle).await
                }
                Err(error) => {
                    log::error!("failed to claim service stdio: {error}");
                    1
                }
            }
        }

        #[cfg(windows)]
        {
            let stdin = tokio::io::stdin();
            let stdout = tokio::io::stdout();
            dispatch::run_service_with_io_and_lifecycle(stdin, stdout, lifecycle).await
        }
    });

    std::process::exit(exit_code);
}

fn app_data_dir_from_args() -> Option<PathBuf> {
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        if arg == "--app-data-dir" {
            return args.next().map(PathBuf::from);
        }
    }
    None
}

fn default_app_data_dir() -> PathBuf {
    if let Some(dev_dir) = workspace_dev_data_dir() {
        return dev_dir;
    }
    dirs::data_dir()
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")))
        .join("org.folknor.ratatoskr")
}

/// When the Service is invoked directly via `cargo run -p app -- --service`
/// (no `--app-data-dir` passed), point at `<workspace>/target/service-dev/`
/// instead of the production data dir. Detection: walk up from `current_exe`
/// looking for an ancestor that has both `Cargo.toml` and a `target` dir.
fn workspace_dev_data_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let mut current = exe.parent()?;
    loop {
        if current.join("Cargo.toml").is_file() && current.join("target").is_dir() {
            return Some(current.join("target").join("service-dev"));
        }
        current = current.parent()?;
    }
}
