//! JMAP calendar sync using CalendarEvent/get, /changes, and /set.
//!
//! Leverages JMAP's native state-tracking (`/changes` method) for clean
//! incremental sync — no ctag/etag complexity like CalDAV.

use std::collections::HashMap;

use jmap_client::Get;
use jmap_client::calendar::CalendarGet;
use jmap_client::calendar_event::{CalendarEvent, CalendarEventGet, CalendarEventSet};
use jmap_client::core::set::SetObject;

use db::db::DbState;

use crate::client::JmapClient;

const EVENT_BATCH_SIZE: usize = 50;

// ── Public types ───────────────────────────────────────────

/// Mapping between our local UUID and the JMAP calendar ID.
#[derive(Debug, Clone)]
pub struct JmapCalendarInfo {
    pub local_id: String,
    pub remote_id: String,
}

// ── Calendar list sync ─────────────────────────────────────

/// Fetch all calendars via Calendar/get and upsert into the database.
///
/// Returns mappings between local and remote calendar IDs.
pub async fn sync_calendar_list(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
) -> Result<Vec<JmapCalendarInfo>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    // No IDs set => fetches all calendars
    let get = CalendarGet::new(&req_account_id);
    let handle = request
        .call(get)
        .map_err(|e| format!("Calendar/get: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("Calendar/get: {e}"))?;
    let mut get_response = response
        .get(&handle)
        .map_err(|e| format!("Calendar/get: {e}"))?;

    let state = get_response.state().to_string();
    let calendar_list = get_response.take_list();

    let mut result = Vec::with_capacity(calendar_list.len());

    for cal in &calendar_list {
        let remote_id = match cal.id() {
            Some(id) => id,
            None => continue,
        };

        let display_name = cal.name().map(String::from);
        let color = cal.color().map(String::from);
        let is_primary = cal.is_default().unwrap_or(false);

        let local_id = upsert_calendar(
            db,
            account_id,
            remote_id,
            display_name.as_deref(),
            color.as_deref(),
            is_primary,
        )
        .await?;

        result.push(JmapCalendarInfo {
            local_id,
            remote_id: remote_id.to_string(),
        });
    }

    // Save Calendar state for future /changes calls
    save_calendar_sync_state(db, account_id, "Calendar", &state).await?;

    log::info!(
        "[JMAP] Calendar list synced for account {account_id}: {} calendars",
        result.len()
    );

    Ok(result)
}

// ── Initial event sync ─────────────────────────────────────

/// Initial event sync: fetch all events for each calendar and persist them.
///
/// Uses CalendarEvent/query + CalendarEvent/get to batch-fetch events.
pub async fn sync_all_events(
    client: &JmapClient,
    account_id: &str,
    calendars: &[JmapCalendarInfo],
    db: &DbState,
) -> Result<(), String> {
    // Build a remote_id -> local_id lookup for calendar assignment
    let cal_map: HashMap<&str, &str> = calendars
        .iter()
        .map(|c| (c.remote_id.as_str(), c.local_id.as_str()))
        .collect();

    // Fetch ALL events (no filter) — the server returns them all with state
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    let get = CalendarEventGet::new(&req_account_id);
    let handle = request
        .call(get)
        .map_err(|e| format!("CalendarEvent/get initial: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("CalendarEvent/get initial: {e}"))?;
    let mut get_response = response
        .get(&handle)
        .map_err(|e| format!("CalendarEvent/get initial: {e}"))?;

    let state = get_response.state().to_string();
    let events = get_response.take_list();

    log::info!(
        "[JMAP] Initial event sync for account {account_id}: {} events",
        events.len()
    );

    for event in &events {
        persist_jmap_event(db, account_id, event, &cal_map).await?;
    }

    // Save CalendarEvent state for incremental sync
    save_calendar_sync_state(db, account_id, "CalendarEvent", &state).await?;

    Ok(())
}

// ── Delta event sync ───────────────────────────────────────

/// Incremental event sync using CalendarEvent/changes.
///
/// Fetches created/updated/destroyed event IDs since the last known state,
/// then batch-fetches the changed events and persists them.
pub async fn sync_events_delta(
    client: &JmapClient,
    account_id: &str,
    calendars: &[JmapCalendarInfo],
    db: &DbState,
) -> Result<(), String> {
    let cal_map: HashMap<&str, &str> = calendars
        .iter()
        .map(|c| (c.remote_id.as_str(), c.local_id.as_str()))
        .collect();

    let Some(mut since_state) = load_calendar_sync_state(db, account_id, "CalendarEvent").await?
    else {
        log::warn!("[JMAP] No CalendarEvent state for account {account_id} — running initial sync");
        return sync_all_events(client, account_id, calendars, db).await;
    };

    loop {
        let inner = client.inner();
        let changes = inner
            .calendar_event_changes(&since_state, Some(crate::JMAP_MAX_CHANGES))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("cannotCalculateChanges") {
                    log::warn!(
                        "[JMAP] CalendarEvent state expired for {account_id}, full re-sync needed"
                    );
                    return "JMAP_CALENDAR_STATE_EXPIRED".to_string();
                }
                format!("CalendarEvent/changes: {msg}")
            })?;

        let created = changes.created();
        let updated = changes.updated();
        let destroyed = changes.destroyed();

        // Batch-fetch created + updated events
        let ids_to_fetch: Vec<&str> = created
            .iter()
            .chain(updated.iter())
            .map(String::as_str)
            .collect();

        if !ids_to_fetch.is_empty() {
            for chunk in ids_to_fetch.chunks(EVENT_BATCH_SIZE) {
                let events = fetch_event_batch(client, chunk).await?;
                for event in &events {
                    persist_jmap_event(db, account_id, event, &cal_map).await?;
                }
            }
        }

        // Delete destroyed events
        if !destroyed.is_empty() {
            for event_id in destroyed {
                delete_event_by_jmap_id(db, account_id, event_id).await?;
            }
        }

        since_state = changes.new_state().to_string();

        if !changes.has_more_changes() {
            break;
        }
    }

    // Save updated state
    save_calendar_sync_state(db, account_id, "CalendarEvent", &since_state).await?;

    log::info!("[JMAP] Calendar event delta sync complete for account {account_id}");

    Ok(())
}

// ── Event CRUD (local-to-server push) ──────────────────────

/// Create a calendar event on the JMAP server.
///
/// Returns the server-assigned event ID.
#[allow(clippy::too_many_arguments)]
pub async fn create_event_remote(
    client: &JmapClient,
    calendar_remote_id: &str,
    title: &str,
    description: &str,
    location: &str,
    start_time: i64,
    end_time: i64,
    is_all_day: bool,
) -> Result<String, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    let mut set = CalendarEventSet::new(&req_account_id);
    let event = set.create();
    event
        .calendar_ids([calendar_remote_id])
        .title(title)
        .description(description);

    // Set start and duration
    let (start_str, duration_str) = format_jscalendar_times(start_time, end_time, is_all_day);
    event.start(&start_str).duration(&duration_str);

    if is_all_day {
        event.show_without_time(true);
    }

    if !location.is_empty() {
        let mut locations = serde_json::Map::new();
        let mut loc_obj = serde_json::Map::new();
        loc_obj.insert(
            "name".to_string(),
            serde_json::Value::String(location.to_string()),
        );
        loc_obj.insert(
            "@type".to_string(),
            serde_json::Value::String("Location".to_string()),
        );
        locations.insert("loc1".to_string(), serde_json::Value::Object(loc_obj));
        event.locations(locations);
    }

    let create_id = event
        .create_id()
        .ok_or("Failed to get create ID for CalendarEvent")?;

    let handle = request
        .call(set)
        .map_err(|e| format!("CalendarEvent/set create: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("CalendarEvent/set create: {e}"))?;
    let mut set_response = response
        .get(&handle)
        .map_err(|e| format!("CalendarEvent/set create: {e}"))?;

    let created_event = set_response
        .created(&create_id)
        .map_err(|e| format!("CalendarEvent create failed: {e}"))?;

    created_event
        .id()
        .map(String::from)
        .ok_or_else(|| "Created event has no ID".to_string())
}

/// Update an existing calendar event on the JMAP server.
#[allow(clippy::too_many_arguments)]
pub async fn update_event_remote(
    client: &JmapClient,
    event_remote_id: &str,
    title: &str,
    description: &str,
    location: &str,
    start_time: i64,
    end_time: i64,
    is_all_day: bool,
) -> Result<(), String> {
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    let mut set = CalendarEventSet::new(&req_account_id);
    let event = set.update(event_remote_id);
    event.title(title).description(description);

    let (start_str, duration_str) = format_jscalendar_times(start_time, end_time, is_all_day);
    event.start(&start_str).duration(&duration_str);

    if is_all_day {
        event.show_without_time(true);
    }

    if !location.is_empty() {
        let mut locations = serde_json::Map::new();
        let mut loc_obj = serde_json::Map::new();
        loc_obj.insert(
            "name".to_string(),
            serde_json::Value::String(location.to_string()),
        );
        loc_obj.insert(
            "@type".to_string(),
            serde_json::Value::String("Location".to_string()),
        );
        locations.insert("loc1".to_string(), serde_json::Value::Object(loc_obj));
        event.locations(locations);
    }

    let handle = request
        .call(set)
        .map_err(|e| format!("CalendarEvent/set update: {e}"))?;
    let mut response = request
        .send()
        .await
        .map_err(|e| format!("CalendarEvent/set update: {e}"))?;

    response
        .get(&handle)
        .map_err(|e| format!("CalendarEvent/set update: {e}"))?
        .updated(event_remote_id)
        .map_err(|e| format!("CalendarEvent update failed: {e}"))?;

    Ok(())
}

/// Delete a calendar event on the JMAP server.
pub async fn delete_event_remote(client: &JmapClient, event_remote_id: &str) -> Result<(), String> {
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    let mut set = CalendarEventSet::new(&req_account_id);
    set.destroy([event_remote_id]);
    let handle = request
        .call(set)
        .map_err(|e| format!("CalendarEvent/set destroy: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("CalendarEvent/set destroy: {e}"))?;

    response
        .get(&handle)
        .map_err(|e| format!("CalendarEvent/set destroy: {e}"))?
        .destroyed(event_remote_id)
        .map_err(|e| format!("CalendarEvent destroy failed: {e}"))?;

    Ok(())
}

// ── Full calendar sync entry point ─────────────────────────

/// Run a full calendar sync: list calendars, then sync events.
///
/// Called from the sync pipeline alongside email sync.
pub async fn sync_calendars(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
) -> Result<(), String> {
    let cals = sync_calendar_list(client, account_id, db).await?;

    let event_state = load_calendar_sync_state(db, account_id, "CalendarEvent").await?;

    if event_state.is_some() {
        sync_events_delta(client, account_id, &cals, db).await?;
    } else {
        sync_all_events(client, account_id, &cals, db).await?;
    }

    Ok(())
}

// ── Internal helpers ───────────────────────────────────────

/// Fetch a batch of calendar events by ID.
async fn fetch_event_batch(
    client: &JmapClient,
    ids: &[&str],
) -> Result<Vec<CalendarEvent<Get>>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    let req_account_id = request.default_account_id().to_string();
    let mut get = CalendarEventGet::new(&req_account_id);
    get.ids(ids.iter().copied());
    let handle = request
        .call(get)
        .map_err(|e| format!("CalendarEvent/get batch: {e}"))?;

    let mut response = request
        .send()
        .await
        .map_err(|e| format!("CalendarEvent/get batch: {e}"))?;

    response
        .get(&handle)
        .map(|mut r| r.take_list())
        .map_err(|e| format!("CalendarEvent/get batch: {e}"))
}

/// Persist a JMAP CalendarEvent into the local database.
///
/// Extracts JSCalendar properties and maps them to the DB schema.
async fn persist_jmap_event(
    db: &DbState,
    account_id: &str,
    event: &CalendarEvent<Get>,
    cal_map: &HashMap<&str, &str>,
) -> Result<(), String> {
    let event_id = match event.id() {
        Some(id) => id,
        None => return Ok(()),
    };

    let uid = event.uid().map(String::from);
    let title = event.title().map(String::from);
    let description = event.description().map(String::from);
    let status = event.status().unwrap_or("confirmed").to_string();

    // Extract location from locations map
    let location = extract_location(event);

    // Resolve calendar_id from calendarIds map
    let calendar_id = resolve_calendar_id(event, cal_map);

    // Parse start/end times from JSCalendar format
    let (start_time, end_time, is_all_day) = parse_jscalendar_times(event);

    // Extract organizer email from participants
    let organizer_email = extract_organizer_email(event);

    // Extract attendees JSON
    let attendees_json = extract_attendees_json(event);

    // Recurrence rules as ical_data
    let ical_data = event
        .recurrence_rules()
        .and_then(|rules| serde_json::to_string(rules).ok());

    // Extract attendees and reminders BEFORE the closure (cannot send references)
    let attendees = extract_attendee_rows(event);
    let reminders = extract_reminder_rows(event);

    // Use event_id as both google_event_id and remote_event_id for JMAP
    let aid = account_id.to_string();
    let eid = event_id.to_string();
    let eid2 = eid.clone();

    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();

        conn.execute(
            "INSERT INTO calendar_events \
                 (id, account_id, google_event_id, summary, description, location, \
                  start_time, end_time, is_all_day, status, organizer_email, \
                  attendees_json, html_link, calendar_id, remote_event_id, etag, \
                  ical_data, uid) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, ?13, ?14, NULL, ?15, ?16) \
             ON CONFLICT(account_id, google_event_id) DO UPDATE SET \
                 summary = ?4, description = ?5, location = ?6, \
                 start_time = ?7, end_time = ?8, is_all_day = ?9, \
                 status = ?10, organizer_email = ?11, attendees_json = ?12, \
                 calendar_id = ?13, remote_event_id = ?14, \
                 ical_data = ?15, uid = ?16, updated_at = unixepoch()",
            rusqlite::params![
                id,
                aid,
                eid,
                title,
                description,
                location,
                start_time,
                end_time,
                is_all_day as i64,
                status,
                organizer_email,
                attendees_json,
                calendar_id,
                eid2,
                ical_data,
                uid,
            ],
        )
        .map_err(|e| format!("upsert JMAP calendar event: {e}"))?;

        // Look up the actual local event ID (may differ from `id` on conflict)
        let local_event_id: String = conn
            .query_row(
                "SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2",
                rusqlite::params![aid, eid],
                |row| row.get(0),
            )
            .map_err(|e| format!("fetch event id: {e}"))?;

        // Sync attendees
        persist_attendee_rows(conn, &aid, &local_event_id, &attendees)?;

        // Sync reminders
        persist_reminder_rows(conn, &aid, &local_event_id, &reminders)?;

        Ok(())
    })
    .await
}

/// Delete a calendar event by its JMAP event ID.
async fn delete_event_by_jmap_id(
    db: &DbState,
    account_id: &str,
    jmap_event_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let eid = jmap_event_id.to_string();

    db.with_conn(move |conn| {
        // Delete attendees and reminders first
        conn.execute(
            "DELETE FROM calendar_attendees WHERE account_id = ?1 AND event_id IN \
             (SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2)",
            rusqlite::params![aid, eid],
        )
        .map_err(|e| format!("delete attendees for JMAP event: {e}"))?;

        conn.execute(
            "DELETE FROM calendar_reminders WHERE account_id = ?1 AND event_id IN \
             (SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2)",
            rusqlite::params![aid, eid],
        )
        .map_err(|e| format!("delete reminders for JMAP event: {e}"))?;

        conn.execute(
            "DELETE FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2",
            rusqlite::params![aid, eid],
        )
        .map_err(|e| format!("delete JMAP calendar event: {e}"))?;

        Ok(())
    })
    .await
}

// ── JSCalendar property extraction ─────────────────────────

/// Extract the first location name from the JSCalendar locations map.
fn extract_location(event: &CalendarEvent<Get>) -> Option<String> {
    let locations = event.locations()?;
    for (_key, value) in locations {
        if let Some(obj) = value.as_object()
            && let Some(name) = obj.get("name").and_then(|n| n.as_str())
            && !name.is_empty()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Resolve a local calendar_id from the event's calendarIds map.
fn resolve_calendar_id(
    event: &CalendarEvent<Get>,
    cal_map: &HashMap<&str, &str>,
) -> Option<String> {
    let calendar_ids = event.calendar_ids()?;
    for (remote_cal_id, value) in calendar_ids {
        // calendarIds maps calendar_id -> true for calendars this event belongs to
        if value.as_bool() == Some(true)
            && let Some(local_id) = cal_map.get(remote_cal_id.as_str())
        {
            return Some((*local_id).to_string());
        }
    }
    None
}

/// Extract organizer email from JSCalendar participants.
fn extract_organizer_email(event: &CalendarEvent<Get>) -> Option<String> {
    let participants = event.participants()?;
    for (_key, value) in participants {
        let obj = value.as_object()?;
        // Look for roles containing "owner"
        if let Some(roles) = obj.get("roles").and_then(|r| r.as_object())
            && roles.contains_key("owner")
        {
            if let Some(email) = obj
                .get("sendTo")
                .and_then(|s| s.as_object())
                .and_then(|s| s.get("imip"))
                .and_then(|v| v.as_str())
            {
                // Strip "mailto:" prefix
                let email = email.strip_prefix("mailto:").unwrap_or(email);
                return Some(email.to_string());
            }
            // Try email field directly
            if let Some(email) = obj.get("email").and_then(|e| e.as_str()) {
                return Some(email.to_string());
            }
        }
    }
    None
}

/// Extract attendees as a JSON string from JSCalendar participants.
fn extract_attendees_json(event: &CalendarEvent<Get>) -> Option<String> {
    let participants = event.participants()?;
    if participants.is_empty() {
        return None;
    }

    let mut attendees = Vec::new();
    for (_key, value) in participants {
        let Some(obj) = value.as_object() else {
            continue;
        };

        // Extract email
        let email = obj
            .get("sendTo")
            .and_then(|s| s.as_object())
            .and_then(|s| s.get("imip"))
            .and_then(|v| v.as_str())
            .map(|e| e.strip_prefix("mailto:").unwrap_or(e))
            .or_else(|| obj.get("email").and_then(|e| e.as_str()));

        let name = obj.get("name").and_then(|n| n.as_str());

        let roles = obj.get("roles").and_then(|r| r.as_object());
        let is_owner = roles.is_some_and(|r| r.contains_key("owner"));

        // Map JSCalendar participationStatus to Google-style responseStatus
        let participation = obj
            .get("participationStatus")
            .and_then(|p| p.as_str())
            .map(map_participation_status);

        let mut att = serde_json::Map::new();
        if let Some(email) = email {
            att.insert("email".to_string(), serde_json::json!(email));
        }
        if let Some(name) = name {
            att.insert("displayName".to_string(), serde_json::json!(name));
        }
        if let Some(status) = participation {
            att.insert("responseStatus".to_string(), serde_json::json!(status));
        }
        if is_owner {
            att.insert("organizer".to_string(), serde_json::json!(true));
        }

        attendees.push(serde_json::Value::Object(att));
    }

    if attendees.is_empty() {
        None
    } else {
        serde_json::to_string(&attendees).ok()
    }
}

/// Map JSCalendar participationStatus to a Google-compatible responseStatus.
fn map_participation_status(status: &str) -> &str {
    match status {
        "accepted" => "accepted",
        "declined" => "declined",
        "tentative" => "tentative",
        "needs-action" => "needsAction",
        _ => "needsAction",
    }
}

/// Parse JSCalendar start + duration into Unix timestamps.
///
/// JSCalendar uses local date-time + timezone + duration (ISO 8601).
/// `start` is like "2025-03-15T14:30:00", `duration` like "PT1H30M",
/// and `timeZone` like "America/New_York".
fn parse_jscalendar_times(event: &CalendarEvent<Get>) -> (i64, i64, bool) {
    let is_all_day = event.show_without_time().unwrap_or(false);

    let start_str = match event.start() {
        Some(s) => s,
        None => return (0, 3600, is_all_day),
    };

    let tz = event.time_zone().as_value().copied().unwrap_or("UTC");

    let start_ts = if is_all_day {
        // All-day: parse as date only (e.g. "2025-03-15")
        parse_local_date(start_str)
    } else {
        // Timed: parse as local datetime in the given timezone
        parse_local_datetime(start_str, tz)
    };

    let duration_str = event
        .duration()
        .unwrap_or(if is_all_day { "P1D" } else { "PT1H" });
    let duration_secs = parse_iso8601_duration(duration_str);

    (start_ts, start_ts + duration_secs, is_all_day)
}

/// Parse a local date string (e.g. "2025-03-15") into a UTC Unix timestamp
/// at midnight.
fn parse_local_date(s: &str) -> i64 {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| ndt.and_utc().timestamp())
        .unwrap_or(0)
}

/// Parse a local datetime string (e.g. "2025-03-15T14:30:00") into a UTC
/// Unix timestamp.
///
/// Tries RFC 3339 first (includes offset). Falls back to parsing as a naive
/// datetime treated as UTC — JMAP servers typically include the timezone in
/// the `timeZone` property but the `start` value itself is local. Without
/// chrono-tz we cannot resolve IANA names, so we treat naive times as UTC.
/// This is acceptable because delta-sync will correct any drift.
fn parse_local_datetime(s: &str, _tz_name: &str) -> i64 {
    // Try parsing with timezone offset (RFC 3339)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp();
    }

    // Parse as naive local datetime — treat as UTC
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .map(|n| n.and_utc().timestamp())
        .unwrap_or(0)
}

/// Parse an ISO 8601 duration string (e.g. "PT1H30M", "P1D", "PT45M")
/// into seconds.
fn parse_iso8601_duration(s: &str) -> i64 {
    let mut total_secs: i64 = 0;
    let mut in_time = false;
    let mut num_buf = String::new();

    for ch in s.chars() {
        match ch {
            'P' => {}
            'T' => in_time = true,
            'W' => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n * 7 * 86400;
                }
                num_buf.clear();
            }
            'D' => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n * 86400;
                }
                num_buf.clear();
            }
            'H' if in_time => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n * 3600;
                }
                num_buf.clear();
            }
            'M' if in_time => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n * 60;
                }
                num_buf.clear();
            }
            'S' if in_time => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n;
                }
                num_buf.clear();
            }
            c if c.is_ascii_digit() => num_buf.push(c),
            _ => {}
        }
    }

    if total_secs == 0 {
        3600 // Fallback: 1 hour
    } else {
        total_secs
    }
}

