use chrono::Datelike;

use super::super::DbState;
use super::super::types::{DbCalendar, DbCalendarAttendee, DbCalendarEvent, DbCalendarReminder};
use crate::db::from_row::FromRow;
use rusqlite::params;

/// Explicit column list for `calendars` table queries.
const CALENDAR_COLS: &str = "\
    id, account_id, provider, remote_id, display_name, color, is_primary, \
    is_visible, sync_token, ctag, created_at, updated_at, sort_order, \
    is_default, provider_id";

/// Explicit column list for `calendar_events` table queries.
const EVENT_COLS: &str = "\
    id, account_id, google_event_id, summary, description, location, \
    start_time, end_time, is_all_day, status, organizer_email, attendees_json, \
    html_link, updated_at, calendar_id, remote_event_id, etag, ical_data, uid, \
    title, timezone, recurrence_rule, organizer_name, rsvp_status, created_at, \
    availability, visibility";

/// Explicit column list for `calendar_attendees` table queries.
const ATTENDEE_COLS: &str = "\
    event_id, account_id, email, name, rsvp_status, is_organizer";

/// Explicit column list for `calendar_reminders` table queries.
const REMINDER_COLS: &str = "\
    id, event_id, account_id, minutes_before, method";

pub async fn db_upsert_calendar(
    db: &DbState,
    account_id: String,
    provider: String,
    remote_id: String,
    display_name: Option<String>,
    color: Option<String>,
    is_primary: bool,
) -> Result<String, String> {
    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   display_name = ?5, color = ?6, is_primary = ?7, updated_at = unixepoch()",
            params![id, account_id, provider, remote_id, display_name, color, is_primary as i64],
        )
        .map_err(|e| e.to_string())?;
        let actual_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, remote_id],
                |row| row.get("id"),
            )
            .map_err(|e| e.to_string())?;
        Ok(actual_id)
    })
    .await
}

