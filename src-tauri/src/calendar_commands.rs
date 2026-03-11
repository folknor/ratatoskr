use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::provider::http::{self, RetryConfig};

const CALDAV_NS: &str = "urn:ietf:params:xml:ns:caldav";
const GOOGLE_CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";
const GOOGLE_CALENDAR_RETRY_CONFIG: RetryConfig = RetryConfig {
    max_attempts: 3,
    initial_backoff_ms: 1000,
};

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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarInfoDto {
    pub remote_id: String,
    pub display_name: String,
    pub color: Option<String>,
    pub is_primary: bool,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarEventDto {
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CalendarSyncResultDto {
    pub created: Vec<CalendarEventDto>,
    pub updated: Vec<CalendarEventDto>,
    pub deleted_remote_ids: Vec<String>,
    pub new_sync_token: Option<String>,
    pub new_ctag: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCalendarListItem {
    id: String,
    summary: String,
    #[serde(default)]
    background_color: Option<String>,
    #[serde(default)]
    primary: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCalendarListResponse {
    #[serde(default)]
    items: Vec<GoogleCalendarListItem>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCalendarEvent {
    id: String,
    #[serde(default)]
    summary: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default)]
    location: Option<String>,
    start: GoogleCalendarDateTime,
    end: GoogleCalendarDateTime,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    organizer: Option<GoogleCalendarOrganizer>,
    #[serde(default)]
    attendees: Option<Vec<serde_json::Value>>,
    #[serde(default)]
    html_link: Option<String>,
    #[serde(default)]
    i_cal_u_i_d: Option<String>,
    #[serde(default)]
    etag: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCalendarDateTime {
    #[serde(default)]
    date_time: Option<String>,
    #[serde(default)]
    date: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCalendarOrganizer {
    email: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleEventListResponse {
    #[serde(default)]
    items: Vec<GoogleCalendarEvent>,
    #[serde(default)]
    next_page_token: Option<String>,
    #[serde(default)]
    next_sync_token: Option<String>,
}

struct CaldavAccountConfig {
    server_url: String,
    username: String,
    password: String,
    principal_url: Option<String>,
    home_url: Option<String>,
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
pub async fn google_calendar_list_calendars(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<CalendarInfoDto>, String> {
    let client = gmail.get(&account_id).await?;
    let http = reqwest::Client::new();
    let url = format!("{GOOGLE_CALENDAR_API_BASE}/users/me/calendarList");
    let response: GoogleCalendarListResponse =
        google_calendar_request(&http, &client, &db, &url).await?;

    Ok(response
        .items
        .into_iter()
        .map(|cal| CalendarInfoDto {
            remote_id: cal.id,
            display_name: cal.summary,
            color: cal.background_color,
            is_primary: cal.primary.unwrap_or(false),
        })
        .collect())
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
    let http = reqwest::Client::new();
    let encoded_id = urlencoding::encode(&calendar_remote_id);
    let mut created = Vec::new();
    let updated = Vec::new();
    let mut deleted_remote_ids = Vec::new();
    let mut page_token: Option<String> = None;
    let mut next_sync_token: Option<String> = None;

    loop {
        let mut params = vec![("maxResults", "250".to_string())];
        if let Some(token) = sync_token.as_ref() {
            params.push(("syncToken", token.clone()));
        } else {
            let mut time_min = chrono::Utc::now() - chrono::Duration::days(90);
            let mut time_max = chrono::Utc::now() + chrono::Duration::days(365);
            params.push(("timeMin", time_min.to_rfc3339()));
            params.push(("timeMax", time_max.to_rfc3339()));
            params.push(("singleEvents", "true".to_string()));
            let _ = (&mut time_min, &mut time_max);
        }
        if let Some(token) = page_token.as_ref() {
            params.push(("pageToken", token.clone()));
        }

        let query = params
            .into_iter()
            .map(|(key, value)| format!("{key}={}", urlencoding::encode(&value)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!("{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_id}/events?{query}");

        let response = match google_calendar_request::<GoogleEventListResponse>(&http, &client, &db, &url).await {
            Ok(value) => value,
            Err(error) => {
                if error.contains("410") || error.to_lowercase().contains("sync token") {
                    return Ok(CalendarSyncResultDto {
                        created: Vec::new(),
                        updated: Vec::new(),
                        deleted_remote_ids: Vec::new(),
                        new_sync_token: None,
                        new_ctag: None,
                    });
                }
                return Err(error);
            }
        };

        for item in response.items {
            if item.status.as_deref() == Some("cancelled") {
                deleted_remote_ids.push(item.id);
            } else {
                created.push(map_google_event(item)?);
            }
        }

        page_token = response.next_page_token;
        if response.next_sync_token.is_some() {
            next_sync_token = response.next_sync_token;
        }

        if page_token.is_none() {
            break;
        }
    }

    Ok(CalendarSyncResultDto {
        created,
        updated,
        deleted_remote_ids,
        new_sync_token: next_sync_token,
        new_ctag: None,
    })
}

#[tauri::command]
pub async fn caldav_list_calendars(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<CalendarInfoDto>, String> {
    let config = load_caldav_account_config(&db, gmail.encryption_key(), &account_id).await?;
    let client = reqwest::Client::new();
    let home_url = resolve_caldav_home_url(&client, &config).await?;
    list_caldav_calendars(&client, &config, &home_url).await
}

#[tauri::command]
pub async fn caldav_fetch_events(
    account_id: String,
    calendar_remote_id: String,
    time_min: String,
    time_max: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<Vec<CalendarEventDto>, String> {
    let config = load_caldav_account_config(&db, gmail.encryption_key(), &account_id).await?;
    let client = reqwest::Client::new();
    fetch_caldav_events(&client, &config, &calendar_remote_id, &time_min, &time_max).await
}

#[tauri::command]
pub async fn caldav_sync_events(
    account_id: String,
    calendar_remote_id: String,
    _sync_token: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<CalendarSyncResultDto, String> {
    let config = load_caldav_account_config(&db, gmail.encryption_key(), &account_id).await?;
    let client = reqwest::Client::new();
    let time_min = (chrono::Utc::now() - chrono::Duration::days(90)).to_rfc3339();
    let time_max = (chrono::Utc::now() + chrono::Duration::days(365)).to_rfc3339();
    let created =
        fetch_caldav_events(&client, &config, &calendar_remote_id, &time_min, &time_max).await?;
    Ok(CalendarSyncResultDto {
        created,
        updated: Vec::new(),
        deleted_remote_ids: Vec::new(),
        new_sync_token: None,
        new_ctag: None,
    })
}

#[tauri::command]
pub async fn caldav_test_connection(
    account_id: String,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<serde_json::Value, String> {
    let config = load_caldav_account_config(&db, gmail.encryption_key(), &account_id).await?;
    let client = reqwest::Client::new();
    let result = match resolve_caldav_home_url(&client, &config).await {
        Ok(home_url) => list_caldav_calendars(&client, &config, &home_url).await,
        Err(error) => Err(error),
    };

    match result {
        Ok(calendars) => Ok(serde_json::json!({
            "success": true,
            "message": format!(
                "Connected — found {} calendar{}",
                calendars.len(),
                if calendars.len() == 1 { "" } else { "s" }
            )
        })),
        Err(error) => Ok(serde_json::json!({
            "success": false,
            "message": error,
        })),
    }
}

#[tauri::command]
pub async fn caldav_create_event(
    account_id: String,
    calendar_remote_id: String,
    event: serde_json::Value,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<CalendarEventDto, String> {
    let config = load_caldav_account_config(&db, gmail.encryption_key(), &account_id).await?;
    let client = reqwest::Client::new();
    let input = parse_caldav_event_input(event)?;
    let uid = uuid::Uuid::new_v4().to_string();
    let ical_data = build_caldav_ical_event(&input, Some(&uid));
    let remote_event_id = join_url_path(&calendar_remote_id, &format!("{uid}.ics"))?;

    caldav_request_with_headers(
        &client,
        &config,
        "PUT",
        &remote_event_id,
        Some(&ical_data),
        None,
        &[("Content-Type", "text/calendar; charset=utf-8")],
    )
    .await?;

    fetch_caldav_event_by_href(&client, &config, &remote_event_id).await
}

#[tauri::command]
pub async fn caldav_update_event(
    account_id: String,
    _calendar_remote_id: String,
    remote_event_id: String,
    event: serde_json::Value,
    etag: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<CalendarEventDto, String> {
    let config = load_caldav_account_config(&db, gmail.encryption_key(), &account_id).await?;
    let client = reqwest::Client::new();
    let input = parse_caldav_event_input(event)?;
    let existing = fetch_caldav_event_by_href(&client, &config, &remote_event_id).await?;
    let merged = merge_caldav_event_input(&existing, &input);
    let ical_data = build_caldav_ical_event(&merged, existing.uid.as_deref());

    let mut headers = vec![("Content-Type", "text/calendar; charset=utf-8")];
    if let Some(etag_value) = etag.as_deref() {
        headers.push(("If-Match", etag_value));
    }

    caldav_request_with_headers(
        &client,
        &config,
        "PUT",
        &remote_event_id,
        Some(&ical_data),
        None,
        &headers,
    )
    .await?;

    fetch_caldav_event_by_href(&client, &config, &remote_event_id).await
}

#[tauri::command]
pub async fn caldav_delete_event(
    account_id: String,
    _calendar_remote_id: String,
    remote_event_id: String,
    etag: Option<String>,
    db: State<'_, DbState>,
    gmail: State<'_, GmailState>,
) -> Result<(), String> {
    let config = load_caldav_account_config(&db, gmail.encryption_key(), &account_id).await?;
    let client = reqwest::Client::new();
    let mut headers = Vec::new();
    if let Some(etag_value) = etag.as_deref() {
        headers.push(("If-Match", etag_value));
    }
    caldav_request_with_headers(
        &client,
        &config,
        "DELETE",
        &remote_event_id,
        None,
        None,
        &headers,
    )
    .await?;
    Ok(())
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
    let http = reqwest::Client::new();
    let encoded_id = urlencoding::encode(&calendar_remote_id);
    let query = [
        ("timeMin", time_min),
        ("timeMax", time_max),
        ("singleEvents", "true".to_string()),
        ("orderBy", "startTime".to_string()),
        ("maxResults", "250".to_string()),
    ]
    .into_iter()
    .map(|(key, value)| format!("{key}={}", urlencoding::encode(&value)))
    .collect::<Vec<_>>()
    .join("&");
    let url = format!("{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_id}/events?{query}");
    let response: GoogleEventListResponse = google_calendar_request(&http, &client, &db, &url).await?;

    response
        .items
        .into_iter()
        .map(map_google_event)
        .collect()
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
    let http = reqwest::Client::new();
    let encoded_id = urlencoding::encode(&calendar_remote_id);
    let url = format!("{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_id}/events");
    let response: GoogleCalendarEvent =
        google_calendar_request_with_body(&http, &client, &db, "POST", &url, Some(event)).await?;
    map_google_event(response)
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
    let http = reqwest::Client::new();
    let encoded_cal_id = urlencoding::encode(&calendar_remote_id);
    let encoded_event_id = urlencoding::encode(&remote_event_id);
    let url = format!(
        "{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_cal_id}/events/{encoded_event_id}"
    );
    let response: GoogleCalendarEvent =
        google_calendar_request_with_body(&http, &client, &db, "PATCH", &url, Some(event)).await?;
    map_google_event(response)
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
    let http = reqwest::Client::new();
    let encoded_cal_id = urlencoding::encode(&calendar_remote_id);
    let encoded_event_id = urlencoding::encode(&remote_event_id);
    let url = format!(
        "{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_cal_id}/events/{encoded_event_id}"
    );
    google_calendar_request_empty(&http, &client, &db, "DELETE", &url).await
}

#[tauri::command]
pub async fn calendar_upsert_provider_events(
    db: State<'_, DbState>,
    account_id: String,
    calendar_remote_id: String,
    events: Vec<CalendarEventInput>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let calendar_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, calendar_remote_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!("Calendar with remote_id '{calendar_remote_id}' not found for account '{account_id}'")
            })?;

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

async fn google_calendar_request<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    client: &crate::gmail::client::GmailClient,
    db: &DbState,
    url: &str,
) -> Result<T, String> {
    google_calendar_request_with_body::<T>(http, client, db, "GET", url, None).await
}

async fn load_caldav_account_config(
    db: &DbState,
    encryption_key: &[u8; 32],
    account_id: &str,
) -> Result<CaldavAccountConfig, String> {
    let key = *encryption_key;
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let row = conn
            .query_row(
                "SELECT email, caldav_url, caldav_username, caldav_password, caldav_principal_url, caldav_home_url
                 FROM accounts WHERE id = ?1",
                rusqlite::params![account_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<String>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()
            .map_err(|e| format!("query caldav account: {e}"))?
            .ok_or_else(|| "Account not found".to_string())?;

        let server_url = row
            .1
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
        let password_raw = row
            .3
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| "CalDAV credentials not configured".to_string())?;
        let password = if crate::provider::crypto::is_encrypted(&password_raw) {
            crate::provider::crypto::decrypt_value(&key, &password_raw)?
        } else {
            password_raw
        };
        let username = row
            .2
            .filter(|value| !value.trim().is_empty())
            .unwrap_or(row.0);

        Ok(CaldavAccountConfig {
            server_url,
            username,
            password,
            principal_url: row.4.filter(|value| !value.trim().is_empty()),
            home_url: row.5.filter(|value| !value.trim().is_empty()),
        })
    })
    .await
}

async fn resolve_caldav_home_url(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
) -> Result<String, String> {
    if let Some(home_url) = config.home_url.as_ref() {
        return Ok(home_url.clone());
    }

    let principal_url = if let Some(principal) = config.principal_url.as_ref() {
        principal.clone()
    } else {
        let body = r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:">
  <d:prop>
    <d:current-user-principal />
  </d:prop>
</d:propfind>"#;
        let response = caldav_request(client, config, "PROPFIND", &config.server_url, Some(body), Some("0")).await?;
        let xml = response.text().await.map_err(|e| format!("read principal response: {e}"))?;
        let href = extract_first_href_for_property(&xml, &["current-user-principal"])
            .ok_or_else(|| "CalDAV discovery failed: current-user-principal not found".to_string())?;
        resolve_href(&config.server_url, &href)?
    };

    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:" xmlns:c="{CALDAV_NS}">
  <d:prop>
    <c:calendar-home-set />
  </d:prop>
</d:propfind>"#
    );
    let response = caldav_request(client, config, "PROPFIND", &principal_url, Some(&body), Some("0")).await?;
    let xml = response.text().await.map_err(|e| format!("read home response: {e}"))?;
    let href = extract_first_href_for_property(&xml, &["calendar-home-set"])
        .ok_or_else(|| "CalDAV discovery failed: calendar-home-set not found".to_string())?;
    resolve_href(&principal_url, &href)
}

async fn list_caldav_calendars(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    home_url: &str,
) -> Result<Vec<CalendarInfoDto>, String> {
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8" ?>
<d:propfind xmlns:d="DAV:" xmlns:c="{CALDAV_NS}" xmlns:cs="http://calendarserver.org/ns/">
  <d:prop>
    <d:displayname />
    <cs:calendar-color />
    <d:resourcetype />
  </d:prop>
</d:propfind>"#
    );
    let response = caldav_request(client, config, "PROPFIND", home_url, Some(&body), Some("1")).await?;
    let xml = response.text().await.map_err(|e| format!("read calendars response: {e}"))?;
    let responses = split_xml_responses(&xml);
    let mut calendars = Vec::new();

    for response_xml in responses {
        if !contains_any_tag(response_xml, &["calendar"]) {
            continue;
        }

        let Some(href) = extract_first_tag_value(response_xml, &["href"]) else {
            continue;
        };
        let remote_id = resolve_href(home_url, &href)?;
        if normalize_url_for_compare(&remote_id) == normalize_url_for_compare(home_url) {
            continue;
        }

        let display_name = extract_first_tag_value(response_xml, &["displayname"])
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| format!("Calendar {}", calendars.len() + 1));
        let color = extract_first_tag_value(response_xml, &["calendar-color"]);

        calendars.push(CalendarInfoDto {
            remote_id,
            display_name,
            color,
            is_primary: calendars.is_empty(),
        });
    }

    Ok(calendars)
}

async fn fetch_caldav_events(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    calendar_remote_id: &str,
    time_min: &str,
    time_max: &str,
) -> Result<Vec<CalendarEventDto>, String> {
    let time_min = chrono::DateTime::parse_from_rfc3339(time_min)
        .map_err(|e| format!("invalid CalDAV timeMin: {e}"))?
        .with_timezone(&chrono::Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string();
    let time_max = chrono::DateTime::parse_from_rfc3339(time_max)
        .map_err(|e| format!("invalid CalDAV timeMax: {e}"))?
        .with_timezone(&chrono::Utc)
        .format("%Y%m%dT%H%M%SZ")
        .to_string();
    let body = format!(
        r#"<?xml version="1.0" encoding="utf-8" ?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="{CALDAV_NS}">
  <d:prop>
    <d:getetag />
    <c:calendar-data />
  </d:prop>
  <c:filter>
    <c:comp-filter name="VCALENDAR">
      <c:comp-filter name="VEVENT">
        <c:time-range start="{time_min}" end="{time_max}" />
      </c:comp-filter>
    </c:comp-filter>
  </c:filter>
</c:calendar-query>"#
    );
    let response =
        caldav_request(client, config, "REPORT", calendar_remote_id, Some(&body), Some("1")).await?;
    let xml = response.text().await.map_err(|e| format!("read events response: {e}"))?;
    let responses = split_xml_responses(&xml);
    let mut events = Vec::new();

    for response_xml in responses {
        let Some(calendar_data) = extract_first_tag_value(response_xml, &["calendar-data"]) else {
            continue;
        };
        let Some(href) = extract_first_tag_value(response_xml, &["href"]) else {
            continue;
        };
        let remote_event_id = resolve_href(calendar_remote_id, &href)?;
        let etag = extract_first_tag_value(response_xml, &["getetag"]);
        let mut event = parse_caldav_ical_event(&calendar_data, &remote_event_id)?;
        event.etag = etag;
        events.push(event);
    }

    Ok(events)
}

async fn caldav_request(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    method: &str,
    url: &str,
    body: Option<&str>,
    depth: Option<&str>,
) -> Result<reqwest::Response, String> {
    caldav_request_with_headers(client, config, method, url, body, depth, &[]).await
}

async fn caldav_request_with_headers(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    method: &str,
    url: &str,
    body: Option<&str>,
    depth: Option<&str>,
    headers: &[(&str, &str)],
) -> Result<reqwest::Response, String> {
    let method = reqwest::Method::from_bytes(method.as_bytes())
        .map_err(|e| format!("invalid CalDAV method {method}: {e}"))?;
    let mut request = client
        .request(method, url)
        .basic_auth(&config.username, Some(&config.password))
        .header("Accept", "application/xml, text/xml, */*");

    if let Some(depth_value) = depth {
        request = request.header("Depth", depth_value);
    }
    if let Some(body_value) = body {
        request = request
            .header("Content-Type", "application/xml; charset=utf-8")
            .body(body_value.to_string());
    }
    for (name, value) in headers {
        request = request.header(*name, *value);
    }

    let response = request
        .send()
        .await
        .map_err(|e| format!("CalDAV request failed: {e}"))?;

    let status = response.status();
    if status.is_success() || status.as_u16() == 207 {
        return Ok(response);
    }

    let body = response.text().await.unwrap_or_default();
    Err(format!("CalDAV error: {status} {body}"))
}

/// Extract the namespace prefix (e.g. "d:", "D:", "ns0:") for a given namespace URI.
/// Returns all matching prefixes plus "" (no prefix) as a fallback.
fn xml_ns_prefixes_for<'a>(xml: &'a str, ns_uri: &str) -> Vec<std::borrow::Cow<'a, str>> {
    let mut prefixes: Vec<std::borrow::Cow<'a, str>> = vec!["".into()];
    let mut pos = 0;
    while let Some(rel) = xml[pos..].find("xmlns") {
        pos += rel + 5;
        let rest = &xml[pos..];
        // xmlns="NS" (default namespace) or xmlns:prefix="NS"
        let (prefix_colon, value_start) = if rest.starts_with(':') {
            let colon_end = rest[1..]
                .find(['=', ' ', '\t', '\r', '\n', '>'])
                .unwrap_or(rest.len());
            let prefix = &rest[1..colon_end + 1];
            let after = &rest[colon_end + 1..];
            let after = after.trim_start_matches(['=', ' ', '\t']);
            (Some(prefix), after)
        } else if rest.starts_with('=') {
            (None, &rest[1..])
        } else {
            continue;
        };
        let value_start = value_start.trim_start();
        let (value, _) = if value_start.starts_with('"') {
            let end = value_start[1..].find('"').unwrap_or(value_start.len());
            (&value_start[1..end + 1], &value_start[end + 2..])
        } else if value_start.starts_with('\'') {
            let end = value_start[1..].find('\'').unwrap_or(value_start.len());
            (&value_start[1..end + 1], &value_start[end + 2..])
        } else {
            continue;
        };
        if value == ns_uri {
            if let Some(prefix) = prefix_colon {
                prefixes.push(format!("{prefix}:").into());
            }
        }
    }
    prefixes
}

fn split_xml_responses(xml: &str) -> Vec<&str> {
    // Find all namespace prefixes used for the DAV: namespace so we can match
    // <response>, <d:response>, <D:response>, <ns0:response>, etc.
    let dav_prefixes = xml_ns_prefixes_for(xml, "DAV:");
    let mut responses = Vec::new();
    let mut search_start = 0;
    let xml_lower = xml.to_lowercase();

    while let Some(start_rel) = xml_lower[search_start..].find('<') {
        let start = search_start + start_rel;
        let after_lt = &xml_lower[start + 1..];

        // Check if this tag is <PREFIX:response> or <response>
        let matched_prefix: Option<&str> = dav_prefixes.iter().find_map(|prefix| {
            let open = format!("{prefix}response");
            if after_lt.starts_with(open.as_str()) {
                let rest = &after_lt[open.len()..];
                if matches!(rest.as_bytes().first(), Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n')) {
                    Some(prefix.as_ref())
                } else {
                    None
                }
            } else {
                None
            }
        });

        let Some(prefix) = matched_prefix else {
            search_start = start + 1;
            continue;
        };

        let close = format!("</{prefix}response>");
        let Some(end_rel) = xml_lower[start..].find(&close) else {
            break;
        };
        let end = start + end_rel + close.len();
        responses.push(&xml[start..end]);
        search_start = end;
    }

    responses
}

fn extract_first_href_for_property(xml: &str, property_names: &[&str]) -> Option<String> {
    for property_name in property_names {
        if let Some(section) = extract_first_element(xml, property_name) {
            if let Some(href) = extract_first_tag_value(section, &["href"]) {
                return Some(href);
            }
        }
    }
    None
}

fn extract_first_tag_value(xml: &str, tag_names: &[&str]) -> Option<String> {
    tag_names.iter().find_map(|tag_name| extract_tag_value(xml, tag_name))
}

fn extract_tag_value(xml: &str, tag_name: &str) -> Option<String> {
    extract_first_element(xml, tag_name).and_then(|element| {
        let start = element.find('>')? + 1;
        let end = element.rfind('<')?;
        Some(html_unescape(&element[start..end]))
    })
}

fn extract_first_element<'a>(xml: &'a str, tag_name: &str) -> Option<&'a str> {
    let xml_lower = xml.to_lowercase();
    let tag_lower = tag_name.to_lowercase();
    // Collect all namespace prefixes present in the document plus common fallbacks
    let all_prefixes = {
        let mut p: Vec<std::borrow::Cow<'_, str>> = Vec::new();
        for ns in ["DAV:", "urn:ietf:params:xml:ns:caldav", "http://calendarserver.org/ns/", "http://apple.com/ns/ical/"] {
            p.extend(xml_ns_prefixes_for(xml, ns));
        }
        // Deduplicate while preserving order
        let mut seen = std::collections::HashSet::new();
        p.retain(|x| seen.insert(x.to_string()));
        p
    };
    for prefix in &all_prefixes {
        let open = format!("<{prefix}{tag_lower}");
        let close = format!("</{prefix}{tag_lower}>");
        if let Some(start) = xml_lower.find(&open) {
            // Verify the character after the tag name is a delimiter (not part of a longer name)
            let after_name = &xml_lower[start + open.len()..];
            if !matches!(after_name.as_bytes().first(), Some(b'>') | Some(b' ') | Some(b'\t') | Some(b'\r') | Some(b'\n')) {
                continue;
            }
            if let Some(end_rel) = xml_lower[start..].find(&close) {
                let end = start + end_rel + close.len();
                return Some(&xml[start..end]);
            }
        }
    }
    None
}

fn contains_any_tag(xml: &str, tag_names: &[&str]) -> bool {
    tag_names
        .iter()
        .any(|tag_name| extract_first_element(xml, tag_name).is_some())
}

fn resolve_href(base: &str, href: &str) -> Result<String, String> {
    reqwest::Url::parse(base)
        .map_err(|e| format!("invalid base url: {e}"))?
        .join(href)
        .map(|url| url.to_string())
        .map_err(|e| format!("invalid CalDAV href {href}: {e}"))
}

fn normalize_url_for_compare(url: &str) -> String {
    url.trim_end_matches('/').to_string()
}

fn html_unescape(value: &str) -> String {
    value
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn parse_caldav_ical_event(ical_data: &str, href: &str) -> Result<CalendarEventDto, String> {
    let lines = unfold_ical_lines(ical_data);

    let mut uid = None;
    let mut summary = None;
    let mut description = None;
    let mut location = None;
    let mut dtstart = None;
    let mut dtend = None;
    let mut status = "confirmed".to_string();
    let mut organizer_email = None;
    let mut is_all_day = false;
    let mut attendees = Vec::<serde_json::Value>::new();

    for line in lines {
        let mut parts = line.splitn(2, ':');
        let Some(name_with_params) = parts.next() else { continue };
        let value = parts.next().unwrap_or_default();
        let mut name_parts = name_with_params.split(';');
        let prop_name = name_parts.next().unwrap_or_default().to_uppercase();
        let params = name_parts.collect::<Vec<_>>().join(";").to_uppercase();

        match prop_name.as_str() {
            "UID" => uid = Some(value.to_string()),
            "SUMMARY" => summary = Some(unescape_ical_text(value)),
            "DESCRIPTION" => description = Some(unescape_ical_text(value)),
            "LOCATION" => location = Some(unescape_ical_text(value)),
            "DTSTART" => {
                dtstart = Some(value.to_string());
                if params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME") {
                    is_all_day = true;
                }
            }
            "DTEND" => dtend = Some(value.to_string()),
            "STATUS" => status = value.to_lowercase(),
            "ORGANIZER" => {
                if let Some(email) = value.strip_prefix("mailto:").or_else(|| value.strip_prefix("MAILTO:")) {
                    organizer_email = Some(email.to_string());
                }
            }
            "ATTENDEE" => {
                if let Some(email) = value.strip_prefix("mailto:").or_else(|| value.strip_prefix("MAILTO:")) {
                    let display_name = extract_param_value(name_with_params, "CN");
                    let response_status = extract_param_value(name_with_params, "PARTSTAT")
                        .map(|value| value.to_lowercase());
                    attendees.push(serde_json::json!({
                        "email": email,
                        "displayName": display_name,
                        "responseStatus": response_status,
                    }));
                }
            }
            _ => {}
        }
    }

    let start_time = dtstart
        .as_deref()
        .map(|value| parse_ical_datetime(value, is_all_day))
        .transpose()?
        .unwrap_or(0);
    let end_time = dtend
        .as_deref()
        .map(|value| parse_ical_datetime(value, is_all_day))
        .transpose()?
        .unwrap_or(start_time + 3600);

    Ok(CalendarEventDto {
        remote_event_id: href.to_string(),
        uid,
        etag: None,
        summary,
        description,
        location,
        start_time,
        end_time,
        is_all_day,
        status,
        organizer_email,
        attendees_json: if attendees.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&attendees)
                    .map_err(|e| format!("serialize CalDAV attendees: {e}"))?,
            )
        },
        html_link: None,
        ical_data: Some(ical_data.to_string()),
    })
}

fn parse_caldav_event_input(value: serde_json::Value) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "invalid CalDAV event payload".to_string())
}

fn merge_caldav_event_input(
    existing: &CalendarEventDto,
    updates: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut merged = serde_json::Map::new();
    merged.insert(
        "summary".to_string(),
        serde_json::Value::String(
            updates
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| existing.summary.clone().unwrap_or_default()),
        ),
    );
    if let Some(description) = updates
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .or_else(|| existing.description.clone())
    {
        merged.insert("description".to_string(), serde_json::Value::String(description));
    }
    if let Some(location) = updates
        .get("location")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .or_else(|| existing.location.clone())
    {
        merged.insert("location".to_string(), serde_json::Value::String(location));
    }
    merged.insert(
        "startTime".to_string(),
        serde_json::Value::String(
            updates
                .get("startTime")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(existing.start_time, 0)
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_default()
                }),
        ),
    );
    merged.insert(
        "endTime".to_string(),
        serde_json::Value::String(
            updates
                .get("endTime")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(existing.end_time, 0)
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_default()
                }),
        ),
    );
    merged.insert(
        "isAllDay".to_string(),
        serde_json::Value::Bool(
            updates
                .get("isAllDay")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(existing.is_all_day),
        ),
    );
    merged
}