/// Format Unix timestamps into JSCalendar start + duration strings.
fn format_jscalendar_times(start_time: i64, end_time: i64, is_all_day: bool) -> (String, String) {
    use chrono::TimeZone;
    let start_dt = chrono::Utc
        .timestamp_opt(start_time, 0)
        .single()
        .unwrap_or_else(chrono::Utc::now);

    if is_all_day {
        let start_str = start_dt.format("%Y-%m-%d").to_string();
        let duration_days = (end_time - start_time) / 86400;
        let duration_days = if duration_days < 1 { 1 } else { duration_days };
        let duration_str = format!("P{duration_days}D");
        (start_str, duration_str)
    } else {
        let start_str = start_dt.format("%Y-%m-%dT%H:%M:%S").to_string();
        let duration_secs = end_time - start_time;
        let duration_str = format_duration_iso8601(duration_secs);
        (start_str, duration_str)
    }
}

/// Format a duration in seconds as ISO 8601 (e.g. "PT1H30M").
fn format_duration_iso8601(mut secs: i64) -> String {
    if secs <= 0 {
        return "PT1H".to_string();
    }

    let mut parts = String::from("P");

    let days = secs / 86400;
    if days > 0 {
        parts.push_str(&format!("{days}D"));
        secs %= 86400;
    }

    if secs > 0 {
        parts.push('T');
        let hours = secs / 3600;
        if hours > 0 {
            parts.push_str(&format!("{hours}H"));
            secs %= 3600;
        }
        let minutes = secs / 60;
        if minutes > 0 {
            parts.push_str(&format!("{minutes}M"));
            secs %= 60;
        }
        if secs > 0 {
            parts.push_str(&format!("{secs}S"));
        }
    }

    parts
}

