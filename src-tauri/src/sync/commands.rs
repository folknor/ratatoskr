#![allow(clippy::let_underscore_must_use)]

use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use ratatoskr_core::calendar::sync::calendar_sync_account_impl;
use crate::categorization::commands::categorize_threads_with_ai_impl;
use crate::db::DbState;
use crate::filters::FilterableMessage;
use ratatoskr_core::sync::notifications::{
    evaluate_notifications, get_ai_categorization_candidates,
};
use crate::filters::commands::filters_apply_to_messages_impl;
use crate::filters::commands::load_enabled_filters;
use crate::filters::commands::load_filterable_messages;
use crate::inline_image_store::InlineImageStoreState;
use crate::progress::{self, TauriProgressReporter};
use crate::provider::commands::provider_sync_auto_for_provider;
use crate::provider::crypto::AppCryptoState;
use crate::search::SearchState;
use crate::smart_labels::commands::load_enabled_criteria_rules;
use crate::smart_labels::commands::load_enabled_rules_for_ai;
use crate::smart_labels::commands::smart_labels_apply_criteria_to_messages_impl;
use crate::smart_labels::commands::smart_labels_classify_and_apply_remainder_impl;
use crate::smart_labels::commands::smart_labels_prepare_ai_remainder_for_messages;
use crate::state::AppState;

use super::config;
use super::types::{
    ImapSyncResult, SyncNotificationsEvent, SyncStatus,
    SyncStatusDonePayload, SyncStatusEvent,
};
use super::{SyncQueueState, SyncState};

const SYNC_INTERVAL_MS: u64 = 60_000;

#[tauri::command]
pub async fn sync_run_accounts(
    app_state: State<'_, AppState>,
    queue: State<'_, SyncQueueState>,
    account_ids: Vec<String>,
) -> Result<(), String> {
    queue_sync_accounts(&app_state, &queue, &account_ids).await
}

#[tauri::command]
pub async fn sync_start_background(
    app_state: State<'_, AppState>,
    queue: State<'_, SyncQueueState>,
    background: State<'_, super::BackgroundSyncState>,
    account_ids: Vec<String>,
    skip_immediate_sync: Option<bool>,
) -> Result<(), String> {
    let app_state = app_state.inner().clone();
    let queue = queue.inner().clone();
    let ids = account_ids;
    let skip_immediate = skip_immediate_sync.unwrap_or(false);

    let task = tokio::spawn(async move {
        if !skip_immediate {
            let account_ids = load_background_account_ids(&app_state.db, &ids).await;
            let _ = queue_sync_accounts(&app_state, &queue, &account_ids).await;
        }

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(SYNC_INTERVAL_MS)).await;
            let account_ids = load_background_account_ids(&app_state.db, &ids).await;
            let _ = queue_sync_accounts(&app_state, &queue, &account_ids).await;
        }
    });

    background.replace(task);
    Ok(())
}

#[tauri::command]
pub async fn sync_stop_background(
    background: State<'_, super::BackgroundSyncState>,
) -> Result<(), String> {
    background.stop();
    Ok(())
}

async fn run_sync_queue(app_state: &AppState, queue: &SyncQueueState) {
    loop {
        let account_ids = queue.take_pending_batch();
        if account_ids.is_empty() {
            if let Some(waiters) = queue.finish_if_idle() {
                for waiter in waiters {
                    let _ = waiter.send(());
                }
                return;
            }
            // Items were enqueued between take_pending_batch and finish_if_idle.
            // Yield to avoid a tight spin while we wait for the next batch.
            tokio::task::yield_now().await;
            continue;
        }

        for account_id in account_ids {
            run_sync_account(app_state, &account_id).await;
        }
    }
}

async fn queue_sync_accounts(
    app_state: &AppState,
    queue: &SyncQueueState,
    account_ids: &[String],
) -> Result<(), String> {
    if account_ids.is_empty() {
        return Ok(());
    }

    let (should_spawn, rx) = queue.enqueue(account_ids);
    if should_spawn {
        let app_state = app_state.clone();
        let queue = queue.clone();
        tokio::spawn(async move {
            run_sync_queue(&app_state, &queue).await;
        });
    }

    rx.await
        .map_err(|_| "Sync queue worker stopped unexpectedly".to_string())
}

async fn load_background_account_ids(db: &DbState, fallback_ids: &[String]) -> Vec<String> {
    match db.with_conn(config::get_active_account_ids).await {
        Ok(account_ids) => account_ids,
        Err(error) => {
            log::warn!("Failed to refresh background sync account list: {error}");
            fallback_ids.to_vec()
        }
    }
}

