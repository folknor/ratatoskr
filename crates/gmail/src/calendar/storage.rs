//! Calendar database operations for the Gmail provider.
//!
//! Uses raw SQL via `DbState::with_conn` to avoid a circular dependency
//! on `ratatoskr-core` (which contains `queries_extra::calendars`).

use rusqlite::params;

use ratatoskr_db::db::DbState;

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

    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary)
                 VALUES (?1, ?2, 'google', ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   display_name = ?4, color = ?5, is_primary = ?6, updated_at = unixepoch()",
            params![id, aid, rid, dname, col, is_primary as i64],
        )
        .map_err(|e| format!("upsert calendar: {e}"))?;

        let actual_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![aid, rid],
                |row| row.get(0),
            )
            .map_err(|e| format!("fetch calendar id: {e}"))?;

        Ok(actual_id)
    })
    .await
}

/// Load the sync token for a calendar.
pub(super) async fn load_sync_token(
    db: &DbState,
    calendar_id: &str,
) -> Result<Option<String>, String> {
    let cid = calendar_id.to_string();
    db.with_conn(move |conn| {
        let result = conn.query_row(
            "SELECT sync_token FROM calendars WHERE id = ?1",
            params![cid],
            |row| row.get::<_, Option<String>>(0),
        );
        match result {
            Ok(token) => Ok(token),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(format!("load sync token: {e}")),
        }
    })
    .await
}

/// Save (or clear) the sync token for a calendar.
pub(super) async fn save_sync_token(
    db: &DbState,
    calendar_id: &str,
    token: Option<&str>,
) -> Result<(), String> {
    let cid = calendar_id.to_string();
    let tok = token.map(String::from);
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE calendars SET sync_token = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![tok, cid],
        )
        .map_err(|e| format!("save sync token: {e}"))?;
        Ok(())
    })
    .await
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

    let ical_data = event
        .recurrence
        .as_ref()
        .map(|rules| rules.join("\n"));

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
        let id = uuid::Uuid::new_v4().to_string();

        // Upsert event
        conn.execute(
            "INSERT INTO calendar_events \
                 (id, account_id, google_event_id, summary, description, location, \
                  start_time, end_time, is_all_day, status, organizer_email, \
                  attendees_json, html_link, calendar_id, remote_event_id, etag, \
                  ical_data, uid) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18) \
             ON CONFLICT(account_id, google_event_id) DO UPDATE SET \
                 summary = ?4, description = ?5, location = ?6, \
                 start_time = ?7, end_time = ?8, is_all_day = ?9, \
                 status = ?10, organizer_email = ?11, attendees_json = ?12, \
                 html_link = ?13, calendar_id = ?14, remote_event_id = ?15, \
                 etag = ?16, ical_data = ?17, uid = ?18, updated_at = unixepoch()",
            params![
                id, aid, eid, summary, description, location,
                start_time, end_time, is_all_day as i64, status, organizer_email,
                attendees_json, html_link, cal_id, eid, etag, ical_data, uid,
            ],
        )
        .map_err(|e| format!("upsert calendar event: {e}"))?;

        // Look up the actual local event ID (may differ from `id` on conflict)
        let local_event_id: String = conn
            .query_row(
                "SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2",
                params![aid, eid],
                |row| row.get(0),
            )
            .map_err(|e| format!("fetch event id: {e}"))?;

        // Sync attendees
        persist_attendees(conn, &aid, &local_event_id, &attendees)?;

        // Sync reminders
        persist_reminders(conn, &aid, &local_event_id, &reminders)?;

        Ok(())
    })
    .await
}

/// Delete an event by its remote (Google) event ID within a specific calendar.
pub(super) async fn delete_event_by_remote_id(
    db: &DbState,
    calendar_local_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
    let cid = calendar_local_id.to_string();
    let eid = remote_event_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
            params![cid, eid],
        )
        .map_err(|e| format!("delete calendar event: {e}"))?;
        Ok(())
    })
    .await
}

// ── Attendees ──────────────────────────────────────────────

fn persist_attendees(
    conn: &rusqlite::Connection,
    account_id: &str,
    event_id: &str,
    attendees: &[super::types::EventAttendee],
) -> Result<(), String> {
    // Clear existing attendees
    conn.execute(
        "DELETE FROM calendar_attendees WHERE account_id = ?1 AND event_id = ?2",
        params![account_id, event_id],
    )
    .map_err(|e| format!("delete attendees: {e}"))?;

    for att in attendees {
        let email = att.email.as_deref().unwrap_or_default();
        if email.is_empty() {
            continue;
        }

        conn.execute(
            "INSERT INTO calendar_attendees \
                 (event_id, account_id, email, name, rsvp_status, is_organizer) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(account_id, event_id, email) DO UPDATE SET \
                 name = ?4, rsvp_status = ?5, is_organizer = ?6",
            params![
                event_id,
                account_id,
                email,
                att.display_name,
                att.response_status,
                att.organizer.unwrap_or(false) as i64,
            ],
        )
        .map_err(|e| format!("upsert attendee: {e}"))?;
    }

    Ok(())
}

// ── Reminders ──────────────────────────────────────────────

fn persist_reminders(
    conn: &rusqlite::Connection,
    account_id: &str,
    event_id: &str,
    reminders: &Option<super::types::EventReminders>,
) -> Result<(), String> {
    // Clear existing reminders
    conn.execute(
        "DELETE FROM calendar_reminders WHERE account_id = ?1 AND event_id = ?2",
        params![account_id, event_id],
    )
    .map_err(|e| format!("delete reminders: {e}"))?;

    let Some(reminders) = reminders else {
        return Ok(());
    };

    for reminder in &reminders.overrides {
        let minutes = reminder.minutes.unwrap_or(10);
        conn.execute(
            "INSERT INTO calendar_reminders \
                 (event_id, account_id, minutes_before, method) \
             VALUES (?1, ?2, ?3, ?4)",
            params![event_id, account_id, minutes, reminder.method],
        )
        .map_err(|e| format!("insert reminder: {e}"))?;
    }

    Ok(())
}

// ── Time parsing ───────────────────────────────────────────

/// Parse start/end times from Google Calendar event format.
///
/// Returns (start_unix, end_unix, is_all_day).
fn parse_event_times(event: &GoogleCalendarEvent) -> (i64, i64, bool) {
    let (start, is_all_day) = event
        .start
        .as_ref()
        .map(|s| parse_single_datetime(s))
        .unwrap_or((0, false));

    let (end, _) = event
        .end
        .as_ref()
        .map(|e| parse_single_datetime(e))
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
    use super::*;
    use super::super::types::EventDateTime;

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
