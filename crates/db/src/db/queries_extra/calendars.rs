use std::collections::{HashMap, HashSet};

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
///
/// `recurrence_id` is the canonical wall-clock form of an override's
/// RECURRENCE-ID (see `migrations.rs` for the form). Selected with the
/// rest of the row so the load path can subtract phantom master instances
/// without a second query.
const EVENT_COLS: &str = "\
    id, account_id, google_event_id, summary, description, location, \
    start_time, end_time, is_all_day, status, organizer_email, attendees_json, \
    html_link, updated_at, calendar_id, remote_event_id, etag, ical_data, uid, \
    title, timezone, recurrence_rule, organizer_name, rsvp_status, created_at, \
    availability, visibility, recurrence_id";

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
    /// Canonical, host-TZ-independent RECURRENCE-ID for override rows.
    /// `None` for master rows. See the schema column comment for the
    /// canonical forms.
    pub recurrence_id: Option<String>,
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
             availability, visibility, recurrence_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                     ?17, ?18, ?19, ?20, ?21, ?22, ?23, unixepoch(), ?24, ?25, ?26)
             ON CONFLICT(account_id, google_event_id) DO UPDATE SET
               summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
               is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
               html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
               ical_data = ?17, uid = ?18, title = ?19, timezone = ?20, recurrence_rule = ?21,
               organizer_name = ?22, rsvp_status = ?23, availability = ?24, visibility = ?25,
               recurrence_id = ?26, updated_at = unixepoch()",
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
            p.visibility,
            p.recurrence_id
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
                 availability, visibility, recurrence_id)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                         ?17, ?18, ?19, ?20, ?21, ?22, ?23, unixepoch(), ?24, ?25, ?26)
                 ON CONFLICT(account_id, google_event_id) DO UPDATE SET
                   summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
                   is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
                   html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
                   ical_data = ?17, uid = ?18, title = ?19, timezone = ?20, recurrence_rule = ?21,
                   organizer_name = ?22, rsvp_status = ?23, availability = ?24, visibility = ?25,
                   recurrence_id = ?26, updated_at = unixepoch()",
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
                p.visibility,
                p.recurrence_id
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
    /// VEVENT UID. The load path uses `(account_id, uid)` to subtract
    /// override slots from the master's RRULE expansion - master and
    /// overrides share UID by construction (RFC 5545 § 3.8.4.4).
    pub uid: Option<String>,
    /// Canonical RECURRENCE-ID for override rows. `None` for master rows.
    /// Carries the wall-clock string from the iCal source (see schema
    /// comment) so the dedup decision is independent of the host's local
    /// zone. Format-equality with the same canonicalization the master
    /// expansion produces is what makes phantom dedup possible.
    pub recurrence_id_canonical: Option<String>,
}

/// Load all calendar events with resolved calendar colors (synchronous).
///
/// Recurring events are expanded into concrete instances; rows that
/// override a specific instance via RECURRENCE-ID are emitted in place
/// of the corresponding master expansion slot. Without that subtraction,
/// a master series with one moved instance would render as N+1 events
/// (the original slot AND the override) - exactly the phantom-override
/// regression review #1 surfaced after the storage-key fix in Round 2.
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
                    e.description, e.availability, e.visibility, e.timezone,
                    e.uid, e.recurrence_id
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
                uid: row.get("uid")?,
                recurrence_id_canonical: row.get("recurrence_id")?,
            })
        })
        .map_err(|e| e.to_string())?;
    let base_events: Vec<CalendarViewEvent> = rows
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    // Build a (account_id, uid) -> set of override canonical strings index
    // before expansion. The master row carries `recurrence_id_canonical =
    // None` and the RRULE; each override row carries its own
    // `recurrence_id_canonical = Some(...)`. While walking the master we
    // canonicalise each candidate timestamp with the same shape and skip
    // it when it lands in this set.
    let mut overrides_by_series: HashMap<(String, String), HashSet<String>> = HashMap::new();
    for ev in &base_events {
        if let (Some(uid), Some(canonical)) =
            (ev.uid.as_ref(), ev.recurrence_id_canonical.as_ref())
        {
            overrides_by_series
                .entry((ev.account_id.clone(), uid.clone()))
                .or_default()
                .insert(canonical.clone());
        }
    }

    // Expand recurring events into concrete instances.
    let mut expanded = Vec::with_capacity(base_events.len());
    for ev in base_events {
        if let Some(ref rrule) = ev.recurrence_rule {
            let overrides = ev
                .uid
                .as_ref()
                .and_then(|uid| {
                    overrides_by_series.get(&(ev.account_id.clone(), uid.clone()))
                })
                .cloned()
                .unwrap_or_default();
            let mut instances = expand_recurrence_with_overrides(&ev, rrule, &overrides);
            expanded.append(&mut instances);
        } else {
            expanded.push(ev);
        }
    }
    expanded.sort_by_key(|e| e.start_time);
    Ok(expanded)
}

/// Hard cap on COUNT to bound allocation. Real-world recurring events stay
/// well under this; a remote server emitting `COUNT=4294967295` cannot pin
/// us to a multi-GB Vec.
const RRULE_MAX_COUNT: usize = 10_000;

/// Hard cap on iteration steps inside any single expander, regardless of how
/// many instances actually get produced. Defends against the "BYDAY filter
/// matches nothing" / "BYMONTHDAY=31 in only February" infinite-loop pattern
/// where `out.len()` never grows. Set well above any legitimate workload:
/// ~30 years of daily checks. If a real RRULE legitimately needs more, COUNT
/// or UNTIL will terminate first.
const RRULE_MAX_STEPS: usize = 12_000;

/// Pick the per-expander instance cap.
///
/// When `rule.count` is present, that's the explicit upper bound (already
/// clamped to `RRULE_MAX_COUNT` at parse time). When `rule.until` is
/// present without COUNT, we let the expander run up to `RRULE_MAX_COUNT`:
/// the time bound (window_end / UNTIL) terminates the loop, and a per-
/// expander default of 800 silently truncated long-UNTIL rules far below
/// what the user asked for - e.g. `FREQ=YEARLY;UNTIL=22000101T000000Z`
/// from 2026 emitted 60 instances and stopped 114 years short of the
/// requested UNTIL. (Round 3 #2.)
///
/// When neither COUNT nor UNTIL is set, we fall back to the
/// `default_unbounded` cap. The 2-year synthesised window
/// (`two_year_window_end`) is the time bound there, but dense BY-rules
/// like `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR` (5/wk × 104wk = 520
/// emissions) would be truncated by a smaller default - the standup
/// vanishes 17 months in. The default below is chosen to cover the
/// densest realistic 2-year span. (Round 3 #4.)
fn instance_cap(rule: &Rrule, default_unbounded: usize) -> usize {
    if let Some(n) = rule.count {
        return n.max(1);
    }
    if rule.until.is_some() {
        return RRULE_MAX_COUNT;
    }
    default_unbounded.clamp(1, RRULE_MAX_COUNT)
}

/// The wall-clock zone an event recurs in. RFC 5545 § 3.3.10: recurring
/// events keep their wall-clock time across DST transitions and across
/// instances - 09:00 every day means 09:00 *in that zone*, not 09:00 in
/// `chrono::Local` (the previous behavior, which silently shifted every
/// recurring event by the user's UTC offset relative to its source zone).
///
/// `Iana` covers any TZID we can resolve via `chrono_tz`. `Local` covers
/// floating events (no TZID stored) and any TZID we couldn't parse - notably
/// Windows zone names like "Pacific Standard Time" that the parse layer
/// resolves at sync time but stores in `event.timezone` as-is. Threading
/// calcard's resolver into expansion would honor those, but that pulls
/// calcard into the db crate; the warn-and-fall-back path keeps the previous
/// behavior for the unresolved tail without infecting the dep graph.
#[derive(Debug, Clone, Copy)]
enum RecurrenceTz {
    Iana(chrono_tz::Tz),
    Local,
}

impl RecurrenceTz {
    fn from_event_timezone(event_tz: Option<&str>) -> Self {
        let Some(raw) = event_tz else {
            return Self::Local;
        };
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Self::Local;
        }
        match trimmed.parse::<chrono_tz::Tz>() {
            Ok(tz) => Self::Iana(tz),
            Err(_) => {
                log::debug!(
                    "RRULE expansion: event.timezone={trimmed:?} did not parse as IANA; \
                     falling back to local zone"
                );
                Self::Local
            }
        }
    }

    fn naive(self, timestamp: i64) -> Option<chrono::NaiveDateTime> {
        use chrono::TimeZone;
        match self {
            Self::Iana(tz) => tz
                .timestamp_opt(timestamp, 0)
                .single()
                .map(|d| d.naive_local()),
            Self::Local => chrono::Local
                .timestamp_opt(timestamp, 0)
                .single()
                .map(|d| d.naive_local()),
        }
    }

    fn resolve(self, naive: chrono::NaiveDateTime) -> Option<i64> {
        match self {
            Self::Iana(tz) => crate::db::time::resolve_local_to_timestamp(naive, &tz),
            Self::Local => crate::db::time::resolve_local_to_timestamp(naive, &chrono::Local),
        }
    }
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
///
/// Convenience wrapper for tests that don't need to subtract override
/// slots; routes through `expand_recurrence_with_overrides` with an
/// empty set. Production callers go through the `_with_overrides` form
/// directly so the override set the load-path built actually flows in.
#[cfg(test)]
fn expand_recurrence(event: &CalendarViewEvent, rrule_str: &str) -> Vec<CalendarViewEvent> {
    expand_recurrence_with_overrides(event, rrule_str, &HashSet::new())
}

