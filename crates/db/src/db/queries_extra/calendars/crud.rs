use super::super::super::{ReadConn, ReadDbState};
use super::super::super::types::{DbCalendar, DbCalendarAttendee, DbCalendarEvent, DbCalendarReminder};
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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

pub async fn db_upsert_calendar_event(
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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

pub async fn db_delete_calendar_event(db: &ReadDbState, event_id: String) -> Result<(), String> {
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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

/// Load attendees for a given event (synchronous).
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

/// Load reminders for a given event (synchronous).
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

/// Load all calendars for the sidebar list (synchronous).
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
         VALUES (?1, ?2, ?1, ?3, ?4, ?5, ?6, ?7, ?8, 'confirmed', ?9,
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

// ── All-account calendar queries (for unified calendar) ────

pub async fn db_get_all_visible_calendars(db: &ReadDbState) -> Result<Vec<DbCalendar>, String> {
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
