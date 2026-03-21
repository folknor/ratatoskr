use super::super::DbState;
use super::super::types::{DbCalendar, DbCalendarAttendee, DbCalendarEvent, DbCalendarReminder};
use crate::db::from_row::FromRow;
use rusqlite::params;

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
            .prepare(
                "SELECT * FROM calendars WHERE account_id = ?1 \
                     ORDER BY sort_order ASC, is_primary DESC, display_name ASC",
            )
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
            .prepare(
                "SELECT * FROM calendars WHERE account_id = ?1 AND is_visible = 1 \
                     ORDER BY sort_order ASC, is_primary DESC, display_name ASC",
            )
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
            "SELECT * FROM calendars WHERE id = ?1",
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

#[allow(clippy::too_many_arguments)]
pub async fn db_upsert_calendar_event(
    db: &DbState,
    account_id: String,
    google_event_id: String,
    summary: Option<String>,
    description: Option<String>,
    location: Option<String>,
    start_time: i64,
    end_time: i64,
    is_all_day: bool,
    status: String,
    organizer_email: Option<String>,
    attendees_json: Option<String>,
    html_link: Option<String>,
    calendar_id: Option<String>,
    remote_event_id: Option<String>,
    etag: Option<String>,
    ical_data: Option<String>,
    uid: Option<String>,
) -> Result<(), String> {
    log::info!("Upserting calendar event: account_id={account_id}, google_event_id={google_event_id}");
    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO calendar_events (id, account_id, google_event_id, summary, description, location, start_time, end_time, is_all_day, status, organizer_email, attendees_json, html_link, calendar_id, remote_event_id, etag, ical_data, uid)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
                 ON CONFLICT(account_id, google_event_id) DO UPDATE SET
                   summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
                   is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
                   html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
                   ical_data = ?17, uid = ?18, updated_at = unixepoch()",
            params![id, account_id, google_event_id, summary, description, location, start_time, end_time, is_all_day as i64, status, organizer_email, attendees_json, html_link, calendar_id, remote_event_id, etag, ical_data, uid],
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
            .prepare(
                "SELECT * FROM calendar_events \
                     WHERE account_id = ?1 AND start_time < ?3 AND end_time > ?2 \
                     ORDER BY start_time ASC",
            )
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
            "SELECT * FROM calendar_events \
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
            "SELECT * FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
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
        conn.execute(
            "DELETE FROM calendar_events WHERE id = ?1",
            params![event_id],
        )
        .map_err(|e| {
            log::error!("Failed to delete calendar event {}: {e}", event_id);
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
            .prepare(
                "SELECT * FROM calendar_attendees \
                 WHERE account_id = ?1 AND event_id = ?2 \
                 ORDER BY is_organizer DESC, email ASC",
            )
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
            .prepare(
                "SELECT * FROM calendar_reminders \
                 WHERE account_id = ?1 AND event_id = ?2 \
                 ORDER BY minutes_before ASC",
            )
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

// ── All-account calendar queries (for unified calendar) ────

pub async fn db_get_all_visible_calendars(
    db: &DbState,
) -> Result<Vec<DbCalendar>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM calendars WHERE is_visible = 1 \
                 ORDER BY account_id, is_primary DESC, sort_order, display_name ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map([], DbCalendar::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}
