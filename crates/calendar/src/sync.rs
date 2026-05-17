use std::collections::{HashMap, HashSet};

use rusqlite::{Row, params};
use service_state::WriteDbState;
use tokio_util::sync::CancellationToken;

use gmail::client::GmailState;
use graph::client::GraphState;
use jmap::client::JmapState;
use db::db::queries_extra::calendar_contacts_writes::{
    CalDavAttendee, CalDavReminder, CalendarEventRow, DiscoveredCalendar,
    delete_caldav_events, delete_calendar_event_by_remote_id, get_calendar_id_by_remote_id,
    reap_orphan_overrides, sync_caldav_attendees, sync_caldav_reminders,
    update_calendar_sync_token, upsert_caldav_event_map, upsert_calendar_event_row,
    upsert_discovered_calendar,
};
use rtsk::caldav::client::CalDavClient;
use rtsk::caldav::parse;
use rtsk::db::ReadDbState;
use rtsk::db::types::DbCalendar;

use super::google::{google_calendar_list_calendars_impl, google_calendar_sync_events_impl};
use super::graph::{graph_calendar_list_calendars_impl, graph_calendar_sync_events_impl};
use super::types::{CalendarEventDto, CalendarInfoDto, CalendarSyncResultDto};

/// Outcome of a calendar sync run.
///
/// `mutated` tracks whether the run wrote anything to the calendar tables
/// before terminating. `CalendarRuntime` reads this independently of
/// `result`: a partial-commit failure (provider error mid-loop after the
/// discovered-calendars upsert succeeded) must still drive a UI reload.
/// Conditioning emission on `result.is_ok()` would leave the UI stale
/// after exactly that case.
pub struct CalendarSyncOutcome {
    pub mutated: bool,
    pub result: Result<(), String>,
}