/// Expand a recurring event into concrete instances, subtracting slots
/// already claimed by RECURRENCE-ID override rows.
///
/// `overrides` carries the canonical wall-clock RECURRENCE-ID strings of
/// the override rows for this UID. For each candidate timestamp produced
/// by the master expansion, we re-canonicalise the candidate using the
/// master's `event.timezone` and `event.all_day` and drop it when the
/// resulting string sits in the override set. The override row itself
/// already exists as a separate non-recurring row (its own start_time
/// drives display), so without this subtraction the user sees both the
/// untouched master slot AND the moved override - the phantom-duplication
/// regression flagged as #1 in the calendar review findings.
fn expand_recurrence_with_overrides(
    event: &CalendarViewEvent,
    rrule_str: &str,
    overrides: &HashSet<String>,
) -> Vec<CalendarViewEvent> {
    let rule = parse_rrule(rrule_str);
    let tz = RecurrenceTz::from_event_timezone(event.timezone.as_deref());
    let Some(freq) = Freq::parse(&rule.freq) else {
        // FREQ is missing or unrecognized. We fall back to a single instance
        // (the master event) so the operator at least sees the event on the
        // calendar; logging here surfaces the malformed rule.
        //
        // Logged at debug rather than warn because this branch fires from
        // every view render via `load_calendar_events_for_view_sync`. A
        // calendar with N malformed RRULEs (and Outlook bridges + Apple
        // Calendar exports do produce these) would otherwise emit N WARN
        // lines per refresh, drowning out actual operational signal. The
        // sync-time parse pass already records the same VEVENTs, so the
        // signal isn't lost - just not repeated. (Round 3 #44.)
        log::debug!(
            "RRULE has unrecognized or missing FREQ; emitting only master instance: {rrule_str}"
        );
        return vec![event.clone()];
    };
    if !rule.unsupported_parts.is_empty() {
        // Recognized but unimplemented BY-rules (e.g. BYSETPOS, BYWEEKNO).
        // Falling through to the expanders would produce a wildly wrong
        // expansion (~22 days/month for `BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1`).
        // Emit only the master instance so the user sees the event in the
        // right place without the noise. Debug-level for the same
        // render-loop reason as the FREQ branch above.
        log::debug!(
            "RRULE uses unsupported parts ({:?}); emitting only master instance: {rrule_str}",
            rule.unsupported_parts
        );
        return vec![event.clone()];
    }
    if matches!(freq, Freq::Yearly)
        && rule.bymonth.is_empty()
        && rule.byday.iter().any(|b| b.ordinal.is_some())
    {
        // YEARLY + ordinal BYDAY without BYMONTH means "the n-th weekday of
        // the year" (RFC 5545 § 3.3.10). The expander only walks per-month
        // ordinals (`nth_weekday_in_month`), so a rule like
        // `FREQ=YEARLY;BYDAY=20MO` would silently emit zero instances - no
        // single month has 20 Mondays. Emit the master instance and log;
        // the year-scope ordinal walk is a real feature, not a defensive
        // tweak, and is left as a follow-up. Debug-level (see above).
        log::debug!(
            "RRULE FREQ=YEARLY with ordinal BYDAY and no BYMONTH would require a year-scope ordinal walk; emitting only master instance: {rrule_str}"
        );
        return vec![event.clone()];
    }

    // Wall-clock duration in the event's zone, not raw UTC seconds. Captures
    // the user's intent ("the meeting is 1 hour long") rather than the UTC
    // span of the master instance ("the meeting is 0 or 2 hours long if it
    // happens to span the DST transition"). Each recurring instance then
    // resolves end_time = start_naive + wall_duration in the event's zone,
    // so an all-day event ending at midnight stays at midnight on every
    // instance regardless of DST gaps - and a timed event stays the same
    // wall-clock length whether or not the master spanned DST.
    //
    // All-day events get a separate path: the parse layer already anchored
    // end_time to `start + days*86400`, so when the master spans the
    // spring-forward boundary `end_naive` lands at `01:00` the day after
    // `end_date()`. A naive `end_naive - start_naive` then gives 25 hours,
    // which propagated to every subsequent recurring instance shows them
    // ending at 01:00 instead of midnight. Compute the duration from the
    // date delta directly so 1 calendar day stays 1 calendar day across
    // DST. Symmetric for fall-back (avoids 23h drift). (Round 3 #22.)
    let raw_duration = event.end_time - event.start_time;
    let wall_duration = if event.all_day {
        match (tz.naive(event.start_time), tz.naive(event.end_time)) {
            (Some(s), Some(e)) => {
                let mut days = e.date().signed_duration_since(s.date()).num_days();
                // If end_naive landed before midnight (DST fall-back, where
                // start+86400 sits at 23:00 the same day), the date delta
                // would underreport by one. Round up so a 1-day event stays
                // 1 day. The condition is: end_naive's clock sits past
                // midnight relative to start_naive (i.e. raw seconds floor-
                // divided by 86400 is at least one).
                if days == 0 && raw_duration > 0 {
                    days = 1;
                }
                chrono::Duration::days(days.max(0))
            }
            _ => chrono::Duration::seconds(raw_duration),
        }
    } else {
        match (tz.naive(event.start_time), tz.naive(event.end_time)) {
            (Some(s), Some(e)) => e.signed_duration_since(s),
            _ => chrono::Duration::seconds(raw_duration),
        }
    };

    // Outer cap: lets explicit COUNT through, lets UNTIL-bounded rules run
    // to RRULE_MAX_COUNT (the time bound terminates), and falls back to a
    // 2-year-window-sized default when neither is set. See `instance_cap`
    // for the rationale and review-finding cross-references.
    let max_instances = instance_cap(&rule, 800);
    if rule.count.is_some() && rule.until.is_some() {
        // RFC 5545 § 3.3.10: COUNT and UNTIL are mutually exclusive. Some
        // emitters send both anyway; we apply BOTH as upper bounds (the
        // intersection is always a subset of either, so the result stays
        // within the more permissive interpretation either rule alone would
        // permit). Logged so an operator can spot misbehaving servers.
        log::debug!(
            "RRULE has both COUNT and UNTIL (mutually exclusive per RFC 5545); applying both as bounds"
        );
    }
    // Window bounds:
    // - UNTIL set: hard bound, applies regardless of COUNT.
    // - COUNT set without UNTIL: no time bound; COUNT alone limits output.
    // - Neither: synthesize a 2-year fallback window so an unbounded rule
    //   doesn't run away.
    let window_end = match (rule.until, rule.count) {
        (Some(until), _) => until,
        (None, Some(_)) => i64::MAX,
        (None, None) => two_year_window_end(event.start_time, tz),
    };

    let mut instances = Vec::with_capacity(max_instances);

    let candidate_starts = match freq {
        Freq::Daily => expand_daily(event.start_time, &rule, tz),
        Freq::Weekly => expand_weekly(event.start_time, &rule, tz),
        Freq::Monthly => expand_monthly(event.start_time, &rule, tz),
        Freq::Yearly => expand_yearly(event.start_time, &rule, tz),
    };

    for (idx, start) in candidate_starts.into_iter().enumerate() {
        if start > window_end {
            break;
        }
        if instances.len() >= max_instances {
            break;
        }
        // Phantom-override dedup: when a stored row carries a
        // RECURRENCE-ID matching this candidate's wall-clock slot, the
        // override row will render in its place (its own start_time drives
        // the displayed time). Emitting the master candidate too produces
        // two events for the same slot - the user sees the original *and*
        // the moved instance.
        if !overrides.is_empty() {
            let canonical = canonical_recurrence_slot(start, tz, event.all_day);
            if overrides.contains(&canonical) {
                continue;
            }
        }
        let mut instance = event.clone();
        if idx > 0 {
            instance.id = format!("{}__recur_{idx}", event.id);
        }
        instance.start_time = start;
        instance.end_time = end_time_for_instance(start, wall_duration, tz, raw_duration);
        // Recurring instances inherit the master's identity; never their
        // own override key. Stash uid alongside so that future code paths
        // (e.g. clicking through to an instance) keep the master link.
        instance.recurrence_id_canonical = None;
        instances.push(instance);
    }

    // Note: when the RRULE produces zero instances (e.g. UNTIL is in the
    // past, or every BYxxx filter rejects every visited candidate), we
    // return an empty Vec rather than synthesizing the original event.
    // The previous fallback hid genuine "this rule expires in the past"
    // states from the caller.
    instances
}

/// Format a master expansion candidate the same way an override's
/// RECURRENCE-ID is canonicalised at parse time, so the two strings
/// compare equal and the load-path can subtract phantoms.
///
/// The format must match `parse::extract_recurrence_id_canonical` exactly:
///   - all-day  -> `YYYYMMDD`
///   - zoned    -> `YYYYMMDDTHHMMSS;TZID=<id>`
///   - floating -> `YYYYMMDDTHHMMSS`
///
/// We intentionally don't emit the `Z` form here: master expansion
/// candidates were resolved through the master's TZID context (whatever
/// `event.timezone` was set to). When the iCal source carried a UTC
/// DTSTART (no TZID), `event.timezone` is None and the master walks in
/// `chrono::Local` - producing a wall-clock candidate that lines up with
/// a floating-form override, which is the convention sane emitters use
/// across master/override pairs anyway. UTC-form override + non-UTC
/// master is a malformed feed; we don't try to dedup that combination.
fn canonical_recurrence_slot(timestamp: i64, tz: RecurrenceTz, all_day: bool) -> String {
    let Some(naive) = tz.naive(timestamp) else {
        // Resolution failed (timestamp out of chrono's range). Return a
        // sentinel that won't collide with any real iCal canonical form;
        // the worst case is one missed dedup on a pathological event.
        return format!("__unresolvable_slot_{timestamp}");
    };
    if all_day {
        return naive.format("%Y%m%d").to_string();
    }
    let body = naive.format("%Y%m%dT%H%M%S").to_string();
    match tz {
        RecurrenceTz::Iana(zone) => format!("{body};TZID={}", zone.name()),
        RecurrenceTz::Local => body,
    }
}

