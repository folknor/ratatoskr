#![allow(clippy::let_underscore_must_use)]

use tauri::{AppHandle, Emitter, Manager, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::filters::commands::filters_apply_to_new_message_ids_impl;
use crate::gmail::client::GmailState;
use crate::graph::client::GraphState;
use crate::inline_image_store::InlineImageStoreState;
use crate::jmap::client::JmapState;
use crate::provider::commands::provider_sync_auto_impl;
use crate::provider::router::get_provider_type;
use crate::search::SearchState;
use crate::smart_labels::commands::smart_labels_apply_criteria_to_new_message_ids_impl;

use super::{SyncQueueState, SyncState};
use super::config;
use super::types::{ImapSyncResult, SyncStatusEvent};

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
    account_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM threads WHERE account_id = ?1",
            rusqlite::params![account_id],
        )
        .map_err(|e| format!("delete threads for account: {e}"))?;
        conn.execute(
            "DELETE FROM messages WHERE account_id = ?1",
            rusqlite::params![account_id],
        )
        .map_err(|e| format!("delete messages for account: {e}"))?;
        super::pipeline::clear_account_history_id(conn, &account_id)?;
        super::pipeline::clear_all_folder_sync_states(conn, &account_id)?;
        Ok(())
    })
    .await
}

#[allow(clippy::too_many_arguments)]
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
            let _ = queue_sync_accounts(&app_handle, &queue, &ids).await;
        }

        loop {
            tokio::time::sleep(std::time::Duration::from_millis(SYNC_INTERVAL_MS)).await;
            let queue: tauri::State<'_, SyncQueueState> = app_handle.state();
            let _ = queue_sync_accounts(&app_handle, &queue, &ids).await;
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

async fn run_sync_account(app: &AppHandle, account_id: &str) {
    let db: tauri::State<'_, DbState> = app.state();
    let gmail: tauri::State<'_, GmailState> = app.state();
    let jmap: tauri::State<'_, JmapState> = app.state();
    let graph: tauri::State<'_, GraphState> = app.state();
    let body_store: tauri::State<'_, BodyStoreState> = app.state();
    let inline_images: tauri::State<'_, InlineImageStoreState> = app.state();
    let search: tauri::State<'_, SearchState> = app.state();

    let provider = match get_provider_type(&db, account_id).await {
        Ok(provider) => provider,
        Err(error) => {
            emit_sync_status(
                app,
                SyncStatusEvent {
                    account_id: account_id.to_string(),
                    provider: "unknown".to_string(),
                    status: "error".to_string(),
                    error: Some(error),
                    new_inbox_message_ids: None,
                    affected_thread_ids: None,
                    is_delta: None,
                    criteria_smart_label_matches: None,
                },
            );
            return;
        }
    };

    emit_sync_status(
        app,
        SyncStatusEvent {
            account_id: account_id.to_string(),
            provider: provider.clone(),
            status: "syncing".to_string(),
            error: None,
            new_inbox_message_ids: None,
            affected_thread_ids: None,
            is_delta: None,
            criteria_smart_label_matches: None,
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
                new_inbox_message_ids: Some(Vec::new()),
                affected_thread_ids: Some(Vec::new()),
                is_delta: Some(false),
                criteria_smart_label_matches: Some(Vec::new()),
            },
        );
        return;
    }

    match provider_sync_auto_impl(
        account_id,
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
            if let Err(error) = filters_apply_to_new_message_ids_impl(
                account_id,
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

            emit_sync_status(
                app,
                SyncStatusEvent {
                    account_id: account_id.to_string(),
                    provider,
                    status: "done".to_string(),
                    error: None,
                    new_inbox_message_ids: Some(result.new_inbox_message_ids),
                    affected_thread_ids: Some(result.affected_thread_ids),
                    is_delta: Some(result.was_delta && !result.fell_back_to_initial),
                    criteria_smart_label_matches: Some(criteria_smart_label_matches),
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
                new_inbox_message_ids: None,
                affected_thread_ids: None,
                is_delta: None,
                criteria_smart_label_matches: None,
            },
        ),
    }
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
