use std::collections::{HashMap, HashSet};

use rusqlite::params;

use crate::db::DbState;
use crate::db::queries_extra::calendars::{
    db_delete_events_for_calendar, db_update_calendar_sync_token, db_upsert_calendar,
    db_upsert_calendar_event,
};

use super::client::CalDavClient;
use super::parse;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a CalDAV calendar sync.
#[derive(Debug)]
pub struct CalDavSyncResult {
    pub calendars_discovered: usize,
    pub events_upserted: usize,
    pub events_deleted: usize,
    pub calendars_skipped_unchanged: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Sync CalDAV calendars and events for an account.
///
/// 1. Discover calendars via PROPFIND on calendar-home-set
/// 2. For each calendar, compare ctag — skip if unchanged
/// 3. If changed, list events (ETags), diff against stored, fetch changed
/// 4. Upsert events to DB, prune deleted events
pub async fn sync_caldav_calendars(
    client: &CalDavClient,
    db: &DbState,
    account_id: &str,
) -> Result<CalDavSyncResult, String> {
    // Step 1: Discover calendars
    let discovered = client.list_calendars().await?;

    log::info!(
        "CalDAV: discovered {} calendars for {account_id}",
        discovered.len()
    );

    let mut total_upserted = 0;
    let mut total_deleted = 0;
    let mut skipped_unchanged = 0;

    for cal in &discovered {
        // Upsert the calendar record
        let calendar_id = db_upsert_calendar(
            db,
            account_id.to_string(),
            "caldav".to_string(),
            cal.href.clone(),
            cal.display_name.clone(),
            cal.color.clone(),
            false, // is_primary — CalDAV doesn't specify a "primary" calendar
        )
        .await?;

        // Check ctag for quick change detection
        let stored_ctag = load_calendar_ctag(db, &calendar_id).await?;

        if let Some(ref remote_ctag) = cal.ctag {
            if stored_ctag.as_ref() == Some(remote_ctag) {
                log::debug!(
                    "CalDAV: calendar {} ctag unchanged, skipping",
                    cal.display_name.as_deref().unwrap_or(&cal.href)
                );
                skipped_unchanged += 1;
                continue;
            }
        }

        // Sync events for this calendar
        let (upserted, deleted) =
            sync_calendar_events(client, db, account_id, &calendar_id, &cal.href).await?;

        total_upserted += upserted;
        total_deleted += deleted;

        // Update stored ctag
        db_update_calendar_sync_token(
            db,
            calendar_id.clone(),
            None,
            cal.ctag.clone(),
        )
        .await?;
    }

    log::info!(
        "CalDAV sync complete for {account_id}: {} calendars ({skipped_unchanged} unchanged), \
         {total_upserted} events upserted, {total_deleted} events deleted",
        discovered.len()
    );

    Ok(CalDavSyncResult {
        calendars_discovered: discovered.len(),
        events_upserted: total_upserted,
        events_deleted: total_deleted,
        calendars_skipped_unchanged: skipped_unchanged,
    })
}

/// Sync events for a single calendar.
///
/// Uses ETag-based diffing: list all events with ETags, compare to stored,
/// fetch only changed/new events, and remove deleted ones.
///
/// Returns `(upserted_count, deleted_count)`.
async fn sync_calendar_events(
    client: &CalDavClient,
    db: &DbState,
    account_id: &str,
    calendar_id: &str,
    calendar_href: &str,
) -> Result<(usize, usize), String> {
    // List all events on the server (URIs + ETags)
    let remote_entries = client.list_events(calendar_href).await?;

    // Load stored ETags for comparison
    let stored_etags = load_stored_etags(db, calendar_id).await?;

    // Determine which events need fetching (new or changed)
    let mut fetch_uris: Vec<String> = Vec::new();
    let remote_uri_set: HashSet<String> = remote_entries.iter().map(|e| e.uri.clone()).collect();

    for entry in &remote_entries {
        match stored_etags.get(&entry.uri) {
            Some(old_etag) if *old_etag == entry.etag => {
                // ETag unchanged, skip
            }
            _ => {
                fetch_uris.push(entry.uri.clone());
            }
        }
    }

    // Determine which events were deleted on the server
    let deleted_uris: Vec<String> = stored_etags
        .keys()
        .filter(|uri| !remote_uri_set.contains(*uri))
        .cloned()
        .collect();

    log::info!(
        "CalDAV sync for calendar {calendar_id}: {} to fetch, {} unchanged, {} deleted",
        fetch_uris.len(),
        remote_entries.len() - fetch_uris.len(),
        deleted_uris.len()
    );

    // Build ETag lookup from remote entries
    let etag_map: HashMap<&str, &str> = remote_entries
        .iter()
        .map(|e| (e.uri.as_str(), e.etag.as_str()))
        .collect();

    // Fetch changed/new iCalendar data
    let uri_refs: Vec<&str> = fetch_uris.iter().map(String::as_str).collect();
    let fetched_icals = client.fetch_events(calendar_href, &uri_refs).await?;

    // Parse and upsert events
    let mut upserted = 0;
    for (uri, ical_data) in &fetched_icals {
        let etag = etag_map.get(uri.as_str()).unwrap_or(&"").to_string();

        match parse::parse_icalendar(ical_data) {
            Ok(events) => {
                for event in &events {
                    upsert_parsed_event(
                        db,
                        account_id,
                        calendar_id,
                        uri,
                        &etag,
                        ical_data,
                        event,
                    )
                    .await?;
                    upserted += 1;
                }
            }
            Err(e) => {
                log::warn!("Failed to parse iCalendar at {uri}: {e}");
            }
        }
    }

    // Delete removed events
    let deleted_count = deleted_uris.len();
    if !deleted_uris.is_empty() {
        let cal_id = calendar_id.to_string();
        let deleted_owned = deleted_uris;
        db.with_conn(move |conn| {
            for uri in &deleted_owned {
                conn.execute(
                    "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
                    params![cal_id, uri],
                )
                .map_err(|e| format!("delete caldav event: {e}"))?;

                // Also remove from mapping table
                conn.execute(
                    "DELETE FROM caldav_event_map WHERE calendar_id = ?1 AND uri = ?2",
                    params![cal_id, uri],
                )
                .map_err(|e| format!("delete caldav event map: {e}"))?;
            }
            Ok(())
        })
        .await?;
    }

    Ok((upserted, deleted_count))
}

/// Upsert a single parsed event into the database.
async fn upsert_parsed_event(
    db: &DbState,
    account_id: &str,
    calendar_id: &str,
    uri: &str,
    etag: &str,
    ical_data: &str,
    event: &parse::ParsedVEvent,
) -> Result<(), String> {
    let uid = event.uid.clone().unwrap_or_else(|| uri.to_string());
    let google_event_id = make_google_event_id(&uid);

    // Serialize attendees as JSON
    let attendees_json = if event.attendees.is_empty() {
        None
    } else {
        let attendees: Vec<serde_json::Value> = event
            .attendees
            .iter()
            .map(|a| {
                serde_json::json!({
                    "email": a.email,
                    "displayName": a.name,
                    "responseStatus": a.partstat.as_deref()
                        .unwrap_or("needsAction").to_lowercase(),
                })
            })
            .collect();
        serde_json::to_string(&attendees).ok()
    };

    let start_time = event.start_time.unwrap_or(0);
    let end_time = event.end_time.unwrap_or(start_time);

    db_upsert_calendar_event(
        db,
        account_id.to_string(),
        google_event_id.clone(),
        event.summary.clone(),
        event.description.clone(),
        event.location.clone(),
        start_time,
        end_time,
        event.is_all_day,
        event.status.clone(),
        event.organizer_email.clone(),
        attendees_json,
        None, // html_link — CalDAV doesn't have one
        Some(calendar_id.to_string()),
        Some(uri.to_string()),
        Some(etag.to_string()),
        Some(ical_data.to_string()),
        event.uid.clone(),
    )
    .await?;

    // Track URI -> ETag mapping for incremental sync
    let cal_id = calendar_id.to_string();
    let uri_owned = uri.to_string();
    let etag_owned = etag.to_string();
    let uid_for_map = uid.clone();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO caldav_event_map \
             (uri, calendar_id, event_uid, etag) \
             VALUES (?1, ?2, ?3, ?4)",
            params![uri_owned, cal_id, uid_for_map, etag_owned],
        )
        .map_err(|e| format!("upsert caldav event map: {e}"))?;
        Ok(())
    })
    .await?;

    // Sync attendees and reminders
    sync_event_attendees(db, account_id, &google_event_id, event).await?;
    sync_event_reminders(db, account_id, &google_event_id, event).await?;

    Ok(())
}

