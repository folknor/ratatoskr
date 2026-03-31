use serde::Deserialize;

use gmail::client::GmailClient;
use rtsk::db::DbState;
use rtsk::provider::http;

use super::types::{CalendarEventDto, CalendarInfoDto, CalendarSyncResultDto};
use super::{GOOGLE_CALENDAR_API_BASE, GOOGLE_CALENDAR_RETRY_CONFIG, shared_http_client};

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
    #[serde(default)]
    recurrence: Option<Vec<String>>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    transparency: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCalendarDateTime {
    #[serde(default)]
    date_time: Option<String>,
    #[serde(default)]
    date: Option<String>,
    #[serde(default)]
    time_zone: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GoogleCalendarOrganizer {
    email: String,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
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

pub async fn google_calendar_list_calendars_impl(
    _account_id: &str,
    db: &DbState,
    client: &GmailClient,
) -> Result<Vec<CalendarInfoDto>, String> {
    let http = shared_http_client();
    let url = format!("{GOOGLE_CALENDAR_API_BASE}/users/me/calendarList");
    let response: GoogleCalendarListResponse =
        google_calendar_request(http, client, db, &url).await?;

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

pub async fn google_calendar_sync_events_impl(
    account_id: &str,
    calendar_remote_id: &str,
    sync_token: Option<String>,
    db: &DbState,
    client: &GmailClient,
) -> Result<CalendarSyncResultDto, String> {
    let _ = account_id;
    let http = shared_http_client();
    let encoded_id = urlencoding::encode(calendar_remote_id);
    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut deleted_remote_ids = Vec::new();
    let mut page_token: Option<String> = None;
    let mut next_sync_token: Option<String> = None;

    loop {
        let mut params = vec![("maxResults", "250".to_string())];
        if let Some(token) = sync_token.as_ref() {
            params.push(("syncToken", token.clone()));
        } else {
            let time_min = chrono::Utc::now() - chrono::Duration::days(90);
            let time_max = chrono::Utc::now() + chrono::Duration::days(365);
            params.push(("timeMin", time_min.to_rfc3339()));
            params.push(("timeMax", time_max.to_rfc3339()));
            params.push(("singleEvents", "true".to_string()));
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

        let response = match google_calendar_request::<GoogleEventListResponse>(
            http, client, db, &url,
        )
        .await
        {
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
                let event = map_google_event(item)?;
                if sync_token.is_some() {
                    updated.push(event);
                } else {
                    created.push(event);
                }
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

pub async fn google_calendar_fetch_events_impl(
    client: &GmailClient,
    db: &DbState,
    calendar_remote_id: &str,
    time_min: &str,
    time_max: &str,
) -> Result<Vec<CalendarEventDto>, String> {
    let http = shared_http_client();
    let encoded_id = urlencoding::encode(calendar_remote_id);
    let query = [
        ("timeMin", time_min),
        ("timeMax", time_max),
        ("singleEvents", "true"),
        ("orderBy", "startTime"),
        ("maxResults", "250"),
    ]
    .into_iter()
    .map(|(key, value)| format!("{key}={}", urlencoding::encode(value)))
    .collect::<Vec<_>>()
    .join("&");
    let url = format!("{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_id}/events?{query}");
    let response: GoogleEventListResponse = google_calendar_request(http, client, db, &url).await?;

    response.items.into_iter().map(map_google_event).collect()
}

pub async fn google_calendar_create_event_impl(
    client: &GmailClient,
    db: &DbState,
    calendar_remote_id: &str,
    event: serde_json::Value,
) -> Result<CalendarEventDto, String> {
    let http = shared_http_client();
    let encoded_id = urlencoding::encode(calendar_remote_id);
    let url = format!("{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_id}/events");
    let response: GoogleCalendarEvent =
        google_calendar_request_with_body(http, client, db, "POST", &url, Some(event)).await?;
    map_google_event(response)
}

pub async fn google_calendar_update_event_impl(
    client: &GmailClient,
    db: &DbState,
    calendar_remote_id: &str,
    remote_event_id: &str,
    event: serde_json::Value,
) -> Result<CalendarEventDto, String> {
    let http = shared_http_client();
    let encoded_cal_id = urlencoding::encode(calendar_remote_id);
    let encoded_event_id = urlencoding::encode(remote_event_id);
    let url =
        format!("{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_cal_id}/events/{encoded_event_id}");
    let response: GoogleCalendarEvent =
        google_calendar_request_with_body(http, client, db, "PATCH", &url, Some(event)).await?;
    map_google_event(response)
}

pub async fn google_calendar_delete_event_impl(
    client: &GmailClient,
    db: &DbState,
    calendar_remote_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
    let http = shared_http_client();
    let encoded_cal_id = urlencoding::encode(calendar_remote_id);
    let encoded_event_id = urlencoding::encode(remote_event_id);
    let url =
        format!("{GOOGLE_CALENDAR_API_BASE}/calendars/{encoded_cal_id}/events/{encoded_event_id}");
    google_calendar_request_empty(http, client, db, "DELETE", &url).await
}

async fn google_calendar_request<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    client: &GmailClient,
    db: &DbState,
    url: &str,
) -> Result<T, String> {
    google_calendar_request_with_body::<T>(http, client, db, "GET", url, None).await
}

async fn google_calendar_request_with_body<T: serde::de::DeserializeOwned>(
    http: &reqwest::Client,
    client: &GmailClient,
    db: &DbState,
    method: &str,
    url: &str,
    json_body: Option<serde_json::Value>,
) -> Result<T, String> {
    let access_token = client.get_access_token(db).await?;
    let request_body = json_body.as_ref();
    let response =
        google_calendar_execute_with_retry(http, method, url, request_body, &access_token).await?;

    if response.status().as_u16() == 401 {
        let refreshed = client.force_refresh_token(db).await?;
        let retry =
            google_calendar_execute_with_retry(http, method, url, request_body, &refreshed).await?;
        return google_calendar_parse_json_response(retry).await;
    }

    google_calendar_parse_json_response(response).await
}

async fn google_calendar_request_empty(
    http: &reqwest::Client,
    client: &GmailClient,
    db: &DbState,
    method: &str,
    url: &str,
) -> Result<(), String> {
    let access_token = client.get_access_token(db).await?;
    let response =
        google_calendar_execute_with_retry(http, method, url, None, &access_token).await?;

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
    json_body: Option<&serde_json::Value>,
    access_token: &str,
) -> Result<reqwest::Response, String> {
    for attempt in 0..GOOGLE_CALENDAR_RETRY_CONFIG.max_attempts {
        let mut request = match method {
            "GET" => http.get(url),
            "POST" => http.post(url),
            "PATCH" => http.patch(url),
            "DELETE" => http.delete(url),
            _ => return Err(format!("Unsupported Google Calendar HTTP method: {method}")),
        }
        .header("Authorization", format!("Bearer {access_token}"));

        if let Some(payload) = json_body {
            request = request.json(payload);
        }

        let response = request
            .send()
            .await
            .map_err(|e| format!("Google Calendar request failed: {e}"))?;

        if response.status().as_u16() != 429 {
            return Ok(response);
        }

        if attempt == GOOGLE_CALENDAR_RETRY_CONFIG.max_attempts - 1 {
            break;
        }

        let delay_ms =
            http::compute_retry_delay(Some(&response), attempt, &GOOGLE_CALENDAR_RETRY_CONFIG);
        tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
    }

    Err("Google Calendar rate limited (429): max retry attempts exceeded".to_string())
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

    // Google uses "transparent" for free time, "opaque" for busy (default).
    let availability = event.transparency.map(|t| {
        if t == "transparent" {
            "free".to_string()
        } else {
            "busy".to_string()
        }
    });

    let recurrence_rule = event
        .recurrence
        .and_then(|rules| rules.into_iter().find(|r| r.starts_with("RRULE:")));

    let organizer_name = event
        .organizer
        .as_ref()
        .and_then(|o| o.display_name.clone());
    let organizer_email = event.organizer.map(|o| o.email);

    let timezone = event.start.time_zone.clone();

    Ok(CalendarEventDto {
        remote_event_id: event.id,
        uid: event.i_cal_u_i_d,
        etag: event.etag,
        summary: event.summary.clone(),
        title: event.summary,
        description: event.description,
        location: event.location,
        start_time,
        end_time,
        is_all_day,
        status: event.status.unwrap_or_else(|| "confirmed".to_string()),
        organizer_email,
        organizer_name,
        attendees_json: event
            .attendees
            .map(|value| serde_json::to_string(&value))
            .transpose()
            .map_err(|e| format!("Failed to serialize Google Calendar attendees: {e}"))?,
        html_link: event.html_link,
        ical_data: None,
        recurrence_rule,
        timezone,
        availability,
        visibility: event.visibility,
        ..CalendarEventDto::default()
    })
}
