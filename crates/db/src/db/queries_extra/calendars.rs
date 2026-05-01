use chrono::Datelike;

use super::super::DbState;
use super::super::types::{DbCalendar, DbCalendarAttendee, DbCalendarEvent, DbCalendarReminder};
use crate::db::from_row::FromRow;
use rusqlite::params;

/// Explicit column list for `calendars` table queries.
const CALENDAR_COLS: &str = "\
    id, account_id, provider, remote_id, display_name, color, is_primary, \
    is_visible, sync_token, ctag, created_at, updated_at, sort_order, \
    is_default, provider_id, can_edit";

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

#[allow(clippy::too_many_arguments)]
pub async fn db_upsert_calendar(
    db: &DbState,
    account_id: String,
    provider: String,
    remote_id: String,
    display_name: Option<String>,
    color: Option<String>,
    is_primary: bool,
    can_edit: bool,
) -> Result<String, String> {
    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary, can_edit)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   display_name = ?5, color = ?6, is_primary = ?7, can_edit = ?8, updated_at = unixepoch()",
            params![id, account_id, provider, remote_id, display_name, color, is_primary as i64, can_edit as i64],
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

#[derive(Debug, Clone)]
pub struct CalendarAttendeeWriteRow {
    pub email: String,
    pub name: Option<String>,
    pub rsvp_status: Option<String>,
    pub is_organizer: bool,
}

#[derive(Debug, Clone)]
pub struct CalendarReminderWriteRow {
    pub minutes_before: i64,
    pub method: Option<String>,
}

#[allow(clippy::too_many_arguments)]
pub fn upsert_calendar_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
    provider: &str,
    remote_id: &str,
    display_name: Option<&str>,
    color: Option<&str>,
    is_primary: bool,
    can_edit: bool,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary, can_edit)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(account_id, remote_id) DO UPDATE SET
               display_name = ?5, color = ?6, is_primary = ?7, can_edit = ?8, updated_at = unixepoch()",
        params![id, account_id, provider, remote_id, display_name, color, is_primary as i64, can_edit as i64],
    )
    .map_err(|e| e.to_string())?;

    conn.query_row(
        "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
        params![account_id, remote_id],
        |row| row.get("id"),
    )
    .map_err(|e| e.to_string())
}

pub fn load_calendar_sync_token_sync(
    conn: &rusqlite::Connection,
    calendar_id: &str,
) -> Result<Option<String>, String> {
    let result = conn.query_row(
        "SELECT sync_token FROM calendars WHERE id = ?1",
        params![calendar_id],
        |row| row.get::<_, Option<String>>(0),
    );
    match result {
        Ok(token) => Ok(token),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(format!("load sync token: {e}")),
    }
}

pub fn save_calendar_sync_token_sync(
    conn: &rusqlite::Connection,
    calendar_id: &str,
    token: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE calendars SET sync_token = ?1, updated_at = unixepoch() WHERE id = ?2",
        params![token, calendar_id],
    )
    .map_err(|e| format!("save sync token: {e}"))?;
    Ok(())
}

pub fn upsert_calendar_event_sync(
    conn: &rusqlite::Connection,
    p: &UpsertCalendarEventParams,
) -> Result<String, String> {
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
    .map_err(|e| e.to_string())?;

    conn.query_row(
        "SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2",
        params![p.account_id, p.google_event_id],
        |row| row.get(0),
    )
    .map_err(|e| format!("fetch event id: {e}"))
}

pub fn replace_event_attendees_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
    event_id: &str,
    attendees: &[CalendarAttendeeWriteRow],
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM calendar_attendees WHERE account_id = ?1 AND event_id = ?2",
        params![account_id, event_id],
    )
    .map_err(|e| format!("delete attendees: {e}"))?;

    for attendee in attendees {
        if attendee.email.is_empty() {
            continue;
        }
        conn.execute(
            "INSERT INTO calendar_attendees (event_id, account_id, email, name, rsvp_status, is_organizer)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, event_id, email) DO UPDATE SET
                   name = ?4, rsvp_status = ?5, is_organizer = ?6",
            params![
                event_id,
                account_id,
                attendee.email,
                attendee.name,
                attendee.rsvp_status,
                attendee.is_organizer as i64
            ],
        )
        .map_err(|e| format!("upsert attendee: {e}"))?;
    }

    Ok(())
}