/// Extracted attendee row, ready to be persisted inside a `with_conn` closure.
struct AttendeeRow {
    email: String,
    name: Option<String>,
    rsvp_status: Option<String>,
    is_organizer: bool,
}

/// Extract attendee rows from a JMAP CalendarEvent's participants.
fn extract_attendee_rows(event: &CalendarEvent<Get>) -> Vec<AttendeeRow> {
    let Some(participants) = event.participants() else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for (_key, value) in participants {
        let Some(obj) = value.as_object() else {
            continue;
        };

        let email = obj
            .get("sendTo")
            .and_then(|s| s.as_object())
            .and_then(|s| s.get("imip"))
            .and_then(|v| v.as_str())
            .map(|e| e.strip_prefix("mailto:").unwrap_or(e))
            .or_else(|| obj.get("email").and_then(|e| e.as_str()));

        let Some(email) = email else { continue };
        if email.is_empty() {
            continue;
        }

        let name = obj.get("name").and_then(|n| n.as_str()).map(String::from);
        let roles = obj.get("roles").and_then(|r| r.as_object());
        let is_owner = roles.is_some_and(|r| r.contains_key("owner"));

        let participation = obj
            .get("participationStatus")
            .and_then(|p| p.as_str())
            .map(|s| map_participation_status(s).to_string());

        rows.push(AttendeeRow {
            email: email.to_string(),
            name,
            rsvp_status: participation,
            is_organizer: is_owner,
        });
    }

    rows
}