async fn run_sync_account(app_state: &AppState, account_id: &str) {
    let db = &app_state.db;
    let body_store = &app_state.body_store;
    let inline_images = &app_state.inline_images;
    let search = &app_state.search;
    let providers = &app_state.providers;

    let sync_config = match db
        .with_conn({
            let account_id = account_id.to_string();
            move |conn| config::get_auto_sync_config(conn, &account_id)
        })
        .await
    {
        Ok(sync_config) => sync_config,
        Err(error) => {
            emit_sync_status(
                app_state.progress.as_ref(),
                SyncStatusEvent {
                    account_id: account_id.to_string(),
                    provider: "unknown".to_string(),
                    status: SyncStatus::Error,
                    error: Some(error),
                    result: None,
                },
            );
            return;
        }
    };
    let provider = sync_config.provider.clone();

    emit_sync_status(
        app_state.progress.as_ref(),
        SyncStatusEvent {
            account_id: account_id.to_string(),
            provider: provider.clone(),
            status: SyncStatus::Syncing,
            error: None,
            result: None,
        },
    );

    if provider == "caldav" {
        match calendar_sync_account_impl(account_id, db, providers.gmail.as_ref()).await {
            Ok(()) => emit_sync_status(
                app_state.progress.as_ref(),
                SyncStatusEvent {
                    account_id: account_id.to_string(),
                    provider,
                    status: SyncStatus::Done,
                    error: None,
                    result: Some(SyncStatusDonePayload {
                        new_inbox_message_ids: Vec::new(),
                        affected_thread_ids: Vec::new(),
                        criteria_smart_label_matches: Vec::new(),
                    }),
                },
            ),
            Err(error) => emit_sync_status(
                app_state.progress.as_ref(),
                SyncStatusEvent {
                    account_id: account_id.to_string(),
                    provider,
                    status: SyncStatus::Error,
                    error: Some(error),
                    result: None,
                },
            ),
        }
        return;
    }

    match provider_sync_auto_for_provider(
        account_id,
        &sync_config.provider,
        sync_config.initial_sync_completed,
        sync_config.sync_period_days,
        db,
        providers,
        body_store,
        inline_images,
        search,
        app_state.progress.as_ref(),
    )
    .await
    {
        Ok(result) => {
            let should_sync_calendar = match db
                .with_conn({
                    let account_id = account_id.to_string();
                    move |conn| {
                        let account = config::get_account(conn, &account_id)?;
                        Ok(config::should_sync_calendar(&account))
                    }
                })
                .await
            {
                Ok(value) => value,
                Err(error) => {
                    log::warn!("Failed to determine calendar follow-up for {account_id}: {error}");
                    false
                }
            };

            let filters = match load_enabled_filters(&db, account_id).await {
                Ok(filters) => filters,
                Err(error) => {
                    log::warn!("Failed to load filters for {account_id}: {error}");
                    Vec::new()
                }
            };
            let criteria_rules = match load_enabled_criteria_rules(&db, account_id).await {
                Ok(rules) => rules,
                Err(error) => {
                    log::warn!(
                        "Failed to load smart label criteria rules for {account_id}: {error}"
                    );
                    Vec::new()
                }
            };
            let loaded_messages = match load_post_sync_messages(
                &db,
                &body_store,
                account_id,
                &result.new_inbox_message_ids,
                &filters,
                &criteria_rules,
            )
            .await
            {
                Ok(messages) => messages,
                Err(error) => {
                    log::warn!("Failed to load post-sync messages for {account_id}: {error}");
                    Vec::new()
                }
            };

            if let Err(error) = filters_apply_to_messages_impl(
                account_id,
                &provider,
                &filters,
                &loaded_messages,
                app_state,
                app_state.progress.as_ref(),
            )
            .await
            {
                log::warn!("Failed to run post-sync filters for {account_id}: {error}");
            }

            let criteria_smart_label_matches = match smart_labels_apply_criteria_to_messages_impl(
                account_id,
                &provider,
                &criteria_rules,
                &loaded_messages,
                app_state,
                app_state.progress.as_ref(),
            )
            .await
            {
                Ok(matches) => matches,
                Err(error) => {
                    log::warn!(
                        "Failed to run smart label criteria matching for {account_id}: {error}"
                    );
                    Vec::new()
                }
            };

            let notifications_to_queue = match evaluate_notifications(
                &db,
                account_id,
                &loaded_messages,
                result.was_delta && !result.fell_back_to_initial,
            )
            .await
            {
                Ok(candidates) => candidates,
                Err(error) => {
                    log::warn!("Failed to evaluate notifications for {account_id}: {error}");
                    Vec::new()
                }
            };

            let (ai_smart_label_threads, ai_smart_label_rules) =
                match smart_labels_prepare_ai_remainder_for_messages(
                    account_id,
                    &loaded_messages,
                    &db,
                    &criteria_smart_label_matches,
                    match load_enabled_rules_for_ai(&db, account_id).await {
                        Ok(rules) => rules,
                        Err(error) => {
                            log::warn!(
                                "Failed to load smart label AI rules for {account_id}: {error}"
                            );
                            Vec::new()
                        }
                    },
                )
                .await
                {
                    Ok(payload) => payload,
                    Err(error) => {
                        log::warn!(
                            "Failed to prepare smart label AI remainder for {account_id}: {error}"
                        );
                        (Vec::new(), Vec::new())
                    }
                };

            if !ai_smart_label_threads.is_empty() && !ai_smart_label_rules.is_empty() {
                let crypto = app_state.crypto.clone();
                let progress = app_state.progress.clone();
                let app_state = app_state.clone();
                let provider_for_ai = provider.clone();
                let account_id_for_ai = account_id.to_string();
                let pre_applied_matches = criteria_smart_label_matches.clone();
                tauri::async_runtime::spawn(async move {
                    if let Err(error) = smart_labels_classify_and_apply_remainder_impl(
                        &account_id_for_ai,
                        &provider_for_ai,
                        &ai_smart_label_threads,
                        &ai_smart_label_rules,
                        &pre_applied_matches,
                        &app_state,
                        &crypto,
                        progress.as_ref(),
                    )
                    .await
                    {
                        log::warn!(
                            "Failed to classify/apply AI smart label remainder for {account_id_for_ai}: {error}"
                        );
                    }
                });
            }

            if !result.affected_thread_ids.is_empty() {
                let db = app_state.db.clone();
                let crypto = app_state.crypto.clone();
                let account_id_for_ai = account_id.to_string();
                tauri::async_runtime::spawn(async move {
                    let candidates = match get_ai_categorization_candidates(&db, &account_id_for_ai)
                        .await
                    {
                        Ok(candidates) => candidates,
                        Err(error) => {
                            log::warn!(
                                "Failed to load AI categorization candidates for {account_id_for_ai}: {error}"
                            );
                            return;
                        }
                    };

                    if let Err(error) = categorize_threads_with_ai_impl(
                        &account_id_for_ai,
                        &candidates,
                        &db,
                        &crypto,
                    )
                    .await
                    {
                        log::warn!(
                            "Failed to categorize AI thread remainder for {account_id_for_ai}: {error}"
                        );
                    }
                });
            }

            if !notifications_to_queue.is_empty() {
                emit_sync_notifications(
                    app_state.progress.as_ref(),
                    SyncNotificationsEvent {
                        account_id: account_id.to_string(),
                        notifications: notifications_to_queue.clone(),
                    },
                );
            }

            if should_sync_calendar
                && let Err(error) =
                    calendar_sync_account_impl(account_id, db, providers.gmail.as_ref()).await
            {
                log::warn!("Failed to run calendar follow-up for {account_id}: {error}");
            }

            emit_sync_status(
                app_state.progress.as_ref(),
                SyncStatusEvent {
                    account_id: account_id.to_string(),
                    provider,
                    status: SyncStatus::Done,
                    error: None,
                    result: Some(SyncStatusDonePayload {
                        new_inbox_message_ids: result.new_inbox_message_ids,
                        affected_thread_ids: result.affected_thread_ids,
                        criteria_smart_label_matches,
                    }),
                },
            )
        }
        Err(error) => emit_sync_status(
            app_state.progress.as_ref(),
            SyncStatusEvent {
                account_id: account_id.to_string(),
                provider,
                status: SyncStatus::Error,
                error: Some(error),
                result: None,
            },
        ),
    }
}