fn build_caldav_ical_event(
    input: &serde_json::Map<String, serde_json::Value>,
    uid: Option<&str>,
) -> String {
    let event_uid = uid
        .map(ToString::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Ratatoskr//CalDAV Client//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{event_uid}"),
        format!("DTSTAMP:{now}"),
    ];

    if let Some(summary) = input.get("summary").and_then(serde_json::Value::as_str) {
        lines.push(format!("SUMMARY:{}", escape_ical_text(summary)));
    }

    let start_time = input.get("startTime").and_then(serde_json::Value::as_str);
    let end_time = input.get("endTime").and_then(serde_json::Value::as_str);
    let is_all_day = input
        .get("isAllDay")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if let (Some(start), Some(end)) = (start_time, end_time) {
        if is_all_day {
            lines.push(format!(
                "DTSTART;VALUE=DATE:{}",
                format_ical_date(start)
            ));
            lines.push(format!("DTEND;VALUE=DATE:{}", format_ical_date(end)));
        } else {
            lines.push(format!("DTSTART:{}", format_ical_datetime(start)));
            lines.push(format!("DTEND:{}", format_ical_datetime(end)));
        }
    }

    if let Some(description) = input.get("description").and_then(serde_json::Value::as_str) {
        lines.push(format!("DESCRIPTION:{}", escape_ical_text(description)));
    }
    if let Some(location) = input.get("location").and_then(serde_json::Value::as_str) {
        lines.push(format!("LOCATION:{}", escape_ical_text(location)));
    }
    if let Some(attendees) = input.get("attendees").and_then(serde_json::Value::as_array) {
        for attendee in attendees {
            if let Some(email) = attendee.get("email").and_then(serde_json::Value::as_str) {
                lines.push(format!("ATTENDEE;RSVP=TRUE:mailto:{email}"));
            }
        }
    }

    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());
    lines.join("\r\n")
}

