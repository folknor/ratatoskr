// Thin binary shim. Almost everything lives in lib.rs so integration tests
// in `tests/` can `use app::*`.

fn main() -> iced::Result {
    if std::env::args().any(|arg| arg == "--service") {
        service::run_service_blocking();
    }
    #[cfg(feature = "test-helpers")]
    if let Some(script) = test_harness_script_arg() {
        return match app::harness::run(script) {
            Ok(()) => Ok(()),
            Err(error) => {
                eprintln!("[harness] {error}");
                std::process::exit(1);
            }
        };
    }
    app::run()
}

#[cfg(feature = "test-helpers")]
fn test_harness_script_arg() -> Option<std::path::PathBuf> {
    let mut args = std::env::args_os();
    while let Some(arg) = args.next() {
        if arg == "--test-harness" {
            return args.next().map(std::path::PathBuf::from);
        }
    }
    None
}