pub fn replace_event_reminders_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
    event_id: &str,
    reminders: &[CalendarReminderWriteRow],
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM calendar_reminders WHERE account_id = ?1 AND event_id = ?2",
        params![account_id, event_id],
    )
    .map_err(|e| format!("delete reminders: {e}"))?;

    for reminder in reminders {
        conn.execute(
            "INSERT INTO calendar_reminders (event_id, account_id, minutes_before, method)
                 VALUES (?1, ?2, ?3, ?4)",
            params![event_id, account_id, reminder.minutes_before, reminder.method],
        )
        .map_err(|e| format!("insert reminder: {e}"))?;
    }

    Ok(())
}

pub fn delete_event_by_remote_id_sync(
    conn: &rusqlite::Connection,
    calendar_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
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
/// Supports a useful subset of RFC 5545 RRULE:
/// - FREQ: DAILY, WEEKLY, MONTHLY, YEARLY
/// - INTERVAL, COUNT, UNTIL
/// - BYDAY (e.g. `BYDAY=MO,WE,FR` on FREQ=WEEKLY/DAILY)
/// - BYMONTHDAY (FREQ=MONTHLY, picks specific day-of-month)
/// - BYMONTH (FREQ=YEARLY, picks specific month)
///
/// Generates instances within a ~2 year window from the event's original
/// start time. EXDATE handling is not yet wired (EXDATE is stored on a
/// separate iCal property, not part of the RRULE string).
fn expand_recurrence(event: &CalendarViewEvent, rrule_str: &str) -> Vec<CalendarViewEvent> {
    let rule = parse_rrule(rrule_str);
    let Some(freq) = Freq::parse(&rule.freq) else {
        return vec![event.clone()];
    };

    let duration = event.end_time - event.start_time;
    let max_instances = rule.count.unwrap_or(365).max(1);
    let window_end = rule
        .until
        .unwrap_or(event.start_time + 2 * 365 * 86400);

    let mut instances = Vec::with_capacity(max_instances);

    let candidate_starts = match freq {
        Freq::Daily => expand_daily(event.start_time, &rule),
        Freq::Weekly => expand_weekly(event.start_time, &rule),
        Freq::Monthly => expand_monthly(event.start_time, &rule),
        Freq::Yearly => expand_yearly(event.start_time, &rule),
    };

    for (idx, start) in candidate_starts.into_iter().enumerate() {
        if start > window_end {
            break;
        }
        if instances.len() >= max_instances {
            break;
        }
        let mut instance = event.clone();
        if idx > 0 {
            instance.id = format!("{}__recur_{idx}", event.id);
        }
        instance.start_time = start;
        instance.end_time = start + duration;
        instances.push(instance);
    }

    if instances.is_empty() {
        instances.push(event.clone());
    }
    instances
}

/// Parsed pieces of an RRULE string. Unknown parts are ignored silently.
#[derive(Debug, Default)]
struct Rrule {
    freq: String,
    interval: i64,
    count: Option<usize>,
    until: Option<i64>,
    byday: Vec<chrono::Weekday>,
    bymonthday: Vec<i32>,
    bymonth: Vec<u32>,
}

#[derive(Debug, Clone, Copy)]
enum Freq {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Freq {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "DAILY" => Some(Self::Daily),
            "WEEKLY" => Some(Self::Weekly),
            "MONTHLY" => Some(Self::Monthly),
            "YEARLY" => Some(Self::Yearly),
            _ => None,
        }
    }
}

