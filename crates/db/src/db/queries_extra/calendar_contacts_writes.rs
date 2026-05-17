//! Calendar / contact-group / message-reaction / seen-address writes that
//! previously lived inline in calendar, graph, and gmail crates. Agent-owned
//! scaffold for Phase 1.6 - functions get added here as call sites in
//! `crates/calendar/src/sync.rs`, `crates/calendar/src/actions.rs`,
//! `crates/calendar/src/caldav/mod.rs`, `crates/graph/src/group_sync.rs`,
//! `crates/graph/src/sync/persistence.rs`, and
//! `crates/gmail/src/contacts/other_contacts.rs` are routed through `db` APIs.
//!
//! Writer-side functions take `&WriteConn` or `&WriteTxn` so Service code
//! cannot accidentally route calendar mutations through a read handle.

use crate::db::{WriteConn, WriteTxn};
use rusqlite::{OptionalExtension, params};
pub use super::calendars::{
    CalendarAttendeeWriteRow, CalendarReminderWriteRow, LocalCalendarEventParams,
};

// ---------------------------------------------------------------------------
// `calendars` table helpers
// ---------------------------------------------------------------------------

/// Look up a calendar's local UUID from (account_id, remote_id).
///
/// Returns `Ok(None)` when no matching calendar exists yet.
pub fn get_calendar_id_by_remote_id(
    conn: &WriteConn<'_>,
    account_id: &str,
    remote_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
        params![account_id, remote_id],
        |row| row.get::<_, String>("id"),
    )
    .optional()
    .map_err(|e| format!("get_calendar_id_by_remote_id: {e}"))
}

/// Input row for `upsert_discovered_calendar`. Bundled to keep the function
/// signature under the `too_many_arguments` lint cap (7).
pub struct DiscoveredCalendar<'a> {
    pub account_id: &'a str,
    pub provider: &'a str,
    pub remote_id: &'a str,
    pub display_name: Option<&'a str>,
    pub color: Option<&'a str>,
    pub is_primary: bool,
    pub can_edit: bool,
}

/// Upsert a single discovered calendar row and return the stable local UUID.
///
/// Generates a new UUID on first insert; on conflict updates metadata only
/// (display_name, color, is_primary, can_edit) and returns the existing id.
pub fn upsert_discovered_calendar(
    conn: &WriteTxn<'_>,
    cal: &DiscoveredCalendar<'_>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.query_row(
        "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary, can_edit) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
         ON CONFLICT(account_id, remote_id) DO UPDATE SET \
           display_name = ?5, color = ?6, is_primary = ?7, can_edit = ?8, updated_at = unixepoch() \
         RETURNING id",
        params![
            id,
            cal.account_id,
            cal.provider,
            cal.remote_id,
            cal.display_name,
            cal.color,
            cal.is_primary as i64,
            cal.can_edit as i64,
        ],
        |row| row.get::<_, String>(0),
    )
    .map_err(|e| format!("upsert_discovered_calendar: {e}"))
}