fn escape_ical_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

fn format_ical_datetime(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.with_timezone(&chrono::Utc).format("%Y%m%dT%H%M%SZ").to_string())
        .unwrap_or_else(|_| value.to_string())
}

fn format_ical_date(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.format("%Y%m%d").to_string())
        .or_else(|_| chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d").map(|date| date.format("%Y%m%d").to_string()))
        .unwrap_or_else(|_| value.replace('-', ""))
}

async fn fetch_caldav_event_by_href(
    client: &reqwest::Client,
    config: &CaldavAccountConfig,
    remote_event_id: &str,
) -> Result<CalendarEventDto, String> {
    let response = caldav_request_with_headers(client, config, "GET", remote_event_id, None, None, &[]).await?;
    let etag = response
        .headers()
        .get("ETag")
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);
    let ical_data = response.text().await.map_err(|e| format!("read CalDAV event: {e}"))?;
    let mut event = parse_caldav_ical_event(&ical_data, remote_event_id)?;
    event.etag = etag;
    Ok(event)
}

fn join_url_path(base: &str, segment: &str) -> Result<String, String> {
    let base = if base.ends_with('/') {
        base.to_string()
    } else {
        format!("{base}/")
    };
    resolve_href(&base, segment)
}

fn unfold_ical_lines(ical_data: &str) -> Vec<String> {
    ical_data
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn unescape_ical_text(value: &str) -> String {
    value
        .replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

fn extract_param_value(name_with_params: &str, key: &str) -> Option<String> {
    for param in name_with_params.split(';').skip(1) {
        let mut parts = param.splitn(2, '=');
        let param_name = parts.next()?.trim();
        let param_value = parts.next()?.trim();
        if param_name.eq_ignore_ascii_case(key) {
            return Some(param_value.trim_matches('"').to_string());
        }
    }
    None
}

fn parse_ical_datetime(value: &str, is_all_day: bool) -> Result<i64, String> {
    if is_all_day {
        let date = chrono::NaiveDate::parse_from_str(value, "%Y%m%d")
            .map_err(|e| format!("invalid all-day CalDAV date {value}: {e}"))?;
        return date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "invalid all-day CalDAV time".to_string())
            .map(|date_time| date_time.and_utc().timestamp());
    }

    if let Some(cleaned) = value.strip_suffix('Z') {
        return chrono::NaiveDateTime::parse_from_str(cleaned, "%Y%m%dT%H%M%S")
            .map_err(|e| format!("invalid UTC CalDAV datetime {value}: {e}"))
            .map(|date_time| date_time.and_utc().timestamp());
    }

    chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
        .map_err(|e| format!("invalid local CalDAV datetime {value}: {e}"))
        .map(|date_time| date_time.and_utc().timestamp())
}

async fn google_calendar_request_with_body<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    client: &crate::gmail::client::GmailClient,
    db: &DbState,
    method: &str,
    url: &str,
    body: Option<serde_json::Value>,
) -> Result<T, String> {
    let access_token = client.get_access_token(db).await?;
    let response =
        google_calendar_execute_with_retry(http, method, url, body.as_ref(), &access_token).await?;

    if response.status().as_u16() == 401 {
        let refreshed = client.force_refresh_token(db).await?;
        let retry = google_calendar_execute_with_retry(http, method, url, body.as_ref(), &refreshed).await?;
        return google_calendar_parse_json_response(retry).await;
    }

    google_calendar_parse_json_response(response).await
}