fn parse_rrule(rrule_str: &str) -> Rrule {
    let body = rrule_str.strip_prefix("RRULE:").unwrap_or(rrule_str);
    let mut out = Rrule {
        interval: 1,
        ..Rrule::default()
    };
    for part in body.split(';') {
        if let Some(val) = part.strip_prefix("FREQ=") {
            out.freq = val.to_string();
        } else if let Some(val) = part.strip_prefix("INTERVAL=") {
            out.interval = val.parse().unwrap_or(1).max(1);
        } else if let Some(val) = part.strip_prefix("COUNT=") {
            out.count = val.parse().ok();
        } else if let Some(val) = part.strip_prefix("UNTIL=") {
            out.until = parse_until_date(val);
        } else if let Some(val) = part.strip_prefix("BYDAY=") {
            out.byday = val.split(',').filter_map(parse_weekday).collect();
        } else if let Some(val) = part.strip_prefix("BYMONTHDAY=") {
            out.bymonthday = val
                .split(',')
                .filter_map(|s| s.trim().parse::<i32>().ok())
                .filter(|d| {
                    let mag = d.unsigned_abs();
                    (1..=31).contains(&mag)
                })
                .collect();
        } else if let Some(val) = part.strip_prefix("BYMONTH=") {
            out.bymonth = val
                .split(',')
                .filter_map(|s| s.trim().parse::<u32>().ok())
                .filter(|m| (1..=12).contains(m))
                .collect();
        }
    }
    out
}

/// Parse an iCal weekday code, ignoring any leading +/-N ordinal prefix
/// (BYDAY supports things like "1MO" or "-1FR" which we treat as plain MO/FR).
fn parse_weekday(spec: &str) -> Option<chrono::Weekday> {
    let trimmed = spec.trim();
    let code = trimmed
        .trim_start_matches(|c: char| c == '-' || c == '+' || c.is_ascii_digit());
    match code {
        "MO" => Some(chrono::Weekday::Mon),
        "TU" => Some(chrono::Weekday::Tue),
        "WE" => Some(chrono::Weekday::Wed),
        "TH" => Some(chrono::Weekday::Thu),
        "FR" => Some(chrono::Weekday::Fri),
        "SA" => Some(chrono::Weekday::Sat),
        "SU" => Some(chrono::Weekday::Sun),
        _ => None,
    }
}

fn expand_daily(start: i64, rule: &Rrule) -> Vec<i64> {
    let cap = rule.count.unwrap_or(366);
    let mut out = Vec::with_capacity(cap);
    let mut current = start;
    while out.len() < cap {
        if rule.byday.is_empty() || matches_weekday(current, &rule.byday) {
            out.push(current);
        }
        current += rule.interval * 86400;
    }
    out
}

fn expand_weekly(start: i64, rule: &Rrule) -> Vec<i64> {
    let cap = rule.count.unwrap_or(366);
    let mut out = Vec::with_capacity(cap);
    let week = 7 * 86400;
    let interval_secs = rule.interval * week;

    if rule.byday.is_empty() {
        // Plain weekly recurrence on the same weekday as the start.
        let mut current = start;
        while out.len() < cap {
            out.push(current);
            current += interval_secs;
        }
        return out;
    }

    // Sort weekdays so each week emits in chronological order.
    let mut days = rule.byday.clone();
    days.sort_by_key(chrono::Weekday::num_days_from_monday);

    let week_start = start_of_week(start);
    let mut week_anchor = week_start;
    while out.len() < cap {
        for &wd in &days {
            let candidate = shift_to_weekday(week_anchor, wd, start);
            if candidate < start {
                continue;
            }
            out.push(candidate);
            if out.len() >= cap {
                break;
            }
        }
        week_anchor += interval_secs;
    }
    out
}