/// Build the `google_event_id` key from a CalDAV UID.
fn make_google_event_id(uid: &str) -> String {
    format!("caldav:{uid}")
}

/// Sync attendees for an event.
async fn sync_event_attendees(
    db: &DbState,
    account_id: &str,
    google_event_id: &str,
    event: &parse::ParsedVEvent,
) -> Result<(), String> {
    if event.attendees.is_empty() {
        return Ok(());
    }

    let aid = account_id.to_string();
    let geid = google_event_id.to_string();
    let attendees = event.attendees.clone();
    let organizer_email = event.organizer_email.clone();
    let organizer_name = event.organizer_name.clone();

    db.with_conn(move |conn| {
        // Find the event id
        let event_id: Option<String> = conn
            .query_row(
                "SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2",
                params![aid, geid],
                |row| row.get("id"),
            )
            .ok();

        let Some(event_id) = event_id else {
            return Ok(());
        };

        // Delete old attendees
        conn.execute(
            "DELETE FROM calendar_attendees WHERE account_id = ?1 AND event_id = ?2",
            params![aid, event_id],
        )
        .map_err(|e| format!("delete attendees: {e}"))?;

        // Insert attendees
        for att in &attendees {
            let rsvp = att.partstat.as_deref().map(|s| s.to_lowercase());
            conn.execute(
                "INSERT INTO calendar_attendees (event_id, account_id, email, name, rsvp_status, is_organizer) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![event_id, aid, att.email, att.name, rsvp, att.is_organizer as i64],
            )
            .map_err(|e| format!("insert attendee: {e}"))?;
        }

        // Add organizer as attendee if not already in the list
        if let Some(ref org_email) = organizer_email {
            let org_lower = org_email.to_lowercase();
            let already_present = attendees.iter().any(|a| a.email.to_lowercase() == org_lower);
            if !already_present {
                conn.execute(
                    "INSERT INTO calendar_attendees (event_id, account_id, email, name, rsvp_status, is_organizer) \
                     VALUES (?1, ?2, ?3, ?4, ?5, 1)",
                    params![event_id, aid, org_email, organizer_name, "accepted"],
                )
                .map_err(|e| format!("insert organizer attendee: {e}"))?;
            }
        }

        Ok(())
    })
    .await
}

