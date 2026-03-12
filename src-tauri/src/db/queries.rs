// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use super::DbState;
use super::types::{
    CategoryCount, DbAttachment, DbContact, DbLabel, DbMessage, DbThread, ThreadCategoryRow,
};
use crate::provider::crypto::AppCryptoState;

pub type SettingsBootstrapSnapshot = ratatoskr_core::db::queries::SettingsBootstrapSnapshot;
pub type SettingsSecretsSnapshot = ratatoskr_core::db::queries::SettingsSecretsSnapshot;
pub type UiBootstrapSnapshot = ratatoskr_core::db::queries::UiBootstrapSnapshot;

// ── Thread queries ───────────────────────────────────────────

#[tauri::command]
pub async fn db_get_threads(
    state: State<'_, DbState>,
    account_id: String,
    label_id: Option<String>,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    state
        .with_conn(move |conn| super::get_threads(conn, account_id, label_id, limit, offset))
        .await
}

#[tauri::command]
pub async fn db_get_threads_for_category(
    state: State<'_, DbState>,
    account_id: String,
    category: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbThread>, String> {
    state
        .with_conn(move |conn| {
            super::get_threads_for_category(conn, account_id, category, limit, offset)
        })
        .await
}

#[tauri::command]
pub async fn db_get_thread_by_id(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Option<DbThread>, String> {
    state
        .with_conn(move |conn| super::get_thread_by_id(conn, account_id, thread_id))
        .await
}

#[tauri::command]
pub async fn db_get_thread_label_ids(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<String>, String> {
    state
        .with_conn(move |conn| super::get_thread_label_ids(conn, account_id, thread_id))
        .await
}

// ── Message queries ──────────────────────────────────────────

#[tauri::command]
pub async fn db_get_messages_for_thread(
    state: State<'_, DbState>,
    body_store: State<'_, crate::body_store::BodyStoreState>,
    account_id: String,
    thread_id: String,
) -> Result<Vec<DbMessage>, String> {
    super::get_messages_for_thread(&state, &body_store, account_id, thread_id).await
}

// ── Label queries ────────────────────────────────────────────

#[tauri::command]
pub async fn db_get_labels(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<DbLabel>, String> {
    state
        .with_conn(move |conn| super::get_labels(conn, account_id))
        .await
}

// ── Settings queries ─────────────────────────────────────────

#[tauri::command]
pub async fn db_get_setting(
    state: State<'_, DbState>,
    key: String,
) -> Result<Option<String>, String> {
    state
        .with_conn(move |conn| super::get_setting(conn, key))
        .await
}

#[tauri::command]
pub async fn db_get_secure_setting(
    state: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
    key: String,
) -> Result<Option<String>, String> {
    let encryption_key = *crypto.encryption_key();
    state
        .with_conn(move |conn| super::get_secure_setting(conn, &encryption_key, key))
        .await
}

#[tauri::command]
pub async fn settings_get_bootstrap_snapshot(
    state: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<SettingsBootstrapSnapshot, String> {
    let encryption_key = *crypto.encryption_key();
    state
        .with_conn(move |conn| super::get_settings_bootstrap_snapshot(conn, &encryption_key))
        .await
}

#[tauri::command]
pub async fn settings_get_secrets_snapshot(
    state: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<SettingsSecretsSnapshot, String> {
    let encryption_key = *crypto.encryption_key();
    state
        .with_conn(move |conn| super::get_settings_secrets_snapshot(conn, &encryption_key))
        .await
}

#[tauri::command]
pub async fn settings_get_ui_bootstrap_snapshot(
    state: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<UiBootstrapSnapshot, String> {
    let encryption_key = *crypto.encryption_key();
    state
        .with_conn(move |conn| super::get_ui_bootstrap_snapshot(conn, &encryption_key))
        .await
}

#[tauri::command]
pub async fn db_set_setting(
    state: State<'_, DbState>,
    key: String,
    value: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::set_setting(conn, key, value))
        .await
}

// ── Thread category queries ──────────────────────────────────

#[tauri::command]
pub async fn db_get_category_unread_counts(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<Vec<CategoryCount>, String> {
    state
        .with_conn(move |conn| super::get_category_unread_counts(conn, account_id))
        .await
}

#[tauri::command]
pub async fn db_get_categories_for_threads(
    state: State<'_, DbState>,
    account_id: String,
    thread_ids: Vec<String>,
) -> Result<Vec<ThreadCategoryRow>, String> {
    if thread_ids.is_empty() {
        return Ok(Vec::new());
    }

    state
        .with_conn(move |conn| super::get_categories_for_threads(conn, account_id, thread_ids))
        .await
}

// ── Thread mutations ─────────────────────────────────────────

#[tauri::command]
pub async fn db_set_thread_read(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_read: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::set_thread_read(conn, account_id, thread_id, is_read))
        .await
}

#[tauri::command]
pub async fn db_set_thread_starred(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_starred: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::set_thread_starred(conn, account_id, thread_id, is_starred))
        .await
}

#[tauri::command]
pub async fn db_set_thread_pinned(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_pinned: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::set_thread_pinned(conn, account_id, thread_id, is_pinned))
        .await
}

#[tauri::command]
pub async fn db_set_thread_muted(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    is_muted: bool,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::set_thread_muted(conn, account_id, thread_id, is_muted))
        .await
}

#[tauri::command]
pub async fn db_delete_thread(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::delete_thread(conn, account_id, thread_id))
        .await
}

// ── Thread label mutations ───────────────────────────────────

#[tauri::command]
pub async fn db_add_thread_label(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::add_thread_label(conn, account_id, thread_id, label_id))
        .await
}

#[tauri::command]
pub async fn db_remove_thread_label(
    state: State<'_, DbState>,
    account_id: String,
    thread_id: String,
    label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::remove_thread_label(conn, account_id, thread_id, label_id))
        .await
}

