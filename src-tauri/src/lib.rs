#[cfg(not(target_os = "linux"))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconId},
};
use tauri::{Emitter, Manager};
use tauri_plugin_autostart::MacosLauncher;

mod body_store;
mod categorization;
mod commands;
mod db;
mod email_actions;
mod filters;
mod gmail;
mod graph;
mod imap;
mod jmap;
mod oauth;
mod provider;
mod search;
mod smtp;
mod sync;
mod threading;

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
fn close_splashscreen(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("splashscreen") {
        _ = w.close();
    }
    if let Some(w) = app.get_webview_window("main") {
        _ = w.show();
        _ = w.set_focus();
    }
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
fn set_tray_tooltip(app: tauri::AppHandle, tooltip: String) -> Result<(), String> {
    #[cfg(not(target_os = "linux"))]
    {
        let tray = app
            .tray_by_id(&TrayIconId::new("main-tray"))
            .ok_or_else(|| "Tray icon not found".to_string())?;
        tray.set_tooltip(Some(&tooltip)).map_err(|e| e.to_string())
    }
    #[cfg(target_os = "linux")]
    {
        _ = tooltip;
        _ = app;
        log::debug!("set_tray_tooltip is not supported on Linux (KSNI tray)");
        Ok(())
    }
}

#[allow(clippy::needless_pass_by_value)]
#[tauri::command]
fn open_devtools(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        w.open_devtools();
    }
}