async fn load_post_sync_messages(
    db: &DbState,
    body_store: &BodyStoreState,
    account_id: &str,
    message_ids: &[String],
    filters: &[(
        crate::filters::FilterCriteria,
        crate::filters::FilterActions,
    )],
    criteria_rules: &[(String, crate::filters::FilterCriteria)],
) -> Result<Vec<FilterableMessage>, String> {
    if message_ids.is_empty() {
        return Ok(Vec::new());
    }

    let body_criteria: Vec<crate::filters::FilterCriteria> = filters
        .iter()
        .filter_map(|(criteria, _)| criteria.body.as_ref().map(|_| criteria.clone()))
        .chain(
            criteria_rules
                .iter()
                .filter_map(|(_, criteria)| criteria.body.as_ref().map(|_| criteria.clone())),
        )
        .collect();

    load_filterable_messages(db, body_store, account_id, message_ids, &body_criteria).await
}

fn emit_sync_status(progress: &dyn progress::ProgressReporter, event: SyncStatusEvent) {
    progress::emit_event(progress, "sync-status", &event);
}

fn emit_sync_notifications(
    progress: &dyn progress::ProgressReporter,
    event: SyncNotificationsEvent,
) {
    progress::emit_event(progress, "sync-notifications", &event);
}

/// Run initial IMAP sync for an account.
///
/// Called from TS when an IMAP account has no history_id (first sync).
/// Returns sync result with new message IDs for post-sync hooks (filters, notifications).
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn sync_imap_initial(
    app: AppHandle,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    sync_state: State<'_, SyncState>,
    account_id: String,
    days_back: Option<i64>,
) -> Result<ImapSyncResult, String> {
    // Prevent concurrent syncs for the same account
    if !sync_state.try_lock_account(&account_id) {
        return Err(format!("Sync already in progress for account {account_id}"));
    }

    let result = async {
        // Read account + config from DB
        let actual_days_back = {
            let account_id = account_id.clone();
            db.with_conn(move |conn| {
                let account = config::get_account(conn, &account_id)?;
                if account.provider != "imap" {
                    return Err(format!("Account {account_id} is not an IMAP account"));
                }
                let days = config::get_sync_period_days(conn);
                Ok(days)
            })
            .await?
        };
        let imap_config = crate::imap::account_config::load_imap_config(
            &db,
            &account_id,
            crypto.encryption_key(),
        )
        .await?;

        let days = days_back.unwrap_or(actual_days_back);

        let reporter = TauriProgressReporter::from_ref(&app);
        super::imap_initial::imap_initial_sync(
            &reporter,
            &db,
            &body_store,
            &inline_images,
            &search,
            &account_id,
            &imap_config,
            days,
        )
        .await
    }
    .await;

    sync_state.unlock_account(&account_id);
    result
}

