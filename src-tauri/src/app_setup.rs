use std::sync::Arc;
use tauri::Manager;

pub fn init_app_state(app: &tauri::App) -> tauri::Result<()> {
    let app_data_dir = app
        .path()
        .app_data_dir()
        .map_err(|e| std::io::Error::other(format!("app data dir: {e}")))?;
    let db_state = crate::db::DbState::init(&app_data_dir)
        .map_err(|e| std::io::Error::other(format!("db init: {e}")))?;
    app.manage(db_state.clone());

    let body_store_state = crate::body_store::BodyStoreState::init(&app_data_dir)
        .map_err(|e| std::io::Error::other(format!("body store init: {e}")))?;
    app.manage(body_store_state.clone());

    let inline_image_store_state =
        crate::inline_image_store::InlineImageStoreState::init(&app_data_dir)
            .map_err(|e| std::io::Error::other(format!("inline image store init: {e}")))?;
    app.manage(inline_image_store_state.clone());

    let search_state = crate::search::SearchState::init(&app_data_dir)
        .map_err(|e| std::io::Error::other(format!("search init: {e}")))?;
    app.manage(search_state.clone());

    app.manage(crate::command_palette::CommandRegistry::new());
    app.manage(crate::sync::SyncState::new());
    app.manage(crate::sync::SyncQueueState::new());
    app.manage(crate::sync::BackgroundSyncState::new());
    app.manage(crate::oauth::PendingOAuthAuthorizations::default());

    let encryption_key = crate::provider::crypto::load_encryption_key(&app_data_dir)
        .unwrap_or_else(|e| {
            log::warn!("Gmail provider: no encryption key ({e}), will init on first use");
            [0u8; 32]
        });
    let crypto_state = crate::provider::crypto::AppCryptoState::new(encryption_key);
    app.manage(crypto_state.clone());

    let gmail_state = crate::gmail::client::GmailState::new(encryption_key);
    app.manage(gmail_state.clone());

    let jmap_state = crate::jmap::client::JmapState::new(encryption_key);
    app.manage(jmap_state.clone());

    let graph_state = crate::graph::client::GraphState::new(encryption_key);
    app.manage(graph_state.clone());

    let providers = crate::state::ProviderStates::new(
        Arc::new(gmail_state),
        Arc::new(jmap_state),
        Arc::new(graph_state),
        encryption_key,
    );
    let app_state = crate::state::AppState {
        db: db_state,
        body_store: body_store_state,
        inline_images: inline_image_store_state,
        search: search_state,
        crypto: crypto_state,
        providers,
        progress: Arc::new(crate::progress::TauriProgressReporter::from_ref(
            app.handle(),
        )),
        app_data_dir,
    };
    app.manage(app_state);

    Ok(())
}
