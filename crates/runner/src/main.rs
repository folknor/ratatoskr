fn main() -> app::RunResult {
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