/// Update the sync_token and/or ctag for a calendar row.
///
/// Uses `COALESCE` so a `None` value leaves the existing column unchanged.
pub fn update_calendar_sync_token(
    conn: &WriteTxn<'_>,
    calendar_id: &str,
    new_sync_token: Option<&str>,
    new_ctag: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE calendars \
         SET sync_token = COALESCE(?1, sync_token), \
             ctag = COALESCE(?2, ctag), \
             updated_at = unixepoch() \
         WHERE id = ?3",
        params![new_sync_token, new_ctag, calendar_id],
    )
    .map_err(|e| format!("update_calendar_sync_token: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `calendar_events` table helpers
// ---------------------------------------------------------------------------

/// Input row for upserting a single calendar event from a provider DTO.
///
/// Mirrors the fields of `CalendarEventDto` from the `calendar` crate so
/// callers can convert without pulling that type into `db`.
#[derive(Debug, Clone, Default)]
pub struct CalendarEventRow {
    pub account_id: String,
    /// Provider-assigned event id (used as the unique key on conflict).
    pub google_event_id: String,
    /// Provider resource id. For CalDAV this is the resource URI, while
    /// `google_event_id` may include a recurrence-id discriminator.
    pub remote_event_id: String,
    pub calendar_id: String,
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
    pub recurrence_id: Option<String>,
}

/// Upsert a calendar event row inside any connection (connection or transaction).
///
/// The conflict key is `(account_id, google_event_id)` where
/// `google_event_id` is stored as `remote_event_id` in the DTO.
/// Generates a new UUID for the `id` column on first insert.
pub fn upsert_calendar_event_row(
    conn: &WriteTxn<'_>,
    row: &CalendarEventRow,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    conn.query_row(
        "INSERT INTO calendar_events \
         (id, account_id, google_event_id, summary, description, location, \
          start_time, end_time, is_all_day, status, organizer_email, attendees_json, \
          html_link, calendar_id, remote_event_id, etag, ical_data, uid, title, \
          timezone, recurrence_rule, organizer_name, rsvp_status, created_at, \
          availability, visibility, recurrence_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, \
                 ?17, ?18, ?19, ?20, ?21, ?22, ?23, unixepoch(), ?24, ?25, ?26) \
         ON CONFLICT(account_id, google_event_id) DO UPDATE SET \
           summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8, \
           is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12, \
           html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16, \
           ical_data = ?17, uid = ?18, title = ?19, timezone = ?20, recurrence_rule = ?21, \
           organizer_name = ?22, rsvp_status = ?23, availability = ?24, visibility = ?25, \
           recurrence_id = ?26, updated_at = unixepoch() \
         RETURNING id",
        params![
            id,
            row.account_id,
            row.google_event_id,
            row.summary,
            row.description,
            row.location,
            row.start_time,
            row.end_time,
            row.is_all_day as i64,
            row.status,
            row.organizer_email,
            row.attendees_json,
            row.html_link,
            row.calendar_id,
            row.remote_event_id,
            row.etag,
            row.ical_data,
            row.uid,
            row.title,
            row.timezone,
            row.recurrence_rule,
            row.organizer_name,
            row.rsvp_status,
            row.availability,
            row.visibility,
            row.recurrence_id,
        ],
        |row| row.get::<_, String>(0),
    )
    .map_err(|e| format!("upsert_calendar_event_row: {e}"))
}

pub fn replace_event_attendees(
    conn: &WriteTxn<'_>,
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

pub fn replace_event_reminders(
    conn: &WriteTxn<'_>,
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

pub fn delete_event_by_account_remote_id(
    conn: &WriteTxn<'_>,
    account_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM calendar_attendees WHERE event_id IN \
         (SELECT id FROM calendar_events WHERE account_id = ?1 AND remote_event_id = ?2)",
        params![account_id, remote_event_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM calendar_reminders WHERE event_id IN \
         (SELECT id FROM calendar_events WHERE account_id = ?1 AND remote_event_id = ?2)",
        params![account_id, remote_event_id],
    )
    .map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM calendar_events WHERE account_id = ?1 AND remote_event_id = ?2",
        params![account_id, remote_event_id],
    )
    .map_err(|e| e.to_string())?;
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

/// A CalDAV reminder for persistence.
#[derive(Debug, Clone)]
pub struct CalDavReminder {
    pub minutes_before: i64,
    pub method: Option<String>,
}

/// Delete events and their map entries for a calendar by remote_event_id.
pub fn delete_caldav_events(
    conn: &WriteTxn<'_>,
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

/// Upsert a URI to ETag mapping in the caldav_event_map.
pub fn upsert_caldav_event_map(
    conn: &WriteTxn<'_>,
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

/// Sync attendees for a calendar event.
pub fn sync_caldav_attendees(
    conn: &WriteTxn<'_>,
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
        .optional()
        .map_err(|e| format!("load caldav attendee event id: {e}"))?;

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

/// Sync reminders for a calendar event.
pub fn sync_caldav_reminders(
    conn: &WriteTxn<'_>,
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
        .optional()
        .map_err(|e| format!("load caldav reminder event id: {e}"))?;

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

/// Drop calendar_event rows for a CalDAV resource whose storage key is not
/// in the seen set, plus their attendees and reminders.
pub fn reap_orphan_overrides(
    conn: &WriteTxn<'_>,
    calendar_id: &str,
    uri: &str,
    seen_keys: &[String],
) -> Result<(), String> {
    if seen_keys.is_empty() {
        return Ok(());
    }

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
    for key in seen_keys {
        bind.push(Box::new(key.clone()));
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

/// Delete a calendar event row identified by (calendar_id, remote_event_id).
pub fn delete_calendar_event_by_remote_id(
    conn: &WriteTxn<'_>,
    calendar_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
        params![calendar_id, remote_event_id],
    )
    .map_err(|e| format!("delete_calendar_event_by_remote_id: {e}"))?;
    Ok(())
}

/// Set the provider remote id and `etag` after a successful provider create.
/// `google_event_id` is the historical cross-provider conflict key, so it is
/// moved from the local provisional id to the provider id at the same time.
pub fn set_calendar_event_remote_id_and_etag(
    conn: &WriteConn<'_>,
    event_id: &str,
    remote_event_id: &str,
    etag: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE calendar_events
         SET google_event_id = ?1, remote_event_id = ?1, etag = ?2
         WHERE id = ?3",
        params![remote_event_id, etag, event_id],
    )
    .map_err(|e| format!("set_calendar_event_remote_id_and_etag: {e}"))?;
    Ok(())
}

/// Update only the `etag` on an existing calendar event (post-provider-update).
pub fn set_calendar_event_etag(
    conn: &WriteConn<'_>,
    event_id: &str,
    etag: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE calendar_events SET etag = ?1 WHERE id = ?2",
        params![etag, event_id],
    )
    .map_err(|e| format!("set_calendar_event_etag: {e}"))?;
    Ok(())
}

/// Update editable local event fields and the provider `etag` in one write.
pub fn update_calendar_event_fields_and_etag(
    conn: &WriteConn<'_>,
    event_id: &str,
    p: &LocalCalendarEventParams,
    etag: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE calendar_events SET
            summary = ?2, description = ?3, location = ?4,
            start_time = ?5, end_time = ?6, is_all_day = ?7,
            calendar_id = ?8, timezone = ?9, recurrence_rule = ?10,
            availability = ?11, visibility = ?12, etag = ?13,
            updated_at = unixepoch()
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
            etag,
        ],
    )
    .map_err(|e| format!("update_calendar_event_fields_and_etag: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `accounts` CalDAV URL helpers
// ---------------------------------------------------------------------------

/// Persist freshly discovered CalDAV principal / home URLs to the `accounts`
/// table. Uses COALESCE so a `None` value leaves the column unchanged.
pub fn set_account_caldav_discovered_urls(
    conn: &WriteConn<'_>,
    account_id: &str,
    principal_url: Option<&str>,
    home_url: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts \
         SET caldav_principal_url = COALESCE(?2, caldav_principal_url), \
             caldav_home_url = COALESCE(?3, caldav_home_url) \
         WHERE id = ?1",
        params![account_id, principal_url, home_url],
    )
    .map_err(|e| format!("set_account_caldav_discovered_urls: {e}"))?;
    Ok(())
}

/// Clear the persisted CalDAV principal / home URLs for an account, forcing
/// full RFC 6764 discovery on the next sync.
pub fn clear_account_caldav_urls(
    conn: &WriteConn<'_>,
    account_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts \
         SET caldav_principal_url = NULL, \
             caldav_home_url = NULL \
         WHERE id = ?1",
        params![account_id],
    )
    .map_err(|e| format!("clear_account_caldav_urls: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `contact_groups` and `contact_group_members` helpers
// ---------------------------------------------------------------------------

/// Input row for upserting a contact group from an Exchange group sync.
#[derive(Debug, Clone)]
pub struct ContactGroupRow {
    /// Stable local ID (e.g. `"exchange-{account_id}-{server_id}"`).
    pub id: String,
    pub name: String,
    pub source: String,
    pub account_id: String,
    pub server_id: String,
    pub email: Option<String>,
    pub group_type: String,
}

/// Upsert a contact group row (INSERT OR UPDATE on conflict by id).
pub fn upsert_contact_group(
    conn: &WriteConn<'_>,
    row: &ContactGroupRow,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO contact_groups (id, name, source, account_id, server_id, email, group_type) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) \
         ON CONFLICT(id) DO UPDATE SET \
           name = excluded.name, \
           email = excluded.email, \
           group_type = excluded.group_type, \
           updated_at = unixepoch()",
        params![
            row.id,
            row.name,
            row.source,
            row.account_id,
            row.server_id,
            row.email,
            row.group_type,
        ],
    )
    .map_err(|e| format!("upsert_contact_group: {e}"))?;
    Ok(())
}

/// Delete all member rows for a contact group (replace pattern: delete then
/// re-insert).
pub fn delete_contact_group_members(
    conn: &WriteTxn<'_>,
    group_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM contact_group_members WHERE group_id = ?1",
        params![group_id],
    )
    .map_err(|e| format!("delete_contact_group_members: {e}"))?;
    Ok(())
}

/// Insert a single email-type member into a contact group (INSERT OR IGNORE).
pub fn insert_contact_group_member_email(
    conn: &WriteTxn<'_>,
    group_id: &str,
    email: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT OR IGNORE INTO contact_group_members (group_id, member_type, member_value) \
         VALUES (?1, 'email', ?2)",
        params![group_id, email],
    )
    .map_err(|e| format!("insert_contact_group_member_email: {e}"))?;
    Ok(())
}

/// Delete a single contact group by its local id (members cascade via FK).
pub fn delete_contact_group_by_id(
    conn: &WriteTxn<'_>,
    group_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM contact_groups WHERE id = ?1",
        params![group_id],
    )
    .map_err(|e| format!("delete_contact_group_by_id: {e}"))?;
    Ok(())
}

/// Delete all contact groups for an account with a given source label.
pub fn delete_contact_groups_for_account_by_source(
    conn: &WriteConn<'_>,
    account_id: &str,
    source: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM contact_groups WHERE account_id = ?1 AND source = ?2",
        params![account_id, source],
    )
    .map_err(|e| format!("delete_contact_groups_for_account_by_source: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `message_reactions` helpers
// ---------------------------------------------------------------------------

/// Upsert a message reaction row. On conflict (same message, account,
/// reactor_email, reaction_type), updates `reacted_at`.
pub fn upsert_message_reaction(
    conn: &WriteTxn<'_>,
    message_id: &str,
    account_id: &str,
    reactor_email: &str,
    reaction_type: &str,
    reacted_at: Option<i64>,
    source: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO message_reactions \
         (message_id, account_id, reactor_email, reactor_name, reaction_type, reacted_at, source) \
         VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6) \
         ON CONFLICT(message_id, account_id, reactor_email, reaction_type) DO UPDATE SET \
           reacted_at = ?5",
        params![message_id, account_id, reactor_email, reaction_type, reacted_at, source],
    )
    .map_err(|e| format!("upsert_message_reaction: {e}"))?;
    Ok(())
}

/// Upsert a message reaction where `reaction_type` should be updated on
/// conflict rather than `reacted_at`. Used for count/metadata rows where
/// the type encodes a numeric value.
pub fn upsert_message_reaction_update_type(
    conn: &WriteTxn<'_>,
    message_id: &str,
    account_id: &str,
    reactor_email: &str,
    reaction_type: &str,
    source: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO message_reactions \
         (message_id, account_id, reactor_email, reactor_name, reaction_type, reacted_at, source) \
         VALUES (?1, ?2, ?3, NULL, ?4, NULL, ?5) \
         ON CONFLICT(message_id, account_id, reactor_email, reaction_type) DO UPDATE SET \
           reaction_type = ?4",
        params![message_id, account_id, reactor_email, reaction_type, source],
    )
    .map_err(|e| format!("upsert_message_reaction_update_type: {e}"))?;
    Ok(())
}

/// Delete a reaction row for a specific (message, account, reactor_email, source).
///
/// Used when the owner removes their reaction and we need to clean up the row.
pub fn delete_message_reaction(
    conn: &WriteTxn<'_>,
    message_id: &str,
    account_id: &str,
    reactor_email: &str,
    source: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM message_reactions \
         WHERE message_id = ?1 AND account_id = ?2 \
           AND reactor_email = ?3 AND source = ?4",
        params![message_id, account_id, reactor_email, source],
    )
    .map_err(|e| format!("delete_message_reaction: {e}"))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// `seen_addresses` helpers
// ---------------------------------------------------------------------------

/// Upsert a seen address from a Google otherContacts sync.
///
/// On conflict with an existing row for the same (account_id, email):
/// - Updates `display_name` only when the existing source is also `google_other`
/// - Always bumps `last_seen_at` to the maximum of old and new
pub fn upsert_seen_address_google_other(
    conn: &WriteTxn<'_>,
    email: &str,
    account_id: &str,
    display_name: Option<&str>,
    now: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO seen_addresses \
         (email, account_id, display_name, display_name_source, source, \
          first_seen_at, last_seen_at) \
         VALUES (?1, ?2, ?3, 'google_other', 'google_other', ?4, ?4) \
         ON CONFLICT(account_id, email) DO UPDATE SET \
           display_name = CASE \
             WHEN seen_addresses.source = 'google_other' \
               THEN COALESCE(excluded.display_name, seen_addresses.display_name) \
             ELSE seen_addresses.display_name \
           END, \
           display_name_source = CASE \
             WHEN seen_addresses.source = 'google_other' THEN 'google_other' \
             ELSE seen_addresses.display_name_source \
           END, \
           last_seen_at = MAX(seen_addresses.last_seen_at, excluded.last_seen_at)",
        params![email, account_id, display_name, now],
    )
    .map_err(|e| format!("upsert_seen_address_google_other: {e}"))?;
    Ok(())
}

/// Delete a seen address row for (email, account_id) where source = 'google_other'.
///
/// Callers should check that no other mapping references the email before
/// calling (i.e. only call when the address is truly orphaned).
pub fn delete_seen_address_google_other(
    conn: &WriteTxn<'_>,
    email: &str,
    account_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM seen_addresses \
         WHERE email = ?1 AND account_id = ?2 AND source = 'google_other'",
        params![email, account_id],
    )
    .map_err(|e| format!("delete_seen_address_google_other: {e}"))?;
    Ok(())
}
