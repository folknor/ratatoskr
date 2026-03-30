//! Google Calendar API v3 sync for Gmail accounts.
//!
//! Shares the same OAuth token as the Gmail provider. Calendar sync runs
//! alongside email sync (called from `sync/mod.rs`).

pub mod types;
mod storage;

use db::db::DbState;

use super::client::GmailClient;
use types::{
    CalendarListEntry, CalendarListResponse, EventListResponse, GoogleCalendarEvent,
};

const CALENDAR_API_BASE: &str = "https://www.googleapis.com/calendar/v3";

// ── Calendar list ──────────────────────────────────────────

/// Fetch the user's calendar list and upsert into the database.
///
/// Returns the list of local calendar IDs (our UUIDs) for the account.
pub async fn sync_calendar_list(
    client: &GmailClient,
    account_id: &str,
    db: &DbState,
) -> Result<Vec<CalendarInfo>, String> {
    let entries = list_all_calendars(client, db).await?;

    let mut result = Vec::with_capacity(entries.len());
    for entry in &entries {
        let local_id = storage::upsert_calendar(
            db,
            account_id,
            &entry.id,
            entry.summary.as_deref(),
            entry.background_color.as_deref(),
            entry.primary.unwrap_or(false),
        )
        .await?;

        result.push(CalendarInfo {
            local_id,
            remote_id: entry.id.clone(),
        });
    }

    Ok(result)
}

/// Mapping between our local UUID and the Google calendar ID.
#[derive(Debug, Clone)]
pub struct CalendarInfo {
    pub local_id: String,
    pub remote_id: String,
}