/// Compute end_time for a recurring instance by walking `wall_duration` in
/// the event's wall-clock zone, falling back to raw-seconds arithmetic if
/// the wall-clock walk overflows or hits a non-resolvable zone state. The
/// fallback is the previous behavior; the new path prevents an all-day
/// recurring event whose master spans DST from inheriting a 23h/47h
/// duration on every subsequent instance.
fn end_time_for_instance(
    start: i64,
    wall_duration: chrono::Duration,
    tz: RecurrenceTz,
    raw_duration: i64,
) -> i64 {
    tz.naive(start)
        .and_then(|n| n.checked_add_signed(wall_duration))
        .and_then(|n| tz.resolve(n))
        .unwrap_or(start + raw_duration)
}

/// Compute the 2-year window-end timestamp using calendar arithmetic so
/// leap years are accounted for. Falls back to a 730-day approximation if
/// the start timestamp is somehow out of chrono's representable range.
fn two_year_window_end(start: i64, tz: RecurrenceTz) -> i64 {
    tz.naive(start)
        .and_then(|n| n.with_year(n.year() + 2))
        .and_then(|n| tz.resolve(n))
        .unwrap_or(start + 730 * 86400)
}

/// A single BYDAY entry. The ordinal prefix (e.g. `1MO`, `-1FR`) is captured
/// alongside the bare weekday so `FREQ=MONTHLY;BYDAY=1MO` ("first Monday of
/// the month") and `FREQ=YEARLY;BYDAY=-1SU` ("last Sunday of the year")
/// expand correctly. For DAILY/WEEKLY/UNTIL the ordinal is ignored (RFC 5545
/// § 3.3.10 says it's only meaningful in MONTHLY/YEARLY).
#[derive(Debug, Clone, Copy)]
struct ByDay {
    /// `None` means "every occurrence of `day` in the period", `Some(n)`
    /// means "the n-th occurrence" (negative counts from the end).
    ordinal: Option<i32>,
    day: chrono::Weekday,
}

/// Parsed pieces of an RRULE string. Unknown parts are ignored silently
/// unless they're in the documented "unsupported but recognized" set
/// (`unsupported_parts`), in which case the rule is treated as malformed
/// rather than silently mis-expanded.
#[derive(Debug, Default)]
struct Rrule {
    freq: String,
    interval: i64,
    count: Option<usize>,
    until: Option<i64>,
    byday: Vec<ByDay>,
    bymonthday: Vec<i32>,
    bymonth: Vec<u32>,
    /// Week-start day. `None` means "use the default" - we treat that as
    /// Monday (RFC 5545 § 3.3.10 default), which matches what most weekly
    /// recurrence views expect. Set explicitly via `WKST=SU` etc.
    ///
    /// Currently consumed only by `expand_weekly`; YEARLY and MONTHLY
    /// ignore it. That's correct only as long as BYWEEKNO stays
    /// unsupported. RFC 5545 § 3.3.10: BYWEEKNO numbers weeks of the year,
    /// where the first week is the one containing the wkst's first
    /// occurrence. Adding BYWEEKNO without plumbing wkst into the YEARLY
    /// expander would silently shift week-1 anchoring by up to 6 days for
    /// any rule that does not opt into the default WKST=MO.
    wkst: Option<chrono::Weekday>,
    /// RFC 5545 BY-rules we recognize but don't yet implement. Populated by
    /// `parse_rrule` so `expand_recurrence` can short-circuit instead of
    /// silently producing wrong expansions (e.g. `BYSETPOS=-1` filtering
    /// only the last weekday of the month would otherwise emit ~22 days
    /// per month). Each entry is the bare key name (`"BYSETPOS"` etc).
    unsupported_parts: Vec<&'static str>,
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
            let raw = val.parse::<i64>().unwrap_or(1);
            if raw < 1 {
                log::debug!(
                    "RRULE INTERVAL={raw} (RFC 5545 requires >=1); clamping to 1"
                );
            }
            out.interval = raw.max(1);
        } else if let Some(val) = part.strip_prefix("COUNT=") {
            // Clamp untrusted COUNT values to a sane upper bound so a remote
            // server cannot trigger pathological allocation. Anything above
            // RRULE_MAX_COUNT lands at the cap; legitimate recurring events
            // never come close.
            let raw = val.parse::<usize>().ok();
            if let Some(n) = raw
                && n > RRULE_MAX_COUNT
            {
                log::debug!(
                    "RRULE COUNT={n} exceeds RRULE_MAX_COUNT={RRULE_MAX_COUNT}; truncating expansion"
                );
            }
            out.count = raw.map(|n| n.min(RRULE_MAX_COUNT));
        } else if let Some(val) = part.strip_prefix("UNTIL=") {
            out.until = parse_until_date(val);
        } else if let Some(val) = part.strip_prefix("BYDAY=") {
            out.byday = val.split(',').filter_map(parse_byday).collect();
        } else if let Some(val) = part.strip_prefix("BYMONTHDAY=") {
            let raw_count = val.split(',').count();
            out.bymonthday = val
                .split(',')
                .filter_map(|s| s.trim().parse::<i32>().ok())
                .filter(|d| {
                    let mag = d.unsigned_abs();
                    (1..=31).contains(&mag)
                })
                .collect();
            if out.bymonthday.len() != raw_count {
                log::debug!(
                    "RRULE BYMONTHDAY=`{val}` had {} of {raw_count} entries dropped (RFC 5545: magnitude must be 1..=31)",
                    raw_count - out.bymonthday.len()
                );
            }
        } else if let Some(val) = part.strip_prefix("BYMONTH=") {
            let raw_count = val.split(',').count();
            out.bymonth = val
                .split(',')
                .filter_map(|s| s.trim().parse::<u32>().ok())
                .filter(|m| (1..=12).contains(m))
                .collect();
            if out.bymonth.len() != raw_count {
                log::debug!(
                    "RRULE BYMONTH=`{val}` had {} of {raw_count} entries dropped (RFC 5545: must be 1..=12)",
                    raw_count - out.bymonth.len()
                );
            }
        } else if let Some(val) = part.strip_prefix("WKST=") {
            out.wkst = parse_weekday_code(val.trim());
        } else {
            // Recognize-but-flag the BY-rules we can't honor. Listing them
            // explicitly (rather than treating any unknown key as malformed)
            // keeps the door open to vendor extensions and future-spec keys
            // without breaking compatibility, while still catching the cases
            // that produce the worst silent expansions.
            for unsupported in [
                "BYSETPOS=",
                "BYWEEKNO=",
                "BYYEARDAY=",
                "BYHOUR=",
                "BYMINUTE=",
                "BYSECOND=",
            ] {
                if part.starts_with(unsupported) {
                    let key = &unsupported[..unsupported.len() - 1];
                    if !out.unsupported_parts.contains(&key) {
                        out.unsupported_parts.push(key);
                    }
                    break;
                }
            }
        }
    }
    out
}

