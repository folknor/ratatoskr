use std::collections::{HashMap, HashSet};

use crate::db::DbState;
use crate::db::queries_extra::calendars::{
    UpsertCalendarEventParams, db_delete_events_for_calendar, db_update_calendar_sync_token,
    db_upsert_calendar, db_upsert_calendar_event,
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
        db_update_calendar_sync_token(db, calendar_id.clone(), None, cal.ctag.clone()).await?;
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
                    upsert_parsed_event(db, account_id, calendar_id, uri, &etag, ical_data, event)
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
            crate::db::queries_extra::caldav_sync::delete_caldav_events_sync(
                conn,
                &cal_id,
                &deleted_owned,
            )
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
        UpsertCalendarEventParams {
            account_id: account_id.to_string(),
            google_event_id: google_event_id.clone(),
            summary: event.summary.clone(),
            description: event.description.clone(),
            location: event.location.clone(),
            start_time,
            end_time,
            is_all_day: event.is_all_day,
            status: event.status.clone(),
            organizer_email: event.organizer_email.clone(),
            attendees_json,
            html_link: None,
            calendar_id: Some(calendar_id.to_string()),
            remote_event_id: Some(uri.to_string()),
            etag: Some(etag.to_string()),
            ical_data: Some(ical_data.to_string()),
            uid: event.uid.clone(),
            title: event.summary.clone(),
            recurrence_rule: event.rrule.clone(),
            organizer_name: event.organizer_name.clone(),
            ..UpsertCalendarEventParams::default()
        },
    )
    .await?;

    // Track URI -> ETag mapping for incremental sync
    let cal_id = calendar_id.to_string();
    let uri_owned = uri.to_string();
    let etag_owned = etag.to_string();
    let uid_for_map = uid.clone();
    db.with_conn(move |conn| {
        crate::db::queries_extra::caldav_sync::upsert_caldav_event_map_sync(
            conn,
            &uri_owned,
            &cal_id,
            &uid_for_map,
            &etag_owned,
        )
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
    let db_attendees: Vec<crate::db::queries_extra::caldav_sync::CalDavAttendee> = event
        .attendees
        .iter()
        .map(|a| crate::db::queries_extra::caldav_sync::CalDavAttendee {
            email: a.email.clone(),
            name: a.name.clone(),
            partstat: a.partstat.clone(),
            is_organizer: a.is_organizer,
        })
        .collect();
    let organizer_email = event.organizer_email.clone();
    let organizer_name = event.organizer_name.clone();

    db.with_conn(move |conn| {
        crate::db::queries_extra::caldav_sync::sync_caldav_attendees_sync(
            conn,
            &aid,
            &geid,
            &db_attendees,
            organizer_email.as_deref(),
            organizer_name.as_deref(),
        )
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
    let db_reminders: Vec<crate::db::queries_extra::caldav_sync::CalDavReminder> = event
        .reminders
        .iter()
        .map(|r| crate::db::queries_extra::caldav_sync::CalDavReminder {
            minutes_before: r.minutes_before,
            method: r.method.clone(),
        })
        .collect();

    db.with_conn(move |conn| {
        crate::db::queries_extra::caldav_sync::sync_caldav_reminders_sync(
            conn,
            &aid,
            &geid,
            &db_reminders,
        )
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
        crate::db::queries_extra::caldav_sync::load_calendar_ctag_sync(conn, &cid)
    })
    .await
}

async fn load_stored_etags(
    db: &DbState,
    calendar_id: &str,
) -> Result<HashMap<String, String>, String> {
    let cid = calendar_id.to_string();
    db.with_conn(move |conn| {
        crate::db::queries_extra::caldav_sync::load_caldav_etags_sync(conn, &cid)
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
        crate::db::queries_extra::caldav_sync::clear_caldav_event_map_sync(conn, &cid)
    })
    .await?;

    // Now do a fresh sync
    sync_calendar_events(client, db, account_id, calendar_id, calendar_href).await
}