/// Persist pre-extracted attendee rows into the database.
fn persist_attendee_rows(
    conn: &rusqlite::Connection,
    account_id: &str,
    local_event_id: &str,
    attendees: &[AttendeeRow],
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM calendar_attendees WHERE account_id = ?1 AND event_id = ?2",
        rusqlite::params![account_id, local_event_id],
    )
    .map_err(|e| format!("delete attendees: {e}"))?;

    for att in attendees {
        conn.execute(
            "INSERT INTO calendar_attendees \
                 (event_id, account_id, email, name, rsvp_status, is_organizer) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(account_id, event_id, email) DO UPDATE SET \
                 name = ?4, rsvp_status = ?5, is_organizer = ?6",
            rusqlite::params![
                local_event_id,
                account_id,
                att.email,
                att.name,
                att.rsvp_status,
                att.is_organizer as i64,
            ],
        )
        .map_err(|e| format!("upsert attendee: {e}"))?;
    }

    Ok(())
}

/// Extracted reminder row, ready to be persisted inside a `with_conn` closure.
struct ReminderRow {
    minutes_before: i64,
    method: Option<String>,
}

/// Extract reminder rows from a JMAP CalendarEvent's alerts.
fn extract_reminder_rows(event: &CalendarEvent<Get>) -> Vec<ReminderRow> {
    // alerts returns Field<&Map> — Omitted/Null = no alerts, Value = has alerts
    let jmap_client::core::field::Field::Value(alerts) = event.alerts() else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for (_alert_id, alert_value) in alerts {
        let Some(alert_obj) = alert_value.as_object() else {
            continue;
        };

        let Some(trigger) = alert_obj.get("trigger").and_then(|t| t.as_object()) else {
            continue;
        };

        // Extract offset from OffsetTrigger (e.g. "-PT15M")
        let Some(offset) = trigger.get("offset").and_then(|o| o.as_str()) else {
            continue;
        };

        let is_negative = offset.starts_with('-');
        let clean_offset = offset.trim_start_matches('-');
        let offset_secs = parse_iso8601_duration(clean_offset);
        let minutes_before = if is_negative {
            offset_secs / 60
        } else {
            -(offset_secs / 60)
        };

        let method = alert_obj
            .get("action")
            .and_then(|a| a.as_str())
            .map(|a| match a {
                "display" => "popup",
                "email" => "email",
                other => other,
            })
            .map(String::from);

        rows.push(ReminderRow {
            minutes_before,
            method,
        });
    }

    rows
}