/// Parse a bare iCal weekday token (no ordinal prefix). Used for `WKST=`
/// and as a helper for the BYDAY parser.
fn parse_weekday_code(code: &str) -> Option<chrono::Weekday> {
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

/// Parse a BYDAY entry, including the optional ordinal prefix.
///
/// `MO` -> ordinal=None, day=Mon (every Monday in the period).
/// `1MO` -> ordinal=Some(1), day=Mon (first Monday).
/// `-1FR` -> ordinal=Some(-1), day=Fri (last Friday).
fn parse_byday(spec: &str) -> Option<ByDay> {
    let trimmed = spec.trim();
    let bytes = trimmed.as_bytes();
    let mut idx = 0;
    let sign: i32 = match bytes.first() {
        Some(b'-') => {
            idx += 1;
            -1
        }
        Some(b'+') => {
            idx += 1;
            1
        }
        _ => 1,
    };
    let digit_start = idx;
    while bytes.get(idx).is_some_and(u8::is_ascii_digit) {
        idx += 1;
    }
    let ordinal = if idx > digit_start {
        let n = std::str::from_utf8(&bytes[digit_start..idx])
            .ok()?
            .parse::<i32>()
            .ok()?;
        // RFC 5545 § 3.3.10: BYDAY ordinal magnitude is 1..=53 (or
        // -53..=-1). Out-of-range values produce no instances at expansion
        // time anyway (no month has 99 Mondays), but the rule then bounds
        // out via `RRULE_MAX_STEPS=12_000` after a noticeable amount of
        // wasted work. Reject upfront with a debug log so the operator
        // can attribute the dropped rule.
        if n == 0 {
            return None;
        }
        if n.unsigned_abs() > 53 {
            log::debug!("RRULE BYDAY ordinal {n} out of range (RFC 5545: 1..=53); dropping entry");
            return None;
        }
        Some(sign * n)
    } else {
        None
    };
    let code = std::str::from_utf8(&bytes[idx..]).ok()?;
    parse_weekday_code(code).map(|day| ByDay { ordinal, day })
}

fn expand_daily(start: i64, rule: &Rrule, tz: RecurrenceTz) -> Vec<i64> {
    // Default unbounded cap matches the 2-year fallback window's worst case
    // for FREQ=DAILY (730 days). UNTIL-bounded rules run to RRULE_MAX_COUNT
    // and let the time bound terminate; see `instance_cap`.
    let cap = instance_cap(rule, 800);
    let mut out = Vec::with_capacity(cap);
    let mut current = start;
    // Step-bounded iteration: a BYDAY filter can reject 6 of every 7
    // candidates, and pathological filters (e.g. `BYDAY=TU` on a daily rule
    // with `INTERVAL=7` starting on Monday) match nothing - without a step
    // cap we spin forever.
    for _ in 0..RRULE_MAX_STEPS {
        if out.len() >= cap {
            break;
        }
        if rule.byday.is_empty()
            || matches_weekday(
                current,
                &rule.byday.iter().map(|b| b.day).collect::<Vec<_>>(),
                tz,
            )
        {
            out.push(current);
        }
        // Advance in calendar days, not raw seconds, so wall-clock time is
        // preserved across DST transitions. A 09:00 daily event spans the
        // spring-forward gap as 09:00 each day, not 10:00 from the
        // transition forward.
        current =
            add_days_in_zone(current, rule.interval, tz).unwrap_or(current + rule.interval * 86400);
    }
    out
}

fn expand_weekly(start: i64, rule: &Rrule, tz: RecurrenceTz) -> Vec<i64> {
    // Bumped from 366 to 800: the previous default truncated dense BY-rules
    // (e.g. `BYDAY=MO,TU,WE,TH,FR` = 5/wk × 104wk = 520 emissions) inside
    // the 2-year synthesised fallback window. The standup vanished from
    // the calendar 17 months in. (Round 3 #4.) UNTIL-bounded rules run to
    // RRULE_MAX_COUNT.
    let cap = instance_cap(rule, 800);
    let mut out = Vec::with_capacity(cap);
    let interval_days = rule.interval * 7;

    if rule.byday.is_empty() {
        // Plain weekly recurrence on the same weekday as the start.
        let mut current = start;
        for _ in 0..RRULE_MAX_STEPS {
            if out.len() >= cap {
                break;
            }
            out.push(current);
            // Calendar-day arithmetic (not raw seconds) so the wall-clock
            // time stays put across DST transitions.
            current = add_days_in_zone(current, interval_days, tz)
                .unwrap_or(current + interval_days * 86400);
        }
        return out;
    }

    let wkst = rule.wkst.unwrap_or(chrono::Weekday::Mon);
    // RFC 5545 § 3.8.5.3 says DTSTART is always part of the recurrence set;
    // a strict reading therefore requires the WEEKLY+BYDAY shape to emit
    // DTSTART even when its weekday is not in BYDAY (e.g. a Tuesday DTSTART
    // with BYDAY=MO,WE). dateutil drops DTSTART in that case and most
    // operational calendars (Apple Calendar, Google Calendar, Outlook)
    // match dateutil. The existing implementation matches that behavior
    // by filtering candidates against `start` below; preserved deliberately
    // here so the calendar matches what users see in the leading
    // implementations.
    // WEEKLY ignores BYDAY ordinals (RFC 5545 § 3.3.10) so we only
    // consider the bare weekday. Sort by week-start anchored offset so
    // each week emits in chronological order rather than Mon-first.
    let mut days: Vec<chrono::Weekday> = rule.byday.iter().map(|b| b.day).collect();
    days.sort_by_key(|d| {
        let wd = d.num_days_from_monday() as i64;
        let from = wkst.num_days_from_monday() as i64;
        (wd - from).rem_euclid(7)
    });

    let week_start = start_of_week(start, wkst, tz);
    let mut week_anchor = week_start;
    // Step-bounded: each "step" is one anchored week. Same DoS guard
    // rationale as `expand_daily`.
    for _ in 0..RRULE_MAX_STEPS {
        if out.len() >= cap {
            break;
        }
        for &wd in &days {
            let candidate = shift_to_weekday(week_anchor, wd, wkst, start, tz);
            if candidate < start {
                continue;
            }
            out.push(candidate);
            if out.len() >= cap {
                break;
            }
        }
        week_anchor = add_days_in_zone(week_anchor, interval_days, tz)
            .unwrap_or(week_anchor + interval_days * 86400);
    }
    out
}

fn expand_monthly(start: i64, rule: &Rrule, tz: RecurrenceTz) -> Vec<i64> {
    // Bumped from 120 to 800: dense BY-rules
    // (`FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR` ~ 22/month) would otherwise
    // truncate to ~5.5 months inside the 2-year fallback window. (Round 3
    // #4.) UNTIL-bounded rules run to RRULE_MAX_COUNT.
    let cap = instance_cap(rule, 800);
    let mut out = Vec::with_capacity(cap);
    let Some(start_dt) = tz.naive(start) else {
        return out;
    };
    let original_day = start_dt.day();

    // Year+month cursors advance by `interval` calendar months per step. The
    // previous shape stepped via `advance_months(current, interval)`, which
    // walks forward to find a month containing `original_day` - correct
    // for default-day MONTHLY (Jan 31 -> Mar 31, never Feb 28) but wrong
    // when explicit BYMONTHDAY/BYDAY is set: e.g. `BYMONTHDAY=1,-1`
    // starting Jan 31 wants Feb 1 / Feb 28 / Apr 1 / Apr 30, but
    // `advance_months` skipped Feb and April entirely because they don't
    // contain day 31. With a cursor we visit every interval-th month and
    // the per-month `collect_monthly_days` / default-day check decides
    // what (if anything) to emit there.
    let mut year = start_dt.year();
    let mut month = start_dt.month();
    // Step-bounded: filters that no visited month satisfies (e.g.
    // BYMONTHDAY=31 with INTERVAL=12 starting in February) would otherwise
    // never grow `out` and would loop forever.
    for _ in 0..RRULE_MAX_STEPS {
        if out.len() >= cap {
            return out;
        }

        let mut day_candidates = if rule.byday.is_empty() && rule.bymonthday.is_empty() {
            // Default: same day-of-month as start, only if it exists in
            // this month. RFC 5545 § 3.3.10: Jan 31 monthly emits Jan 31,
            // Mar 31, May 31 ... and skips short months entirely rather
            // than clamping to day 28.
            if days_in_month(year, month) >= original_day {
                vec![original_day]
            } else {
                Vec::new()
            }
        } else {
            collect_monthly_days(year, month, &rule.byday, &rule.bymonthday)
        };
        day_candidates.sort_unstable();
        day_candidates.dedup();

        for day in day_candidates {
            if let Some(ts) = with_year_month_day(start, year, month, day, tz)
                && ts >= start
            {
                out.push(ts);
                if out.len() >= cap {
                    return out;
                }
            }
        }

        // Advance month cursor by `interval` calendar months.
        let total = i64::from(month) - 1 + rule.interval;
        let new_month = u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1);
        let year_step = i32::try_from(total.div_euclid(12)).unwrap_or(0);
        year = match year.checked_add(year_step) {
            Some(y) => y,
            None => break,
        };
        month = new_month;
    }
    out
}

/// Resolve a month's candidate day-of-month values from BYDAY + BYMONTHDAY.
///
/// - BYDAY without an ordinal: every occurrence of that weekday in the month.
/// - BYDAY with an ordinal: only the n-th occurrence (positive: from start;
///   negative: from end). Returns no days if the n-th doesn't exist.
/// - BYMONTHDAY: explicit days (negative counts from end of month).
/// - Both set: intersection (RFC 5545 § 3.3.10).
fn collect_monthly_days(
    year: i32,
    month: u32,
    byday: &[ByDay],
    bymonthday: &[i32],
) -> Vec<u32> {
    let dim = days_in_month(year, month);

    let byday_days: Vec<u32> = byday
        .iter()
        .flat_map(|b| match b.ordinal {
            None => weekday_occurrences_in_month(year, month, b.day),
            Some(n) => nth_weekday_in_month(year, month, b.day, n)
                .into_iter()
                .collect(),
        })
        .collect();

    #[allow(clippy::cast_possible_wrap)]
    let dim_i = dim as i32;
    let bymonthday_days: Vec<u32> = bymonthday
        .iter()
        .filter_map(|d| {
            let resolved = if *d < 0 { dim_i + d + 1 } else { *d };
            if resolved < 1 || resolved > dim_i {
                None
            } else {
                #[allow(clippy::cast_sign_loss)]
                Some(resolved as u32)
            }
        })
        .collect();

    match (byday.is_empty(), bymonthday.is_empty()) {
        (true, true) => Vec::new(),
        (false, true) => byday_days,
        (true, false) => bymonthday_days,
        // Intersection: the day must satisfy both filters.
        (false, false) => byday_days
            .into_iter()
            .filter(|d| bymonthday_days.contains(d))
            .collect(),
    }
}

/// All days-of-month within `year`/`month` that fall on `weekday`.
fn weekday_occurrences_in_month(year: i32, month: u32, weekday: chrono::Weekday) -> Vec<u32> {
    let dim = days_in_month(year, month);
    (1..=dim)
        .filter(|&d| {
            chrono::NaiveDate::from_ymd_opt(year, month, d)
                .map(|date| date.weekday())
                == Some(weekday)
        })
        .collect()
}

