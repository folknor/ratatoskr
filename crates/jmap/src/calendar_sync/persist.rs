use std::collections::HashMap;

use jmap_client::Get;
use jmap_client::calendar_event::CalendarEvent;

use db::db::ReadDbState;
use db::db::queries_extra::{
    CalendarAttendeeWriteRow, CalendarReminderWriteRow, UpsertCalendarEventParams,
    delete_event_by_remote_id_sync, replace_event_attendees_sync, replace_event_reminders_sync,
    upsert_calendar_event_sync, upsert_calendar_sync,
};

use super::payload::{
    extract_attendee_rows, extract_attendees_json, extract_location, extract_organizer_email,
    extract_reminder_rows, parse_jscalendar_times, resolve_calendar_id,
};

/// Persist a JMAP CalendarEvent into the local database.
///
/// Extracts JSCalendar properties and maps them to the DB schema.
pub(super) async fn persist_jmap_event(
    db: &ReadDbState,
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
        let local_event_id = upsert_calendar_event_sync(
            conn,
            &UpsertCalendarEventParams {
                account_id: aid.clone(),
                google_event_id: eid.clone(),
                summary: title,
                description,
                location,
                start_time,
                end_time,
                is_all_day,
                status,
                organizer_email,
                attendees_json,
                html_link: None,
                calendar_id,
                remote_event_id: Some(eid2),
                etag: None,
                ical_data,
                uid,
                title: None,
                timezone: None,
                recurrence_rule: None,
                organizer_name: None,
                rsvp_status: None,
                availability: None,
                visibility: None,
                // JMAP's CalendarEvent type carries `recurrenceOverrides` as
                // a JSCalendar map keyed by RECURRENCE-ID; the sync path
                // currently flattens those into the master row. Leave None
                // until the override-as-row path lands here too.
                recurrence_id: None,
            },
        )?;

        let attendee_rows: Vec<CalendarAttendeeWriteRow> = attendees
            .iter()
            .map(|att| CalendarAttendeeWriteRow {
                email: att.email.clone(),
                name: att.name.clone(),
                rsvp_status: att.rsvp_status.clone(),
                is_organizer: att.is_organizer,
            })
            .collect();
        replace_event_attendees_sync(conn, &aid, &local_event_id, &attendee_rows)?;

        let reminder_rows: Vec<CalendarReminderWriteRow> = reminders
            .iter()
            .map(|rem| CalendarReminderWriteRow {
                minutes_before: rem.minutes_before,
                method: rem.method.clone(),
            })
            .collect();
        replace_event_reminders_sync(conn, &aid, &local_event_id, &reminder_rows)?;
        Ok(())
    })
    .await
}

/// Delete a calendar event by its JMAP event ID.
pub(super) async fn delete_event_by_jmap_id(
    db: &ReadDbState,
    account_id: &str,
    jmap_event_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let eid = jmap_event_id.to_string();

    db.with_conn(move |conn| delete_event_by_remote_id_sync(conn, &aid, &eid)).await
}

// ── Sync state persistence ─────────────────────────────────

/// Save a JMAP sync state for calendar objects.
///
/// Reuses the existing `jmap_sync_state` table via sync.
pub(super) async fn save_calendar_sync_state(
    db: &ReadDbState,
    account_id: &str,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    sync::state::save_jmap_sync_state(db, account_id, state_type, state).await
}

/// Load a JMAP sync state for calendar objects.
pub(super) async fn load_calendar_sync_state(
    db: &ReadDbState,
    account_id: &str,
    state_type: &str,
) -> Result<Option<String>, String> {
    sync::state::load_jmap_sync_state(db, account_id, state_type).await
}

// ── Calendar DB helpers ────────────────────────────────────

/// Upsert a calendar entry. Returns the local UUID.
pub(super) async fn upsert_calendar(
    db: &ReadDbState,
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
        // JMAP calendars discovered via Calendar/get are owned by the
        // authenticated user; sharing/permissions land via JMAP Sharing
        // (already wired for mailboxes) and would override this in a future
        // pass.
        upsert_calendar_sync(
            conn,
            &aid,
            "jmap",
            &rid,
            dname.as_deref(),
            col.as_deref(),
            is_primary,
            true,
        )
    })
    .await
}
