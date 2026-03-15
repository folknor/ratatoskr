#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{
    DbAccount, DbAllowlistEntry, DbNotificationVip, DbPhishingAllowlistEntry,
    DbWritingStyleProfile, DbFolderSyncState,
};

db_command! {
    fn db_get_all_accounts(state) -> Vec<DbAccount>;
    fn db_get_account(state, id: String) -> Option<DbAccount>;
    fn db_get_account_by_email(state, email: String) -> Option<DbAccount>;
    fn db_insert_account(
        state,
        id: String,
        email: String,
        display_name: Option<String>,
        avatar_url: Option<String>,
        access_token: Option<String>,
        refresh_token: Option<String>,
        token_expires_at: Option<i64>,
        provider: String,
        auth_method: String,
        imap_host: Option<String>,
        imap_port: Option<i64>,
        imap_security: Option<String>,
        smtp_host: Option<String>,
        smtp_port: Option<i64>,
        smtp_security: Option<String>,
        imap_password: Option<String>,
        oauth_provider: Option<String>,
        oauth_client_id: Option<String>,
        oauth_client_secret: Option<String>,
        imap_username: Option<String>,
        accept_invalid_certs: Option<i64>,
        caldav_url: Option<String>,
        caldav_username: Option<String>,
        caldav_password: Option<String>,
        caldav_principal_url: Option<String>,
        caldav_home_url: Option<String>,
        calendar_provider: Option<String>,
        jmap_url: Option<String>
    ) -> ();
    fn db_update_account_tokens(state, id: String, access_token: String, token_expires_at: i64) -> ();
    fn db_update_account_all_tokens(state, id: String, access_token: String, refresh_token: String, token_expires_at: i64) -> ();
    fn db_update_account_sync_state(state, id: String, history_id: String) -> ();
    fn db_clear_account_history_id(state, id: String) -> ();
    fn db_update_account_caldav(
        state,
        id: String,
        caldav_url: String,
        caldav_username: String,
        caldav_password: String,
        caldav_principal_url: Option<String>,
        caldav_home_url: Option<String>,
        calendar_provider: String
    ) -> ();
}

// db_delete_account has custom logic (body store + inline image cleanup), so hand-written.
#[tauri::command]
pub async fn db_delete_account(
    state: tauri::State<'_, crate::db::DbState>,
    body_store: tauri::State<'_, crate::body_store::BodyStoreState>,
    inline_images: tauri::State<'_, crate::inline_image_store::InlineImageStoreState>,
    app_state: tauri::State<'_, crate::state::AppState>,
    id: String,
) -> Result<(), String> {
    // Collect message IDs and inline image hashes BEFORE cascade-deleting
    let (message_ids, inline_hashes) = {
        let account_id = id.clone();
        state
            .with_conn(move |conn| {
                let mut stmt = conn
                    .prepare("SELECT id FROM messages WHERE account_id = ?1")
                    .map_err(|e| format!("prepare account message ids: {e}"))?;
                let msg_ids = stmt
                    .query_map(rusqlite::params![&account_id], |row| row.get::<_, String>(0))
                    .map_err(|e| format!("query account message ids: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect account message ids: {e}"))?;
                let hashes =
                    ratatoskr_core::inline_image_store::collect_inline_hashes_for_account(
                        conn,
                        &account_id,
                    )?;
                Ok((msg_ids, hashes))
            })
            .await?
    };

    ratatoskr_core::db::queries_extra::db_delete_account(&state, id).await?;

    // Clean up orphaned bodies and inline images after cascade deletion
    if !message_ids.is_empty() {
        let _ = body_store.delete(message_ids).await;
    }
    if !inline_hashes.is_empty() {
        let _ = inline_images.delete_unreferenced(&state, inline_hashes).await;
    }

    // Evict file-based attachment cache entries now over the limit
    let _ = ratatoskr_core::attachment_cache::enforce_cache_limit(
        &state,
        &app_state.app_data_dir,
    )
    .await;

    Ok(())
}

// ── Allowlists ──────────────────────────────────────────────

db_command! {
    fn db_add_to_allowlist(state, id: String, account_id: String, sender_address: String) -> ();
    fn db_get_allowlisted_senders(state, account_id: String, sender_addresses: Vec<String>) -> Vec<String>;
    fn db_is_allowlisted(state, account_id: String, sender_address: String) -> bool;
    fn db_remove_from_allowlist(state, account_id: String, sender_address: String) -> ();
    fn db_get_allowlist_for_account(state, account_id: String) -> Vec<DbAllowlistEntry>;
}

// ── VIP Senders ─────────────────────────────────────────────

db_command! {
    fn db_add_vip_sender(state, id: String, account_id: String, email: String, display_name: Option<String>) -> ();
    fn db_remove_vip_sender(state, account_id: String, email: String) -> ();
    fn db_is_vip_sender(state, account_id: String, email: String) -> bool;
    fn db_get_vip_senders(state, account_id: String) -> Vec<String>;
    fn db_get_all_vip_senders(state, account_id: String) -> Vec<DbNotificationVip>;
}

// ── Phishing Allowlist ──────────────────────────────────────

db_command! {
    fn db_is_phishing_allowlisted(state, account_id: String, sender_address: String) -> bool;
    fn db_add_to_phishing_allowlist(state, account_id: String, sender_address: String) -> ();
    fn db_remove_from_phishing_allowlist(state, account_id: String, sender_address: String) -> ();
    fn db_get_phishing_allowlist(state, account_id: String) -> Vec<DbPhishingAllowlistEntry>;
}

// ── Writing Style Profiles ──────────────────────────────────

db_command! {
    fn db_get_writing_style_profile(state, account_id: String) -> Option<DbWritingStyleProfile>;
    fn db_upsert_writing_style_profile(state, account_id: String, profile_text: String, sample_count: i64) -> ();
    fn db_delete_writing_style_profile(state, account_id: String) -> ();
}

// ── Folder Sync State ───────────────────────────────────────

db_command! {
    fn db_get_folder_sync_state(state, account_id: String, folder_path: String) -> Option<DbFolderSyncState>;
    fn db_upsert_folder_sync_state(state, account_id: String, folder_path: String, uidvalidity: Option<i64>, last_uid: i64, modseq: Option<i64>, last_sync_at: Option<i64>) -> ();
    fn db_delete_folder_sync_state(state, account_id: String, folder_path: String) -> ();
    fn db_clear_all_folder_sync_states(state, account_id: String) -> ();
    fn db_get_all_folder_sync_states(state, account_id: String) -> Vec<DbFolderSyncState>;
}
