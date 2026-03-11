#![allow(clippy::let_underscore_must_use)]

use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::inline_image_store::InlineImageStoreState;
use crate::search::SearchState;

use super::SyncState;
use super::config;
use super::types::ImapSyncResult;

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