fn expand_monthly(start: i64, rule: &Rrule) -> Vec<i64> {
    let cap = rule.count.unwrap_or(120);
    let mut out = Vec::with_capacity(cap);
    let mut current = start;
    while out.len() < cap {
        if rule.bymonthday.is_empty() {
            out.push(current);
        } else {
            for &day in &rule.bymonthday {
                if let Some(ts) = set_day_of_month(current, day) {
                    if ts >= start {
                        out.push(ts);
                    }
                    if out.len() >= cap {
                        return out;
                    }
                }
            }
        }
        current = advance_months(current, rule.interval);
    }
    out
}

fn expand_yearly(start: i64, rule: &Rrule) -> Vec<i64> {
    let cap = rule.count.unwrap_or(60);
    let mut out = Vec::with_capacity(cap);
    let mut current = start;
    while out.len() < cap {
        if rule.bymonth.is_empty() {
            out.push(current);
        } else {
            for &month in &rule.bymonth {
                if let Some(ts) = set_month(current, month) {
                    if ts >= start {
                        out.push(ts);
                    }
                    if out.len() >= cap {
                        return out;
                    }
                }
            }
        }
        current = advance_months(current, rule.interval * 12);
    }
    out
}

fn matches_weekday(timestamp: i64, days: &[chrono::Weekday]) -> bool {
    use chrono::TimeZone;
    let Some(dt) = chrono::Local.timestamp_opt(timestamp, 0).single() else {
        return false;
    };
    let wd = dt.naive_local().date().weekday();
    days.contains(&wd)
}

fn start_of_week(timestamp: i64) -> i64 {
    use chrono::TimeZone;
    let Some(dt) = chrono::Local.timestamp_opt(timestamp, 0).single() else {
        return timestamp;
    };
    let weekday = dt.naive_local().date().weekday();
    let days_back = weekday.num_days_from_monday() as i64;
    timestamp - days_back * 86400
}

fn shift_to_weekday(week_anchor: i64, target: chrono::Weekday, time_source: i64) -> i64 {
    use chrono::TimeZone;
    let target_offset = target.num_days_from_monday() as i64;
    let candidate_date = week_anchor + target_offset * 86400;
    let Some(time_dt) = chrono::Local.timestamp_opt(time_source, 0).single() else {
        return candidate_date;
    };
    let Some(date_dt) = chrono::Local.timestamp_opt(candidate_date, 0).single() else {
        return candidate_date;
    };
    let new_naive = date_dt.naive_local().date().and_time(time_dt.time());
    chrono::Local
        .from_local_datetime(&new_naive)
        .single()
        .map_or(candidate_date, |dt| dt.timestamp())
}

fn set_day_of_month(timestamp: i64, day: i32) -> Option<i64> {
    use chrono::TimeZone;
    let dt = chrono::Local.timestamp_opt(timestamp, 0).single()?;
    let naive = dt.naive_local();
    let dim = days_in_month(naive.year(), naive.month());
    #[allow(clippy::cast_possible_wrap)]
    let dim_i = dim as i32;
    let resolved_day = if day < 0 { dim_i + day + 1 } else { day };
    if resolved_day < 1 || resolved_day > dim_i {
        return None;
    }
    #[allow(clippy::cast_sign_loss)]
    let new_date =
        chrono::NaiveDate::from_ymd_opt(naive.year(), naive.month(), resolved_day as u32)?;
    let new_naive = new_date.and_time(naive.time());
    chrono::Local
        .from_local_datetime(&new_naive)
        .single()
        .map(|d| d.timestamp())
}

