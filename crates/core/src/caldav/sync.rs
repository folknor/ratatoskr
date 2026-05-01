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
/// 2. For each calendar, compare ctag - skip if unchanged
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
        // Honor the server's current-user-privilege-set when present: any
        // of `<write/>`, `<write-content/>`, or `<all/>` inside a granted
        // privilege flips us to editable. When the block is absent (older
        // servers, some Exchange CalDAV bridges) we default to editable
        // for compatibility - the previous unconditional `true` behavior.
        let can_edit = cal.can_edit.unwrap_or(true);
        let calendar_id = db_upsert_calendar(
            db,
            account_id.to_string(),
            "caldav".to_string(),
            cal.href.clone(),
            cal.display_name.clone(),
            cal.color.clone(),
            false, // is_primary - CalDAV doesn't specify a "primary" calendar
            can_edit,
        )
        .await?;

        // Check ctag for quick change detection
        let stored_ctag = load_calendar_ctag(db, &calendar_id).await?;

        if let Some(ref remote_ctag) = cal.ctag
            && stored_ctag.as_ref() == Some(remote_ctag)
        {
            log::debug!(
                "CalDAV: calendar {} ctag unchanged, skipping",
                cal.display_name.as_deref().unwrap_or(&cal.href)
            );
            skipped_unchanged += 1;
            continue;
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

    // Determine which events were deleted on the server. Defensive guard:
    // if the remote set came back completely empty but we have stored
    // entries, suspect a bad response (PROPFIND returning a 200/empty body,
    // 207 with no <response> children, transient server error masquerading
    // as success) rather than honoring it as "every event was deleted".
    // Clearing the local cache here is irreversible from the user's
    // perspective; preserving it costs nothing because the next successful
    // sync naturally reconciles. Real "user deleted everything" still
    // works through the explicit `full_resync_calendar` path.
    let deleted_uris: Vec<String> = if remote_entries.is_empty() && !stored_etags.is_empty() {
        log::warn!(
            "CalDAV sync for calendar {calendar_id}: server returned 0 events but local cache has {} - \
             suspecting a transient server failure and skipping the deletion step. \
             Use full_resync_calendar to force-clear if this is intentional.",
            stored_etags.len()
        );
        Vec::new()
    } else {
        stored_etags
            .keys()
            .filter(|uri| !remote_uri_set.contains(*uri))
            .cloned()
            .collect()
    };

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

    // Parse and upsert events.
    //
    // A single CalDAV resource (`uri`) carries a master VEVENT plus zero or
    // more RECURRENCE-ID overrides for the same UID. After upserting the
    // VEVENTs we see in this fetch, we prune any rows for the same resource
    // whose storage key isn't in the seen set - that reaps abandoned
    // overrides when the user removes an exception via the web UI but the
    // resource itself stays alive (so the URI-deletion path at the top of
    // this function never fires for it).
    //
    // The (uri, calendar_id, uid, etag) map row is the same for every
    // VEVENT in the resource (same href, same etag), so we lift the map
    // upsert out of the per-VEVENT loop and write it once. (Round 3 #28.)
    let mut upserted = 0;
    for (uri, ical_data) in &fetched_icals {
        let etag = etag_map.get(uri.as_str()).unwrap_or(&"").to_string();

        match parse::parse_icalendar(ical_data) {
            Ok(events) => {
                let mut seen_keys: Vec<String> = Vec::with_capacity(events.len());
                let mut representative_uid: Option<String> = None;
                for event in &events {
                    let key = upsert_parsed_event(
                        db, account_id, calendar_id, uri, &etag, ical_data, event,
                    )
                    .await?;
                    if representative_uid.is_none() {
                        representative_uid = Some(
                            event
                                .uid
                                .clone()
                                .unwrap_or_else(|| href_synthetic_uid(uri)),
                        );
                    }
                    seen_keys.push(key);
                    upserted += 1;
                }
                if let Some(uid_for_map) = representative_uid {
                    let cal_id_for_map = calendar_id.to_string();
                    let uri_for_map = uri.clone();
                    let etag_for_map = etag.clone();
                    db.with_conn(move |conn| {
                        crate::db::queries_extra::caldav_sync::upsert_caldav_event_map_sync(
                            conn,
                            &uri_for_map,
                            &cal_id_for_map,
                            &uid_for_map,
                            &etag_for_map,
                        )
                    })
                    .await?;
                }
                if !seen_keys.is_empty() {
                    let cal_id_owned = calendar_id.to_string();
                    let uri_owned = uri.clone();
                    db.with_conn(move |conn| {
                        crate::db::queries_extra::caldav_sync::reap_orphan_overrides_sync(
                            conn,
                            &cal_id_owned,
                            &uri_owned,
                            &seen_keys,
                        )
                    })
                    .await?;
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

/// Upsert a single parsed event into the database. Returns the storage key
/// (`google_event_id`) the row was upserted under, so the caller can
/// distinguish overrides from masters when reaping abandoned overrides
/// after a multi-VEVENT resource shrinks.
async fn upsert_parsed_event(
    db: &DbState,
    account_id: &str,
    calendar_id: &str,
    uri: &str,
    etag: &str,
    ical_data: &str,
    event: &parse::ParsedVEvent,
) -> Result<String, String> {
    // RFC 5545 § 3.6.1 makes UID MUST. Real-world emitters violate this
    // rarely but it does happen (some legacy bridges, ad-hoc scripts).
    // Refusing to upsert UID-less events would drop user-visible data the
    // server has, so we synthesize a stable dedup key from the resource
    // href.
    //
    // The synthetic UID is namespaced (`href={uri}` rather than just
    // `{uri}`) so it cannot collide with a real UID that happens to be
    // shaped like another resource's href. (Round 3 #29.) No emitter
    // does this in practice but the type system allowed it; the
    // namespace prefix closes the door for free.
    let uid = event.uid.clone().unwrap_or_else(|| {
        log::warn!(
            "CalDAV VEVENT at {uri} has no UID (RFC 5545 violation); synthesizing dedup key from href"
        );
        href_synthetic_uid(uri)
    });
    let google_event_id = make_google_event_id(&uid, event.recurrence_id.as_deref());

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

    // Refuse to persist events that have no DTSTART. The previous
    // `unwrap_or(0)` fallback rendered them in the calendar at the Unix
    // epoch (1970-01-01 00:00 UTC) - a confusing artefact rather than
    // diagnosable data. Logging the URI so an operator chasing a missing
    // event can find the dropped resource.
    let Some(start_time) = event.start_time else {
        log::warn!(
            "CalDAV VEVENT at {uri} has no usable DTSTART; refusing to persist as epoch event"
        );
        // We still return the synthesized key so the caller's reap-orphans
        // step doesn't delete an in-flight override row simply because we
        // chose to skip its persist this round.
        return Ok(google_event_id);
    };
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
            // IANA-form zone name from DTSTART. Drives the RRULE expander's
            // wall-clock walk; without this `RecurrenceTz::from_event_timezone`
            // gets None and falls back to chrono::Local, so a NY-zoned
            // master would render in the user's host zone and shift
            // every recurring instance by the offset. (Round 3 #5.)
            timezone: event.timezone.clone(),
            recurrence_rule: event.rrule.clone(),
            organizer_name: event.organizer_name.clone(),
            recurrence_id: event.recurrence_id.clone(),
            ..UpsertCalendarEventParams::default()
        },
    )
    .await?;

    // The URI -> ETag map row is identical for every VEVENT in the same
    // resource, so the caller writes it once after this loop completes
    // (see `sync_calendar_events`). (Round 3 #28.)

    // Sync attendees and reminders
    sync_event_attendees(db, account_id, &google_event_id, event).await?;
    sync_event_reminders(db, account_id, &google_event_id, event).await?;

    Ok(google_event_id)
}

/// Build a synthetic UID for a VEVENT that has no UID. The `href=` prefix
/// keeps the result distinct from any real UID that might happen to
/// equal another resource's href - the previous shape collided keys
/// silently in that pathological case. (Round 3 #29.)
fn href_synthetic_uid(uri: &str) -> String {
    format!("href={uri}")
}

/// Build the `google_event_id` key from a CalDAV UID, folding in the
/// RECURRENCE-ID for override instances. RFC 5545 § 3.8.4.4: a recurring
/// event series can ship with a master VEVENT (no RECURRENCE-ID) plus
/// one VEVENT per override instance, all sharing the same UID. Without
/// the RECURRENCE-ID discriminator, master + override would collide on
/// `(account_id, google_event_id)` and one would silently overwrite the
/// other on every sync.
///
/// The key carries the wall-clock canonical form rather than a resolved
/// Unix timestamp - resolving floating and all-day RECURRENCE-IDs at
/// upsert time silently re-anchors them in `chrono::Local`, so a sync
/// run on a UTC host and another on a NY host would mint two distinct
/// storage keys for the same override and orphan the previous row on TZ
/// change. The string form preserves the exact bytes the iCal source
/// carried; matching emitters (Apple, Google, Outlook) keep the form
/// stable across syncs by construction.
fn make_google_event_id(uid: &str, recurrence_id: Option<&str>) -> String {
    match recurrence_id {
        Some(rid) => format!("caldav:{uid}::recurrence-id={rid}"),
        None => format!("caldav:{uid}"),
    }
}

/// Sync attendees for an event.
///
/// We never short-circuit on an empty input list. The DB helper deletes
/// the existing attendees before inserting the new set, so an empty
/// remote attendee list is the signal "the event no longer has
/// attendees" and the local rows must be removed. The previous early-
/// return left stale local attendee rows behind whenever a remote
/// update cleared the list. (Round 3 #27.)
async fn sync_event_attendees(
    db: &DbState,
    account_id: &str,
    google_event_id: &str,
    event: &parse::ParsedVEvent,
) -> Result<(), String> {
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
///
/// Like `sync_event_attendees`, we never short-circuit on an empty input
/// list: the DB helper implements replacement semantics (delete-then-
/// insert), so an empty list is what removes stale local VALARM rows
/// after a remote update clears them. (Round 3 #27.)
async fn sync_event_reminders(
    db: &DbState,
    account_id: &str,
    google_event_id: &str,
    event: &parse::ParsedVEvent,
) -> Result<(), String> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_google_event_id_master_uses_uid_only() {
        let key = make_google_event_id("uid-123@example.com", None);
        assert_eq!(key, "caldav:uid-123@example.com");
    }

    #[test]
    fn make_google_event_id_override_includes_recurrence_id() {
        // RFC 5545 § 3.8.4.4: master + override VEVENTs share UID and are
        // discriminated by RECURRENCE-ID. Storage key must reflect that or
        // the two rows collide on (account_id, google_event_id) and the
        // last-written silently overwrites the other.
        let master = make_google_event_id("uid-123@example.com", None);
        let override_a =
            make_google_event_id("uid-123@example.com", Some("20260315T100000Z"));
        let override_b =
            make_google_event_id("uid-123@example.com", Some("20260322T100000Z"));
        assert_ne!(master, override_a);
        assert_ne!(master, override_b);
        assert_ne!(override_a, override_b);
    }

    #[test]
    fn make_google_event_id_keys_are_host_tz_independent() {
        // Regression guard for review #19/#20: the previous key shape used a
        // resolved Unix timestamp, so a floating RECURRENCE-ID synced on a
        // UTC host and the same one synced on a NY host produced two
        // distinct keys (one row per host). The wall-clock string form is
        // identical regardless of where the parser ran.
        let floating =
            make_google_event_id("uid-1@example.com", Some("20260315T100000"));
        let all_day =
            make_google_event_id("uid-1@example.com", Some("20260315"));
        // Both forms must yield ASCII keys that have nothing to do with the
        // host's local zone (no offset arithmetic baked in).
        assert!(
            floating
                .strip_prefix("caldav:uid-1@example.com::recurrence-id=")
                .is_some_and(|tail| tail == "20260315T100000")
        );
        assert!(
            all_day
                .strip_prefix("caldav:uid-1@example.com::recurrence-id=")
                .is_some_and(|tail| tail == "20260315")
        );
    }
}
