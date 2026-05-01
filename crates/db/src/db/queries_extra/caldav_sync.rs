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

/// Drop calendar_event rows for a CalDAV resource whose storage key isn't
/// in the seen set, plus their attendees and reminders.
///
/// Reaps abandoned RECURRENCE-ID overrides when a single iCal resource
/// shrinks - a series that arrived as `master + override-A + override-B`
/// and then comes back as `master + override-A` would otherwise leave
/// override-B's row behind indefinitely. The whole-resource deletion in
/// `delete_caldav_events_sync` only fires when the entire URI vanishes
/// from the server's listing, so it doesn't catch in-resource shrinkage.
///
/// Bounded by `seen_keys` plus the existing `(calendar_id, uri)` index, so
/// the cost stays proportional to the number of overrides that resource
/// actually carried (typically zero or single digits).
pub fn reap_orphan_overrides_sync(
    conn: &Connection,
    calendar_id: &str,
    uri: &str,
    seen_keys: &[String],
) -> Result<(), String> {
    if seen_keys.is_empty() {
        // Defensive: no seen keys means we'd delete every row for this
        // resource. The caller's iCal parse must have failed midway; in
        // that case the URI-deletion path is the right place to drop the
        // resource, not here.
        return Ok(());
    }

    // Build a list of seen keys with sql placeholders. All keys are
    // ASCII-only `caldav:...` strings so there's no escape concern, but
    // bind them as parameters anyway to keep this analogous to other
    // dynamic-IN queries in the crate.
    let placeholders = seen_keys
        .iter()
        .enumerate()
        .map(|(i, _)| format!("?{}", i + 3))
        .collect::<Vec<_>>()
        .join(", ");
    let select_sql = format!(
        "SELECT id FROM calendar_events \
         WHERE calendar_id = ?1 AND remote_event_id = ?2 \
           AND google_event_id NOT IN ({placeholders})"
    );

    let mut bind: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(seen_keys.len() + 2);
    bind.push(Box::new(calendar_id.to_string()));
    bind.push(Box::new(uri.to_string()));
    for k in seen_keys {
        bind.push(Box::new(k.clone()));
    }
    let bind_refs: Vec<&dyn rusqlite::types::ToSql> = bind.iter().map(AsRef::as_ref).collect();

    let mut stmt = conn
        .prepare(&select_sql)
        .map_err(|e| format!("prepare reap-overrides: {e}"))?;
    let orphan_ids: Vec<String> = stmt
        .query_map(bind_refs.as_slice(), |row| row.get::<_, String>(0))
        .map_err(|e| format!("query orphan overrides: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect orphan ids: {e}"))?;

    for id in &orphan_ids {
        // Cascade attendees and reminders before the row itself; matches
        // the manual cascade in `delete_event_by_remote_id_sync`.
        conn.execute(
            "DELETE FROM calendar_attendees WHERE event_id = ?1",
            params![id],
        )
        .map_err(|e| format!("reap orphan attendees: {e}"))?;
        conn.execute(
            "DELETE FROM calendar_reminders WHERE event_id = ?1",
            params![id],
        )
        .map_err(|e| format!("reap orphan reminders: {e}"))?;
        conn.execute("DELETE FROM calendar_events WHERE id = ?1", params![id])
            .map_err(|e| format!("reap orphan event: {e}"))?;
    }

    if !orphan_ids.is_empty() {
        log::debug!(
            "CalDAV: reaped {} orphan override row(s) for resource {uri}",
            orphan_ids.len()
        );
    }
    Ok(())
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