// ── Label mutations ──────────────────────────────────────────

#[tauri::command]
pub async fn db_upsert_label(state: State<'_, DbState>, label: DbLabel) -> Result<(), String> {
    state
        .with_conn(move |conn| super::upsert_label(conn, label))
        .await
}

#[tauri::command]
pub async fn db_delete_label(
    state: State<'_, DbState>,
    account_id: String,
    label_id: String,
) -> Result<(), String> {
    state
        .with_conn(move |conn| super::delete_label(conn, account_id, label_id))
        .await
}

// ── Contact queries ─────────────────────────────────────────

#[tauri::command]
pub async fn db_search_contacts(
    state: State<'_, DbState>,
    query: String,
    limit: i64,
) -> Result<Vec<DbContact>, String> {
    state
        .with_conn(move |conn| super::search_contacts(conn, query, limit))
        .await
}

#[tauri::command]
pub async fn db_get_contact_by_email(
    state: State<'_, DbState>,
    email: String,
) -> Result<Option<DbContact>, String> {
    state
        .with_conn(move |conn| super::get_contact_by_email(conn, email))
        .await
}

// ── Attachment queries ──────────────────────────────────────

#[tauri::command]
pub async fn db_get_attachments_for_message(
    state: State<'_, DbState>,
    account_id: String,
    message_id: String,
) -> Result<Vec<DbAttachment>, String> {
    state
        .with_conn(move |conn| super::get_attachments_for_message(conn, account_id, message_id))
        .await
}

// ── Count queries ───────────────────────────────────────────

#[tauri::command]
pub async fn db_get_thread_count(
    state: State<'_, DbState>,
    account_id: String,
    label_id: Option<String>,
) -> Result<i64, String> {
    state
        .with_conn(move |conn| super::get_thread_count(conn, account_id, label_id))
        .await
}

#[tauri::command]
pub async fn db_get_unread_count(
    state: State<'_, DbState>,
    account_id: String,
) -> Result<i64, String> {
    state
        .with_conn(move |conn| super::get_unread_count(conn, account_id))
        .await
}