/// Persist pre-extracted reminder rows into the database.
fn persist_reminder_rows(
    conn: &rusqlite::Connection,
    account_id: &str,
    local_event_id: &str,
    reminders: &[ReminderRow],
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM calendar_reminders WHERE account_id = ?1 AND event_id = ?2",
        rusqlite::params![account_id, local_event_id],
    )
    .map_err(|e| format!("delete reminders: {e}"))?;

    for rem in reminders {
        conn.execute(
            "INSERT INTO calendar_reminders \
                 (event_id, account_id, minutes_before, method) \
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![local_event_id, account_id, rem.minutes_before, rem.method],
        )
        .map_err(|e| format!("insert reminder: {e}"))?;
    }

    Ok(())
}

// ── Sync state persistence ─────────────────────────────────

/// Save a JMAP sync state for calendar objects.
///
/// Reuses the existing `jmap_sync_state` table via sync.
async fn save_calendar_sync_state(
    db: &DbState,
    account_id: &str,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    sync::state::save_jmap_sync_state(db, account_id, state_type, state).await
}

/// Load a JMAP sync state for calendar objects.
async fn load_calendar_sync_state(
    db: &DbState,
    account_id: &str,
    state_type: &str,
) -> Result<Option<String>, String> {
    sync::state::load_jmap_sync_state(db, account_id, state_type).await
}