pub async fn db_get_calendars_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbCalendar>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {CALENDAR_COLS} FROM calendars WHERE account_id = ?1 \
                     ORDER BY sort_order ASC, is_primary DESC, display_name ASC"
            ))
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], DbCalendar::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_visible_calendars(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbCalendar>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {CALENDAR_COLS} FROM calendars WHERE account_id = ?1 AND is_visible = 1 \
                     ORDER BY sort_order ASC, is_primary DESC, display_name ASC"
            ))
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], DbCalendar::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_set_calendar_visibility(
    db: &DbState,
    calendar_id: String,
    visible: bool,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE calendars SET is_visible = ?1, updated_at = unixepoch() WHERE id = ?2",
            params![visible as i64, calendar_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_calendar_sync_token(
    db: &DbState,
    calendar_id: String,
    sync_token: Option<String>,
    ctag: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE calendars SET sync_token = ?1, ctag = ?2, updated_at = unixepoch() WHERE id = ?3",
            params![sync_token, ctag, calendar_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_calendars_for_account(
    db: &DbState,
    account_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM calendars WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_calendar_by_id(
    db: &DbState,
    calendar_id: String,
) -> Result<Option<DbCalendar>, String> {
    db.with_conn(move |conn| {
        let result = conn.query_row(
            &format!("SELECT {CALENDAR_COLS} FROM calendars WHERE id = ?1"),
            params![calendar_id],
            DbCalendar::from_row,
        );
        match result {
            Ok(calendar) => Ok(Some(calendar)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

/// Parameters for upserting a calendar event.
#[derive(Debug, Clone, Default)]
pub struct UpsertCalendarEventParams {
    pub account_id: String,
    pub google_event_id: String,
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
    pub calendar_id: Option<String>,
    pub remote_event_id: Option<String>,
    pub etag: Option<String>,
    pub ical_data: Option<String>,
    pub uid: Option<String>,
    pub title: Option<String>,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub organizer_name: Option<String>,
    pub rsvp_status: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
}

pub async fn db_upsert_calendar_event(
    db: &DbState,
    p: UpsertCalendarEventParams,
) -> Result<(), String> {
    log::info!(
        "Upserting calendar event: account_id={}, google_event_id={}",
        p.account_id,
        p.google_event_id
    );
    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO calendar_events (id, account_id, google_event_id, summary, description, \
                 location, start_time, end_time, is_all_day, status, organizer_email, \
                 attendees_json, html_link, calendar_id, remote_event_id, etag, ical_data, uid, \
                 title, timezone, recurrence_rule, organizer_name, rsvp_status, created_at, \
                 availability, visibility)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                         ?17, ?18, ?19, ?20, ?21, ?22, ?23, unixepoch(), ?24, ?25)
                 ON CONFLICT(account_id, google_event_id) DO UPDATE SET
                   summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
                   is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
                   html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
                   ical_data = ?17, uid = ?18, title = ?19, timezone = ?20, recurrence_rule = ?21,
                   organizer_name = ?22, rsvp_status = ?23, availability = ?24, visibility = ?25,
                   updated_at = unixepoch()",
            params![
                id,
                p.account_id,
                p.google_event_id,
                p.summary,
                p.description,
                p.location,
                p.start_time,
                p.end_time,
                p.is_all_day as i64,
                p.status,
                p.organizer_email,
                p.attendees_json,
                p.html_link,
                p.calendar_id,
                p.remote_event_id,
                p.etag,
                p.ical_data,
                p.uid,
                p.title,
                p.timezone,
                p.recurrence_rule,
                p.organizer_name,
                p.rsvp_status,
                p.availability,
                p.visibility
            ],
        )
        .map_err(|e| {
            log::error!("Failed to upsert calendar event: {e}");
            e.to_string()
        })?;
        Ok(())
    })
    .await
}

pub async fn db_get_calendar_events_in_range(
    db: &DbState,
    account_id: String,
    start_time: i64,
    end_time: i64,
) -> Result<Vec<DbCalendarEvent>, String> {
    log::debug!("Loading calendar events: account_id={account_id}, range={start_time}..{end_time}");
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {EVENT_COLS} FROM calendar_events \
                     WHERE account_id = ?1 AND start_time < ?3 AND end_time > ?2 \
                     ORDER BY start_time ASC"
            ))
            .map_err(|e| e.to_string())?;
        stmt.query_map(
            params![account_id, start_time, end_time],
            DbCalendarEvent::from_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_calendar_events_in_range_multi(
    db: &DbState,
    account_id: String,
    calendar_ids: Vec<String>,
    start_time: i64,
    end_time: i64,
) -> Result<Vec<DbCalendarEvent>, String> {
    if calendar_ids.is_empty() {
        return db_get_calendar_events_in_range(db, account_id, start_time, end_time).await;
    }
    db.with_conn(move |conn| {
        let placeholders = calendar_ids
            .iter()
            .enumerate()
            .map(|(i, _)| format!("?{}", i + 4))
            .collect::<Vec<_>>()
            .join(", ");
        let sql = format!(
            "SELECT {EVENT_COLS} FROM calendar_events \
                 WHERE account_id = ?1 AND start_time < ?3 AND end_time > ?2 \
                   AND (calendar_id IN ({placeholders}) OR calendar_id IS NULL) \
                 ORDER BY start_time ASC"
        );
        let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        param_values.push(Box::new(account_id));
        param_values.push(Box::new(start_time));
        param_values.push(Box::new(end_time));
        for cid in &calendar_ids {
            param_values.push(Box::new(cid.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            param_values.iter().map(AsRef::as_ref).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        stmt.query_map(param_refs.as_slice(), DbCalendarEvent::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_delete_events_for_calendar(
    db: &DbState,
    calendar_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        // Cascade: delete attendees and reminders for all events in this calendar.
        conn.execute(
            "DELETE FROM calendar_attendees WHERE event_id IN \
             (SELECT id FROM calendar_events WHERE calendar_id = ?1)",
            params![calendar_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM calendar_reminders WHERE event_id IN \
             (SELECT id FROM calendar_events WHERE calendar_id = ?1)",
            params![calendar_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM calendar_events WHERE calendar_id = ?1",
            params![calendar_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_event_by_remote_id(
    db: &DbState,
    calendar_id: String,
    remote_event_id: String,
) -> Result<Option<DbCalendarEvent>, String> {
    db.with_conn(move |conn| {
        let result = conn.query_row(
            &format!("SELECT {EVENT_COLS} FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2"),
            params![calendar_id, remote_event_id],
            DbCalendarEvent::from_row,
        );
        match result {
            Ok(event) => Ok(Some(event)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

pub async fn db_delete_event_by_remote_id(
    db: &DbState,
    calendar_id: String,
    remote_event_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        // Cascade: delete attendees and reminders before the event.
        conn.execute(
            "DELETE FROM calendar_attendees WHERE event_id IN \
             (SELECT id FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2)",
            params![calendar_id, remote_event_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM calendar_reminders WHERE event_id IN \
             (SELECT id FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2)",
            params![calendar_id, remote_event_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
            params![calendar_id, remote_event_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_calendar_event(db: &DbState, event_id: String) -> Result<(), String> {
    log::info!("Deleting calendar event: id={event_id}");
    db.with_conn(move |conn| {
        // Cascade: delete attendees and reminders before the event.
        conn.execute(
            "DELETE FROM calendar_attendees WHERE event_id = ?1",
            params![event_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM calendar_reminders WHERE event_id = ?1",
            params![event_id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute(
            "DELETE FROM calendar_events WHERE id = ?1",
            params![event_id],
        )
        .map_err(|e| {
            log::error!("Failed to delete calendar event {event_id}: {e}");
            e.to_string()
        })?;
        Ok(())
    })
    .await
}

// ── Attendee queries ───────────────────────────────────────

pub async fn db_get_event_attendees(
    db: &DbState,
    account_id: String,
    event_id: String,
) -> Result<Vec<DbCalendarAttendee>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {ATTENDEE_COLS} FROM calendar_attendees \
                 WHERE account_id = ?1 AND event_id = ?2 \
                 ORDER BY is_organizer DESC, email ASC"
            ))
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id, event_id], DbCalendarAttendee::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_upsert_event_attendee(
    db: &DbState,
    account_id: String,
    event_id: String,
    email: String,
    name: Option<String>,
    rsvp_status: Option<String>,
    is_organizer: bool,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO calendar_attendees (event_id, account_id, email, name, rsvp_status, is_organizer)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, event_id, email) DO UPDATE SET
                   name = ?4, rsvp_status = ?5, is_organizer = ?6",
            params![event_id, account_id, email, name, rsvp_status, is_organizer as i64],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_attendees_for_event(
    db: &DbState,
    account_id: String,
    event_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM calendar_attendees WHERE account_id = ?1 AND event_id = ?2",
            params![account_id, event_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

// ── Reminder queries ───────────────────────────────────────

pub async fn db_get_event_reminders(
    db: &DbState,
    account_id: String,
    event_id: String,
) -> Result<Vec<DbCalendarReminder>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {REMINDER_COLS} FROM calendar_reminders \
                 WHERE account_id = ?1 AND event_id = ?2 \
                 ORDER BY minutes_before ASC"
            ))
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id, event_id], DbCalendarReminder::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_add_event_reminder(
    db: &DbState,
    account_id: String,
    event_id: String,
    minutes_before: i64,
    method: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO calendar_reminders (event_id, account_id, minutes_before, method)
                 VALUES (?1, ?2, ?3, ?4)",
            params![event_id, account_id, minutes_before, method],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_reminders_for_event(
    db: &DbState,
    account_id: String,
    event_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM calendar_reminders WHERE account_id = ?1 AND event_id = ?2",
            params![account_id, event_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

// ── Synchronous calendar event helpers (for app-layer use) ──

/// Get a single calendar event by its DB id (synchronous).
pub fn get_calendar_event_sync(
    conn: &rusqlite::Connection,
    event_id: &str,
) -> Result<Option<DbCalendarEvent>, String> {
    let result = conn.query_row(
        &format!("SELECT {EVENT_COLS} FROM calendar_events WHERE id = ?1"),
        params![event_id],
        DbCalendarEvent::from_row,
    );
    match result {
        Ok(event) => Ok(Some(event)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Load attendees for a given event (synchronous).
pub fn get_event_attendees_sync(
    conn: &rusqlite::Connection,
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

/// Load reminders for a given event (synchronous).
pub fn get_event_reminders_sync(
    conn: &rusqlite::Connection,
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

/// Load all calendars for the sidebar list (synchronous).
pub fn load_calendars_for_sidebar_sync(
    conn: &rusqlite::Connection,
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

/// Set calendar visibility (synchronous).
pub fn set_calendar_visibility_sync(
    conn: &rusqlite::Connection,
    calendar_id: &str,
    visible: bool,
) -> Result<(), String> {
    conn.execute(
        "UPDATE calendars SET is_visible = ?1, updated_at = unixepoch() WHERE id = ?2",
        params![visible as i64, calendar_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Parameters for creating/updating a local calendar event (synchronous).
#[derive(Debug, Clone, Default)]
pub struct LocalCalendarEventParams {
    pub account_id: String,
    pub summary: String,
    pub description: String,
    pub location: String,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub calendar_id: Option<String>,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
}

/// Create a new local calendar event (synchronous). Returns the new event ID.
pub fn create_calendar_event_sync(
    conn: &rusqlite::Connection,
    p: &LocalCalendarEventParams,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO calendar_events
            (id, account_id, google_event_id, summary, description,
             location, start_time, end_time, is_all_day, status,
             calendar_id, timezone, recurrence_rule, availability,
             visibility, created_at)
         VALUES (?1, ?2, NULL, ?3, ?4, ?5, ?6, ?7, ?8, 'confirmed', ?9,
                 ?10, ?11, ?12, ?13, unixepoch())",
        params![
            id,
            p.account_id,
            p.summary,
            p.description,
            p.location,
            p.start_time,
            p.end_time,
            p.is_all_day as i64,
            p.calendar_id,
            p.timezone,
            p.recurrence_rule,
            p.availability,
            p.visibility,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(id)
}

/// Update an existing calendar event (synchronous).
pub fn update_calendar_event_sync(
    conn: &rusqlite::Connection,
    event_id: &str,
    p: &LocalCalendarEventParams,
) -> Result<(), String> {
    conn.execute(
        "UPDATE calendar_events SET
            summary = ?2, description = ?3, location = ?4,
            start_time = ?5, end_time = ?6, is_all_day = ?7,
            calendar_id = ?8, timezone = ?9, recurrence_rule = ?10,
            availability = ?11, visibility = ?12, updated_at = unixepoch()
         WHERE id = ?1",
        params![
            event_id,
            p.summary,
            p.description,
            p.location,
            p.start_time,
            p.end_time,
            p.is_all_day as i64,
            p.calendar_id,
            p.timezone,
            p.recurrence_rule,
            p.availability,
            p.visibility,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete a calendar event by id (synchronous), cascading to attendees and reminders.
pub fn delete_calendar_event_sync(
    conn: &rusqlite::Connection,
    event_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM calendar_attendees WHERE event_id = ?1",
        params![event_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM calendar_reminders WHERE event_id = ?1",
        params![event_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM calendar_events WHERE id = ?1",
        params![event_id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// A calendar event with resolved calendar color, suitable for view rendering.
#[derive(Debug, Clone)]
pub struct CalendarViewEvent {
    pub id: String,
    pub title: String,
    pub start_time: i64,
    pub end_time: i64,
    pub all_day: bool,
    pub color: String,
    pub calendar_name: Option<String>,
    pub location: Option<String>,
    pub recurrence_rule: Option<String>,
    pub calendar_id: Option<String>,
    pub account_id: String,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub rsvp_status: Option<String>,
    pub description: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
    pub timezone: Option<String>,
}

/// Load all calendar events with resolved calendar colors (synchronous).
pub fn load_calendar_events_for_view_sync(
    conn: &rusqlite::Connection,
) -> Result<Vec<CalendarViewEvent>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT e.id, e.summary, e.title, e.start_time, e.end_time,
                    e.is_all_day, COALESCE(c.color, '#3498db') AS color,
                    c.display_name AS calendar_name, e.location,
                    e.recurrence_rule, e.calendar_id, e.account_id,
                    e.organizer_name, e.organizer_email, e.rsvp_status,
                    e.description, e.availability, e.visibility, e.timezone
             FROM calendar_events e
             LEFT JOIN calendars c
               ON c.account_id = e.account_id AND c.id = e.calendar_id
             WHERE c.is_visible = 1 OR e.calendar_id IS NULL
             ORDER BY e.start_time ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            // Prefer `title` over `summary` (title is the v63 canonical field).
            let title_v63: Option<String> = row.get("title")?;
            let summary: Option<String> = row.get("summary")?;
            let display_title = title_v63.or(summary).unwrap_or_default();
            Ok(CalendarViewEvent {
                id: row.get::<_, String>("id")?,
                title: display_title,
                start_time: row.get("start_time")?,
                end_time: row.get("end_time")?,
                all_day: row.get::<_, i64>("is_all_day")? != 0,
                color: row
                    .get::<_, Option<String>>("color")?
                    .unwrap_or_else(|| "#3498db".to_string()),
                calendar_name: row.get("calendar_name")?,
                location: row.get("location")?,
                recurrence_rule: row.get("recurrence_rule")?,
                calendar_id: row.get("calendar_id")?,
                account_id: row.get("account_id")?,
                organizer_name: row.get("organizer_name")?,
                organizer_email: row.get("organizer_email")?,
                rsvp_status: row.get("rsvp_status")?,
                description: row.get("description")?,
                availability: row.get("availability")?,
                visibility: row.get("visibility")?,
                timezone: row.get("timezone")?,
            })
        })
        .map_err(|e| e.to_string())?;
    let base_events: Vec<CalendarViewEvent> = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // Expand recurring events into concrete instances.
    let mut expanded = Vec::with_capacity(base_events.len());
    for ev in base_events {
        if let Some(ref rrule) = ev.recurrence_rule {
            let mut instances = expand_recurrence(&ev, rrule);
            expanded.append(&mut instances);
        } else {
            expanded.push(ev);
        }
    }
    expanded.sort_by_key(|e| e.start_time);
    Ok(expanded)
}

/// Expand a recurring event into concrete instances based on its RRULE.
///
/// Supports DAILY, WEEKLY, MONTHLY, YEARLY frequencies with INTERVAL
/// and COUNT/UNTIL. Generates instances for a ~2 year window from the
/// event's original start time.
fn expand_recurrence(event: &CalendarViewEvent, rrule_str: &str) -> Vec<CalendarViewEvent> {
    let rule = rrule_str.strip_prefix("RRULE:").unwrap_or(rrule_str);

    let mut freq = "";
    let mut interval: i64 = 1;
    let mut count: Option<usize> = None;
    let mut until: Option<i64> = None;

    for part in rule.split(';') {
        if let Some(val) = part.strip_prefix("FREQ=") {
            freq = val;
        } else if let Some(val) = part.strip_prefix("INTERVAL=") {
            interval = val.parse().unwrap_or(1).max(1);
        } else if let Some(val) = part.strip_prefix("COUNT=") {
            count = val.parse().ok();
        } else if let Some(val) = part.strip_prefix("UNTIL=") {
            // Basic UNTIL parsing: YYYYMMDD or YYYYMMDDTHHMMSSZ
            until = parse_until_date(val);
        }
    }

    let duration = event.end_time - event.start_time;
    let max_instances = count.unwrap_or(365); // Cap at 365 instances if no COUNT.
    let window_end = until.unwrap_or(event.start_time + 2 * 365 * 86400);

    let mut instances = vec![event.clone()]; // Include original.
    let mut current_start = event.start_time;
    let mut generated = 1usize;

    loop {
        if generated >= max_instances {
            break;
        }

        let next_start = match freq {
            "DAILY" => current_start + interval * 86400,
            "WEEKLY" => current_start + interval * 7 * 86400,
            "MONTHLY" => advance_months(current_start, interval),
            "YEARLY" => advance_months(current_start, interval * 12),
            _ => break, // Unknown frequency — don't expand.
        };

        if next_start > window_end {
            break;
        }

        let mut instance = event.clone();
        instance.id = format!("{}__recur_{generated}", event.id);
        instance.start_time = next_start;
        instance.end_time = next_start + duration;
        instances.push(instance);

        current_start = next_start;
        generated += 1;
    }

    instances
}

/// Advance a Unix timestamp by N months.
fn advance_months(timestamp: i64, months: i64) -> i64 {
    use chrono::TimeZone;
    let Some(dt) = chrono::Local.timestamp_opt(timestamp, 0).single() else {
        return timestamp + months * 30 * 86400; // Fallback.
    };
    let naive = dt.naive_local();
    let total_months = naive.month() as i64 - 1 + months;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let new_month = ((total_months % 12) + 1) as u32;
    #[allow(clippy::cast_possible_truncation)]
    let new_year = naive.year() + (total_months / 12) as i32;
    let new_day = naive.day().min(days_in_month(new_year, new_month));
    let Some(new_date) = chrono::NaiveDate::from_ymd_opt(new_year, new_month, new_day) else {
        return timestamp + months * 30 * 86400;
    };
    let new_naive = new_date.and_time(naive.time());
    chrono::Local
        .from_local_datetime(&new_naive)
        .single()
        .map_or(timestamp + months * 30 * 86400, |dt| dt.timestamp())
}

/// Days in a given month.
fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || year % 400 == 0 {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

/// Parse an UNTIL date string (YYYYMMDD or YYYYMMDDTHHMMSSZ) to Unix timestamp.
fn parse_until_date(val: &str) -> Option<i64> {
    let date_part = &val[..val.len().min(8)];
    let year: i32 = date_part.get(0..4)?.parse().ok()?;
    let month: u32 = date_part.get(4..6)?.parse().ok()?;
    let day: u32 = date_part.get(6..8)?.parse().ok()?;
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let dt = date.and_hms_opt(23, 59, 59)?;
    Some(dt.and_utc().timestamp())
}

// ── All-account calendar queries (for unified calendar) ────

pub async fn db_get_all_visible_calendars(db: &DbState) -> Result<Vec<DbCalendar>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(&format!(
                "SELECT {CALENDAR_COLS} FROM calendars WHERE is_visible = 1 \
                 ORDER BY account_id, is_primary DESC, sort_order, display_name ASC"
            ))
            .map_err(|e| e.to_string())?;
        stmt.query_map([], DbCalendar::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}
