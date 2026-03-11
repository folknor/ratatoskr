#![allow(clippy::let_underscore_must_use)]

use tauri::{AppHandle, Emitter, Manager, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::db::queries::row_to_message;
use crate::db::queries_extra::load_recent_rule_categorized_threads;
use crate::filters::commands::filters_apply_to_new_message_ids_impl;
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::provider::commands::provider_sync_auto_for_provider;
use crate::search::SearchState;
use crate::smart_labels::commands::smart_labels_apply_criteria_to_new_message_ids_impl;
use crate::smart_labels::commands::smart_labels_prepare_ai_remainder_impl;

use super::{SyncQueueState, SyncState};
use super::config;
use super::types::{
    AICategorizationCandidate, ImapSyncResult, NotificationCandidate, SyncStatusEvent,
};

const SYNC_INTERVAL_MS: u64 = 60_000;

#[tauri::command]
pub async fn sync_prepare_full_sync(
    db: State<'_, DbState>,
    account_ids: Vec<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        for account_id in account_ids {
            super::pipeline::clear_account_history_id(conn, &account_id)?;
        }
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn sync_prepare_account_resync(
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    account_id: String,
) -> Result<(), String> {
    let message_ids = db
        .with_conn({
            let account_id = account_id.clone();
            move |conn| {
                let mut stmt = conn
                    .prepare("SELECT id FROM messages WHERE account_id = ?1")
                    .map_err(|e| format!("prepare resync message query: {e}"))?;
                stmt.query_map(rusqlite::params![account_id], |row| row.get::<_, String>(0))
                    .map_err(|e| format!("query resync message ids: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect resync message ids: {e}"))
            }
        })
        .await?;

    body_store.delete(message_ids).await?;

    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin resync transaction: {e}"))?;
        tx.execute(
            "DELETE FROM threads WHERE account_id = ?1",
            rusqlite::params![account_id],
        )
        .map_err(|e| format!("delete threads for account: {e}"))?;
        super::pipeline::clear_account_history_id(&tx, &account_id)?;
        super::pipeline::clear_all_folder_sync_states(&tx, &account_id)?;
        tx.commit()
            .map_err(|e| format!("commit resync transaction: {e}"))?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn sync_run_accounts(
    app: AppHandle,
    queue: State<'_, SyncQueueState>,
    account_ids: Vec<String>,
) -> Result<(), String> {
    queue_sync_accounts(&app, &queue, &account_ids).await
}

#[tauri::command]
pub async fn sync_start_background(
    app: AppHandle,
    background: State<'_, super::BackgroundSyncState>,
    account_ids: Vec<String>,
    skip_immediate_sync: Option<bool>,
) -> Result<(), String> {
    let app_handle = app.clone();
    let ids = account_ids;
    let skip_immediate = skip_immediate_sync.unwrap_or(false);

    let task = tokio::spawn(async move {
        if !skip_immediate {
            let queue: tauri::State<'_, SyncQueueState> = app_handle.state();
            let account_ids = load_background_account_ids(&app_handle, &ids).await;
            let _ = queue_sync_accounts(&app_handle, &queue, &account_ids).await;
        }

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(SYNC_INTERVAL_MS)).await;
            let queue: tauri::State<'_, SyncQueueState> = app_handle.state();
            let account_ids = load_background_account_ids(&app_handle, &ids).await;
            let _ = queue_sync_accounts(&app_handle, &queue, &account_ids).await;
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

async fn run_sync_queue(app: AppHandle) {
    loop {
        let queue: tauri::State<'_, SyncQueueState> = app.state();
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
            run_sync_account(&app, &account_id).await;
        }
    }
}

async fn queue_sync_accounts(
    app: &AppHandle,
    queue: &SyncQueueState,
    account_ids: &[String],
) -> Result<(), String> {
    if account_ids.is_empty() {
        return Ok(());
    }

    let (should_spawn, rx) = queue.enqueue(account_ids);
    if should_spawn {
        let app_handle = app.clone();
        tokio::spawn(async move {
            run_sync_queue(app_handle).await;
        });
    }

    rx.await
        .map_err(|_| "Sync queue worker stopped unexpectedly".to_string())
}

async fn load_background_account_ids(
    app: &AppHandle,
    fallback_ids: &[String],
) -> Vec<String> {
    let db: tauri::State<'_, DbState> = app.state();
    match db.with_conn(config::get_active_account_ids).await {
        Ok(account_ids) => account_ids,
        Err(error) => {
            log::warn!("Failed to refresh background sync account list: {error}");
            fallback_ids.to_vec()
        }
    }
}

async fn run_sync_account(app: &AppHandle, account_id: &str) {
    let db: tauri::State<'_, DbState> = app.state();
    let gmail: tauri::State<'_, GmailState> = app.state();
    let jmap: tauri::State<'_, JmapState> = app.state();
    let graph: tauri::State<'_, GraphState> = app.state();
    let body_store: tauri::State<'_, BodyStoreState> = app.state();
    let inline_images: tauri::State<'_, InlineImageStoreState> = app.state();
    let search: tauri::State<'_, SearchState> = app.state();

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
                app,
                SyncStatusEvent {
                    account_id: account_id.to_string(),
                    provider: "unknown".to_string(),
                    status: "error".to_string(),
                    error: Some(error),
                    should_sync_calendar: None,
                    new_inbox_message_ids: None,
                    affected_thread_ids: None,
                    criteria_smart_label_matches: None,
                    notifications_to_queue: None,
                    ai_categorization_candidates: None,
                    ai_smart_label_threads: None,
                    ai_smart_label_rules: None,
                },
            );
            return;
        }
    };
    let provider = sync_config.provider.clone();

    emit_sync_status(
        app,
        SyncStatusEvent {
            account_id: account_id.to_string(),
            provider: provider.clone(),
            status: "syncing".to_string(),
            error: None,
            should_sync_calendar: None,
            new_inbox_message_ids: None,
            affected_thread_ids: None,
            criteria_smart_label_matches: None,
            notifications_to_queue: None,
            ai_categorization_candidates: None,
            ai_smart_label_threads: None,
            ai_smart_label_rules: None,
        },
    );

    if provider == "caldav" {
        emit_sync_status(
            app,
            SyncStatusEvent {
                account_id: account_id.to_string(),
                provider,
                status: "done".to_string(),
                error: None,
                should_sync_calendar: Some(true),
                new_inbox_message_ids: Some(Vec::new()),
                affected_thread_ids: Some(Vec::new()),
                criteria_smart_label_matches: Some(Vec::new()),
                notifications_to_queue: Some(Vec::new()),
                ai_categorization_candidates: Some(Vec::new()),
                ai_smart_label_threads: Some(Vec::new()),
                ai_smart_label_rules: Some(Vec::new()),
            },
        );
        return;
    }

    match provider_sync_auto_for_provider(
        account_id,
        &sync_config.provider,
        sync_config.has_history,
        sync_config.sync_period_days,
        &db,
        &gmail,
        &jmap,
        &graph,
        &body_store,
        &inline_images,
        &search,
        app,
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
                    log::warn!(
                        "Failed to determine calendar follow-up for {account_id}: {error}"
                    );
                    false
                }
            };

            if let Err(error) = filters_apply_to_new_message_ids_impl(
                account_id,
                &provider,
                &result.new_inbox_message_ids,
                &db,
                &gmail,
                &jmap,
                &graph,
                &body_store,
                &inline_images,
                &search,
                app,
            )
            .await
            {
                log::warn!("Failed to run post-sync filters for {account_id}: {error}");
            }

            let criteria_smart_label_matches =
                match smart_labels_apply_criteria_to_new_message_ids_impl(
                    account_id,
                    &provider,
                    &result.new_inbox_message_ids,
                    &db,
                    &gmail,
                    &jmap,
                    &graph,
                    &body_store,
                    &inline_images,
                    &search,
                    app,
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
                &result.new_inbox_message_ids,
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
                match smart_labels_prepare_ai_remainder_impl(
                    account_id,
                    &result.new_inbox_message_ids,
                    &db,
                    &body_store,
                    &criteria_smart_label_matches,
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

            let ai_categorization_candidates = if result.affected_thread_ids.is_empty() {
                Vec::new()
            } else {
                match get_ai_categorization_candidates(&db, account_id).await {
                    Ok(candidates) => candidates,
                    Err(error) => {
                        log::warn!(
                            "Failed to load AI categorization candidates for {account_id}: {error}"
                        );
                        Vec::new()
                    }
                }
            };

            emit_sync_status(
                app,
                SyncStatusEvent {
                    account_id: account_id.to_string(),
                    provider,
                    status: "done".to_string(),
                    error: None,
                    should_sync_calendar: Some(should_sync_calendar),
                    new_inbox_message_ids: Some(result.new_inbox_message_ids),
                    affected_thread_ids: Some(result.affected_thread_ids),
                    criteria_smart_label_matches: Some(criteria_smart_label_matches),
                    notifications_to_queue: Some(notifications_to_queue),
                    ai_categorization_candidates: Some(ai_categorization_candidates),
                    ai_smart_label_threads: Some(ai_smart_label_threads),
                    ai_smart_label_rules: Some(ai_smart_label_rules),
                },
            )
        }
        Err(error) => emit_sync_status(
            app,
            SyncStatusEvent {
                account_id: account_id.to_string(),
                provider,
                status: "error".to_string(),
                error: Some(error),
                should_sync_calendar: None,
                new_inbox_message_ids: None,
                affected_thread_ids: None,
                criteria_smart_label_matches: None,
                notifications_to_queue: None,
                ai_categorization_candidates: None,
                ai_smart_label_threads: None,
                ai_smart_label_rules: None,
            },
        ),
    }
}

