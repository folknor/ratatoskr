use super::super::super::types::{
    DbCalendar, DbCalendarAttendee, DbCalendarEvent, DbCalendarReminder,
};
use super::super::super::ReadConn;
use crate::db::from_row::FromRow;
use rusqlite::params;

const CALENDAR_COLS: &str = "\
    id, account_id, provider, remote_id, display_name, color, is_primary, \
    is_visible, sync_token, ctag, created_at, updated_at, sort_order, \
    is_default, provider_id, can_edit";

const EVENT_COLS: &str = "\
    id, account_id, google_event_id, summary, description, location, \
    start_time, end_time, is_all_day, status, organizer_email, attendees_json, \
    html_link, updated_at, calendar_id, remote_event_id, etag, ical_data, uid, \
    title, timezone, recurrence_rule, organizer_name, rsvp_status, created_at, \
    availability, visibility, recurrence_id";

const ATTENDEE_COLS: &str = "\
    event_id, account_id, email, name, rsvp_status, is_organizer";

const REMINDER_COLS: &str = "\
    id, event_id, account_id, minutes_before, method";

pub fn get_calendar_event_sync(
    conn: &ReadConn<'_>,
    event_id: &str,
) -> Result<Option<DbCalendarEvent>, String> {
    let result = conn.query_row(
        &format!("SELECT {EVENT_COLS} FROM calendar_events WHERE id = ?1"),
        params![event_id],
        DbCalendarEvent::from_row,
    );
    match result {
        Ok(event) => Ok(Some(event)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

pub fn get_event_attendees_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
    event_id: &str,
) -> Result<Vec<DbCalendarAttendee>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {ATTENDEE_COLS} FROM calendar_attendees
             WHERE account_id = ?1 AND event_id = ?2
             ORDER BY is_organizer DESC, email ASC"
        ))
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, event_id], DbCalendarAttendee::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn get_event_reminders_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
    event_id: &str,
) -> Result<Vec<DbCalendarReminder>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {REMINDER_COLS} FROM calendar_reminders
             WHERE account_id = ?1 AND event_id = ?2
             ORDER BY minutes_before ASC"
        ))
        .map_err(|e| e.to_string())?;

    stmt.query_map(params![account_id, event_id], DbCalendarReminder::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

pub fn load_calendars_for_sidebar_sync(
    conn: &ReadConn<'_>,
) -> Result<Vec<DbCalendar>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT {CALENDAR_COLS} FROM calendars
             ORDER BY account_id, sort_order ASC, is_primary DESC, display_name ASC"
        ))
        .map_err(|e| e.to_string())?;

    stmt.query_map([], DbCalendar::from_row)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}
