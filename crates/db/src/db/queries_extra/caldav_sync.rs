//! CalDAV sync persistence: event map, attendee/reminder sync, ctag/etag management.

use std::collections::HashMap;

use rusqlite::{Connection, params};

/// Delete events and their map entries for a calendar by remote_event_id.
pub fn delete_caldav_events_sync(
    conn: &Connection,
    calendar_id: &str,
    uris: &[String],
) -> Result<(), String> {
    for uri in uris {
        conn.execute(
            "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
            params![calendar_id, uri],
        )
        .map_err(|e| format!("delete caldav event: {e}"))?;

        conn.execute(
            "DELETE FROM caldav_event_map WHERE calendar_id = ?1 AND uri = ?2",
            params![calendar_id, uri],
        )
        .map_err(|e| format!("delete caldav event map: {e}"))?;
    }
    Ok(())
}

/// Upsert a URI→ETag mapping in the caldav_event_map.
pub fn upsert_caldav_event_map_sync(
    conn: &Connection,
    uri: &str,
    calendar_id: &str,
    event_uid: &str,
    etag: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR REPLACE INTO caldav_event_map \
         (uri, calendar_id, event_uid, etag) \
         VALUES (?1, ?2, ?3, ?4)",
        params![uri, calendar_id, event_uid, etag],
    )
    .map_err(|e| format!("upsert caldav event map: {e}"))?;
    Ok(())
}

/// A CalDAV attendee for persistence.
#[derive(Debug, Clone)]
pub struct CalDavAttendee {
    pub email: String,
    pub name: Option<String>,
    pub partstat: Option<String>,
    pub is_organizer: bool,
}

/// Sync attendees for a calendar event.
/// Looks up the event by google_event_id, replaces attendees, optionally adds organizer.
pub fn sync_caldav_attendees_sync(
    conn: &Connection,
    account_id: &str,
    google_event_id: &str,
    attendees: &[CalDavAttendee],
    organizer_email: Option<&str>,
    organizer_name: Option<&str>,
) -> Result<(), String> {
    let event_id: Option<String> = conn
        .query_row(
            "SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2",
            params![account_id, google_event_id],
            |row| row.get("id"),
        )
        .ok();

    let Some(event_id) = event_id else {
        return Ok(());
    };

    conn.execute(
        "DELETE FROM calendar_attendees WHERE account_id = ?1 AND event_id = ?2",
        params![account_id, event_id],
    )
    .map_err(|e| format!("delete attendees: {e}"))?;

    for att in attendees {
        let rsvp = att.partstat.as_deref().map(str::to_lowercase);
        conn.execute(
            "INSERT INTO calendar_attendees (event_id, account_id, email, name, rsvp_status, is_organizer) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![event_id, account_id, att.email, att.name, rsvp, att.is_organizer as i64],
        )
        .map_err(|e| format!("insert attendee: {e}"))?;
    }

    if let Some(org_email) = organizer_email {
        let org_lower = org_email.to_lowercase();
        let already_present = attendees.iter().any(|a| a.email.to_lowercase() == org_lower);
        if !already_present {
            conn.execute(
                "INSERT INTO calendar_attendees (event_id, account_id, email, name, rsvp_status, is_organizer) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                params![event_id, account_id, org_email, organizer_name, "accepted"],
            )
            .map_err(|e| format!("insert organizer attendee: {e}"))?;
        }
    }

    Ok(())
}

/// A CalDAV reminder for persistence.
#[derive(Debug, Clone)]
pub struct CalDavReminder {
    pub minutes_before: i64,
    pub method: Option<String>,
}

/// Sync reminders for a calendar event.
pub fn sync_caldav_reminders_sync(
    conn: &Connection,
    account_id: &str,
    google_event_id: &str,
    reminders: &[CalDavReminder],
) -> Result<(), String> {
    let event_id: Option<String> = conn
        .query_row(
            "SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2",
            params![account_id, google_event_id],
            |row| row.get("id"),
        )
        .ok();

    let Some(event_id) = event_id else {
        return Ok(());
    };

    conn.execute(
        "DELETE FROM calendar_reminders WHERE account_id = ?1 AND event_id = ?2",
        params![account_id, event_id],
    )
    .map_err(|e| format!("delete reminders: {e}"))?;

    for rem in reminders {
        conn.execute(
            "INSERT INTO calendar_reminders (event_id, account_id, minutes_before, method) \
             VALUES (?1, ?2, ?3, ?4)",
            params![event_id, account_id, rem.minutes_before, rem.method],
        )
        .map_err(|e| format!("insert reminder: {e}"))?;
    }

    Ok(())
}

/// Load the stored ctag for a calendar.
pub fn load_calendar_ctag_sync(
    conn: &Connection,
    calendar_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT ctag FROM calendars WHERE id = ?1",
        params![calendar_id],
        |row| row.get::<_, Option<String>>("ctag"),
    )
    .map_err(|e| format!("load calendar ctag: {e}"))
}

/// Load stored ETags for events in a calendar.
pub fn load_caldav_etags_sync(
    conn: &Connection,
    calendar_id: &str,
) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT uri, etag FROM caldav_event_map WHERE calendar_id = ?1")
        .map_err(|e| format!("prepare etag query: {e}"))?;

    let rows = stmt
        .query_map(params![calendar_id], |row| {
            Ok((
                row.get::<_, String>("uri")?,
                row.get::<_, Option<String>>("etag")?,
            ))
        })
        .map_err(|e| format!("query etags: {e}"))?;

    let mut map = HashMap::new();
    for row in rows {
        let (uri, etag) = row.map_err(|e| format!("read etag row: {e}"))?;
        if let Some(etag) = etag {
            map.insert(uri, etag);
        }
    }

    Ok(map)
}

/// Clear the event map for a calendar (used during full resync).
pub fn clear_caldav_event_map_sync(
    conn: &Connection,
    calendar_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM caldav_event_map WHERE calendar_id = ?1",
        params![calendar_id],
    )
    .map_err(|e| format!("clear caldav event map: {e}"))?;
    Ok(())
}