fn set_month(timestamp: i64, month: u32) -> Option<i64> {
    use chrono::TimeZone;
    let dt = chrono::Local.timestamp_opt(timestamp, 0).single()?;
    let naive = dt.naive_local();
    let new_day = naive.day().min(days_in_month(naive.year(), month));
    let new_date = chrono::NaiveDate::from_ymd_opt(naive.year(), month, new_day)?;
    let new_naive = new_date.and_time(naive.time());
    chrono::Local
        .from_local_datetime(&new_naive)
        .single()
        .map(|d| d.timestamp())
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Timelike};

    fn local_ts(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> i64 {
        let date = chrono::NaiveDate::from_ymd_opt(year, month, day)
            .expect("valid date")
            .and_hms_opt(hour, minute, 0)
            .expect("valid time");
        chrono::Local
            .from_local_datetime(&date)
            .single()
            .expect("unambiguous")
            .timestamp()
    }

    fn make_event(start: i64, duration: i64) -> CalendarViewEvent {
        CalendarViewEvent {
            id: "evt".to_string(),
            title: String::new(),
            start_time: start,
            end_time: start + duration,
            all_day: false,
            color: String::new(),
            calendar_name: None,
            location: None,
            recurrence_rule: None,
            calendar_id: None,
            account_id: String::new(),
            organizer_name: None,
            organizer_email: None,
            rsvp_status: None,
            description: None,
            availability: None,
            visibility: None,
            timezone: None,
        }
    }

    fn weekday_of(ts: i64) -> chrono::Weekday {
        chrono::Local
            .timestamp_opt(ts, 0)
            .single()
            .expect("local")
            .naive_local()
            .date()
            .weekday()
    }

    #[test]
    fn weekly_byday_emits_each_listed_day() {
        // 2026-03-09 is a Monday. RRULE picks Monday/Wednesday/Friday for 6 weeks.
        let start = local_ts(2026, 3, 9, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=WEEKLY;BYDAY=MO,WE,FR;COUNT=6");
        assert_eq!(instances.len(), 6);
        let weekdays: Vec<_> = instances
            .iter()
            .map(|e| weekday_of(e.start_time))
            .collect();
        assert_eq!(
            weekdays,
            vec![
                chrono::Weekday::Mon,
                chrono::Weekday::Wed,
                chrono::Weekday::Fri,
                chrono::Weekday::Mon,
                chrono::Weekday::Wed,
                chrono::Weekday::Fri,
            ]
        );
    }

    #[test]
    fn weekly_byday_preserves_time_of_day() {
        // 2026-03-09 09:30 Mon. BYDAY=MO,WE - time-of-day must stay 09:30 on
        // every emitted instance, even when the day shifts.
        let start = local_ts(2026, 3, 9, 9, 30);
        let event = make_event(start, 1800);
        let instances = expand_recurrence(&event, "FREQ=WEEKLY;BYDAY=MO,WE;COUNT=4");
        for inst in &instances {
            let dt = chrono::Local
                .timestamp_opt(inst.start_time, 0)
                .single()
                .expect("local");
            assert_eq!(dt.naive_local().time().hour(), 9);
            assert_eq!(dt.naive_local().time().minute(), 30);
        }
    }

    #[test]
    fn monthly_bymonthday_picks_specific_day() {
        // FREQ=MONTHLY;BYMONTHDAY=15 starting on 2026-01-10 emits the 15th of
        // Jan, Feb, Mar, ... not the 10th.
        let start = local_ts(2026, 1, 10, 12, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=MONTHLY;BYMONTHDAY=15;COUNT=3");
        assert_eq!(instances.len(), 3);
        for inst in &instances {
            let dt = chrono::Local
                .timestamp_opt(inst.start_time, 0)
                .single()
                .expect("local");
            assert_eq!(dt.naive_local().date().day(), 15);
        }
    }

    #[test]
    fn yearly_with_until_clamps_window() {
        // Annual on 2026-06-01, UNTIL 2028-06-01 -> 3 instances.
        let start = local_ts(2026, 6, 1, 9, 0);
        let event = make_event(start, 3600);
        let instances =
            expand_recurrence(&event, "FREQ=YEARLY;UNTIL=20280701T000000Z");
        assert_eq!(instances.len(), 3);
        assert_eq!(weekday_of(instances[0].start_time), weekday_of(start));
    }

    #[test]
    fn unknown_freq_returns_single_instance() {
        let start = local_ts(2026, 1, 1, 9, 0);
        let event = make_event(start, 1800);
        let instances = expand_recurrence(&event, "FREQ=BOGUS");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].start_time, start);
    }
}
