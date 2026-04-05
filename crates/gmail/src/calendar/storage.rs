//! Calendar database operations for the Gmail provider.
//!
//! Uses raw SQL via `DbState::with_conn` to avoid a circular dependency
//! on `rtsk` (which contains `queries_extra::calendars`).

use db::db::DbState;
use db::db::queries_extra::{
    CalendarAttendeeWriteRow, CalendarReminderWriteRow, UpsertCalendarEventParams,
    delete_event_by_remote_id_sync, load_calendar_sync_token_sync, replace_event_attendees_sync,
    replace_event_reminders_sync, save_calendar_sync_token_sync, upsert_calendar_event_sync,
    upsert_calendar_sync,
};

use super::CalendarInfo;
use super::types::GoogleCalendarEvent;

// ── Calendar CRUD ──────────────────────────────────────────

/// Upsert a calendar entry. Returns the local UUID for the calendar.
pub(super) async fn upsert_calendar(
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

    db.with_conn(move |conn| upsert_calendar_sync(conn, &aid, "google", &rid, dname.as_deref(), col.as_deref(), is_primary)).await
}

/// Load the sync token for a calendar.
pub(super) async fn load_sync_token(
    db: &DbState,
    calendar_id: &str,
) -> Result<Option<String>, String> {
    let cid = calendar_id.to_string();
    db.with_conn(move |conn| load_calendar_sync_token_sync(conn, &cid)).await
}

/// Save (or clear) the sync token for a calendar.
pub(super) async fn save_sync_token(
    db: &DbState,
    calendar_id: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let cid = calendar_id.to_string();
    let tok = token.map(String::from);
    db.with_conn(move |conn| save_calendar_sync_token_sync(conn, &cid, tok.as_deref())).await
}

// ── Event CRUD ─────────────────────────────────────────────