/// Run delta IMAP sync for an account.
///
/// Called from TS when an IMAP account has a history_id (subsequent syncs).
/// Returns sync result with new message IDs for post-sync hooks.
///
/// Includes automatic recovery: if delta finds 0 new messages and the DB has
/// 0 threads (indicating a failed initial sync), clears sync state and falls
/// back to a full initial sync — all within a single invoke, no extra IPC.
#[allow(clippy::too_many_arguments)]
#[tauri::command]
pub async fn sync_imap_delta(
    app: AppHandle,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
    body_store: State<'_, BodyStoreState>,
    inline_images: State<'_, InlineImageStoreState>,
    search: State<'_, SearchState>,
    sync_state: State<'_, SyncState>,
    account_id: String,
    days_back: Option<i64>,
) -> Result<ImapSyncResult, String> {
    if !sync_state.try_lock_account(&account_id) {
        return Err(format!("Sync already in progress for account {account_id}"));
    }

    let result = async {
        let actual_days_back = {
            let account_id = account_id.clone();
            db.with_conn(move |conn| {
                let account = config::get_account(conn, &account_id)?;
                if account.provider != "imap" {
                    return Err(format!("Account {account_id} is not an IMAP account"));
                }
                let days = config::get_sync_period_days(conn);
                Ok(days)
            }).await?
        };
        let imap_config =
            crate::imap::account_config::load_imap_config(&db, &account_id, crypto.encryption_key())
                .await?;

        let days = days_back.unwrap_or(actual_days_back);

        let reporter = TauriProgressReporter::from_ref(&app);
        let result = super::imap_delta::imap_delta_sync(
            &reporter,
            &db,
            &body_store,
            &inline_images,
            &search,
            &account_id,
            &imap_config,
            days,
        ).await?;

        // Recovery: if delta found nothing and DB has no threads AND no folder
        // sync states, the previous initial sync likely failed. An account with
        // 0 threads but existing folder sync states is just a legitimately empty
        // mailbox — don't wipe its state.
        if result.stored_count == 0 {
            let (thread_count, folder_state_count) = {
                let aid = account_id.clone();
                db.with_conn(move |conn| {
                    let tc = super::pipeline::get_thread_count(conn, &aid)?;
                    let fsc = super::pipeline::get_all_folder_sync_states(conn, &aid)?;
                    Ok((tc, fsc.len()))
                }).await?
            };

            if thread_count == 0 && folder_state_count == 0 {
                log::warn!(
                    "[sync] Delta found 0 messages, DB has 0 threads for {account_id} — forcing full re-sync"
                );

                // Clear history_id and folder sync states in one DB call
                {
                    let aid = account_id.clone();
                    db.with_conn(move |conn| {
                        super::pipeline::clear_account_history_id(conn, &aid)?;
                        super::pipeline::clear_all_folder_sync_states(conn, &aid)?;
                        Ok(())
                    }).await?;
                }

                return super::imap_initial::imap_initial_sync(
                    &reporter,
                    &db,
                    &body_store,
                    &inline_images,
                    &search,
                    &account_id,
                    &imap_config,
                    days,
                ).await;
            }
        }

        Ok(result)
    }.await;

    sync_state.unlock_account(&account_id);
    result
}