async fn get_ai_categorization_candidates(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<AICategorizationCandidate>, String> {
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let auto_categorize = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'ai_auto_categorize'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok();
        if auto_categorize.as_deref() == Some("false") {
            return Ok(Vec::new());
        }

        load_recent_rule_categorized_threads(conn, &account_id, 20).map(|threads| {
            threads
                .into_iter()
                .map(|thread| AICategorizationCandidate {
                    id: thread.id,
                    subject: thread.subject,
                    snippet: thread.snippet,
                    from_address: thread.from_address,
                })
                .collect()
        })
    })
    .await
}

async fn evaluate_notifications(
    db: &DbState,
    account_id: &str,
    message_ids: &[String],
    is_delta: bool,
) -> Result<Vec<NotificationCandidate>, String> {
    if !is_delta || message_ids.is_empty() {
        return Ok(Vec::new());
    }

    let account_id = account_id.to_string();
    let message_ids_vec = message_ids.to_vec();
    db.with_conn(move |conn| {
        let notifications_enabled: Option<String> = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'notifications_enabled'",
                [],
                |row| row.get(0),
            )
            .ok();
        if notifications_enabled.as_deref() == Some("false") {
            return Ok(Vec::new());
        }

        let smart_notifications = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'smart_notifications'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .unwrap_or_else(|| "true".to_string())
            == "true";
        let notify_categories = conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'notify_categories'",
                [],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .unwrap_or_else(|| "Primary".to_string());
        let allowed_categories: std::collections::HashSet<String> = notify_categories
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let vip_senders: std::collections::HashSet<String> = {
            let mut stmt = conn
                .prepare("SELECT email_address FROM notification_vips WHERE account_id = ?1")
                .map_err(|e| e.to_string())?;
            stmt.query_map(rusqlite::params![account_id], |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
                .into_iter()
                .map(|email| email.trim().to_lowercase())
                .collect()
        };

        let mut messages = Vec::new();
        for chunk in message_ids_vec.chunks(500) {
            let placeholders: String = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql =
                format!("SELECT * FROM messages WHERE account_id = ?1 AND id IN ({placeholders})");
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(account_id.clone()));
            for id in chunk {
                param_values.push(Box::new(id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), row_to_message)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            messages.extend(rows);
        }

        let thread_ids: Vec<String> = messages.iter().map(|msg| msg.thread_id.clone()).collect();

        // Only check muted status for the relevant thread IDs, not every muted thread in the account.
        let muted_thread_ids: std::collections::HashSet<String> = if thread_ids.is_empty() {
            std::collections::HashSet::new()
        } else {
            let placeholders: String = thread_ids
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT id FROM threads WHERE account_id = ?1 AND is_muted = 1 AND id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
                vec![Box::new(account_id.clone())];
            for id in &thread_ids {
                param_values.push(Box::new(id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?
                .into_iter()
                .collect()
        };

        let mut category_by_thread: std::collections::HashMap<String, String> =
            std::collections::HashMap::new();
        for chunk in thread_ids.chunks(100) {
            if chunk.is_empty() {
                continue;
            }
            let placeholders: String = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| format!("?{}", i + 2))
                .collect::<Vec<_>>()
                .join(", ");
            let sql = format!(
                "SELECT thread_id, category FROM thread_categories WHERE account_id = ?1 AND thread_id IN ({placeholders})"
            );
            let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            param_values.push(Box::new(account_id.clone()));
            for id in chunk {
                param_values.push(Box::new(id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(AsRef::as_ref).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            category_by_thread.extend(rows);
        }

        let mut candidates = Vec::new();
        for msg in messages {
            if muted_thread_ids.contains(&msg.thread_id) {
                continue;
            }
            let from_address_normalized = msg
                .from_address
                .as_ref()
                .map(|email| email.trim().to_lowercase());
            let should_notify = if !smart_notifications {
                true
            } else if let Some(from_address) = from_address_normalized.as_ref() {
                if vip_senders.contains(from_address) {
                    true
                } else {
                    let category = category_by_thread
                        .get(&msg.thread_id)
                        .map(String::as_str)
                        .unwrap_or("Primary");
                    allowed_categories.contains(category)
                }
            } else {
                let category = category_by_thread
                    .get(&msg.thread_id)
                    .map(String::as_str)
                    .unwrap_or("Primary");
                allowed_categories.contains(category)
            };

            if should_notify {
                candidates.push(NotificationCandidate {
                    thread_id: msg.thread_id,
                    from_name: msg.from_name,
                    from_address: msg.from_address,
                    subject: msg.subject,
                });
            }
        }

        Ok(candidates)
    })
    .await
}

fn emit_sync_status(app: &AppHandle, event: SyncStatusEvent) {
    if let Err(error) = app.emit("sync-status", &event) {
        log::warn!("Failed to emit sync-status event: {error}");
    }
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
    gmail: State<'_, GmailState>,
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
        let imap_config =
            crate::imap::account_config::load_imap_config(&db, &account_id, gmail.encryption_key())
                .await?;

        let days = days_back.unwrap_or(actual_days_back);

        super::imap_initial::imap_initial_sync(
            &app,
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
    gmail: State<'_, GmailState>,
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
            crate::imap::account_config::load_imap_config(&db, &account_id, gmail.encryption_key())
                .await?;

        let days = days_back.unwrap_or(actual_days_back);

        let result = super::imap_delta::imap_delta_sync(
            &app,
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
                    &app,
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