pub async fn calendar_sync_account_impl(
    account_id: &str,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    gmail: &GmailState,
    graph: &GraphState,
    jmap: &JmapState,
    cancellation_token: &CancellationToken,
) -> CalendarSyncOutcome {
    if cancellation_token.is_cancelled() {
        return CalendarSyncOutcome {
            mutated: false,
            result: Err("calendar sync cancelled".to_string()),
        };
    }
    let provider_result = read_db
        .with_read_mapped({
            let account_id = account_id.to_string();
            move |conn| {
                let row = match conn.query_row(
                    "SELECT provider, calendar_provider, caldav_url FROM accounts WHERE id = ?1",
                    params![account_id],
                    |row| {
                        Ok((
                            row.get::<_, String>("provider")?,
                            row.get::<_, Option<String>>("calendar_provider")?,
                            row.get::<_, Option<String>>("caldav_url")?,
                        ))
                    },
                ) {
                    Ok(row) => Some(row),
                    Err(db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => None,
                    Err(e) => return Err(e.to_string()),
                };
                Ok(row.and_then(|(provider, calendar_provider, caldav_url)| {
                    let has_caldav_url = caldav_url
                        .as_deref()
                        .is_some_and(|value| !value.trim().is_empty());

                    if calendar_provider.as_deref() == Some("google_api") || provider == "gmail_api"
                    {
                        Some("google_api")
                    } else if calendar_provider.as_deref() == Some("graph") || provider == "graph" {
                        Some("graph")
                    } else if calendar_provider.as_deref() == Some("caldav")
                        || (provider == "caldav" && has_caldav_url)
                    {
                        Some("caldav")
                    } else if calendar_provider.as_deref() == Some("jmap") || provider == "jmap" {
                        Some("jmap")
                    } else {
                        None
                    }
                }))
            }
        },
        |e| e,
    )
        .await;

    let provider = match provider_result {
        Ok(p) => p,
        Err(e) => {
            return CalendarSyncOutcome {
                mutated: false,
                result: Err(e),
            };
        }
    };

    let mut mutated = false;
    let result = match provider {
        Some("google_api") => {
            sync_google_calendar_account(
                account_id,
                write_db,
                read_db,
                gmail,
                cancellation_token,
                &mut mutated,
            )
            .await
        }
        Some("graph") => {
            sync_graph_calendar_account(
                account_id,
                write_db,
                read_db,
                graph,
                cancellation_token,
                &mut mutated,
            )
            .await
        }
        Some("caldav") => {
            sync_caldav_calendar_account(
                account_id,
                write_db,
                read_db,
                gmail.encryption_key(),
                cancellation_token,
                &mut mutated,
            )
            .await
        }
        Some("jmap") => {
            super::jmap::sync_jmap_calendar_account(
                account_id,
                write_db,
                read_db,
                jmap,
                cancellation_token,
                &mut mutated,
            )
            .await
        }
        _ => Err(format!(
            "No calendar provider configured for account {account_id}"
        )),
    };

    CalendarSyncOutcome { mutated, result }
}

pub async fn upsert_discovered_calendars_impl(
    db: &WriteDbState,
    account_id: &str,
    provider: &str,
    calendars: Vec<CalendarInfoDto>,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let provider = provider.to_string();
    db.with_write(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for calendar in calendars {
            upsert_discovered_calendar(
                &tx,
                &DiscoveredCalendar {
                    account_id: &account_id,
                    provider: &provider,
                    remote_id: &calendar.remote_id,
                    display_name: Some(&calendar.display_name),
                    color: calendar.color.as_deref(),
                    is_primary: calendar.is_primary,
                    can_edit: calendar.can_edit,
                },
            )?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn apply_calendar_sync_result_impl(
    db: &WriteDbState,
    account_id: &str,
    calendar_remote_id: &str,
    sync_result: CalendarSyncResultDto,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    db.with_write(move |conn| {
        let calendar_id: String = get_calendar_id_by_remote_id(conn, &account_id, &calendar_remote_id)?
            .ok_or_else(|| format!("calendar not found: account={account_id} remote={calendar_remote_id}"))?;

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        for event in sync_result.created.into_iter().chain(sync_result.updated) {
            let row = calendar_event_dto_to_row(&account_id, &calendar_id, &event);
            upsert_calendar_event_row(&tx, &row)?;
        }

        for remote_event_id in sync_result.deleted_remote_ids {
            delete_calendar_event_by_remote_id(&tx, &calendar_id, &remote_event_id)?;
        }

        if sync_result.new_sync_token.is_some() || sync_result.new_ctag.is_some() {
            update_calendar_sync_token(
                &tx,
                &calendar_id,
                sync_result.new_sync_token.as_deref(),
                sync_result.new_ctag.as_deref(),
            )?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn upsert_provider_events_impl(
    db: &WriteDbState,
    account_id: &str,
    calendar_remote_id: &str,
    events: Vec<CalendarEventDto>,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    db.with_write(move |conn| {
        let calendar_id: String = get_calendar_id_by_remote_id(conn, &account_id, &calendar_remote_id)?
            .ok_or_else(|| {
                format!("Calendar with remote_id '{calendar_remote_id}' not found for account '{account_id}'")
            })?;

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for event in events {
            let row = calendar_event_dto_to_row(&account_id, &calendar_id, &event);
            upsert_calendar_event_row(&tx, &row)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn delete_provider_event_impl(
    db: &WriteDbState,
    account_id: &str,
    calendar_remote_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    let remote_event_id = remote_event_id.to_string();
    db.with_write(move |conn| {
        let calendar_id: String = get_calendar_id_by_remote_id(conn, &account_id, &calendar_remote_id)?
            .ok_or_else(|| format!("calendar not found: account={account_id} remote={calendar_remote_id}"))?;
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        delete_calendar_event_by_remote_id(&tx, &calendar_id, &remote_event_id)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn load_visible_calendars(
    db: &ReadDbState,
    account_id: &str,
) -> Result<Vec<DbCalendar>, String> {
    let account_id = account_id.to_string();
    db.with_read_mapped(
        move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, provider, remote_id, display_name, color, \
                            is_primary, is_visible, sync_token, ctag, created_at, \
                            updated_at, sort_order, is_default, provider_id, can_edit \
                     FROM calendars WHERE account_id = ?1 AND is_visible = 1 \
                     ORDER BY is_primary DESC, display_name ASC",
                )
                .map_err(|e| e.to_string())?;
            stmt.query_map(params![account_id], row_to_db_calendar)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        },
        |e| e,
    )
    .await
}

fn row_to_db_calendar(row: &Row<'_>) -> rusqlite::Result<DbCalendar> {
    Ok(DbCalendar {
        id: row.get("id")?,
        account_id: row.get("account_id")?,
        provider: row.get("provider")?,
        remote_id: row.get("remote_id")?,
        display_name: row.get("display_name")?,
        color: row.get("color")?,
        is_primary: row.get("is_primary")?,
        is_visible: row.get("is_visible")?,
        sync_token: row.get("sync_token")?,
        ctag: row.get("ctag")?,
        created_at: row.get("created_at")?,
        updated_at: row.get("updated_at")?,
        sort_order: row.get("sort_order")?,
        is_default: row.get("is_default")?,
        provider_id: row.get("provider_id")?,
        can_edit: row.get("can_edit")?,
    })
}

async fn sync_google_calendar_account(
    account_id: &str,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    gmail: &GmailState,
    cancellation_token: &CancellationToken,
    mutated: &mut bool,
) -> Result<(), String> {
    let gmail_key = *gmail.encryption_key();
    let client = gmail
        .get_or_try_insert_with(account_id, || {
            gmail::client::GmailClient::from_account(read_db, account_id, gmail_key)
        })
        .await?;
    let calendars = google_calendar_list_calendars_impl(account_id, read_db, &client).await?;
    upsert_discovered_calendars_impl(write_db, account_id, "google", calendars).await?;
    *mutated = true;
    let visible_calendars = load_visible_calendars(read_db, account_id).await?;

    for calendar in visible_calendars {
        // Cancellation checkpoint - mirrors the IMAP and JMAP per-mailbox
        // patterns. Calendar sync is idempotent against CalDAV CTags /
        // Exchange ETags, so a cancelled run resumes from wherever the next
        // run finds the provider state - no marker-file repair needed.
        // Point-checks between RPC boundaries, not mid-RPC.
        if cancellation_token.is_cancelled() {
            return Err("calendar sync cancelled".to_string());
        }
        let sync_result = google_calendar_sync_events_impl(
            account_id,
            &calendar.remote_id,
            calendar.sync_token,
            read_db,
            &client,
            cancellation_token,
        )
        .await?;
        apply_calendar_sync_result_impl(write_db, account_id, &calendar.remote_id, sync_result).await?;
    }

    Ok(())
}

async fn sync_graph_calendar_account(
    account_id: &str,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    graph: &GraphState,
    cancellation_token: &CancellationToken,
    mutated: &mut bool,
) -> Result<(), String> {
    let graph_key = *graph.encryption_key();
    let client = graph
        .get_or_try_insert_with(account_id, || {
            graph::client::GraphClient::from_account(read_db, account_id, graph_key)
        })
        .await?;
    let calendars = graph_calendar_list_calendars_impl(account_id, read_db, &client).await?;
    upsert_discovered_calendars_impl(write_db, account_id, "graph", calendars).await?;
    *mutated = true;
    let visible_calendars = load_visible_calendars(read_db, account_id).await?;

    for calendar in visible_calendars {
        if cancellation_token.is_cancelled() {
            return Err("calendar sync cancelled".to_string());
        }
        let sync_result = graph_calendar_sync_events_impl(
            account_id,
            &calendar.remote_id,
            calendar.sync_token,
            read_db,
            &client,
            cancellation_token,
        )
        .await?;
        apply_calendar_sync_result_impl(write_db, account_id, &calendar.remote_id, sync_result).await?;
    }

    Ok(())
}

async fn sync_caldav_calendar_account(
    account_id: &str,
    write_db: &WriteDbState,
    db: &ReadDbState,
    encryption_key: &[u8; 32],
    cancellation_token: &CancellationToken,
    mutated: &mut bool,
) -> Result<(), String> {
    let config =
        super::caldav::load_caldav_account_config(db, encryption_key, account_id).await?;
    let used_persisted = !(config.home_url().is_none() && config.principal_url().is_none());

    // Both the client construction and the actual sync can fail when the
    // persisted URLs (`caldav_principal_url`, `caldav_home_url`) point at
    // a stale endpoint. Stale principal alone is enough: with persisted
    // principal + no home, `build_client_from_config` runs `discover()`
    // which skips principal discovery (since it's already set) and tries
    // a calendar-home PROPFIND against the stale principal, which 404s
    // and propagates here. The retry block below covers BOTH failure
    // sites - construction *and* sync - so a stale principal doesn't
    // wedge the account permanently. (Round 3 #32.)
    let needs_discovery_now = config.home_url().is_none();
    let attempt = run_caldav_sync_attempt(
        account_id,
        write_db,
        db,
        &config,
        needs_discovery_now,
        cancellation_token,
        mutated,
    )
    .await;
    match attempt {
        Ok(()) => Ok(()),
        Err(err) if used_persisted => {
            log::warn!(
                "CalDAV sync for {account_id} failed with persisted URLs ({err}); \
                 clearing principal/home and rediscovering"
            );
            super::caldav::clear_persisted_caldav_urls(write_db, account_id).await;
            let refreshed = super::caldav::load_caldav_account_config(db, encryption_key, account_id)
                .await?;
            // After clearing, both URLs are None so this branch always runs
            // discovery; pass `true` to persist the freshly discovered values.
            run_caldav_sync_attempt(
                account_id,
                write_db,
                db,
                &refreshed,
                true,
                cancellation_token,
                mutated,
            )
            .await
        }
        Err(err) => Err(err),
    }
}

async fn run_caldav_sync_attempt(
    account_id: &str,
    write_db: &WriteDbState,
    db: &ReadDbState,
    config: &super::caldav::CaldavAccountConfig,
    persist_after_build: bool,
    cancellation_token: &CancellationToken,
    mutated: &mut bool,
) -> Result<(), String> {
    let client = super::caldav::build_client_from_config(config).await?;
    if persist_after_build {
        super::caldav::persist_discovery_results(
            write_db,
            account_id,
            client.principal_url(),
            client.calendar_home_url(),
        )
        .await;
    }
    let outcome = sync_caldav_calendars(&client, write_db, db, account_id, cancellation_token)
        .await?;
    // Any non-zero CalDAV write counts means we touched the calendar tables.
    // `calendars_discovered` triggers `db_upsert_calendar` per row even when
    // the ctag turns out unchanged afterwards, so we conservatively flag it.
    if outcome.calendars_discovered > 0
        || outcome.events_upserted > 0
        || outcome.events_deleted > 0
    {
        *mutated = true;
    }
    Ok(())
}

/// Convert a `CalendarEventDto` from the provider layer into the DB-crate row
/// type, binding account and calendar context.
pub fn calendar_event_dto_to_row(
    account_id: &str,
    calendar_id: &str,
    event: &CalendarEventDto,
) -> CalendarEventRow {
    CalendarEventRow {
        account_id: account_id.to_string(),
        google_event_id: event.remote_event_id.clone(),
        remote_event_id: event.remote_event_id.clone(),
        calendar_id: calendar_id.to_string(),
        summary: event.summary.clone(),
        description: event.description.clone(),
        location: event.location.clone(),
        start_time: event.start_time,
        end_time: event.end_time,
        is_all_day: event.is_all_day,
        status: event.status.clone(),
        organizer_email: event.organizer_email.clone(),
        attendees_json: event.attendees_json.clone(),
        html_link: event.html_link.clone(),
        etag: event.etag.clone(),
        ical_data: event.ical_data.clone(),
        uid: event.uid.clone(),
        title: event.title.clone(),
        timezone: event.timezone.clone(),
        recurrence_rule: event.recurrence_rule.clone(),
        organizer_name: event.organizer_name.clone(),
        rsvp_status: event.rsvp_status.clone(),
        availability: event.availability.clone(),
        visibility: event.visibility.clone(),
        recurrence_id: None,
    }
}

#[derive(Debug)]
struct CalDavSyncResult {
    calendars_discovered: usize,
    events_upserted: usize,
    events_deleted: usize,
}

async fn sync_caldav_calendars(
    client: &CalDavClient,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    account_id: &str,
    cancellation_token: &CancellationToken,
) -> Result<CalDavSyncResult, String> {
    if cancellation_token.is_cancelled() {
        return Err("calendar sync cancelled".to_string());
    }

    let discovered = client.list_calendars().await?;

    log::info!(
        "CalDAV: discovered {} calendars for {account_id}",
        discovered.len()
    );

    let mut total_upserted = 0;
    let mut total_deleted = 0;
    let mut skipped_unchanged = 0;

    for cal in &discovered {
        if cancellation_token.is_cancelled() {
            return Err("calendar sync cancelled".to_string());
        }

        let can_edit = cal.can_edit.unwrap_or(true);
        let account_id_owned = account_id.to_string();
        let remote_id = cal.href.clone();
        let display_name = cal.display_name.clone();
        let color = cal.color.clone();
        let calendar_id = write_db
            .with_write(move |conn| {
                let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                let id = upsert_discovered_calendar(
                    &tx,
                    &DiscoveredCalendar {
                        account_id: &account_id_owned,
                        provider: "caldav",
                        remote_id: &remote_id,
                        display_name: display_name.as_deref(),
                        color: color.as_deref(),
                        is_primary: false,
                        can_edit,
                    },
                )?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok(id)
            })
            .await?;

        let stored_ctag = load_calendar_ctag(read_db, &calendar_id).await?;

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

        let (upserted, deleted) = sync_caldav_calendar_events(
            client,
            write_db,
            read_db,
            account_id,
            &calendar_id,
            &cal.href,
            cancellation_token,
        )
        .await?;

        total_upserted += upserted;
        total_deleted += deleted;

        let calendar_id_for_update = calendar_id.clone();
        let ctag = cal.ctag.clone();
        write_db
            .with_write(move |conn| {
                let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                update_calendar_sync_token(&tx, &calendar_id_for_update, None, ctag.as_deref())?;
                tx.commit().map_err(|e| e.to_string())?;
                Ok(())
            })
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
    })
}

async fn sync_caldav_calendar_events(
    client: &CalDavClient,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    account_id: &str,
    calendar_id: &str,
    calendar_href: &str,
    cancellation_token: &CancellationToken,
) -> Result<(usize, usize), String> {
    let remote_listing = client.list_events(calendar_href).await?;
    let remote_entries = remote_listing.entries;
    let failed_uris: HashSet<String> = remote_listing.failed_uris.into_iter().collect();
    let stored_etags = load_stored_etags(read_db, calendar_id).await?;

    let mut fetch_uris: Vec<String> = Vec::new();
    let remote_uri_set: HashSet<String> = remote_entries.iter().map(|e| e.uri.clone()).collect();

    for entry in &remote_entries {
        match stored_etags.get(&entry.uri) {
            Some(old_etag) if *old_etag == entry.etag => {}
            _ => fetch_uris.push(entry.uri.clone()),
        }
    }

    let deleted_uris: Vec<String> = if remote_entries.is_empty() && !stored_etags.is_empty() {
        log::warn!(
            "CalDAV sync for calendar {calendar_id}: server returned 0 events but local cache has {} - \
             suspecting a transient server failure and skipping the deletion step. \
             Use full_resync_calendar to force-clear if this is intentional.",
            stored_etags.len()
        );
        Vec::new()
    } else {
        let candidates: Vec<String> = stored_etags
            .keys()
            .filter(|uri| !remote_uri_set.contains(*uri))
            .filter(|uri| !failed_uris.contains(*uri))
            .cloned()
            .collect();
        if !failed_uris.is_empty() {
            let preserved = stored_etags
                .keys()
                .filter(|uri| failed_uris.contains(*uri))
                .count();
            if preserved > 0 {
                log::warn!(
                    "CalDAV sync for calendar {calendar_id}: server reported {preserved} stored events as failing in this 207; preserving local copies."
                );
            }
        }
        candidates
    };

    log::info!(
        "CalDAV sync for calendar {calendar_id}: {} to fetch, {} unchanged, {} deleted",
        fetch_uris.len(),
        remote_entries.len() - fetch_uris.len(),
        deleted_uris.len()
    );

    let etag_map: HashMap<&str, &str> = remote_entries
        .iter()
        .map(|e| (e.uri.as_str(), e.etag.as_str()))
        .collect();

    let uri_refs: Vec<&str> = fetch_uris.iter().map(String::as_str).collect();
    let fetched_icals = client.fetch_events(calendar_href, &uri_refs).await?;

    let mut upserted = 0;
    for (uri, ical_data) in &fetched_icals {
        if cancellation_token.is_cancelled() {
            return Err("calendar sync cancelled".to_string());
        }
        let etag = etag_map.get(uri.as_str()).unwrap_or(&"").to_string();

        match parse::parse_icalendar(ical_data) {
            Ok(events) => {
                let mut seen_keys: Vec<String> = Vec::with_capacity(events.len());
                let mut representative_uid: Option<String> = None;
                for event in &events {
                    let key = upsert_caldav_parsed_event(
                        write_db,
                        account_id,
                        calendar_id,
                        uri,
                        &etag,
                        ical_data,
                        event,
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
                    write_db
                        .with_write(move |conn| {
                            upsert_caldav_event_map(
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
                    write_db
                        .with_write(move |conn| {
                            reap_orphan_overrides(conn, &cal_id_owned, &uri_owned, &seen_keys)
                        })
                        .await?;
                }
            }
            Err(e) => {
                log::warn!("Failed to parse iCalendar at {uri}: {e}");
            }
        }
    }

    let deleted_count = deleted_uris.len();
    if !deleted_uris.is_empty() {
        let cal_id = calendar_id.to_string();
        let deleted_owned = deleted_uris;
        write_db
            .with_write(move |conn| delete_caldav_events(conn, &cal_id, &deleted_owned))
            .await?;
    }

    Ok((upserted, deleted_count))
}

async fn upsert_caldav_parsed_event(
    write_db: &WriteDbState,
    account_id: &str,
    calendar_id: &str,
    uri: &str,
    etag: &str,
    ical_data: &str,
    event: &parse::ParsedVEvent,
) -> Result<String, String> {
    let uid = event.uid.clone().unwrap_or_else(|| {
        log::warn!(
            "CalDAV VEVENT at {uri} has no UID (RFC 5545 violation); synthesizing dedup key from href"
        );
        href_synthetic_uid(uri)
    });
    let google_event_id = make_google_event_id(&uid, event.recurrence_id.as_deref());

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

    let Some(start_time) = event.start_time else {
        log::warn!(
            "CalDAV VEVENT at {uri} has no usable DTSTART; refusing to persist as epoch event"
        );
        return Ok(google_event_id);
    };
    let end_time = event.end_time.unwrap_or(start_time);

    let row = CalendarEventRow {
        account_id: account_id.to_string(),
        google_event_id: google_event_id.clone(),
        remote_event_id: uri.to_string(),
        calendar_id: calendar_id.to_string(),
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
        etag: Some(etag.to_string()),
        ical_data: Some(ical_data.to_string()),
        uid: event.uid.clone(),
        title: event.summary.clone(),
        timezone: event.timezone.clone(),
        recurrence_rule: event.rrule.clone(),
        organizer_name: event.organizer_name.clone(),
        rsvp_status: None,
        availability: None,
        visibility: None,
        recurrence_id: event.recurrence_id.clone(),
    };

    write_db
        .with_write(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            upsert_calendar_event_row(&tx, &row)?;
            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await?;

    sync_event_attendees(write_db, account_id, &google_event_id, event).await?;
    sync_event_reminders(write_db, account_id, &google_event_id, event).await?;

    Ok(google_event_id)
}

fn href_synthetic_uid(uri: &str) -> String {
    format!("href={uri}")
}

fn make_google_event_id(uid: &str, recurrence_id: Option<&str>) -> String {
    match recurrence_id {
        Some(rid) => format!("caldav:{uid}::recurrence-id={rid}"),
        None => format!("caldav:{uid}"),
    }
}

async fn sync_event_attendees(
    db: &WriteDbState,
    account_id: &str,
    google_event_id: &str,
    event: &parse::ParsedVEvent,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let geid = google_event_id.to_string();
    let db_attendees: Vec<CalDavAttendee> = event
        .attendees
        .iter()
        .map(|a| CalDavAttendee {
            email: a.email.clone(),
            name: a.name.clone(),
            partstat: a.partstat.clone(),
            is_organizer: a.is_organizer,
        })
        .collect();
    let organizer_email = event.organizer_email.clone();
    let organizer_name = event.organizer_name.clone();

    db.with_write(move |conn| {
        sync_caldav_attendees(
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

async fn sync_event_reminders(
    db: &WriteDbState,
    account_id: &str,
    google_event_id: &str,
    event: &parse::ParsedVEvent,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let geid = google_event_id.to_string();
    let db_reminders: Vec<CalDavReminder> = event
        .reminders
        .iter()
        .map(|r| CalDavReminder {
            minutes_before: r.minutes_before,
            method: r.method.clone(),
        })
        .collect();

    db.with_write(move |conn| sync_caldav_reminders(conn, &aid, &geid, &db_reminders))
        .await
}

async fn load_calendar_ctag(db: &ReadDbState, calendar_id: &str) -> Result<Option<String>, String> {
    let cid = calendar_id.to_string();
    db.with_read(move |conn| {
        match conn.query_row(
            "SELECT ctag FROM calendars WHERE id = ?1",
            params![cid],
            |row| row.get::<_, Option<String>>("ctag"),
        ) {
            Ok(ctag) => Ok(ctag),
            Err(db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(format!("load calendar ctag: {e}")),
        }
    })
    .await
}

async fn load_stored_etags(
    db: &ReadDbState,
    calendar_id: &str,
) -> Result<HashMap<String, String>, String> {
    let cid = calendar_id.to_string();
    db.with_read(move |conn| {
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
        let floating =
            make_google_event_id("uid-1@example.com", Some("20260315T100000"));
        let all_day =
            make_google_event_id("uid-1@example.com", Some("20260315"));
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