#[allow(clippy::too_many_lines)]
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Set explicit AUMID on Windows so toast notifications show "Ratatoskr"
    // instead of "Windows PowerShell"
    #[cfg(windows)]
    {
        use windows::core::w;
        use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        unsafe {
            _ = SetCurrentProcessExplicitAppUserModelID(w!("com.folknor.ratatoskr"));
        }
    }

    tauri::Builder::default()
        // Single instance MUST be first
        .plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                _ = window.show();
                _ = window.set_focus();
                _ = window.unminimize();
            }
            // Forward args for deep linking
            _ = app.emit("single-instance-args", argv);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--hidden"]),
        ))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_sql::Builder::default().build())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_os::init())
        .invoke_handler(tauri::generate_handler![
            oauth::start_oauth_server,
            oauth::oauth_exchange_token,
            oauth::oauth_refresh_token,
            set_tray_tooltip,
            close_splashscreen,
            open_devtools,
            commands::imap_test_connection,
            commands::imap_list_folders,
            commands::imap_fetch_messages,
            commands::imap_fetch_new_uids,
            commands::imap_search_all_uids,
            commands::imap_fetch_message_body,
            commands::imap_fetch_raw_message,
            commands::imap_set_flags,
            commands::imap_move_messages,
            commands::imap_delete_messages,
            commands::imap_get_folder_status,
            commands::imap_fetch_attachment,
            commands::imap_append_message,
            commands::imap_search_folder,
            commands::imap_sync_folder,
            commands::imap_raw_fetch_diagnostic,
            commands::imap_delta_check,
            commands::smtp_send_email,
            commands::smtp_test_connection,
            // Rust-owned DB commands (Phase 1)
            db::queries::db_get_threads,
            db::queries::db_get_threads_for_category,
            db::queries::db_get_thread_by_id,
            db::queries::db_get_thread_label_ids,
            db::queries::db_get_messages_for_thread,
            db::queries::db_get_labels,
            db::queries::db_get_setting,
            db::queries::db_get_all_settings,
            db::queries::db_set_setting,
            db::queries::db_get_category_unread_counts,
            db::queries::db_get_categories_for_threads,
            // Rust-owned DB commands (Phase 1 — mutations)
            db::queries::db_set_thread_read,
            db::queries::db_set_thread_starred,
            db::queries::db_set_thread_pinned,
            db::queries::db_set_thread_muted,
            db::queries::db_delete_thread,
            db::queries::db_add_thread_label,
            db::queries::db_remove_thread_label,
            db::queries::db_upsert_label,
            db::queries::db_delete_label,
            // Rust-owned DB commands (Phase 2 — contacts, attachments, counts)
            db::queries::db_search_contacts,
            db::queries::db_get_contact_by_email,
            db::queries::db_get_attachments_for_message,
            db::queries::db_get_thread_count,
            db::queries::db_get_unread_count,
            // Rust-owned DB commands (Phase 1.5 — full CRUD)
            db::queries_extra::db_get_all_contacts,
            db::queries_extra::db_upsert_contact,
            db::queries_extra::db_update_contact,
            db::queries_extra::db_update_contact_notes,
            db::queries_extra::db_delete_contact,
            db::queries_extra::db_get_contact_stats,
            db::queries_extra::db_get_contacts_from_same_domain,
            db::queries_extra::db_get_latest_auth_result,
            db::queries_extra::db_get_recent_threads_with_contact,
            db::queries_extra::db_get_attachments_from_contact,
            db::queries_extra::db_get_filters_for_account,
            db::queries_extra::db_insert_filter,
            db::queries_extra::db_update_filter,
            db::queries_extra::db_delete_filter,
            db::queries_extra::db_get_smart_folders,
            db::queries_extra::db_get_smart_folder_by_id,
            db::queries_extra::db_insert_smart_folder,
            db::queries_extra::db_update_smart_folder,
            db::queries_extra::db_delete_smart_folder,
            db::queries_extra::db_update_smart_folder_sort_order,
            db::queries_extra::db_get_smart_label_rules_for_account,
            db::queries_extra::db_insert_smart_label_rule,
            db::queries_extra::db_update_smart_label_rule,
            db::queries_extra::db_delete_smart_label_rule,
            db::queries_extra::db_insert_follow_up_reminder,
            db::queries_extra::db_get_follow_up_for_thread,
            db::queries_extra::db_cancel_follow_up_for_thread,
            db::queries_extra::db_get_active_follow_up_thread_ids,
            db::queries_extra::db_check_follow_up_reminders,
            db::queries_extra::db_get_quick_steps_for_account,
            db::queries_extra::db_get_enabled_quick_steps_for_account,
            db::queries_extra::db_insert_quick_step,
            db::queries_extra::db_update_quick_step,
            db::queries_extra::db_delete_quick_step,
            db::queries_extra::db_add_to_allowlist,
            db::queries_extra::db_get_allowlisted_senders,
            db::queries_extra::db_add_vip_sender,
            db::queries_extra::db_remove_vip_sender,
            db::queries_extra::db_is_vip_sender,
            db::queries_extra::db_set_thread_category,
            db::queries_extra::db_get_bundle_rules,
            db::queries_extra::db_get_bundle_summaries,
            db::queries_extra::db_get_held_thread_ids,
            db::queries_extra::db_attachment_cache_total_size,
            db::queries_extra::db_uncached_recent_attachments,
            // Body store (Phase 2 — compressed body storage)
            body_store::commands::body_store_put,
            body_store::commands::body_store_put_batch,
            body_store::commands::body_store_get,
            body_store::commands::body_store_get_batch,
            body_store::commands::body_store_delete,
            body_store::commands::body_store_stats,
            body_store::commands::body_store_migrate,
            // Tantivy full-text search (Phase 3)
            search::commands::search_messages,
            search::commands::index_message,
            search::commands::index_messages_batch,
            search::commands::delete_search_document,
            search::commands::rebuild_search_index,
            // Email actions — local DB + pending op queue (Phase 5)
            email_actions::commands::email_action_archive,
            email_actions::commands::email_action_trash,
            email_actions::commands::email_action_permanent_delete,
            email_actions::commands::email_action_spam,
            email_actions::commands::email_action_mark_read,
            email_actions::commands::email_action_star,
            email_actions::commands::email_action_snooze,
            email_actions::commands::email_action_unsnooze,
            email_actions::commands::email_action_unsnooze_batch,
            email_actions::commands::email_action_pin,
            email_actions::commands::email_action_unpin,
            email_actions::commands::email_action_mute,
            email_actions::commands::email_action_unmute,
            email_actions::commands::email_action_add_label,
            email_actions::commands::email_action_remove_label,
            email_actions::commands::email_action_move_to_folder,
            // Pending operations queue
            db::pending_ops::db_pending_ops_enqueue,
            db::pending_ops::db_pending_ops_get,
            db::pending_ops::db_pending_ops_update_status,
            db::pending_ops::db_pending_ops_delete,
            db::pending_ops::db_pending_ops_increment_retry,
            db::pending_ops::db_pending_ops_count,
            db::pending_ops::db_pending_ops_failed_count,
            db::pending_ops::db_pending_ops_for_resource,
            db::pending_ops::db_pending_ops_compact,
            db::pending_ops::db_pending_ops_clear_failed,
            db::pending_ops::db_pending_ops_retry_failed,
            // Filter engine (Phase 6)
            filters::commands::filters_evaluate,
            // JWZ threading (Phase 6)
            threading::commands::threading_build_threads,
            threading::commands::threading_update_threads,
            // Categorization rule engine (Phase 6)
            categorization::commands::categorize_thread_by_rules,
            categorization::commands::categorize_threads_by_rules,
            // IMAP sync engine (Phase 4)
            sync::commands::sync_imap_initial,
            sync::commands::sync_imap_delta,
            // Gmail API provider (Rust)
            gmail::commands::gmail_init_client,
            gmail::commands::gmail_remove_client,
            gmail::commands::gmail_test_connection,
            gmail::commands::gmail_list_labels,
            gmail::commands::gmail_create_label,
            gmail::commands::gmail_update_label,
            gmail::commands::gmail_delete_label,
            gmail::commands::gmail_list_threads,
            gmail::commands::gmail_get_thread,
            gmail::commands::gmail_modify_thread,
            gmail::commands::gmail_delete_thread,
            gmail::commands::gmail_get_message,
            gmail::commands::gmail_get_parsed_message,
            gmail::commands::gmail_send_email,
            gmail::commands::gmail_fetch_attachment,
            gmail::commands::gmail_get_history,
            gmail::commands::gmail_create_draft,
            gmail::commands::gmail_update_draft,
            gmail::commands::gmail_delete_draft,
            gmail::commands::gmail_list_drafts,
            gmail::commands::gmail_fetch_send_as,
            // Gmail sync engine (Phase 2)
            gmail::commands::gmail_sync_initial,
            gmail::commands::gmail_sync_delta,
            // JMAP provider
            jmap::commands::jmap_init_client,
            jmap::commands::jmap_remove_client,
            jmap::commands::jmap_test_connection,
            jmap::commands::jmap_discover_url,
            jmap::commands::jmap_get_profile,
            jmap::commands::jmap_sync_initial,
            jmap::commands::jmap_sync_delta,
            jmap::commands::jmap_list_folders,
            jmap::commands::jmap_create_folder,
            jmap::commands::jmap_rename_folder,
            jmap::commands::jmap_delete_folder,
            jmap::commands::jmap_archive,
            jmap::commands::jmap_trash,
            jmap::commands::jmap_permanent_delete,
            jmap::commands::jmap_mark_read,
            jmap::commands::jmap_star,
            jmap::commands::jmap_spam,
            jmap::commands::jmap_move_to_folder,
            jmap::commands::jmap_add_label,
            jmap::commands::jmap_remove_label,
            jmap::commands::jmap_send_email,
            jmap::commands::jmap_create_draft,
            jmap::commands::jmap_update_draft,
            jmap::commands::jmap_delete_draft,
            jmap::commands::jmap_fetch_attachment,
            // Graph provider commands (Phase 3b)
            graph::commands::graph_init_client,
            graph::commands::graph_remove_client,
            graph::commands::graph_test_connection,
            graph::commands::graph_get_profile,
            // Provider-agnostic commands (Phase 3a)
            provider::commands::provider_sync_initial,
            provider::commands::provider_sync_delta,
            provider::commands::provider_archive,
            provider::commands::provider_trash,
            provider::commands::provider_permanent_delete,
            provider::commands::provider_mark_read,
            provider::commands::provider_star,
            provider::commands::provider_spam,
            provider::commands::provider_move_to_folder,
            provider::commands::provider_add_tag,
            provider::commands::provider_remove_tag,
            provider::commands::provider_send_email,
            provider::commands::provider_create_draft,
            provider::commands::provider_update_draft,
            provider::commands::provider_delete_draft,
            provider::commands::provider_fetch_attachment,
            provider::commands::provider_list_folders,
        ])
        .setup(|app| {
            {
                let level = if cfg!(debug_assertions) {
                    log::LevelFilter::Debug
                } else {
                    log::LevelFilter::Info
                };
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(level)
                        .level_for("sqlx::query", log::LevelFilter::Warn)
                        .build(),
                )?;
            }

            // Initialize Rust-owned SQLite database (same ratatoskr.db file)
            // and tantivy search index
            {
                let app_data_dir = app.path().app_data_dir().map_err(|e| {
                    Box::new(std::io::Error::other(format!("app data dir: {e}")))
                })?;
                let db_state = db::DbState::init(&app_data_dir).map_err(|e| {
                    Box::new(std::io::Error::other(format!("db init: {e}")))
                })?;
                app.manage(db_state);

                let body_store_state =
                    body_store::BodyStoreState::init(&app_data_dir).map_err(|e| {
                        Box::new(std::io::Error::other(format!("body store init: {e}")))
                    })?;
                app.manage(body_store_state);

                let search_state = search::SearchState::init(&app_data_dir).map_err(|e| {
                    Box::new(std::io::Error::other(format!("search init: {e}")))
                })?;
                app.manage(search_state);

                app.manage(sync::SyncState::new());

                // Gmail provider state — load encryption key for token decryption
                let encryption_key = provider::crypto::load_encryption_key(&app_data_dir)
                    .unwrap_or_else(|e| {
                        log::warn!("Gmail provider: no encryption key ({e}), will init on first use");
                        [0u8; 32]
                    });
                app.manage(gmail::client::GmailState::new(encryption_key));

                // JMAP provider state — shares the same encryption key
                app.manage(jmap::client::JmapState::new(encryption_key));

                // Graph provider state — shares the same encryption key
                app.manage(graph::client::GraphState::new(encryption_key));
            }

            #[cfg(not(target_os = "linux"))]
            {
                // Build system tray menu
                let show = MenuItem::with_id(app, "show", "Show Ratatoskr", true, None::<&str>)?;
                let check_mail =
                    MenuItem::with_id(app, "check_mail", "Check for Mail", true, None::<&str>)?;
                let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
                let menu = Menu::with_items(app, &[&show, &check_mail, &quit])?;

                let icon = app
                    .default_window_icon()
                    .cloned()
                    .expect("app should have a default icon configured in tauri.conf.json bundle");

                TrayIconBuilder::with_id("main-tray")
                    .icon(icon)
                    .tooltip("Ratatoskr")
                    .menu(&menu)
                    .show_menu_on_left_click(false)
                    .on_menu_event(|app, event| match event.id.as_ref() {
                        "show" => {
                            if let Some(window) = app.get_webview_window("main") {
                                _ = window.show();
                                _ = window.set_focus();
                            }
                        }
                        "check_mail" => {
                            if let Some(window) = app.get_webview_window("main") {
                                _ = window.emit("tray-check-mail", ());
                            }
                        }
                        "quit" => {
                            app.exit(0);
                        }
                        _ => {}
                    })
                    .on_tray_icon_event(|tray, event| {
                        if let tauri::tray::TrayIconEvent::DoubleClick { .. } = event {
                            let app = tray.app_handle();
                            if let Some(window) = app.get_webview_window("main") {
                                _ = window.show();
                                _ = window.set_focus();
                            }
                        }
                    })
                    .build(app)?;
            }

            #[cfg(target_os = "linux")]
            {
                use tray_item::{IconSource, TrayItem};

                let app_handle = app.handle().clone();

                std::thread::spawn(move || {
                    let mut tray = match TrayItem::new("Ratatoskr", IconSource::Resource("mail-read")) {
                        Ok(t) => t,
                        Err(e) => {
                            log::warn!("Failed to create system tray: {e}");
                            return;
                        }
                    };

                    let app_handle_show = app_handle.clone();
                    if let Err(e) = tray.add_menu_item("Show Ratatoskr", move || {
                        if let Some(window) = app_handle_show.get_webview_window("main") {
                            _ = window.show();
                            _ = window.set_focus();
                        }
                    }) {
                        log::warn!("Failed to add tray menu item 'Show Ratatoskr': {e}");
                    }

                    let app_handle_check = app_handle.clone();
                    if let Err(e) = tray.add_menu_item("Check for Mail", move || {
                        if let Some(window) = app_handle_check.get_webview_window("main") {
                            _ = window.emit("tray-check-mail", ());
                        }
                    }) {
                        log::warn!("Failed to add tray menu item 'Check for Mail': {e}");
                    }

                    let app_handle_quit = app_handle.clone();
                    if let Err(e) = tray.add_menu_item("Quit", move || {
                        app_handle_quit.exit(0);
                    }) {
                        log::warn!("Failed to add tray menu item 'Quit': {e}");
                    }

                    loop {
                        std::thread::park();
                    }
                });
            }

            // On Windows/Linux, remove decorations for custom titlebar.
            // macOS uses titleBarStyle: "overlay" from config instead, which
            // preserves native event routing in WKWebView.
            #[cfg(not(target_os = "macos"))]
            {
                if let Some(window) = app.get_webview_window("main") {
                    _ = window.set_decorations(false);
                }
            }

            // Start hidden in tray if launched with --hidden (autostart)
            if std::env::args().any(|a| a == "--hidden") {
                if let Some(window) = app.get_webview_window("main") {
                    _ = window.hide();
                }
                // Also close splash screen when starting hidden
                if let Some(splash) = app.get_webview_window("splashscreen") {
                    _ = splash.close();
                }
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            // Minimize to tray on close instead of quitting (main window only)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event
                && window.label() == "main"
            {
                _ = window.hide();
                api.prevent_close();
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");

    log::info!("Tauri application exited normally");
}
