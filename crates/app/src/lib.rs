// ── ARCHITECTURE NOTE ───────────────────────────────────
//
// `lib.rs` is the crate root. Sibling modules live next to it; `main.rs` is
// a thin shim that handles the `--service` short-circuit and defers to
// `app::run()`. Tests in `tests/` import via `app::*` because of this lib
// target.
//
//   - `app`          - `App` struct + `boot/title/theme/daemon_theme`
//   - `message`      - `Message` enum
//   - `subscription` - `App::subscription`
//   - `update`       - `App::update` dispatcher
//   - `main_view`    - `App::view` + main-window rendering
//   - `helpers`      - navigation/thread loading helpers and free fns
//   - `handlers/*`   - feature-scoped handler methods on `App`
// ────────────────────────────────────────────────────────

pub(crate) mod action_resolve;
mod app;
mod appearance;
mod command_dispatch;
mod command_resolver;
mod component;
mod db;
mod display;
mod font;
mod handlers;
mod helpers;
mod icon;
mod main_view;
mod message;
pub mod notification_queue;
mod pop_out;
pub mod service_client;
mod service_subscription;
mod subscription;
mod ui;
mod update;
mod window_state;

pub use app::{App, AppMode, Divider, ReadyApp};
pub(crate) use app::PendingChord;
pub(crate) use helpers::load_accounts;
pub use message::Message;
pub use service_client::ServiceClient;

use std::path::PathBuf;

pub(crate) static APP_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
pub(crate) static DEFAULT_SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();

#[allow(clippy::unwrap_in_result)]
pub fn run() -> iced::Result {
    env_logger::init();
    log::info!("Ratatoskr starting");
    #[cfg(feature = "hotpath")]
    let _hotpath = hotpath::HotpathGuardBuilder::new("ratatoskr::main")
        .percentiles(&[50.0, 95.0, 99.0])
        .functions_limit(0)
        .build();

    #[cfg(feature = "dev-seed")]
    let app_data_dir = {
        let dev_dir = dirs::data_dir()
            .expect("no data dir")
            .join("org.folknor.ratatoskr.dev");

        let config = dev_seed::Config::load_or_default();

        // Always regenerate - ephemeral dev database
        if dev_dir.exists() {
            std::fs::remove_dir_all(&dev_dir).ok();
        }
        std::fs::create_dir_all(&dev_dir).expect("create dev data dir");

        log::info!(
            "Dev-seed: generating ephemeral database in {}",
            dev_dir.display()
        );
        dev_seed::seed_database(&config, &dev_dir).expect("dev-seed failed");

        dev_dir
    };

    #[cfg(not(feature = "dev-seed"))]
    let app_data_dir = dirs::data_dir()
        .expect("no data dir")
        .join("org.folknor.ratatoskr");

    // The DB is no longer opened here. Phase 1.5 makes the Service the
    // canonical owner of boot-time DB work (migrations, recovery, sweep,
    // backfill); the UI's `ReadyApp::from_boot_ready` opens its own
    // read-side ReadDbState after the Service handshake completes.

    let detected_scale = display::detect_default_scale();
    let _ = DEFAULT_SCALE.set(detected_scale);

    let system_font_family = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok();
        rt.and_then(|rt| {
            let fonts = rt.block_on(system_fonts::SystemFonts::detect());
            fonts.ui.map(|f| f.family)
        })
    };
    font::set_system_ui_font(system_font_family);

    let _ = APP_DATA_DIR.set(app_data_dir);

    let mut app = iced::daemon(App::boot, App::update, App::view)
        .title(App::title)
        .theme(App::daemon_theme)
        .scale_factor(|app, _window| app.scale_factor())
        .subscription(App::subscription)
        .default_font(font::text());

    for f in font::load() {
        app = app.font(f);
    }

    app.run()
}
