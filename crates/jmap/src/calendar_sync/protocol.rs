use bifrost_jmap::Get;
use bifrost_jmap::calendar_event::{CalendarEvent, CalendarEventGet, CalendarEventSet};
use bifrost_jmap::core::set::SetObject;

use crate::client::JmapClient;

use super::payload::format_jscalendar_times;

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

// ── Internal helpers ───────────────────────────────────────

/// Fetch a batch of calendar events by ID.
pub(super) async fn fetch_event_batch(
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
