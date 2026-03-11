use rusqlite::{OptionalExtension, params};
use serde::{Deserialize, Serialize};
use tauri::State;

use crate::db::DbState;
use crate::gmail::client::GmailState;
use crate::provider::http::{self, RetryConfig};

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

async fn google_calendar_request<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    client: &crate::gmail::client::GmailClient,
    db: &DbState,
    url: &str,
) -> Result<T, String> {
    google_calendar_request_with_body::<T>(http, client, db, "GET", url, None).await
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
            .header("Authorization", format!("Bearer {access_token}"))
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
            .and_utc()
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
            .and_utc()
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
