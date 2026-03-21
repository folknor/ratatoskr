use ratatoskr_core::db::DbState;
use ratatoskr_core::graph::calendar_sync::{
    graph_create_event, graph_delete_event, graph_list_calendars, graph_sync_calendar_events,
    graph_update_event, GraphDateTimeTimeZone, GraphEventCreate,
};
use ratatoskr_core::graph::client::GraphClient;

use super::types::{CalendarEventDto, CalendarInfoDto, CalendarSyncResultDto};

/// List calendars for a Graph/Microsoft account.
pub async fn graph_calendar_list_calendars_impl(
    _account_id: &str,
    db: &DbState,
    client: &GraphClient,
) -> Result<Vec<CalendarInfoDto>, String> {
    let calendars = graph_list_calendars(client, db).await?;

    Ok(calendars
        .into_iter()
        .map(|cal| CalendarInfoDto {
            remote_id: cal.remote_id,
            display_name: cal.display_name,
            color: cal.color,
            is_primary: cal.is_primary,
        })
        .collect())
}

/// Sync events for a single calendar using Graph delta queries.
pub async fn graph_calendar_sync_events_impl(
    _account_id: &str,
    calendar_remote_id: &str,
    sync_token: Option<String>,
    db: &DbState,
    client: &GraphClient,
) -> Result<CalendarSyncResultDto, String> {
    let result =
        graph_sync_calendar_events(client, db, calendar_remote_id, sync_token.as_deref()).await?;

    let created: Result<Vec<CalendarEventDto>, String> =
        result.created.into_iter().map(graph_event_to_dto).collect();
    let updated: Result<Vec<CalendarEventDto>, String> =
        result.updated.into_iter().map(graph_event_to_dto).collect();

    Ok(CalendarSyncResultDto {
        created: created?,
        updated: updated?,
        deleted_remote_ids: result.deleted_remote_ids,
        // Graph uses delta links (full URLs) as the sync token
        new_sync_token: result.new_delta_link,
        new_ctag: None,
    })
}

/// Create a calendar event via Graph API.
pub async fn graph_calendar_create_event_impl(
    client: &GraphClient,
    db: &DbState,
    calendar_remote_id: &str,
    event: serde_json::Value,
) -> Result<CalendarEventDto, String> {
    let create_req = json_to_graph_event_create(&event)?;
    let response = graph_create_event(client, db, calendar_remote_id, &create_req).await?;
    graph_event_to_dto(response)
}

/// Update a calendar event via Graph API.
pub async fn graph_calendar_update_event_impl(
    client: &GraphClient,
    db: &DbState,
    remote_event_id: &str,
    event: serde_json::Value,
) -> Result<CalendarEventDto, String> {
    let response = graph_update_event(client, db, remote_event_id, &event).await?;
    graph_event_to_dto(response)
}

/// Delete a calendar event via Graph API.
pub async fn graph_calendar_delete_event_impl(
    client: &GraphClient,
    db: &DbState,
    remote_event_id: &str,
) -> Result<(), String> {
    graph_delete_event(client, db, remote_event_id).await
}

// ── Conversion helpers ────────────────────────────────────

fn graph_event_to_dto(
    event: ratatoskr_core::graph::calendar_sync::GraphCalendarEvent,
) -> Result<CalendarEventDto, String> {
    Ok(CalendarEventDto {
        remote_event_id: event.remote_event_id,
        uid: event.uid,
        etag: event.etag,
        summary: event.summary,
        description: event.description,
        location: event.location,
        start_time: event.start_time,
        end_time: event.end_time,
        is_all_day: event.is_all_day,
        status: event.status,
        organizer_email: event.organizer_email,
        attendees_json: event.attendees_json,
        html_link: event.html_link,
        ical_data: event.ical_data,
    })
}

/// Convert a generic JSON event payload to a `GraphEventCreate` request.
fn json_to_graph_event_create(value: &serde_json::Value) -> Result<GraphEventCreate, String> {
    let subject = value.get("subject").or_else(|| value.get("summary"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let description = value.get("description")
        .and_then(|v| v.as_str())
        .map(String::from);

    let body = description.map(|desc| {
        ratatoskr_core::graph::calendar_sync::GraphEventBodyInput {
            content_type: "text".to_string(),
            content: desc,
        }
    });

    let is_all_day = value.get("isAllDay")
        .or_else(|| value.get("is_all_day"))
        .and_then(|v| v.as_bool());

    let start = parse_event_datetime(value, "start", "startDateTime", is_all_day.unwrap_or(false))?;
    let end = parse_event_datetime(value, "end", "endDateTime", is_all_day.unwrap_or(false))?;

    let location = value.get("location")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| ratatoskr_core::graph::calendar_sync::GraphLocationInput {
            display_name: s.to_string(),
        });

    Ok(GraphEventCreate {
        subject,
        body,
        start,
        end,
        is_all_day,
        location,
        attendees: None,
        recurrence: None,
        is_reminder_on: None,
        reminder_minutes_before_start: None,
    })
}

/// Parse a datetime from the JSON event payload into a Graph `DateTimeTimeZone`.
fn parse_event_datetime(
    value: &serde_json::Value,
    key: &str,
    alt_key: &str,
    is_all_day: bool,
) -> Result<GraphDateTimeTimeZone, String> {
    // Try nested Graph format first: { "dateTime": "...", "timeZone": "..." }
    if let Some(obj) = value.get(key).and_then(|v| v.as_object()) {
        if let Some(dt) = obj.get("dateTime").and_then(|v| v.as_str()) {
            let tz = obj
                .get("timeZone")
                .and_then(|v| v.as_str())
                .unwrap_or("UTC");
            return Ok(GraphDateTimeTimeZone {
                date_time: dt.to_string(),
                time_zone: tz.to_string(),
            });
        }
    }

    // Try flat ISO string
    let dt_str = value.get(key).or_else(|| value.get(alt_key))
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("Missing '{key}' in event payload"))?;

    if is_all_day && dt_str.len() == 10 {
        // Date-only: "2024-01-15"
        Ok(GraphDateTimeTimeZone {
            date_time: format!("{dt_str}T00:00:00.0000000"),
            time_zone: "UTC".to_string(),
        })
    } else {
        Ok(GraphDateTimeTimeZone {
            date_time: dt_str.to_string(),
            time_zone: "UTC".to_string(),
        })
    }
}
