#![allow(clippy::let_underscore_must_use)]

use tauri::State;

use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::provider::crypto::AppCryptoState;

use super::caldav;
use super::google;
use super::sync;
use super::types::{
    CalendarEventDto, CalendarInfoDto, CalendarInfoInput, CalendarSyncResultDto,
};

// ── Google Calendar commands ────────────────────────────────

#[tauri::command]
pub async fn google_calendar_list_calendars(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<CalendarInfoDto>, String> {
    let client = gmail.get(&account_id).await?;
    google::google_calendar_list_calendars_impl(&account_id, &db, &client).await
}

#[tauri::command]
pub async fn google_calendar_sync_events(
    account_id: String,
    calendar_remote_id: String,
    sync_token: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<CalendarSyncResultDto, String> {
    let client = gmail.get(&account_id).await?;
    google::google_calendar_sync_events_impl(
        &account_id,
        &calendar_remote_id,
        sync_token,
        &db,
        &client,
    )
    .await
}

#[tauri::command]
pub async fn google_calendar_fetch_events(
    account_id: String,
    calendar_remote_id: String,
    time_min: String,
    time_max: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<CalendarEventDto>, String> {
    let client = gmail.get(&account_id).await?;
    google::google_calendar_fetch_events_impl(
        &client,
        &db,
        &calendar_remote_id,
        &time_min,
        &time_max,
    )
    .await
}

#[tauri::command]
pub async fn google_calendar_create_event(
    account_id: String,
    calendar_remote_id: String,
    event: serde_json::Value,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<CalendarEventDto, String> {
    let client = gmail.get(&account_id).await?;
    google::google_calendar_create_event_impl(&client, &db, &calendar_remote_id, event).await
}

#[tauri::command]
pub async fn google_calendar_update_event(
    account_id: String,
    calendar_remote_id: String,
    remote_event_id: String,
    event: serde_json::Value,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<CalendarEventDto, String> {
    let client = gmail.get(&account_id).await?;
    google::google_calendar_update_event_impl(
        &client,
        &db,
        &calendar_remote_id,
        &remote_event_id,
        event,
    )
    .await
}

#[tauri::command]
pub async fn google_calendar_delete_event(
    account_id: String,
    calendar_remote_id: String,
    remote_event_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let client = gmail.get(&account_id).await?;
    google::google_calendar_delete_event_impl(
        &client,
        &db,
        &calendar_remote_id,
        &remote_event_id,
    )
    .await
}

// ── CalDAV commands ─────────────────────────────────────────

#[tauri::command]
pub async fn caldav_list_calendars(
    account_id: String,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<Vec<CalendarInfoDto>, String> {
    caldav::caldav_list_calendars_impl(&account_id, &db, crypto.encryption_key()).await
}

#[tauri::command]
pub async fn caldav_fetch_events(
    account_id: String,
    calendar_remote_id: String,
    time_min: String,
    time_max: String,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<Vec<CalendarEventDto>, String> {
    caldav::caldav_fetch_events_impl(
        &db,
        crypto.encryption_key(),
        &account_id,
        &calendar_remote_id,
        &time_min,
        &time_max,
    )
    .await
}

#[tauri::command]
pub async fn caldav_sync_events(
    account_id: String,
    calendar_remote_id: String,
    _sync_token: Option<String>,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<CalendarSyncResultDto, String> {
    caldav::caldav_sync_events_impl(&account_id, &calendar_remote_id, &db, crypto.encryption_key())
        .await
}

#[tauri::command]
pub async fn caldav_test_connection(
    account_id: String,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<serde_json::Value, String> {
    caldav::caldav_test_connection_impl(&db, crypto.encryption_key(), &account_id).await
}

#[tauri::command]
pub async fn caldav_create_event(
    account_id: String,
    calendar_remote_id: String,
    event: serde_json::Value,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<CalendarEventDto, String> {
    caldav::caldav_create_event_impl(
        &db,
        crypto.encryption_key(),
        &account_id,
        &calendar_remote_id,
        event,
    )
    .await
}

#[tauri::command]
pub async fn caldav_update_event(
    account_id: String,
    _calendar_remote_id: String,
    remote_event_id: String,
    event: serde_json::Value,
    etag: Option<String>,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<CalendarEventDto, String> {
    caldav::caldav_update_event_impl(
        &db,
        crypto.encryption_key(),
        &account_id,
        &remote_event_id,
        event,
        etag,
    )
    .await
}

#[tauri::command]
pub async fn caldav_delete_event(
    account_id: String,
    _calendar_remote_id: String,
    remote_event_id: String,
    etag: Option<String>,
    db: State<'_, DbState>,
    crypto: State<'_, AppCryptoState>,
) -> Result<(), String> {
    caldav::caldav_delete_event_impl(
        &db,
        crypto.encryption_key(),
        &account_id,
        &remote_event_id,
        etag,
    )
    .await
}

// ── Sync / DB commands ──────────────────────────────────────

#[tauri::command]
pub async fn calendar_sync_account(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    sync::calendar_sync_account_impl(&account_id, &db, &gmail).await
}

#[tauri::command]
pub async fn calendar_upsert_discovered_calendars(
    db: State<'_, DbState>,
    account_id: String,
    provider: String,
    calendars: Vec<CalendarInfoInput>,
) -> Result<(), String> {
    let calendar_dtos: Vec<CalendarInfoDto> = calendars
        .into_iter()
        .map(|c| CalendarInfoDto {
            remote_id: c.remote_id,
            display_name: c.display_name,
            color: c.color,
            is_primary: c.is_primary,
        })
        .collect();
    sync::upsert_discovered_calendars_impl(&db, &account_id, &provider, calendar_dtos).await
}

#[tauri::command]
pub async fn calendar_upsert_provider_events(
    db: State<'_, DbState>,
    account_id: String,
    calendar_remote_id: String,
    events: Vec<CalendarEventDto>,
) -> Result<(), String> {
    sync::upsert_provider_events_impl(&db, &account_id, &calendar_remote_id, events).await
}

#[tauri::command]
pub async fn calendar_apply_sync_result(
    db: State<'_, DbState>,
    account_id: String,
    calendar_remote_id: String,
    created: Vec<CalendarEventDto>,
    updated: Vec<CalendarEventDto>,
    deleted_remote_ids: Vec<String>,
    new_sync_token: Option<String>,
    new_ctag: Option<String>,
) -> Result<(), String> {
    sync::apply_calendar_sync_result_impl(
        &db,
        &account_id,
        &calendar_remote_id,
        CalendarSyncResultDto {
            created,
            updated,
            deleted_remote_ids,
            new_sync_token,
            new_ctag,
        },
    )
    .await
}

#[tauri::command]
pub async fn calendar_delete_provider_event(
    db: State<'_, DbState>,
    account_id: String,
    calendar_remote_id: String,
    remote_event_id: String,
) -> Result<(), String> {
    sync::delete_provider_event_impl(&db, &account_id, &calendar_remote_id, &remote_event_id)
        .await
}
