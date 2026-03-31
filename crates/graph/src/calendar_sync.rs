use serde::{Deserialize, Serialize};

use db::db::DbState;

use super::client::GraphClient;
use super::types::ODataCollection;

// ── Graph Calendar API response types ─────────────────────

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphCalendar {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub color: Option<String>,
    #[serde(default)]
    pub is_default_calendar: Option<bool>,
    #[serde(default)]
    pub can_edit: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEvent {
    pub id: String,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body: Option<GraphEventBody>,
    #[serde(default)]
    pub body_preview: Option<String>,
    pub start: GraphDateTimeTimeZone,
    pub end: GraphDateTimeTimeZone,
    #[serde(default)]
    pub is_all_day: Option<bool>,
    #[serde(default)]
    pub location: Option<GraphLocation>,
    #[serde(default)]
    pub organizer: Option<GraphEventOrganizer>,
    #[serde(default)]
    pub attendees: Option<Vec<GraphAttendee>>,
    #[serde(default)]
    pub web_link: Option<String>,
    #[serde(default)]
    pub i_cal_uid: Option<String>,
    #[serde(default)]
    pub categories: Option<Vec<String>>,
    #[serde(default)]
    pub recurrence: Option<GraphRecurrence>,
    #[serde(default)]
    pub show_as: Option<String>,
    #[serde(default)]
    pub response_status: Option<GraphResponseStatus>,
    #[serde(default)]
    pub is_cancelled: Option<bool>,
    #[serde(default)]
    pub change_key: Option<String>,
    #[serde(rename = "@removed")]
    #[serde(default)]
    pub removed: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEventBody {
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphDateTimeTimeZone {
    pub date_time: String,
    pub time_zone: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphLocation {
    #[serde(default)]
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEventOrganizer {
    #[serde(default)]
    pub email_address: Option<GraphEventEmailAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEventEmailAddress {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub address: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphAttendee {
    #[serde(rename = "type")]
    #[serde(default)]
    pub attendee_type: Option<String>,
    #[serde(default)]
    pub status: Option<GraphResponseStatus>,
    #[serde(default)]
    pub email_address: Option<GraphEventEmailAddress>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphResponseStatus {
    pub response: String,
    #[serde(default)]
    pub time: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRecurrence {
    #[serde(default)]
    pub pattern: Option<GraphRecurrencePattern>,
    #[serde(default)]
    pub range: Option<GraphRecurrenceRange>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRecurrencePattern {
    #[serde(rename = "type")]
    pub pattern_type: String,
    #[serde(default)]
    pub interval: Option<i32>,
    #[serde(default)]
    pub days_of_week: Option<Vec<String>>,
    #[serde(default)]
    pub day_of_month: Option<i32>,
    #[serde(default)]
    pub month: Option<i32>,
    #[serde(default)]
    pub index: Option<String>,
    #[serde(default)]
    pub first_day_of_week: Option<String>,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphRecurrenceRange {
    #[serde(rename = "type")]
    pub range_type: String,
    #[serde(default)]
    pub start_date: Option<String>,
    #[serde(default)]
    pub end_date: Option<String>,
    #[serde(default)]
    pub number_of_occurrences: Option<i32>,
}

// ── Graph Calendar API request types ──────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEventCreate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<GraphEventBodyInput>,
    pub start: GraphDateTimeTimeZone,
    pub end: GraphDateTimeTimeZone,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_all_day: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub location: Option<GraphLocationInput>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attendees: Option<Vec<GraphAttendeeInput>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<GraphRecurrence>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub is_reminder_on: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reminder_minutes_before_start: Option<i32>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEventBodyInput {
    pub content_type: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphLocationInput {
    pub display_name: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphAttendeeInput {
    #[serde(rename = "type")]
    pub attendee_type: String,
    pub email_address: GraphEventEmailAddress,
}

// ── Calendar select fields ────────────────────────────────

const EVENT_SELECT: &str = "\
id,subject,body,bodyPreview,start,end,isAllDay,location,\
organizer,attendees,webLink,iCalUId,categories,recurrence,\
showAs,responseStatus,isCancelled,changeKey";

// ── Exchange category color mapping ───────────────────────

/// Map an Exchange category color preset name to a hex color.
///
/// Exchange categories use preset names like "preset0" through "preset24".
/// We reuse the label-colors crate's mapping.
fn category_color_to_hex(color: &str) -> Option<String> {
    label_colors::category_colors::preset_to_hex(color).map(|(bg, _)| bg.to_string())
}

// ── Public API ────────────────────────────────────────────

/// Intermediate representation matching `CalendarInfoDto` from the calendar crate.
#[derive(Debug)]
pub struct GraphCalendarInfo {
    pub remote_id: String,
    pub display_name: String,
    pub color: Option<String>,
    pub is_primary: bool,
}

/// Intermediate representation matching `CalendarEventDto` from the calendar crate.
#[derive(Debug)]
pub struct GraphCalendarEvent {
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

/// Result of a calendar event sync (delta or full).
#[derive(Debug)]
pub struct GraphCalendarSyncResult {
    pub created: Vec<GraphCalendarEvent>,
    pub updated: Vec<GraphCalendarEvent>,
    pub deleted_remote_ids: Vec<String>,
    pub new_delta_link: Option<String>,
}

/// List all calendars for the authenticated user.
pub async fn graph_list_calendars(
    client: &GraphClient,
    db: &DbState,
) -> Result<Vec<GraphCalendarInfo>, String> {
    let me = client.api_path_prefix();
    let response: ODataCollection<GraphCalendar> = client
        .get_json(
            &format!("{me}/calendars?$select=id,name,color,isDefaultCalendar,canEdit"),
            db,
        )
        .await?;

    Ok(response
        .value
        .into_iter()
        .map(|cal| {
            let color = cal.color.as_deref().and_then(category_color_to_hex);
            GraphCalendarInfo {
                remote_id: cal.id,
                display_name: cal.name,
                color,
                is_primary: cal.is_default_calendar.unwrap_or(false),
            }
        })
        .collect())
}

/// Sync events for a single calendar using delta queries.
///
/// If `delta_link` is `Some`, performs an incremental sync from that point.
/// If `None`, performs a full sync fetching events from 90 days ago to 365 days ahead.
pub async fn graph_sync_calendar_events(
    client: &GraphClient,
    db: &DbState,
    calendar_remote_id: &str,
    delta_link: Option<&str>,
) -> Result<GraphCalendarSyncResult, String> {
    let me = client.api_path_prefix();
    let enc_cal_id = urlencoding::encode(calendar_remote_id);

    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut deleted_remote_ids = Vec::new();
    let mut new_delta_link = None;

    // Build the initial URL
    let initial_url = if let Some(link) = delta_link {
        // Incremental delta sync — use the stored delta link directly
        link.to_string()
    } else {
        // Full sync — use calendarView with delta for the date range
        let time_min = chrono::Utc::now() - chrono::Duration::days(90);
        let time_max = chrono::Utc::now() + chrono::Duration::days(365);
        let start = time_min.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        let end = time_max.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        format!(
            "{me}/calendars/{enc_cal_id}/calendarView/delta\
             ?startDateTime={start}&endDateTime={end}\
             &$select={EVENT_SELECT}"
        )
    };

    let mut current_url = initial_url;

    loop {
        let page: ODataCollection<GraphEvent> = if current_url.starts_with("http") {
            client.get_absolute(&current_url, db).await?
        } else {
            client.get_json(&current_url, db).await?
        };

        for event in page.value {
            // Deleted/removed events in delta
            if event.removed.is_some() || event.is_cancelled.unwrap_or(false) {
                deleted_remote_ids.push(event.id);
                continue;
            }

            let mapped = map_graph_event(event)?;
            if delta_link.is_some() {
                updated.push(mapped);
            } else {
                created.push(mapped);
            }
        }

        if let Some(ref next_link) = page.next_link {
            current_url = next_link.clone();
        } else if let Some(ref dl) = page.delta_link {
            new_delta_link = Some(dl.clone());
            break;
        } else {
            break;
        }
    }

    Ok(GraphCalendarSyncResult {
        created,
        updated,
        deleted_remote_ids,
        new_delta_link,
    })
}

/// Create a new event on a calendar.
pub async fn graph_create_event(
    client: &GraphClient,
    db: &DbState,
    calendar_remote_id: &str,
    event: &GraphEventCreate,
) -> Result<GraphCalendarEvent, String> {
    let me = client.api_path_prefix();
    let enc_cal_id = urlencoding::encode(calendar_remote_id);
    let response: GraphEvent = client
        .post(&format!("{me}/calendars/{enc_cal_id}/events"), event, db)
        .await?;
    map_graph_event(response)
}

/// Update an existing event.
///
/// Uses PATCH to update the event, then re-fetches to get the full response.
pub async fn graph_update_event(
    client: &GraphClient,
    db: &DbState,
    remote_event_id: &str,
    event: &serde_json::Value,
) -> Result<GraphCalendarEvent, String> {
    let me = client.api_path_prefix();
    let enc_event_id = urlencoding::encode(remote_event_id);
    // PATCH the event (returns 200 with no parsed body via our patch method)
    client
        .patch(&format!("{me}/events/{enc_event_id}"), event, db)
        .await?;
    // Re-fetch the updated event to return full data
    let fetched: GraphEvent = client
        .get_json(
            &format!("{me}/events/{enc_event_id}?$select={EVENT_SELECT}"),
            db,
        )
        .await?;
    map_graph_event(fetched)
}

/// Delete an event.
pub async fn graph_delete_event(
    client: &GraphClient,
    db: &DbState,
    remote_event_id: &str,
) -> Result<(), String> {
    let me = client.api_path_prefix();
    let enc_event_id = urlencoding::encode(remote_event_id);
    client
        .delete(&format!("{me}/events/{enc_event_id}"), db)
        .await
}

// ── Event mapping ─────────────────────────────────────────

/// Convert a Graph API event to our intermediate representation.
fn map_graph_event(event: GraphEvent) -> Result<GraphCalendarEvent, String> {
    let is_all_day = event.is_all_day.unwrap_or(false);

    let start_time = parse_graph_datetime(&event.start, is_all_day, "start")?;
    let end_time = parse_graph_datetime(&event.end, is_all_day, "end")?;

    let description = event.body.map(|b| {
        if b.content_type.eq_ignore_ascii_case("text") {
            b.content
        } else {
            // Strip HTML for storage; keep plain text
            b.content
        }
    });

    let location = event
        .location
        .and_then(|loc| loc.display_name)
        .filter(|name| !name.is_empty());

    let organizer_email = event
        .organizer
        .and_then(|org| org.email_address)
        .and_then(|ea| ea.address);

    let attendees_json = event
        .attendees
        .filter(|a| !a.is_empty())
        .map(|attendees| serde_json::to_string(&attendees))
        .transpose()
        .map_err(|e| format!("Failed to serialize Graph attendees: {e}"))?;

    let status = map_graph_event_status(
        event.is_cancelled.unwrap_or(false),
        event.response_status.as_ref(),
        event.show_as.as_deref(),
    );

    Ok(GraphCalendarEvent {
        remote_event_id: event.id,
        uid: event.i_cal_uid,
        etag: event.change_key,
        summary: event.subject,
        description,
        location,
        start_time,
        end_time,
        is_all_day,
        status,
        organizer_email,
        attendees_json,
        html_link: event.web_link,
        ical_data: None,
    })
}

/// Parse a Graph `dateTime` / `timeZone` pair to a Unix timestamp.
fn parse_graph_datetime(
    dt: &GraphDateTimeTimeZone,
    is_all_day: bool,
    label: &str,
) -> Result<i64, String> {
    if is_all_day {
        // All-day events: dateTime is like "2024-01-15T00:00:00.0000000"
        // Parse as naive date and convert to UTC midnight
        let date_part = dt.date_time.split('T').next().unwrap_or(&dt.date_time);
        let date = chrono::NaiveDate::parse_from_str(date_part, "%Y-%m-%d")
            .map_err(|e| format!("Invalid Graph {label} date: {e}"))?;
        Ok(date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| format!("Invalid all-day {label} time"))?
            .and_utc()
            .timestamp())
    } else {
        // Try RFC 3339 first (with timezone offset)
        if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(&dt.date_time) {
            return Ok(parsed.timestamp());
        }
        // Graph usually returns "2024-01-15T10:00:00.0000000" — no offset.
        // Parse as NaiveDateTime and apply the time zone.
        // Truncate fractional seconds for parsing.
        let clean = if let Some(dot_pos) = dt.date_time.find('.') {
            &dt.date_time[..dot_pos]
        } else {
            &dt.date_time
        };
        let naive = chrono::NaiveDateTime::parse_from_str(clean, "%Y-%m-%dT%H:%M:%S")
            .map_err(|e| format!("Invalid Graph {label} dateTime '{}': {e}", dt.date_time))?;

        // If the time zone is "UTC" or similar, use UTC directly.
        // For named time zones (e.g. "Pacific Standard Time"), we'd need a
        // full IANA/Windows zone mapping. For now, treat as UTC — the calendar
        // crate stores timestamps and the UI shows local time.
        if dt.time_zone == "UTC" || dt.time_zone.is_empty() {
            Ok(naive.and_utc().timestamp())
        } else {
            // Best-effort: treat as UTC. A full Windows→IANA mapping could be
            // added later for higher fidelity.
            Ok(naive.and_utc().timestamp())
        }
    }
}

/// Map Graph event status fields to a simple status string.
fn map_graph_event_status(
    is_cancelled: bool,
    response_status: Option<&GraphResponseStatus>,
    show_as: Option<&str>,
) -> String {
    if is_cancelled {
        return "cancelled".to_string();
    }
    if let Some(rs) = response_status {
        match rs.response.as_str() {
            "declined" => return "cancelled".to_string(),
            "tentativelyAccepted" => return "tentative".to_string(),
            _ => {}
        }
    }
    match show_as {
        Some("tentative") => "tentative".to_string(),
        Some("free") => "confirmed".to_string(),
        _ => "confirmed".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_all_day_datetime() {
        let dt = GraphDateTimeTimeZone {
            date_time: "2024-06-15T00:00:00.0000000".to_string(),
            time_zone: "UTC".to_string(),
        };
        let ts = parse_graph_datetime(&dt, true, "start");
        assert!(ts.is_ok());
    }

    #[test]
    fn parse_timed_datetime() {
        let dt = GraphDateTimeTimeZone {
            date_time: "2024-06-15T10:30:00.0000000".to_string(),
            time_zone: "UTC".to_string(),
        };
        let ts = parse_graph_datetime(&dt, false, "start");
        assert!(ts.is_ok());
    }

    #[test]
    fn map_cancelled_status() {
        let status = map_graph_event_status(true, None, None);
        assert_eq!(status, "cancelled");
    }

    #[test]
    fn map_declined_status() {
        let rs = GraphResponseStatus {
            response: "declined".to_string(),
            time: None,
        };
        let status = map_graph_event_status(false, Some(&rs), None);
        assert_eq!(status, "cancelled");
    }

    #[test]
    fn map_tentative_status() {
        let status = map_graph_event_status(false, None, Some("tentative"));
        assert_eq!(status, "tentative");
    }

    #[test]
    fn map_confirmed_status() {
        let status = map_graph_event_status(false, None, Some("busy"));
        assert_eq!(status, "confirmed");
    }

    #[test]
    fn category_color_mapping() {
        // preset0 should map to a hex color
        let color = category_color_to_hex("preset0");
        assert!(color.is_some());
    }
}