/// Sync reminders for an event.
async fn sync_event_reminders(
    db: &DbState,
    account_id: &str,
    google_event_id: &str,
    event: &parse::ParsedVEvent,
) -> Result<(), String> {
    if event.reminders.is_empty() {
        return Ok(());
    }

    let aid = account_id.to_string();
    let geid = google_event_id.to_string();
    let reminders = event.reminders.clone();

    db.with_conn(move |conn| {
        let event_id: Option<String> = conn
            .query_row(
                "SELECT id FROM calendar_events WHERE account_id = ?1 AND google_event_id = ?2",
                params![aid, geid],
                |row| row.get("id"),
            )
            .ok();

        let Some(event_id) = event_id else {
            return Ok(());
        };

        // Delete old reminders
        conn.execute(
            "DELETE FROM calendar_reminders WHERE account_id = ?1 AND event_id = ?2",
            params![aid, event_id],
        )
        .map_err(|e| format!("delete reminders: {e}"))?;

        // Insert reminders
        for rem in &reminders {
            conn.execute(
                "INSERT INTO calendar_reminders (event_id, account_id, minutes_before, method) \
                 VALUES (?1, ?2, ?3, ?4)",
                params![event_id, aid, rem.minutes_before, rem.method],
            )
            .map_err(|e| format!("insert reminder: {e}"))?;
        }

        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// CTag / ETag persistence
// ---------------------------------------------------------------------------

/// Load the stored ctag for a calendar from the calendars table.
async fn load_calendar_ctag(db: &DbState, calendar_id: &str) -> Result<Option<String>, String> {
    let cid = calendar_id.to_string();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT ctag FROM calendars WHERE id = ?1",
            params![cid],
            |row| row.get::<_, Option<String>>("ctag"),
        )
        .map_err(|e| format!("load calendar ctag: {e}"))
    })
    .await
}

/// Load stored ETags for events in a calendar from the mapping table.
async fn load_stored_etags(
    db: &DbState,
    calendar_id: &str,
) -> Result<HashMap<String, String>, String> {
    let cid = calendar_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT uri, etag FROM caldav_event_map WHERE calendar_id = ?1")
            .map_err(|e| format!("prepare etag query: {e}"))?;

        let rows = stmt
            .query_map(params![cid], |row| {
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
    })
    .await
}

/// Full resync: delete all events for a calendar and re-fetch everything.
///
/// Use this when incremental sync gets confused or for first-time sync.
pub async fn full_resync_calendar(
    client: &CalDavClient,
    db: &DbState,
    account_id: &str,
    calendar_id: &str,
    calendar_href: &str,
) -> Result<(usize, usize), String> {
    // Delete all existing events for this calendar
    db_delete_events_for_calendar(db, calendar_id.to_string()).await?;

    // Clear the event map
    let cid = calendar_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM caldav_event_map WHERE calendar_id = ?1",
            params![cid],
        )
        .map_err(|e| format!("clear caldav event map: {e}"))?;
        Ok(())
    })
    .await?;

    // Now do a fresh sync
    sync_calendar_events(client, db, account_id, calendar_id, calendar_href).await
}