/// Upsert a Google Calendar event into the database, including attendees
/// and reminders.
pub(super) async fn upsert_event(
    db: &DbState,
    account_id: &str,
    cal: &CalendarInfo,
    event: &GoogleCalendarEvent,
) -> Result<(), String> {
    let event_id = event.id.as_deref().unwrap_or_default();
    if event_id.is_empty() {
        return Ok(());
    }

    let (start_time, end_time, is_all_day) = parse_event_times(event);
    let status = event.status.as_deref().unwrap_or("confirmed").to_string();
    let organizer_email = event.organizer.as_ref().and_then(|o| o.email.clone());

    let attendees_json = if event.attendees.is_empty() {
        None
    } else {
        serde_json::to_string(&event.attendees).ok()
    };

    let ical_data = event.recurrence.as_ref().map(|rules| rules.join("\n"));

    // Clone values for the closure
    let aid = account_id.to_string();
    let eid = event_id.to_string();
    let summary = event.summary.clone();
    let description = event.description.clone();
    let location = event.location.clone();
    let html_link = event.html_link.clone();
    let cal_id = cal.local_id.clone();
    let etag = event.etag.clone();
    let uid = event.i_cal_uid.clone();
    let attendees = event.attendees.clone();
    let reminders = event.reminders.clone();

    db.with_conn(move |conn| {
        let local_event_id = upsert_calendar_event_sync(
            conn,
            &UpsertCalendarEventParams {
                account_id: aid.clone(),
                google_event_id: eid.clone(),
                summary,
                description,
                location,
                start_time,
                end_time,
                is_all_day,
                status,
                organizer_email,
                attendees_json,
                html_link,
                calendar_id: Some(cal_id),
                remote_event_id: Some(eid.clone()),
                etag,
                ical_data,
                uid,
                title: None,
                timezone: None,
                recurrence_rule: None,
                organizer_name: None,
                rsvp_status: None,
                availability: None,
                visibility: None,
            },
        )?;

        let attendee_rows: Vec<CalendarAttendeeWriteRow> = attendees
            .iter()
            .filter_map(|att| {
                let email = att.email.clone().unwrap_or_default();
                (!email.is_empty()).then(|| CalendarAttendeeWriteRow {
                    email,
                    name: att.display_name.clone(),
                    rsvp_status: att.response_status.clone(),
                    is_organizer: att.organizer.unwrap_or(false),
                })
            })
            .collect();
        replace_event_attendees_sync(conn, &aid, &local_event_id, &attendee_rows)?;

        let reminder_rows: Vec<CalendarReminderWriteRow> = reminders
            .as_ref()
            .map(|r| {
                r.overrides
                    .iter()
                    .map(|rem| CalendarReminderWriteRow {
                        minutes_before: rem.minutes.unwrap_or(10),
                        method: rem.method.clone(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        replace_event_reminders_sync(conn, &aid, &local_event_id, &reminder_rows)?;
        Ok(())
    }).await
}

/// Delete an event by its remote (Google) event ID within a specific calendar.
pub(super) async fn delete_event_by_remote_id(
    db: &DbState,
    calendar_local_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
    let cid = calendar_local_id.to_string();
    let eid = remote_event_id.to_string();

    db.with_conn(move |conn| delete_event_by_remote_id_sync(conn, &cid, &eid)).await
}

// ── Time parsing ───────────────────────────────────────────

/// Parse start/end times from Google Calendar event format.
///
/// Returns (start_unix, end_unix, is_all_day).
fn parse_event_times(event: &GoogleCalendarEvent) -> (i64, i64, bool) {
    let (start, is_all_day) = event
        .start
        .as_ref()
        .map(parse_single_datetime)
        .unwrap_or((0, false));

    let (end, _) = event
        .end
        .as_ref()
        .map(parse_single_datetime)
        .unwrap_or((start + 3600, false));

    (start, end, is_all_day)
}

/// Parse a single `EventDateTime` into a Unix timestamp.
///
/// Returns (unix_timestamp, is_all_day).
fn parse_single_datetime(dt: &super::types::EventDateTime) -> (i64, bool) {
    // All-day events use the `date` field (e.g. "2025-01-15")
    if let Some(date_str) = &dt.date {
        let parsed = chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .map(|d| {
                d.and_hms_opt(0, 0, 0)
                    .map(|ndt| ndt.and_utc().timestamp())
                    .unwrap_or(0)
            })
            .unwrap_or(0);
        return (parsed, true);
    }

    // Timed events use `dateTime` (RFC 3339)
    if let Some(datetime_str) = &dt.date_time {
        let parsed = chrono::DateTime::parse_from_rfc3339(datetime_str)
            .map(|d| d.timestamp())
            .unwrap_or(0);
        return (parsed, false);
    }

    (0, false)
}

// ── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::super::types::EventDateTime;
    use super::*;

    #[test]
    fn parse_timed_event() {
        let dt = EventDateTime {
            date_time: Some("2025-03-15T14:30:00-04:00".to_string()),
            date: None,
            time_zone: None,
        };
        let (ts, is_all_day) = parse_single_datetime(&dt);
        assert!(!is_all_day);
        assert!(ts > 0);
        // 2025-03-15T14:30:00-04:00 = 2025-03-15T18:30:00Z
        assert_eq!(ts, 1_742_063_400);
    }

    #[test]
    fn parse_all_day_event() {
        let dt = EventDateTime {
            date_time: None,
            date: Some("2025-01-15".to_string()),
            time_zone: None,
        };
        let (ts, is_all_day) = parse_single_datetime(&dt);
        assert!(is_all_day);
        assert!(ts > 0);
    }

    #[test]
    fn parse_empty_datetime() {
        let dt = EventDateTime {
            date_time: None,
            date: None,
            time_zone: None,
        };
        let (ts, is_all_day) = parse_single_datetime(&dt);
        assert_eq!(ts, 0);
        assert!(!is_all_day);
    }

    #[test]
    fn parse_event_times_defaults() {
        let event = GoogleCalendarEvent {
            id: Some("test".to_string()),
            status: None,
            html_link: None,
            summary: None,
            description: None,
            location: None,
            start: None,
            end: None,
            recurrence: None,
            organizer: None,
            attendees: vec![],
            etag: None,
            i_cal_uid: None,
            recurring_event_id: None,
            reminders: None,
            updated: None,
        };
        let (start, end, is_all_day) = parse_event_times(&event);
        assert_eq!(start, 0);
        assert_eq!(end, 3600); // Default 1-hour
        assert!(!is_all_day);
    }
}
