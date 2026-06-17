//! JMAP calendar sync using CalendarEvent/get, /changes, and /set.
//!
//! Leverages JMAP's native state-tracking (`/changes` method) for clean
//! incremental sync - no ctag/etag complexity like CalDAV.

use std::collections::HashMap;

use bifrost_jmap::calendar::CalendarGet;
use bifrost_jmap::calendar_event::CalendarEventGet;

use crate::client::JmapClient;

mod payload;
mod persist;
mod protocol;

pub use protocol::{create_event_remote, delete_event_remote, update_event_remote};

use persist::jmap_event_record;
pub use persist::{
    JmapCalendarAttendeeRecord, JmapCalendarEventRecord, JmapCalendarReminderRecord,
};
use protocol::fetch_event_batch;

const EVENT_BATCH_SIZE: usize = 50;

// ── Public types ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct JmapDiscoveredCalendar {
    pub remote_id: String,
    pub display_name: Option<String>,
    pub color: Option<String>,
    pub is_primary: bool,
}

#[derive(Debug, Clone)]
pub struct JmapCalendarListSync {
    pub state: String,
    pub calendars: Vec<JmapDiscoveredCalendar>,
}

#[derive(Debug, Clone)]
pub struct JmapCalendarEventSync {
    pub state: String,
    pub events: Vec<JmapCalendarEventRecord>,
    pub deleted_remote_ids: Vec<String>,
}

// ── Calendar list sync ─────────────────────────────────────

pub async fn fetch_calendar_list(client: &JmapClient) -> Result<JmapCalendarListSync, String> {
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

    let mut calendars = Vec::with_capacity(calendar_list.len());

    for cal in &calendar_list {
        let remote_id = match cal.id() {
            Some(id) => id,
            None => continue,
        };

        let display_name = cal.name().map(String::from);
        let color = cal.color().map(String::from);
        let is_primary = cal.is_default().unwrap_or(false);

        calendars.push(JmapDiscoveredCalendar {
            remote_id: remote_id.to_string(),
            display_name,
            color,
            is_primary,
        });
    }

    Ok(JmapCalendarListSync { state, calendars })
}

// ── Initial event sync ─────────────────────────────────────

pub async fn fetch_all_events(
    client: &JmapClient,
    account_id: &str,
    cal_map: &HashMap<&str, &str>,
) -> Result<JmapCalendarEventSync, String> {
    // Fetch ALL events (no filter) - the server returns them all with state
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

    let mut records = Vec::with_capacity(events.len());
    for event in &events {
        if let Some(record) = jmap_event_record(event, cal_map) {
            records.push(record);
        }
    }

    Ok(JmapCalendarEventSync {
        state,
        events: records,
        deleted_remote_ids: Vec::new(),
    })
}

// ── Delta event sync ───────────────────────────────────────

pub async fn fetch_events_delta(
    client: &JmapClient,
    account_id: &str,
    cal_map: &HashMap<&str, &str>,
    mut since_state: String,
) -> Result<JmapCalendarEventSync, String> {
    let mut records = Vec::new();
    let mut deleted_remote_ids = Vec::new();

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

        let ids_to_fetch: Vec<&str> = created
            .iter()
            .chain(updated.iter())
            .map(String::as_str)
            .collect();

        if !ids_to_fetch.is_empty() {
            for chunk in ids_to_fetch.chunks(EVENT_BATCH_SIZE) {
                let events = fetch_event_batch(client, chunk).await?;
                for event in &events {
                    if let Some(record) = jmap_event_record(event, cal_map) {
                        records.push(record);
                    }
                }
            }
        }

        deleted_remote_ids.extend(destroyed.iter().cloned());
        since_state = changes.new_state().to_string();

        if !changes.has_more_changes() {
            break;
        }
    }

    log::info!("[JMAP] Calendar event delta sync complete for account {account_id}");

    Ok(JmapCalendarEventSync {
        state: since_state,
        events: records,
        deleted_remote_ids,
    })
}