/// The n-th occurrence of `weekday` in `year`/`month`. Positive `n` counts
/// from the start of the month; negative counts from the end.
fn nth_weekday_in_month(
    year: i32,
    month: u32,
    weekday: chrono::Weekday,
    n: i32,
) -> Option<u32> {
    let occurrences = weekday_occurrences_in_month(year, month, weekday);
    if n > 0 {
        let idx = usize::try_from(n - 1).ok()?;
        occurrences.get(idx).copied()
    } else if n < 0 {
        let from_end = usize::try_from(-n - 1).ok()?;
        occurrences.iter().rev().nth(from_end).copied()
    } else {
        None
    }
}

fn expand_yearly(start: i64, rule: &Rrule, tz: RecurrenceTz) -> Vec<i64> {
    // YEARLY's unbounded default sits lower than the others because the
    // 2-year fallback window only emits ~2 instances per realistic rule
    // (annual events). 200 covers ~80 years for repeated holidays
    // (`BYMONTH=12;BYMONTHDAY=25;COUNT=...`) without ever hitting in
    // practice. UNTIL-bounded rules - the case
    // `FREQ=YEARLY;UNTIL=22000101T000000Z` from 2026 (which previously
    // emitted 60 of 174 instances) - run to RRULE_MAX_COUNT and let UNTIL
    // do the work. (Round 3 #2.)
    let cap = instance_cap(rule, 200);
    let mut out = Vec::with_capacity(cap);
    let Some(start_dt) = tz.naive(start) else {
        return out;
    };
    let original_month = start_dt.month();
    let original_day = start_dt.day();

    // Year cursor advances by `interval` years per step. Previous shape stepped
    // via `advance_months(current, interval * 12)`, which walks forward to
    // find a month that contains the original day-of-month - correct for
    // MONTHLY (Jan 31 -> Mar 31) but wrong for YEARLY (Feb 29 -> March 29
    // of the next non-leap year). With a year cursor and the per-iteration
    // `days_in_month` check below, Feb 29 yearly correctly skips non-leap
    // years and emits only on real leap years.
    // RRULE INTERVAL is bounded above by what callers can plausibly emit; an
    // i32 is more than enough for any real recurrence and `try_from` keeps a
    // wedged INTERVAL=2_000_000_000 from silently casting to a negative
    // step. On overflow we step by 1 year and let the COUNT/UNTIL/RRULE_MAX
    // bounds terminate.
    let interval_years: i32 = i32::try_from(rule.interval).unwrap_or(1).max(1);
    let mut year = start_dt.year();
    // Sparse YEARLY rules (e.g. `BYMONTH=2;BYMONTHDAY=29`, the leap-day-
    // only case) emit one instance every four calendar years walked. The
    // shared 12_000-step bound bottoms out at 3_000 emissions for those,
    // so a `COUNT=10000` request silently truncates before reaching the
    // cap. Use a YEARLY-specific upper bound large enough that even the
    // sparsest realistic rule (every 8 years for an 8-year-cycle holiday)
    // can still hit RRULE_MAX_COUNT before this fires. Each step here is
    // O(1) calendar arithmetic so the step bound is cheap to raise.
    const YEARLY_MAX_STEPS: usize = 80_000;
    for _ in 0..YEARLY_MAX_STEPS {
        if out.len() >= cap {
            return out;
        }

        // Months to visit this year: explicit BYMONTH set, or the start's
        // own month as the default.
        let months: Vec<u32> = if rule.bymonth.is_empty() {
            vec![original_month]
        } else {
            rule.bymonth.clone()
        };

        for month in &months {
            let day_candidates = if rule.byday.is_empty() && rule.bymonthday.is_empty() {
                // Default: same day-of-month as start, skipped if the target
                // month doesn't have that day (Feb 29 in non-leap years).
                if days_in_month(year, *month) >= original_day {
                    vec![original_day]
                } else {
                    Vec::new()
                }
            } else {
                let mut days =
                    collect_monthly_days(year, *month, &rule.byday, &rule.bymonthday);
                days.sort_unstable();
                days.dedup();
                days
            };

            for day in day_candidates {
                if let Some(ts) = with_year_month_day(start, year, *month, day, tz)
                    && ts >= start
                {
                    out.push(ts);
                    if out.len() >= cap {
                        return out;
                    }
                }
            }
        }
        year = match year.checked_add(interval_years) {
            Some(y) => y,
            None => break,
        };
    }
    out
}

/// Resolve a wall-clock instant on a specific calendar date, preserving the
/// time-of-day of the original timestamp in the event's recurrence zone.
fn with_year_month_day(
    timestamp: i64,
    year: i32,
    month: u32,
    day: u32,
    tz: RecurrenceTz,
) -> Option<i64> {
    let naive = tz.naive(timestamp)?;
    let new_date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
    let new_naive = new_date.and_time(naive.time());
    tz.resolve(new_naive)
}

fn matches_weekday(timestamp: i64, days: &[chrono::Weekday], tz: RecurrenceTz) -> bool {
    let Some(naive) = tz.naive(timestamp) else {
        return false;
    };
    let wd = naive.date().weekday();
    days.contains(&wd)
}

/// Advance `timestamp` by `days` calendar days in the event's recurrence
/// zone, preserving wall-clock time across DST transitions. Returns `None`
/// only if the resulting NaiveDateTime or zone resolution overflows
/// (essentially unreachable for any plausible recurrence window).
fn add_days_in_zone(timestamp: i64, days: i64, tz: RecurrenceTz) -> Option<i64> {
    let naive = tz.naive(timestamp)?;
    let new_naive = naive.checked_add_signed(chrono::Duration::days(days))?;
    tz.resolve(new_naive)
}

fn start_of_week(timestamp: i64, week_start: chrono::Weekday, tz: RecurrenceTz) -> i64 {
    let Some(naive) = tz.naive(timestamp) else {
        return timestamp;
    };
    let current = naive.date().weekday();
    // Modular distance from `week_start` to `current`, walking forward
    // through the week (so a Sun-anchored week with current=Sat -> 6 days
    // back, and a Mon-anchored week with current=Sun -> 6 days back).
    let from = week_start.num_days_from_monday() as i64;
    let to = current.num_days_from_monday() as i64;
    let days_back = (to - from).rem_euclid(7);
    add_days_in_zone(timestamp, -days_back, tz).unwrap_or_else(|| {
        // `add_days_in_zone` only returns None when the resulting
        // NaiveDateTime or zone resolution overflows - in practice that
        // requires walking back across a 24-hour-skipped day (Pacific/Apia
        // Dec 30 2011). Falling back to the un-walked timestamp lets the
        // weekly expander continue to emit instances anchored on the
        // original day-of-week rather than emitting nothing; the
        // alternative (returning the un-walked timestamp silently) was the
        // previous behavior. Logged so the operator can attribute "weekly
        // instances are off by some days" to a zone-skip event.
        log::debug!(
            "start_of_week: add_days_in_zone(-{days_back}) failed (likely walking through a 24h-skipped day); falling back to un-shifted anchor"
        );
        timestamp
    })
}

