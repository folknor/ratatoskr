use tauri::{AppHandle, State};

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;

use super::config;
use super::SyncState;
use super::types::ImapSyncResult;

/// Run initial IMAP sync for an account.
///
/// Called from TS when an IMAP account has no history_id (first sync).
/// Returns sync result with new message IDs for post-sync hooks (filters, notifications).
#[tauri::command]
pub async fn sync_imap_initial(
    app: AppHandle,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
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
        let (imap_config, actual_days_back) = {
            let account_id = account_id.clone();
            db.with_conn(move |conn| {
                let account = config::get_account(conn, &account_id)?;
                if account.provider != "imap" {
                    return Err(format!("Account {} is not an IMAP account", account_id));
                }
                let imap_config = config::build_imap_config(&account)?;
                let days = config::get_sync_period_days(conn);
                Ok((imap_config, days))
            }).await?
        };

        let days = days_back.unwrap_or(actual_days_back);

        super::imap_initial::imap_initial_sync(
            &app,
            &db,
            &body_store,
            &search,
            &account_id,
            &imap_config,
            days,
        ).await
    }.await;

    sync_state.unlock_account(&account_id);
    result
}

/// Run delta IMAP sync for an account.
///
/// Called from TS when an IMAP account has a history_id (subsequent syncs).
/// Returns sync result with new message IDs for post-sync hooks.
#[tauri::command]
pub async fn sync_imap_delta(
    app: AppHandle,
    db: State<'_, DbState>,
    body_store: State<'_, BodyStoreState>,
    search: State<'_, SearchState>,
    sync_state: State<'_, SyncState>,
    account_id: String,
    days_back: Option<i64>,
) -> Result<ImapSyncResult, String> {
    if !sync_state.try_lock_account(&account_id) {
        return Err(format!("Sync already in progress for account {account_id}"));
    }

    let result = async {
        let (imap_config, actual_days_back) = {
            let account_id = account_id.clone();
            db.with_conn(move |conn| {
                let account = config::get_account(conn, &account_id)?;
                if account.provider != "imap" {
                    return Err(format!("Account {} is not an IMAP account", account_id));
                }
                let imap_config = config::build_imap_config(&account)?;
                let days = config::get_sync_period_days(conn);
                Ok((imap_config, days))
            }).await?
        };

        let days = days_back.unwrap_or(actual_days_back);

        super::imap_delta::imap_delta_sync(
            &app,
            &db,
            &body_store,
            &search,
            &account_id,
            &imap_config,
            days,
        ).await
    }.await;

    sync_state.unlock_account(&account_id);
    result
}