async fn google_calendar_request_empty(
    http: &reqwest::Client,
    client: &crate::gmail::client::GmailClient,
    db: &DbState,
    method: &str,
    url: &str,
) -> Result<(), String> {
    let access_token = client.get_access_token(db).await?;
    let response = google_calendar_execute_with_retry(http, method, url, None, &access_token).await?;

    if response.status().as_u16() == 401 {
        let refreshed = client.force_refresh_token(db).await?;
        let retry = google_calendar_execute_with_retry(http, method, url, None, &refreshed).await?;
        return google_calendar_check_response_status(retry).await;
    }

    google_calendar_check_response_status(response).await
}

async fn google_calendar_execute_with_retry(
    http: &reqwest::Client,
    method: &str,
    url: &str,
    body: Option<&serde_json::Value>,
    access_token: &str,
) -> Result<reqwest::Response, String> {
    let mut last_response = None;

    for attempt in 0..GOOGLE_CALENDAR_RETRY_CONFIG.max_attempts {
        let mut request = match method {
            "GET" => http.get(url),
            "POST" => http.post(url),
            "PATCH" => http.patch(url),
            "DELETE" => http.delete(url),
            _ => return Err(format!("Unsupported Google Calendar HTTP method: {method}")),
        }
        .header("Authorization", format!("Bearer {access_token}"))
        .header("Content-Type", "application/json");

        if let Some(payload) = body {
            request = request.json(payload);
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Google Calendar request failed: {e}"))?;

        if response.status().as_u16() != 429 {
            return Ok(response);
        }

        last_response = Some(response);
        if attempt == GOOGLE_CALENDAR_RETRY_CONFIG.max_attempts - 1 {
            break;
        }

        let delay_ms =
            http::compute_retry_delay(last_response.as_ref(), attempt, &GOOGLE_CALENDAR_RETRY_CONFIG);
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }

    last_response.ok_or_else(|| "No response received".to_string())
}

async fn google_calendar_parse_json_response<T: serde::de::DeserializeOwned>(
    response: reqwest::Response,
) -> Result<T, String> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Google Calendar API error: {status} {body}"));
    }

    response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Google Calendar response: {e}"))
}