fn shift_to_weekday(
    week_anchor: i64,
    target: chrono::Weekday,
    week_start: chrono::Weekday,
    time_source: i64,
    tz: RecurrenceTz,
) -> i64 {
    // Modular offset from `week_start` to `target`, so that within a
    // Sunday-anchored week the offset for Saturday is 6 (not -1) and for
    // Monday is 1 (not 0).
    let to = target.num_days_from_monday() as i64;
    let from = week_start.num_days_from_monday() as i64;
    let target_offset = (to - from).rem_euclid(7);
    let Some(anchor_naive) = tz.naive(week_anchor) else {
        return week_anchor;
    };
    let Some(time_naive) = tz.naive(time_source) else {
        return week_anchor;
    };
    // Day arithmetic in calendar units, not raw seconds. Then reattach the
    // intended wall-clock time and re-resolve in the event's zone, falling
    // through gap/ambiguous via `resolve_local_to_timestamp`.
    let Some(target_date) = anchor_naive
        .date()
        .checked_add_signed(chrono::Duration::days(target_offset))
    else {
        return week_anchor;
    };
    let new_naive = target_date.and_time(time_naive.time());
    tz.resolve(new_naive).unwrap_or(week_anchor)
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

/// Parse an UNTIL value (RFC 5545 § 3.3.10).
///
/// Three valid forms per spec:
/// - `YYYYMMDD` (DATE only) - "everything up to end of that local day." We
///   anchor end-of-day in `chrono::Local` because DATE-only UNTIL implies
///   floating DTSTART (which RFC 5545 § 3.3.5 says is interpreted in the
///   user's calendar zone). Anchoring to UTC midnight 23:59:59 clips
///   evening occurrences in west-of-UTC zones and includes next-day
///   occurrences in east-of-UTC zones.
/// - `YYYYMMDDTHHMMSSZ` (DATE-TIME, UTC) - the wall-clock instant in UTC.
///   We preserve the exact time, not collapse to 23:59:59.
/// - `YYYYMMDDTHHMMSS` (DATE-TIME, floating) - per RFC 5545 only valid
///   when DTSTART is floating. Anchored in `chrono::Local` for the same
///   reason as DATE-only.
///
/// Anything else (offset like `+0100`, sub-minute precision, trailing
/// garbage) is rejected with `None` rather than silently mis-anchored.
fn parse_until_date(val: &str) -> Option<i64> {
    let date_part = val.get(..8)?;
    let year: i32 = date_part.get(0..4)?.parse().ok()?;
    let month: u32 = date_part.get(4..6)?.parse().ok()?;
    let day: u32 = date_part.get(6..8)?.parse().ok()?;
    // Reject obviously bogus calendar years. RFC 5545 § 3.3.5 doesn't fix a
    // range, but iCalendar in practice carries only Gregorian dates and
    // year 0 / negatives produce a deeply negative UTC instant - the rule
    // then emits zero instances, which is bounded but a confusing way for
    // a malformed UNTIL to manifest. 9999 is chrono's outer year for
    // representable timestamps; values past that round-trip into chrono::
    // MAX and silently land elsewhere.
    if !(1..=9999).contains(&year) {
        log::debug!("RRULE UNTIL year {year} outside 1..=9999; rejecting");
        return None;
    }
    let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;

    // DATE-only form: exactly 8 chars.
    if val.len() == 8 {
        let dt = date.and_hms_opt(23, 59, 59)?;
        return crate::db::time::resolve_local_to_timestamp(dt, &chrono::Local);
    }

    // DATE-TIME form must be exactly 15 (floating) or 16 (UTC) chars and
    // have a `T` at index 8.
    if val.as_bytes().get(8) != Some(&b'T') {
        log::debug!("RRULE UNTIL has unrecognized form: {val}");
        return None;
    }
    let time_part = val.get(9..15)?;
    let hour: u32 = time_part.get(0..2)?.parse().ok()?;
    let minute: u32 = time_part.get(2..4)?.parse().ok()?;
    let second: u32 = time_part.get(4..6)?.parse().ok()?;
    let dt = date.and_hms_opt(hour, minute, second)?;

    match (val.len(), val.as_bytes().get(15)) {
        // Floating: 15 chars, no trailing character.
        (15, None) => crate::db::time::resolve_local_to_timestamp(dt, &chrono::Local),
        // UTC: 16 chars, trailing 'Z'.
        (16, Some(&b'Z')) => Some(dt.and_utc().timestamp()),
        // Anything else (offset like +0100, fractional seconds, trailing
        // garbage) is malformed; rejecting prevents silent UTC mis-anchor.
        _ => {
            log::debug!("RRULE UNTIL has unsupported trailing characters: {val}");
            None
        }
    }
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
            uid: None,
            recurrence_id_canonical: None,
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
    fn daily_with_unsatisfiable_byday_terminates() {
        // Reviewer A #1: Monday DTSTART with FREQ=DAILY;INTERVAL=7;BYDAY=TU
        // can never match - the candidate weekday is always Monday. Without
        // the step bound this spun forever. Confirm we return empty (or at
        // least terminate) instead of looping.
        let monday = local_ts(2026, 3, 9, 9, 0); // 2026-03-09 is a Monday
        let event = make_event(monday, 3600);
        let instances = expand_recurrence(
            &event,
            "FREQ=DAILY;INTERVAL=7;BYDAY=TU;COUNT=1",
        );
        // Implementation returns the original event when expansion produces
        // zero matches (`instances.is_empty()` fallback). Either zero or one
        // is acceptable here - what matters is that we returned at all.
        assert!(instances.len() <= 1);
    }

    #[test]
    fn monthly_with_unsatisfiable_bymonthday_terminates() {
        // Reviewer A #2: February DTSTART with FREQ=MONTHLY;INTERVAL=12;
        // BYMONTHDAY=31 - no visited month is February-with-day-31.
        let feb = local_ts(2026, 2, 1, 9, 0);
        let event = make_event(feb, 3600);
        let instances = expand_recurrence(
            &event,
            "FREQ=MONTHLY;INTERVAL=12;BYMONTHDAY=31;COUNT=1",
        );
        assert!(instances.len() <= 1);
    }

    #[test]
    fn count_clamped_to_max() {
        // Untrusted COUNT must not pin allocation. RRULE_MAX_COUNT (10_000)
        // is the cap; an upstream `COUNT=999999` should still expand only
        // up to that many entries.
        let start = local_ts(2026, 1, 1, 9, 0);
        let event = make_event(start, 1800);
        let instances = expand_recurrence(&event, "FREQ=DAILY;COUNT=999999");
        assert!(instances.len() <= RRULE_MAX_COUNT);
    }

    #[test]
    fn monthly_jan_31_skips_short_months_not_clamps() {
        // RFC 5545 § 3.3.10: a Jan 31 monthly recurrence emits Jan 31, then
        // Mar 31, May 31, ... - never Feb 28, Mar 28, .... Previously we
        // clamped to the last valid day and never recovered, so every
        // subsequent instance landed on the 28th.
        let start = local_ts(2026, 1, 31, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=MONTHLY;COUNT=4");
        let days: Vec<u32> = instances
            .iter()
            .map(|e| {
                chrono::Local
                    .timestamp_opt(e.start_time, 0)
                    .single()
                    .expect("local")
                    .naive_local()
                    .date()
                    .day()
            })
            .collect();
        assert_eq!(days, vec![31, 31, 31, 31]);
    }

    #[test]
    fn monthly_byday_first_monday_emits_first_monday() {
        // FREQ=MONTHLY;BYDAY=1MO -> the first Monday of each month.
        // Starting in March 2026 (March 9, 2026 is a Monday and the second
        // Monday; the first Monday of March is March 2).
        let start = local_ts(2026, 3, 9, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=MONTHLY;BYDAY=1MO;COUNT=4");
        let dates: Vec<(i32, u32, u32)> = instances
            .iter()
            .map(|e| {
                let dt = chrono::Local
                    .timestamp_opt(e.start_time, 0)
                    .single()
                    .expect("local")
                    .naive_local();
                (dt.year(), dt.month(), dt.day())
            })
            .collect();
        // Apr 6, May 4, Jun 1, Jul 6 - Mar is omitted because the first
        // Monday (Mar 2) is before DTSTART (Mar 9). The four results all
        // sit on a Monday.
        assert_eq!(instances.len(), 4);
        for (_, _, day) in &dates {
            assert!(*day <= 7, "day {day} should be in the first week of the month");
        }
        for inst in &instances {
            assert_eq!(weekday_of(inst.start_time), chrono::Weekday::Mon);
        }
    }

    #[test]
    fn monthly_byday_last_friday_emits_last_friday() {
        // FREQ=MONTHLY;BYDAY=-1FR -> last Friday of each month.
        // Start: 2026-03-27 (a Friday, the last of March 2026).
        let start = local_ts(2026, 3, 27, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=MONTHLY;BYDAY=-1FR;COUNT=4");
        assert_eq!(instances.len(), 4);
        // Confirm they're all on Friday and within the last 7 days of the
        // month (>= dim - 6).
        for inst in &instances {
            let dt = chrono::Local
                .timestamp_opt(inst.start_time, 0)
                .single()
                .expect("local")
                .naive_local();
            let dim = days_in_month(dt.year(), dt.month());
            assert_eq!(dt.weekday(), chrono::Weekday::Fri);
            assert!(
                dt.day() >= dim - 6,
                "day {} not in last week of {}/{}",
                dt.day(),
                dt.year(),
                dt.month()
            );
        }
    }

    #[test]
    fn monthly_bymonthday_first_and_last_visits_short_months() {
        // FREQ=MONTHLY;BYMONTHDAY=1,-1 means "first and last day of every
        // month." Starting on Jan 31, the previous shape stepped via
        // `advance_months` which walked forward looking for a month
        // containing day 31 - so Feb (28 days) and April (30 days) were
        // skipped entirely, missing the user's intended Feb 1 / Feb 28 /
        // Apr 1 / Apr 30 emissions.
        let start = local_ts(2026, 1, 31, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(
            &event,
            "FREQ=MONTHLY;BYMONTHDAY=1,-1;COUNT=5",
        );
        assert_eq!(instances.len(), 5);
        // Expected: Jan 31, Feb 1, Feb 28, Mar 1, Mar 31.
        let dates: Vec<(u32, u32)> = instances
            .iter()
            .map(|e| {
                let dt = chrono::Local
                    .timestamp_opt(e.start_time, 0)
                    .single()
                    .expect("local")
                    .naive_local();
                (dt.month(), dt.day())
            })
            .collect();
        assert_eq!(
            dates,
            vec![(1, 31), (2, 1), (2, 28), (3, 1), (3, 31)]
        );
    }

    #[test]
    fn yearly_ordinal_byday_without_bymonth_falls_back_to_master() {
        // FREQ=YEARLY;BYDAY=20MO means "the 20th Monday of the year" per
        // RFC 5545 § 3.3.10. The expander only handles per-month ordinal
        // BYDAY today (no year-scope walker), so without BYMONTH set this
        // would silently emit zero instances. The fallback emits the
        // master so the operator at least sees the event, with a WARN.
        let start = local_ts(2026, 1, 1, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=YEARLY;BYDAY=20MO;COUNT=3");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].start_time, start);
    }

    #[test]
    fn yearly_feb_29_skips_non_leap_years() {
        // FREQ=YEARLY on a Feb 29 DTSTART previously stepped via
        // `advance_months(current, 12)`, which walked forward to a month
        // containing day 29 - landing on March 29 of the next non-leap year
        // instead of correctly waiting until the next leap year. Both
        // dateutil and RFC 5545 (clamping non-existent dates within a
        // FREQ=YEARLY default) say to skip non-leap years entirely.
        let start = local_ts(2024, 2, 29, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=YEARLY;COUNT=3");
        assert_eq!(instances.len(), 3);
        // Each instance must be Feb 29 in a leap year. Convert each instance
        // back to local date and verify month/day; the expected sequence is
        // 2024, 2028, 2032 (every 4th year while the leap rule applies).
        let mut expected_years = [2024, 2028, 2032].iter();
        for inst in &instances {
            let dt = chrono::Local
                .timestamp_opt(inst.start_time, 0)
                .single()
                .expect("local")
                .naive_local();
            assert_eq!(dt.month(), 2);
            assert_eq!(dt.day(), 29);
            assert_eq!(dt.year(), *expected_years.next().expect("3 leap years"));
        }
    }

    #[test]
    fn yearly_byday_first_monday_of_march() {
        // FREQ=YEARLY;BYMONTH=3;BYDAY=1MO -> first Monday of March each year.
        let start = local_ts(2026, 3, 2, 9, 0); // 2026-03-02 is the first Monday of March
        let event = make_event(start, 3600);
        let instances =
            expand_recurrence(&event, "FREQ=YEARLY;BYMONTH=3;BYDAY=1MO;COUNT=3");
        assert_eq!(instances.len(), 3);
        for inst in &instances {
            let dt = chrono::Local
                .timestamp_opt(inst.start_time, 0)
                .single()
                .expect("local")
                .naive_local();
            assert_eq!(dt.month(), 3);
            assert_eq!(dt.weekday(), chrono::Weekday::Mon);
            assert!(dt.day() <= 7);
        }
    }

    #[test]
    fn unknown_freq_returns_single_instance() {
        let start = local_ts(2026, 1, 1, 9, 0);
        let event = make_event(start, 1800);
        let instances = expand_recurrence(&event, "FREQ=BOGUS");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].start_time, start);
    }

    #[test]
    fn until_with_time_preserves_time_portion() {
        // UNTIL=20260315T120000Z means "stop at 12:00 UTC on 2026-03-15".
        // The previous parser collapsed this to 23:59:59 UTC, which kept
        // afternoon instances that should have been excluded.
        let start = local_ts(2026, 3, 15, 9, 0);
        let event = make_event(start, 3600);
        let until = chrono::NaiveDate::from_ymd_opt(2026, 3, 15)
            .and_then(|d| d.and_hms_opt(12, 0, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        let instances =
            expand_recurrence(&event, "FREQ=DAILY;UNTIL=20260315T120000Z");
        assert!(!instances.is_empty());
        for inst in &instances {
            assert!(
                inst.start_time <= until,
                "instance {} > UNTIL {until}",
                inst.start_time
            );
        }
    }

    #[test]
    fn empty_expansion_returns_empty_not_original() {
        // UNTIL is in the past relative to the start: zero instances should
        // be emitted, not a single fallback copy of the original event.
        let start = local_ts(2030, 1, 1, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=DAILY;UNTIL=20290101T000000Z");
        assert!(instances.is_empty());
    }

    #[test]
    fn rrule_with_bysetpos_falls_back_to_master_instance() {
        // FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1 means "last weekday
        // of each month". We don't implement BYSETPOS, so the previous
        // expander would emit ~22 days/month. The fix: detect BYSETPOS and
        // emit only the master instance (still visible on the calendar)
        // rather than 20+ wrong daily entries.
        let start = local_ts(2026, 1, 30, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(
            &event,
            "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR;BYSETPOS=-1;COUNT=12",
        );
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].start_time, start);
    }

    #[test]
    fn rrule_with_byweekno_falls_back_to_master_instance() {
        // BYWEEKNO is also unsupported; same fallback as BYSETPOS.
        let start = local_ts(2026, 1, 5, 9, 0);
        let event = make_event(start, 3600);
        let instances = expand_recurrence(&event, "FREQ=YEARLY;BYWEEKNO=20;COUNT=3");
        assert_eq!(instances.len(), 1);
        assert_eq!(instances[0].start_time, start);
    }

    #[test]
    fn parse_until_date_strict_z_form() {
        // 16-char with Z is valid UTC.
        let utc = parse_until_date("20260315T120000Z").expect("valid UTC UNTIL");
        let expected = chrono::NaiveDate::from_ymd_opt(2026, 3, 15)
            .and_then(|d| d.and_hms_opt(12, 0, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        assert_eq!(utc, expected);
    }

    #[test]
    fn parse_until_date_15_char_floating_resolves_in_local() {
        // 15-char no-Z form is floating; anchored in chrono::Local. Only
        // assert that parsing succeeds and is distinct from the UTC-anchored
        // value (when local != UTC). The exact timestamp depends on the
        // host's TZ, so we don't pin a specific value.
        let parsed = parse_until_date("20260315T120000").expect("floating UNTIL");
        let utc_equiv = chrono::NaiveDate::from_ymd_opt(2026, 3, 15)
            .and_then(|d| d.and_hms_opt(12, 0, 0))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        // In any non-UTC zone parsed != utc_equiv. In UTC they would match;
        // we just confirm parsed is finite and within a sensible window.
        let one_day = 86_400;
        assert!((parsed - utc_equiv).abs() <= 14 * 3600 + one_day);
    }

    #[test]
    fn parse_until_date_rejects_garbage_after_time() {
        // Sub-minute precision (".5"), embedded offsets ("+0100"), or any
        // trailing characters that aren't "Z" should reject rather than
        // silently mis-parse.
        assert!(parse_until_date("20260315T120000.5").is_none());
        assert!(parse_until_date("20260315T120000+0100").is_none());
        assert!(parse_until_date("20260315T120000X").is_none());
    }

    #[test]
    fn parse_until_date_date_only_anchors_in_local() {
        // 8-char DATE-only form anchors at 23:59:59 in chrono::Local rather
        // than UTC midnight - prevents clipping of evening occurrences for
        // west-of-UTC users and over-inclusion for east-of-UTC users.
        let parsed = parse_until_date("20260315").expect("date-only UNTIL");
        let utc_eod = chrono::NaiveDate::from_ymd_opt(2026, 3, 15)
            .and_then(|d| d.and_hms_opt(23, 59, 59))
            .map(|d| d.and_utc().timestamp())
            .expect("valid");
        let one_day = 86_400;
        assert!((parsed - utc_eod).abs() <= 14 * 3600 + one_day);
    }

    #[test]
    fn monthly_with_event_timezone_anchors_in_event_zone() {
        // Repro from the review findings: a monthly event with
        // TZID=Pacific/Kiritimati at 09:00 on the 1st of the month must
        // emit the 1st of every month *in Pacific/Kiritimati* regardless
        // of the host's local zone. The pre-fix expander resolved the
        // master timestamp through chrono::Local: on a UTC- or west-of-UTC
        // host the wall-clock date silently shifted to Dec 31 (Kiritimati
        // is UTC+14), original_day became 31, and the rule emitted only
        // months containing day 31.
        use chrono_tz::Tz;
        let kiritimati: Tz = "Pacific/Kiritimati".parse().expect("valid IANA");
        let dt = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
            .and_then(|d| d.and_hms_opt(9, 0, 0))
            .expect("valid");
        let start = kiritimati
            .from_local_datetime(&dt)
            .single()
            .expect("unambiguous")
            .timestamp();
        let mut event = make_event(start, 3600);
        event.timezone = Some("Pacific/Kiritimati".to_string());
        let instances = expand_recurrence(&event, "FREQ=MONTHLY;COUNT=12");
        assert_eq!(instances.len(), 12);
        for (i, inst) in instances.iter().enumerate() {
            let local = kiritimati
                .timestamp_opt(inst.start_time, 0)
                .single()
                .expect("kiritimati instant resolves");
            assert_eq!(
                local.naive_local().date().day(),
                1,
                "instance {i} not on the 1st of its month in Pacific/Kiritimati"
            );
            assert_eq!(local.naive_local().time().hour(), 9);
        }
    }

    #[test]
    fn daily_with_event_timezone_preserves_wall_clock_across_dst() {
        // Daily event at 09:00 America/New_York spanning the spring-forward
        // transition (2026-03-08 02:00 EST -> 03:00 EDT). Each instance
        // must remain at 09:00 in NY local time, so the UTC offset between
        // consecutive days varies by exactly one hour across the boundary.
        // Pre-fix expansion went through chrono::Local on a non-NY host -
        // the daylight-saving boundary the user actually experiences
        // depends on the host, not the event, so the 09:00-in-NY invariant
        // was silently violated for any user not in the eastern US.
        use chrono_tz::Tz;
        let ny: Tz = "America/New_York".parse().expect("valid IANA");
        let dt = chrono::NaiveDate::from_ymd_opt(2026, 3, 6)
            .and_then(|d| d.and_hms_opt(9, 0, 0))
            .expect("valid");
        let start = ny
            .from_local_datetime(&dt)
            .single()
            .expect("unambiguous")
            .timestamp();
        let mut event = make_event(start, 3600);
        event.timezone = Some("America/New_York".to_string());
        // Cover a window that includes the 2026-03-08 transition.
        let instances = expand_recurrence(&event, "FREQ=DAILY;COUNT=7");
        assert_eq!(instances.len(), 7);
        for inst in &instances {
            let local = ny
                .timestamp_opt(inst.start_time, 0)
                .single()
                .expect("NY instant resolves");
            assert_eq!(local.naive_local().time().hour(), 9);
            assert_eq!(local.naive_local().time().minute(), 0);
        }
    }

    #[test]
    fn recurring_all_day_via_parse_path_keeps_one_day_across_dst() {
        // Round 3 #22 regression guard. The CalDAV/Graph parse layer now
        // anchors all-day DTEND to `start + days*86400` rather than
        // resolving DTEND in chrono::Local. For a 1-day all-day event
        // whose master spans the spring-forward boundary
        // (2026-03-08 in America/New_York), the master's end_time lands
        // at 01:00 NY the next day - 25 hours after start, not 24.
        // Without the all-day branch in expand_recurrence the wall_duration
        // would be 25 hours and every subsequent recurring instance would
        // emit at "ends 01:00 the next day," shifting the displayed
        // end-time by an hour for every week after the transition.
        use chrono_tz::Tz;
        let ny: Tz = "America/New_York".parse().expect("valid IANA");
        let mar8 = ny
            .from_local_datetime(
                &chrono::NaiveDate::from_ymd_opt(2026, 3, 8)
                    .expect("valid")
                    .and_hms_opt(0, 0, 0)
                    .expect("midnight"),
            )
            .single()
            .expect("unambiguous")
            .timestamp();
        // Parse-path output: end = start + 86400 (the new anchor shape).
        let raw_end = mar8 + 86_400;
        let mut event = make_event(mar8, raw_end - mar8);
        event.all_day = true;
        event.timezone = Some("America/New_York".to_string());
        event.end_time = raw_end;
        let instances = expand_recurrence(&event, "FREQ=WEEKLY;COUNT=3");
        assert_eq!(instances.len(), 3);
        for (i, inst) in instances.iter().enumerate() {
            let end_local = ny
                .timestamp_opt(inst.end_time, 0)
                .single()
                .expect("NY end resolves");
            // Instance i=0 is the master itself; its end may be 01:00 NY
            // the next day because the parse-path anchor sits there. What
            // matters is that subsequent instances (which expand from a
            // 1-day wall_duration) land at midnight rather than 01:00.
            if i == 0 {
                continue;
            }
            assert_eq!(
                end_local.naive_local().time().hour(),
                0,
                "post-DST instance {i} end_time was not midnight in NY"
            );
            assert_eq!(end_local.naive_local().time().minute(), 0);
        }
    }

    #[test]
    fn weekly_all_day_in_event_timezone_keeps_24h_duration() {
        // Recurring all-day event whose master spans the spring-forward
        // transition. The wall-clock duration in the event's zone is 24h
        // (midnight to midnight), but the raw-seconds delta is 23h. The
        // pre-fix expander cached the raw delta and propagated 23h to
        // every subsequent instance, so the displayed end-time drifted to
        // 23:00 the day before. Threading event.timezone through the walk
        // and computing wall-clock duration fixes both at once.
        use chrono_tz::Tz;
        let ny: Tz = "America/New_York".parse().expect("valid IANA");
        let day = chrono::NaiveDate::from_ymd_opt(2026, 3, 8).expect("valid");
        let next = chrono::NaiveDate::from_ymd_opt(2026, 3, 9).expect("valid");
        let start = ny
            .from_local_datetime(&day.and_hms_opt(0, 0, 0).expect("midnight"))
            .single()
            .expect("unambiguous")
            .timestamp();
        let end = ny
            .from_local_datetime(&next.and_hms_opt(0, 0, 0).expect("midnight"))
            .single()
            .expect("unambiguous")
            .timestamp();
        let mut event = make_event(start, end - start);
        event.timezone = Some("America/New_York".to_string());
        // Override the duration directly to capture the 23h master span,
        // then verify subsequent instances still resolve to midnight.
        event.end_time = end;
        let instances = expand_recurrence(&event, "FREQ=WEEKLY;COUNT=3");
        assert_eq!(instances.len(), 3);
        for (i, inst) in instances.iter().enumerate() {
            let end_local = ny
                .timestamp_opt(inst.end_time, 0)
                .single()
                .expect("NY end resolves");
            assert_eq!(
                end_local.naive_local().time().hour(),
                0,
                "instance {i} end_time hour was not midnight in NY"
            );
            assert_eq!(end_local.naive_local().time().minute(), 0);
        }
    }

    #[test]
    fn yearly_until_distant_emits_full_range_not_60_cap() {
        // Round 3 #2 regression guard: previously expand_yearly defaulted
        // to a 60-instance cap when COUNT was absent. A
        // `FREQ=YEARLY;UNTIL=...` rule reaching far into the future would
        // emit only 60 instances and silently stop. With UNTIL set, the
        // cap rises to RRULE_MAX_COUNT and the time bound terminates.
        let start = local_ts(2026, 6, 1, 9, 0);
        let event = make_event(start, 3600);
        // 100 years of yearly emissions is well past the old 60-cap and
        // well under RRULE_MAX_COUNT.
        let instances = expand_recurrence(
            &event,
            "FREQ=YEARLY;UNTIL=21260601T000000Z",
        );
        assert!(
            instances.len() >= 100,
            "expected >= 100 yearly instances; got {} (likely truncated by old cap)",
            instances.len()
        );
    }

    #[test]
    fn weekly_byday_dense_unbounded_passes_old_366_cap() {
        // Round 3 #4 regression guard: WEEKLY+BYDAY emitting 5 days/week
        // over the synthesised 2-year fallback window is ~520 instances.
        // The previous expand_weekly default of 366 silently truncated -
        // the standup vanished 17 months in. Cap is now 800.
        let start = local_ts(2026, 1, 5, 9, 0); // Monday
        let event = make_event(start, 3600);
        let instances =
            expand_recurrence(&event, "FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR");
        assert!(
            instances.len() > 366,
            "weekly weekday rule capped at {} (old 366 cap regressed?)",
            instances.len()
        );
    }

    #[test]
    fn monthly_byday_dense_unbounded_passes_old_120_cap() {
        // Round 3 #4: MONTHLY+BYDAY=MO,TU,WE,TH,FR emits ~22 instances
        // per month - the previous 120 cap truncated at ~5.5 months.
        let start = local_ts(2026, 1, 1, 9, 0);
        let event = make_event(start, 3600);
        let instances =
            expand_recurrence(&event, "FREQ=MONTHLY;BYDAY=MO,TU,WE,TH,FR");
        assert!(
            instances.len() > 120,
            "monthly weekday rule capped at {} (old 120 cap regressed?)",
            instances.len()
        );
    }

    #[test]
    fn override_slot_is_subtracted_from_master_expansion() {
        // Regression guard for review #1: the master series and an override
        // row coexist on `(account_id, uid)` in the database. Without
        // subtracting the override slot from the master expansion the
        // calendar shows BOTH the original Mar 11 09:00 instance AND the
        // moved override - two events for one slot.
        //
        // Use a NY-zoned event so the canonical form on the override side
        // (`YYYYMMDDTHHMMSS;TZID=America/New_York`) lines up with what
        // `canonical_recurrence_slot` emits during expansion.
        use chrono_tz::Tz;
        let ny: Tz = "America/New_York".parse().expect("valid IANA");
        let dt = chrono::NaiveDate::from_ymd_opt(2026, 3, 9)
            .and_then(|d| d.and_hms_opt(9, 0, 0))
            .expect("valid");
        let start = ny
            .from_local_datetime(&dt)
            .single()
            .expect("unambiguous")
            .timestamp();
        let mut event = make_event(start, 3600);
        event.timezone = Some("America/New_York".to_string());
        let mut overrides = HashSet::new();
        // Override pins the Wed 2026-03-11 09:00 NY slot - matching the
        // canonical form the master expansion will produce for that day.
        overrides.insert("20260311T090000;TZID=America/New_York".to_string());
        let instances = expand_recurrence_with_overrides(
            &event,
            "FREQ=DAILY;COUNT=5",
            &overrides,
        );
        // 5 candidates (Mon-Fri), 1 phantom subtracted -> 4 emitted.
        assert_eq!(instances.len(), 4);
        // None of the kept instances may sit at the Wed 09:00 slot.
        for inst in &instances {
            let local = ny
                .timestamp_opt(inst.start_time, 0)
                .single()
                .expect("NY instant resolves");
            assert!(
                local.naive_local().date()
                    != chrono::NaiveDate::from_ymd_opt(2026, 3, 11).expect("valid"),
                "phantom override slot was not subtracted"
            );
        }
    }

    #[test]
    fn override_dedup_skipped_when_uid_missing() {
        // Defensive: when the master row's `uid` is None (legacy data, or a
        // provider that doesn't surface UID), there's nothing to key the
        // override set on. Expansion proceeds as if no overrides existed.
        let start = local_ts(2026, 3, 9, 9, 0);
        let event = make_event(start, 3600);
        // event.uid is already None from make_event.
        let mut overrides = HashSet::new();
        overrides.insert("20260311T090000".to_string());
        // Explicit empty set - this is what the load-path passes when uid
        // is missing - so the dedup path doesn't engage.
        let instances = expand_recurrence_with_overrides(
            &event,
            "FREQ=DAILY;COUNT=5",
            &HashSet::new(),
        );
        assert_eq!(instances.len(), 5, "dedup must not engage without uid");
        let _ = overrides;
    }

    #[test]
    fn wkst_sunday_anchors_week_to_sunday() {
        // 2026-03-08 is a Sunday. With WKST=SU and BYDAY=SU,WE, a recurrence
        // starting on the prior Wednesday should emit the Wednesday first
        // (within the first week) and the following Sunday next - chronological
        // order anchored to the Sunday-week.
        let wed = local_ts(2026, 3, 4, 9, 0); // 2026-03-04 is a Wednesday
        let event = make_event(wed, 3600);
        let instances = expand_recurrence(&event, "FREQ=WEEKLY;BYDAY=SU,WE;WKST=SU;COUNT=4");
        assert_eq!(instances.len(), 4);
        let weekdays: Vec<_> = instances
            .iter()
            .map(|e| weekday_of(e.start_time))
            .collect();
        // Sunday-anchored week: Wed -> Sun -> Wed -> Sun
        assert_eq!(
            weekdays,
            vec![
                chrono::Weekday::Wed,
                chrono::Weekday::Sun,
                chrono::Weekday::Wed,
                chrono::Weekday::Sun,
            ]
        );
    }
}