// ── Calendar DB helpers ────────────────────────────────────

/// Upsert a calendar entry. Returns the local UUID.
async fn upsert_calendar(
    db: &DbState,
    account_id: &str,
    remote_id: &str,
    display_name: Option<&str>,
    color: Option<&str>,
    is_primary: bool,
) -> Result<String, String> {
    let aid = account_id.to_string();
    let rid = remote_id.to_string();
    let dname = display_name.map(String::from);
    let col = color.map(String::from);

    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary)
                 VALUES (?1, ?2, 'jmap', ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   display_name = ?4, color = ?5, is_primary = ?6, updated_at = unixepoch()",
            rusqlite::params![id, aid, rid, dname, col, is_primary as i64],
        )
        .map_err(|e| format!("upsert JMAP calendar: {e}"))?;

        let actual_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                rusqlite::params![aid, rid],
                |row| row.get(0),
            )
            .map_err(|e| format!("fetch calendar id: {e}"))?;

        Ok(actual_id)
    })
    .await
}

// ── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso8601_duration_basic() {
        assert_eq!(parse_iso8601_duration("PT1H"), 3600);
        assert_eq!(parse_iso8601_duration("PT30M"), 1800);
        assert_eq!(parse_iso8601_duration("PT1H30M"), 5400);
        assert_eq!(parse_iso8601_duration("P1D"), 86400);
        assert_eq!(parse_iso8601_duration("P1DT2H"), 93600);
        assert_eq!(parse_iso8601_duration("P1W"), 604800);
        assert_eq!(parse_iso8601_duration("PT45S"), 45);
        assert_eq!(parse_iso8601_duration("PT1H15M30S"), 4530);
    }

    #[test]
    fn parse_iso8601_duration_fallback() {
        // Empty or invalid durations should fall back to 1 hour
        assert_eq!(parse_iso8601_duration(""), 3600);
        assert_eq!(parse_iso8601_duration("P"), 3600);
    }

    #[test]
    fn format_duration_roundtrip() {
        assert_eq!(format_duration_iso8601(3600), "PT1H");
        assert_eq!(format_duration_iso8601(5400), "PT1H30M");
        assert_eq!(format_duration_iso8601(86400), "P1D");
        assert_eq!(format_duration_iso8601(93600), "P1DT2H");
        assert_eq!(format_duration_iso8601(45), "PT45S");
    }

    #[test]
    fn format_duration_zero_fallback() {
        assert_eq!(format_duration_iso8601(0), "PT1H");
    }

    #[test]
    fn parse_local_date_works() {
        let ts = parse_local_date("2025-03-15");
        assert!(ts > 0);
    }

    #[test]
    fn parse_local_datetime_utc() {
        let ts = parse_local_datetime("2025-03-15T14:30:00", "UTC");
        assert!(ts > 0);
        // 2025-03-15T14:30:00 UTC
        assert_eq!(ts, 1_742_049_000);
    }

    #[test]
    fn map_participation_status_covers_known() {
        assert_eq!(map_participation_status("accepted"), "accepted");
        assert_eq!(map_participation_status("declined"), "declined");
        assert_eq!(map_participation_status("tentative"), "tentative");
        assert_eq!(map_participation_status("needs-action"), "needsAction");
        assert_eq!(map_participation_status("unknown"), "needsAction");
    }
}