/// Paginate `calendarList.list` to fetch all calendars.
async fn list_all_calendars(
    client: &GmailClient,
    db: &DbState,
) -> Result<Vec<CalendarListEntry>, String> {
    let mut all = Vec::new();
    let mut page_token: Option<String> = None;

    loop {
        let mut url = format!("{CALENDAR_API_BASE}/users/me/calendarList?maxResults=250");
        if let Some(pt) = &page_token {
            url.push_str(&format!("&pageToken={pt}"));
        }

        let resp: CalendarListResponse = client.get_absolute(&url, db).await?;
        all.extend(resp.items);

        page_token = resp.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    Ok(all)
}

// ── Event sync ─────────────────────────────────────────────

/// Sync events for a single calendar, using incremental sync when possible.
///
/// Uses the `syncToken` from the events API to avoid re-fetching unchanged
/// events. Falls back to full sync when the sync token is expired (410).
pub async fn sync_calendar_events(
    client: &GmailClient,
    account_id: &str,
    cal: &CalendarInfo,
    db: &DbState,
) -> Result<(), String> {
    let sync_token = storage::load_sync_token(db, &cal.local_id).await?;

    match sync_token {
        Some(token) => {
            match incremental_event_sync(client, account_id, cal, &token, db).await {
                Ok(()) => Ok(()),
                Err(e) if e.contains("410") || e.contains("fullSyncRequired") => {
                    log::warn!(
                        "Calendar sync token expired for {} ({}), falling back to full sync",
                        cal.remote_id,
                        cal.local_id,
                    );
                    storage::save_sync_token(db, &cal.local_id, None).await?;
                    full_event_sync(client, account_id, cal, db).await
                }
                Err(e) => Err(e),
            }
        }
        None => full_event_sync(client, account_id, cal, db).await,
    }
}

/// Full event sync: fetch all events from now minus 30 days to now plus 365 days.
async fn full_event_sync(
    client: &GmailClient,
    account_id: &str,
    cal: &CalendarInfo,
    db: &DbState,
) -> Result<(), String> {
    let now = chrono::Utc::now();
    let time_min = (now - chrono::Duration::days(30))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();
    let time_max = (now + chrono::Duration::days(365))
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string();

    let encoded_cal_id = urlencoding::encode(&cal.remote_id);
    let base_url = format!(
        "{CALENDAR_API_BASE}/calendars/{encoded_cal_id}/events\
         ?maxResults=250&singleEvents=false\
         &timeMin={}&timeMax={}",
        urlencoding::encode(&time_min),
        urlencoding::encode(&time_max),
    );

    let mut page_token: Option<String> = None;
    let mut next_sync_token: Option<String> = None;

    loop {
        let url = if let Some(pt) = &page_token {
            format!("{base_url}&pageToken={pt}")
        } else {
            base_url.clone()
        };

        let resp: EventListResponse = client.get_absolute(&url, db).await?;

        for event in &resp.items {
            storage::upsert_event(db, account_id, cal, event).await?;
        }

        if resp.next_sync_token.is_some() {
            next_sync_token = resp.next_sync_token;
        }

        page_token = resp.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    if next_sync_token.is_some() {
        storage::save_sync_token(db, &cal.local_id, next_sync_token.as_deref()).await?;
    }

    Ok(())
}

/// Incremental event sync using a sync token from a previous sync.
async fn incremental_event_sync(
    client: &GmailClient,
    account_id: &str,
    cal: &CalendarInfo,
    sync_token: &str,
    db: &DbState,
) -> Result<(), String> {
    let encoded_cal_id = urlencoding::encode(&cal.remote_id);
    let base_url = format!(
        "{CALENDAR_API_BASE}/calendars/{encoded_cal_id}/events\
         ?maxResults=250&syncToken={}",
        urlencoding::encode(sync_token),
    );

    let mut page_token: Option<String> = None;
    let mut next_sync_token: Option<String> = None;

    loop {
        let url = if let Some(pt) = &page_token {
            format!("{base_url}&pageToken={pt}")
        } else {
            base_url.clone()
        };

        let resp: EventListResponse = client.get_absolute(&url, db).await?;

        for event in &resp.items {
            let event_id = event.id.as_deref().unwrap_or_default();
            let status = event.status.as_deref().unwrap_or("confirmed");

            if status == "cancelled" {
                storage::delete_event_by_remote_id(db, &cal.local_id, event_id).await?;
            } else {
                storage::upsert_event(db, account_id, cal, event).await?;
            }
        }

        if resp.next_sync_token.is_some() {
            next_sync_token = resp.next_sync_token;
        }

        page_token = resp.next_page_token;
        if page_token.is_none() {
            break;
        }
    }

    if next_sync_token.is_some() {
        storage::save_sync_token(db, &cal.local_id, next_sync_token.as_deref()).await?;
    }

    Ok(())
}

// ── Event CRUD ─────────────────────────────────────────────

/// Create a calendar event via the Google Calendar API.
///
/// Returns the created event.
pub async fn create_event(
    client: &GmailClient,
    calendar_remote_id: &str,
    event: &GoogleCalendarEvent,
    db: &DbState,
) -> Result<GoogleCalendarEvent, String> {
    let encoded = urlencoding::encode(calendar_remote_id);
    let url = format!("{CALENDAR_API_BASE}/calendars/{encoded}/events");
    client.post_absolute(&url, event, db).await
}

/// Update an existing calendar event.
///
/// Returns the updated event.
pub async fn update_event(
    client: &GmailClient,
    calendar_remote_id: &str,
    event_id: &str,
    event: &GoogleCalendarEvent,
    db: &DbState,
) -> Result<GoogleCalendarEvent, String> {
    let encoded_cal = urlencoding::encode(calendar_remote_id);
    let encoded_event = urlencoding::encode(event_id);
    let url = format!(
        "{CALENDAR_API_BASE}/calendars/{encoded_cal}/events/{encoded_event}"
    );
    client.put_absolute(&url, event, db).await
}

/// Delete a calendar event.
pub async fn delete_event(
    client: &GmailClient,
    calendar_remote_id: &str,
    event_id: &str,
    db: &DbState,
) -> Result<(), String> {
    let encoded_cal = urlencoding::encode(calendar_remote_id);
    let encoded_event = urlencoding::encode(event_id);
    let url = format!(
        "{CALENDAR_API_BASE}/calendars/{encoded_cal}/events/{encoded_event}"
    );
    client.delete_absolute(&url, db).await
}

// ── Full calendar sync entry point ─────────────────────────

/// Run a full calendar sync: list calendars, then sync events for each.
///
/// Called from the sync pipeline alongside email sync.
pub async fn sync_calendars(
    client: &GmailClient,
    account_id: &str,
    db: &DbState,
) -> Result<(), String> {
    let cals = sync_calendar_list(client, account_id, db).await?;

    for cal in &cals {
        if let Err(e) = sync_calendar_events(client, account_id, cal, db).await {
            log::error!(
                "Calendar event sync failed for {} ({}): {e}",
                cal.remote_id,
                cal.local_id,
            );
        }
    }

    Ok(())
}
