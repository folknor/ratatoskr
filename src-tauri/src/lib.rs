use tauri::{Emitter, Manager};
#[cfg(not(target_os = "linux"))]
use tauri::{
    menu::{Menu, MenuItem},
    tray::{TrayIconBuilder, TrayIconId},
};
use tauri_plugin_autostart::MacosLauncher;

mod account_commands;
mod attachment_cache;
mod body_store;
mod categorization;
mod commands;
mod db;
mod discovery;
mod email_actions;
mod filters;
mod gmail;
mod graph;
mod imap;
mod inline_image_store;
mod jmap;
mod oauth;
mod provider;
mod search;
mod smart_labels;
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
        use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
        use windows::core::w;
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
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_os::init())
        .invoke_handler(tauri::generate_handler![
            account_commands::account_create_gmail_via_oauth,
            account_commands::account_create_graph_via_oauth,
            account_commands::account_authorize_oauth_provider,
            account_commands::account_create_imap_oauth,
            account_commands::account_reauthorize_gmail,
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
            // Batch 1: settings, aiCache, linkScanResults, writingStyleProfiles, folderSyncState
            db::queries_extra::db_get_ai_cache,
            db::queries_extra::db_set_ai_cache,
            db::queries_extra::db_delete_ai_cache,
            db::queries_extra::db_get_cached_scan_result,
            db::queries_extra::db_cache_scan_result,
            db::queries_extra::db_delete_scan_results,
            db::queries_extra::db_get_writing_style_profile,
            db::queries_extra::db_upsert_writing_style_profile,
            db::queries_extra::db_delete_writing_style_profile,
            db::queries_extra::db_get_folder_sync_state,
            db::queries_extra::db_upsert_folder_sync_state,
            db::queries_extra::db_delete_folder_sync_state,
            db::queries_extra::db_clear_all_folder_sync_states,
            db::queries_extra::db_get_all_folder_sync_states,
            // Batch 2: notificationVips, imageAllowlist, phishingAllowlist, templates, signatures
            db::queries_extra::db_get_vip_senders,
            db::queries_extra::db_get_all_vip_senders,
            db::queries_extra::db_is_allowlisted,
            db::queries_extra::db_remove_from_allowlist,
            db::queries_extra::db_get_allowlist_for_account,
            db::queries_extra::db_is_phishing_allowlisted,
            db::queries_extra::db_add_to_phishing_allowlist,
            db::queries_extra::db_remove_from_phishing_allowlist,
            db::queries_extra::db_get_phishing_allowlist,
            db::queries_extra::db_get_templates_for_account,
            db::queries_extra::db_insert_template,
            db::queries_extra::db_update_template,
            db::queries_extra::db_delete_template,
            db::queries_extra::db_get_signatures_for_account,
            db::queries_extra::db_get_default_signature,
            db::queries_extra::db_insert_signature,
            db::queries_extra::db_update_signature,
            db::queries_extra::db_delete_signature,
            // Batch 3: sendAsAliases, localDrafts, scheduledEmails, labels, attachments
            db::queries_extra::db_get_aliases_for_account,
            db::queries_extra::db_upsert_alias,
            db::queries_extra::db_get_default_alias,
            db::queries_extra::db_set_default_alias,
            db::queries_extra::db_delete_alias,
            db::queries_extra::db_save_local_draft,
            db::queries_extra::db_get_local_draft,
            db::queries_extra::db_get_unsynced_drafts,
            db::queries_extra::db_mark_draft_synced,
            db::queries_extra::db_delete_local_draft,
            db::queries_extra::db_get_pending_scheduled_emails,
            db::queries_extra::db_get_scheduled_emails_for_account,
            db::queries_extra::db_insert_scheduled_email,
            db::queries_extra::db_update_scheduled_email_status,
            db::queries_extra::db_delete_scheduled_email,
            db::queries_extra::db_upsert_label_coalesce,
            db::queries_extra::db_delete_labels_for_account,
            db::queries_extra::db_update_label_sort_order,
            db::queries_extra::db_upsert_attachment,
            db::queries_extra::db_get_attachments_for_account,
            db::queries_extra::db_get_attachment_senders,
            // Batch 4: smartFolders/quickSteps/smartLabelRules/followUpReminders/filters (TS-only rewrites)
            // Batch 5: bundleRules, threadCategories, calendars, calendarEvents
            db::queries_extra::db_get_bundle_rule,
            db::queries_extra::db_set_bundle_rule,
            db::queries_extra::db_hold_thread,
            db::queries_extra::db_is_thread_held,
            db::queries_extra::db_release_held_threads,
            db::queries_extra::db_update_last_delivered,
            db::queries_extra::db_get_bundle_summary,
            db::queries_extra::db_get_thread_category,
            db::queries_extra::db_get_thread_category_with_manual,
            db::queries_extra::db_get_recent_rule_categorized_thread_ids,
            db::queries_extra::db_set_thread_categories_batch,
            db::queries_extra::db_get_uncategorized_inbox_thread_ids,
            db::queries_extra::db_upsert_calendar,
            db::queries_extra::db_get_calendars_for_account,
            db::queries_extra::db_get_visible_calendars,
            db::queries_extra::db_set_calendar_visibility,
            db::queries_extra::db_update_calendar_sync_token,
            db::queries_extra::db_delete_calendars_for_account,
            db::queries_extra::db_get_calendar_by_id,
            db::queries_extra::db_upsert_calendar_event,
            db::queries_extra::db_get_calendar_events_in_range,
            db::queries_extra::db_get_calendar_events_in_range_multi,
            db::queries_extra::db_delete_events_for_calendar,
            db::queries_extra::db_get_event_by_remote_id,
            db::queries_extra::db_delete_event_by_remote_id,
            db::queries_extra::db_delete_calendar_event,
            // Batch 6: accounts, contacts
            db::queries_extra::db_get_all_accounts,
            db::queries_extra::db_get_account,
            db::queries_extra::db_get_account_by_email,
            db::queries_extra::db_insert_account,
            db::queries_extra::db_update_account_tokens,
            db::queries_extra::db_update_account_all_tokens,
            db::queries_extra::db_update_account_sync_state,
            db::queries_extra::db_clear_account_history_id,
            db::queries_extra::db_delete_account,
            db::queries_extra::db_update_account_caldav,
            db::queries_extra::db_update_contact_avatar,
            // Batch 7: threads, messages, tasks
            db::queries_extra::db_upsert_thread,
            db::queries_extra::db_set_thread_labels,
            db::queries_extra::db_delete_all_threads_for_account,
            db::queries_extra::db_get_muted_thread_ids,
            db::queries_extra::db_get_unread_inbox_count,
            db::queries_extra::db_get_messages_by_ids,
            db::queries_extra::db_upsert_message,
            db::queries_extra::db_delete_message,
            db::queries_extra::db_update_message_thread_ids,
            db::queries_extra::db_delete_all_messages_for_account,
            db::queries_extra::db_get_recent_sent_messages,
            db::queries_extra::db_get_tasks_for_account,
            db::queries_extra::db_get_task_by_id,
            db::queries_extra::db_get_tasks_for_thread,
            db::queries_extra::db_get_subtasks,
            db::queries_extra::db_insert_task,
            db::queries_extra::db_update_task,
            db::queries_extra::db_delete_task,
            db::queries_extra::db_complete_task,
            db::queries_extra::db_uncomplete_task,
            db::queries_extra::db_reorder_tasks,
            db::queries_extra::db_get_incomplete_task_count,
            db::queries_extra::db_get_task_tags,
            db::queries_extra::db_upsert_task_tag,
            db::queries_extra::db_delete_task_tag,
            // Non-db/ service queries
            db::queries_extra::db_get_snoozed_threads_due,
            db::queries_extra::db_record_unsubscribe_action,
            db::queries_extra::db_get_subscriptions,
            db::queries_extra::db_get_unsubscribe_status,
            db::queries_extra::db_get_imap_uids_for_messages,
            db::queries_extra::db_find_special_folder,
            db::queries_extra::db_update_message_imap_folder,
            db::queries_extra::db_update_attachment_cached,
            db::queries_extra::db_get_attachment_cache_size,
            db::queries_extra::db_get_oldest_cached_attachments,
            db::queries_extra::db_clear_attachment_cache_entry,
            db::queries_extra::db_clear_all_attachment_cache,
            db::queries_extra::db_count_cached_by_hash,
            db::queries_extra::db_get_inbox_threads_for_backfill,
            db::queries_extra::db_update_scheduled_email_attachments,
            db::queries_extra::db_query_raw_select,
            // Body store (Phase 2 — compressed body storage)
            body_store::commands::body_store_put,
            body_store::commands::body_store_put_batch,
            body_store::commands::body_store_get,
            body_store::commands::body_store_get_batch,
            body_store::commands::body_store_delete,
            body_store::commands::body_store_stats,
            body_store::commands::body_store_migrate,
            // Inline image store (content-addressed blob store)
            inline_image_store::commands::inline_image_get,
            inline_image_store::commands::inline_image_stats,
            inline_image_store::commands::inline_image_clear,
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
            db::pending_ops::db_pending_ops_recover_executing,
            // Filter engine (Phase 6)
            filters::commands::filters_evaluate,
            filters::commands::filters_apply_to_new_message_ids,
            smart_labels::commands::smart_labels_apply_criteria_to_new_message_ids,
            // JWZ threading (Phase 6)
            threading::commands::threading_build_threads,
            threading::commands::threading_update_threads,
            // Categorization rule engine (Phase 6)
            categorization::commands::categorize_thread_by_rules,
            categorization::commands::categorize_threads_by_rules,
            // IMAP sync engine (Phase 4)
            sync::commands::sync_run_accounts,
            sync::commands::sync_start_background,
            sync::commands::sync_stop_background,
            sync::commands::sync_imap_initial,
            sync::commands::sync_imap_delta,
            sync::commands::sync_prepare_full_sync,
            sync::commands::sync_prepare_account_resync,
            // Gmail API provider (Rust)
            gmail::commands::gmail_init_client,
            gmail::commands::gmail_get_access_token,
            gmail::commands::gmail_force_refresh_token,
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
            jmap::commands::jmap_get_profile,
            jmap::commands::jmap_sync_initial,
            jmap::commands::jmap_sync_delta,
            jmap::commands::jmap_list_folders,
            jmap::commands::jmap_create_folder,
            jmap::commands::jmap_rename_folder,
            jmap::commands::jmap_delete_folder,
            jmap::commands::jmap_fetch_attachment,
            // Graph provider commands (Phase 3b)
            graph::commands::graph_init_client,
            graph::commands::graph_remove_client,
            graph::commands::graph_test_connection,
            graph::commands::graph_get_profile,
            // Provider-agnostic commands (Phase 3a)
            provider::commands::provider_sync_initial,
            provider::commands::provider_sync_delta,
            provider::commands::provider_sync_auto,
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
            provider::commands::provider_fetch_message,
            provider::commands::provider_fetch_raw_message,
            provider::commands::provider_test_connection,
            provider::commands::provider_get_profile,
            provider::commands::provider_list_folders,
            provider::commands::provider_create_folder,
            provider::commands::provider_rename_folder,
            provider::commands::provider_delete_folder,
            // Discovery
            discovery::commands::discover_email_config,
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
                let app_data_dir = app
                    .path()
                    .app_data_dir()
                    .map_err(|e| Box::new(std::io::Error::other(format!("app data dir: {e}"))))?;
                let db_state = db::DbState::init(&app_data_dir)
                    .map_err(|e| Box::new(std::io::Error::other(format!("db init: {e}"))))?;
                app.manage(db_state);

                let body_store_state =
                    body_store::BodyStoreState::init(&app_data_dir).map_err(|e| {
                        Box::new(std::io::Error::other(format!("body store init: {e}")))
                    })?;
                app.manage(body_store_state);

                let inline_image_store_state = inline_image_store::InlineImageStoreState::init(
                    &app_data_dir,
                )
                .map_err(|e| {
                    Box::new(std::io::Error::other(format!(
                        "inline image store init: {e}"
                    )))
                })?;
                app.manage(inline_image_store_state);

                let search_state = search::SearchState::init(&app_data_dir)
                    .map_err(|e| Box::new(std::io::Error::other(format!("search init: {e}"))))?;
                app.manage(search_state);

                app.manage(sync::SyncState::new());
                app.manage(sync::SyncQueueState::new());
                app.manage(sync::BackgroundSyncState::new());

                // Gmail provider state — load encryption key for token decryption
                let encryption_key = provider::crypto::load_encryption_key(&app_data_dir)
                    .unwrap_or_else(|e| {
                        log::warn!(
                            "Gmail provider: no encryption key ({e}), will init on first use"
                        );
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
                    let mut tray =
                        match TrayItem::new("Ratatoskr", IconSource::Resource("mail-read")) {
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