async fn google_calendar_check_response_status(response: reqwest::Response) -> Result<(), String> {
    if response.status().is_success() || response.status().as_u16() == 204 {
        return Ok(());
    }

    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(format!("Google Calendar API error: {status} {body}"))
}

fn map_google_event(event: GoogleCalendarEvent) -> Result<CalendarEventDto, String> {
    let is_all_day = event.start.date.is_some();
    let start_time = if let Some(date_time) = event.start.date_time {
        chrono::DateTime::parse_from_rfc3339(&date_time)
            .map_err(|e| format!("Invalid Google Calendar start dateTime: {e}"))?
            .timestamp()
    } else if let Some(date) = event.start.date {
        chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|e| format!("Invalid Google Calendar start date: {e}"))?
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "Invalid all-day start time".to_string())?
            .and_local_timezone(chrono::Local)
            .single()
            .ok_or_else(|| "Ambiguous all-day start time (DST transition)".to_string())?
            .timestamp()
    } else {
        return Err("Google Calendar event missing start".to_string());
    };

    let end_time = if let Some(date_time) = event.end.date_time {
        chrono::DateTime::parse_from_rfc3339(&date_time)
            .map_err(|e| format!("Invalid Google Calendar end dateTime: {e}"))?
            .timestamp()
    } else if let Some(date) = event.end.date {
        chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|e| format!("Invalid Google Calendar end date: {e}"))?
            .and_hms_opt(23, 59, 59)
            .ok_or_else(|| "Invalid all-day end time".to_string())?
            .and_local_timezone(chrono::Local)
            .single()
            .ok_or_else(|| "Ambiguous all-day end time (DST transition)".to_string())?
            .timestamp()
    } else {
        return Err("Google Calendar event missing end".to_string());
    };

    Ok(CalendarEventDto {
        remote_event_id: event.id,
        uid: event.i_cal_u_i_d,
        etag: event.etag,
        summary: event.summary,
        description: event.description,
        location: event.location,
        start_time,
        end_time,
        is_all_day,
        status: event.status.unwrap_or_else(|| "confirmed".to_string()),
        organizer_email: event.organizer.map(|value| value.email),
        attendees_json: event
            .attendees
            .map(|value| serde_json::to_string(&value))
            .transpose()
            .map_err(|e| format!("Failed to serialize Google Calendar attendees: {e}"))?,
        html_link: event.html_link,
        ical_data: None,
    })
}
