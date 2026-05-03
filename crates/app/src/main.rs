// Thin binary shim. Almost everything lives in lib.rs so integration tests
// in `tests/` can `use app::*`.

fn main() -> iced::Result {
    if std::env::args().any(|arg| arg == "--service") {
        service::run_service_blocking();
    }
    app::run()
}
