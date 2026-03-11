use rusqlite::{OptionalExtension, params};
use serde::Deserialize;
use tauri::State;

use crate::db::DbState;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarInfoInput {
    pub remote_id: String,
    pub display_name: String,
    pub color: Option<String>,
    pub is_primary: bool,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventInput {
    pub remote_event_id: String,
    pub uid: Option<String>,
    pub etag: Option<String>,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub status: String,
    pub organizer_email: Option<String>,
    pub attendees_json: Option<String>,
    pub html_link: Option<String>,
    pub ical_data: Option<String>,
}

#[tauri::command]
pub async fn calendar_upsert_discovered_calendars(
    db: State<'_, DbState>,
    account_id: String,
    provider: String,
    calendars: Vec<CalendarInfoInput>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for calendar in calendars {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   display_name = ?5, color = ?6, is_primary = ?7, updated_at = unixepoch()",
                params![
                    id,
                    account_id,
                    provider,
                    calendar.remote_id,
                    calendar.display_name,
                    calendar.color,
                    calendar.is_primary as i64,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn calendar_upsert_provider_events(
    db: State<'_, DbState>,
    account_id: String,
    calendar_remote_id: String,
    events: Vec<CalendarEventInput>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let calendar_id: Option<String> = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, calendar_remote_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for event in events {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO calendar_events (id, account_id, google_event_id, summary, description, location, start_time, end_time, is_all_day, status, organizer_email, attendees_json, html_link, calendar_id, remote_event_id, etag, ical_data, uid)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                 ON CONFLICT(account_id, google_event_id) DO UPDATE SET
                   summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
                   is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
                   html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
                   ical_data = ?17, uid = ?18, updated_at = unixepoch()",
                params![
                    id,
                    account_id,
                    event.remote_event_id,
                    event.summary,
                    event.description,
                    event.location,
                    event.start_time,
                    event.end_time,
                    event.is_all_day as i64,
                    event.status,
                    event.organizer_email,
                    event.attendees_json,
                    event.html_link,
                    calendar_id,
                    event.remote_event_id,
                    event.etag,
                    event.ical_data,
                    event.uid,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn calendar_apply_sync_result(
    db: State<'_, DbState>,
    account_id: String,
    calendar_remote_id: String,
    created: Vec<CalendarEventInput>,
    updated: Vec<CalendarEventInput>,
    deleted_remote_ids: Vec<String>,
    new_sync_token: Option<String>,
    new_ctag: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let calendar_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, calendar_remote_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        for event in created.into_iter().chain(updated) {
            let id = uuid::Uuid::new_v4().to_string();
            tx.execute(
                "INSERT INTO calendar_events (id, account_id, google_event_id, summary, description, location, start_time, end_time, is_all_day, status, organizer_email, attendees_json, html_link, calendar_id, remote_event_id, etag, ical_data, uid)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                 ON CONFLICT(account_id, google_event_id) DO UPDATE SET
                   summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
                   is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
                   html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
                   ical_data = ?17, uid = ?18, updated_at = unixepoch()",
                params![
                    id,
                    account_id,
                    event.remote_event_id,
                    event.summary,
                    event.description,
                    event.location,
                    event.start_time,
                    event.end_time,
                    event.is_all_day as i64,
                    event.status,
                    event.organizer_email,
                    event.attendees_json,
                    event.html_link,
                    calendar_id,
                    event.remote_event_id,
                    event.etag,
                    event.ical_data,
                    event.uid,
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        for remote_event_id in deleted_remote_ids {
            tx.execute(
                "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
                params![calendar_id, remote_event_id],
            )
            .map_err(|e| e.to_string())?;
        }

        if new_sync_token.is_some() || new_ctag.is_some() {
            tx.execute(
                "UPDATE calendars SET sync_token = ?1, ctag = ?2, updated_at = unixepoch() WHERE id = ?3",
                params![new_sync_token, new_ctag, calendar_id],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

#[tauri::command]
pub async fn calendar_delete_provider_event(
    db: State<'_, DbState>,
    account_id: String,
    calendar_remote_id: String,
    remote_event_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let calendar_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, calendar_remote_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;

        conn.execute(
            "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
            params![calendar_id, remote_event_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
