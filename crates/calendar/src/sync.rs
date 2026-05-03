use rusqlite::{OptionalExtension, Row, params};

use gmail::client::GmailState;
use graph::client::GraphState;
use rtsk::db::ReadDbState;
use rtsk::db::queries_extra::{
    CalendarEventRow, delete_calendar_event_by_remote_id, get_calendar_id_by_remote_id,
    DiscoveredCalendar, update_calendar_sync_token, upsert_calendar_event_row,
    upsert_discovered_calendar,
};
use rtsk::db::types::DbCalendar;

use super::google::{google_calendar_list_calendars_impl, google_calendar_sync_events_impl};
use super::graph::{graph_calendar_list_calendars_impl, graph_calendar_sync_events_impl};
use super::types::{CalendarEventDto, CalendarInfoDto, CalendarSyncResultDto};

pub async fn calendar_sync_account_impl(
    account_id: &str,
    db: &ReadDbState,
    gmail: &GmailState,
    graph: &GraphState,
) -> Result<(), String> {
    let provider = db
        .with_conn({
            let account_id = account_id.to_string();
            move |conn| {
                conn.query_row(
                    "SELECT provider, calendar_provider, caldav_url FROM accounts WHERE id = ?1",
                    params![account_id],
                    |row| {
                        Ok((
                            row.get::<_, String>("provider")?,
                            row.get::<_, Option<String>>("calendar_provider")?,
                            row.get::<_, Option<String>>("caldav_url")?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| e.to_string())
                .map(|row| {
                    let (provider, calendar_provider, caldav_url) = row?;
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
                    } else {
                        None
                    }
                })
            }
        })
        .await?;

    match provider {
        Some("google_api") => sync_google_calendar_account(account_id, db, gmail).await,
        Some("graph") => sync_graph_calendar_account(account_id, db, graph).await,
        Some("caldav") => {
            sync_caldav_calendar_account(account_id, db, gmail.encryption_key()).await
        }
        _ => Err(format!(
            "No calendar provider configured for account {account_id}"
        )),
    }
}

/// Convenience entry point for syncing a single account's calendars.
///
/// Constructs ephemeral `GmailState` / `GraphState` internally so callers
/// only need `ReadDbState` + encryption key (same pattern as `sync_delta_for_account`).
pub async fn calendar_sync_account(
    account_id: &str,
    db: &ReadDbState,
    encryption_key: [u8; 32],
) -> Result<(), String> {
    let gmail = gmail::client::new_gmail_state(encryption_key);
    let graph = graph::client::new_graph_state(encryption_key);
    calendar_sync_account_impl(account_id, db, &gmail, &graph).await
}

pub async fn upsert_discovered_calendars_impl(
    db: &ReadDbState,
    account_id: &str,
    provider: &str,
    calendars: Vec<CalendarInfoDto>,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let provider = provider.to_string();
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for calendar in calendars {
            upsert_discovered_calendar(
                &tx,
                &DiscoveredCalendar {
                    account_id: &account_id,
                    provider: &provider,
                    remote_id: &calendar.remote_id,
                    display_name: &calendar.display_name,
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
    db: &ReadDbState,
    account_id: &str,
    calendar_remote_id: &str,
    sync_result: CalendarSyncResultDto,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    db.with_conn(move |conn| {
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
    db: &ReadDbState,
    account_id: &str,
    calendar_remote_id: &str,
    events: Vec<CalendarEventDto>,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    db.with_conn(move |conn| {
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
    db: &ReadDbState,
    account_id: &str,
    calendar_remote_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    let remote_event_id = remote_event_id.to_string();
    db.with_conn(move |conn| {
        let calendar_id: String = get_calendar_id_by_remote_id(conn, &account_id, &calendar_remote_id)?
            .ok_or_else(|| format!("calendar not found: account={account_id} remote={calendar_remote_id}"))?;
        delete_calendar_event_by_remote_id(conn, &calendar_id, &remote_event_id)?;
        Ok(())
    })
    .await
}

pub async fn load_visible_calendars(
    db: &ReadDbState,
    account_id: &str,
) -> Result<Vec<DbCalendar>, String> {
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
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
    })
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
    db: &ReadDbState,
    gmail: &GmailState,
) -> Result<(), String> {
    let client = gmail.get(account_id).await?;
    let calendars = google_calendar_list_calendars_impl(account_id, db, &client).await?;
    upsert_discovered_calendars_impl(db, account_id, "google", calendars).await?;
    let visible_calendars = load_visible_calendars(db, account_id).await?;

    for calendar in visible_calendars {
        let sync_result = google_calendar_sync_events_impl(
            account_id,
            &calendar.remote_id,
            calendar.sync_token,
            db,
            &client,
        )
        .await?;
        apply_calendar_sync_result_impl(db, account_id, &calendar.remote_id, sync_result).await?;
    }

    Ok(())
}

async fn sync_graph_calendar_account(
    account_id: &str,
    db: &ReadDbState,
    graph: &GraphState,
) -> Result<(), String> {
    let client = graph.get(account_id).await?;
    let calendars = graph_calendar_list_calendars_impl(account_id, db, &client).await?;
    upsert_discovered_calendars_impl(db, account_id, "graph", calendars).await?;
    let visible_calendars = load_visible_calendars(db, account_id).await?;

    for calendar in visible_calendars {
        let sync_result = graph_calendar_sync_events_impl(
            account_id,
            &calendar.remote_id,
            calendar.sync_token,
            db,
            &client,
        )
        .await?;
        apply_calendar_sync_result_impl(db, account_id, &calendar.remote_id, sync_result).await?;
    }

    Ok(())
}

async fn sync_caldav_calendar_account(
    account_id: &str,
    db: &ReadDbState,
    encryption_key: &[u8; 32],
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
    let attempt = run_caldav_sync_attempt(account_id, db, &config, needs_discovery_now).await;
    match attempt {
        Ok(()) => Ok(()),
        Err(err) if used_persisted => {
            log::warn!(
                "CalDAV sync for {account_id} failed with persisted URLs ({err}); \
                 clearing principal/home and rediscovering"
            );
            super::caldav::clear_persisted_caldav_urls(db, account_id).await;
            let refreshed = super::caldav::load_caldav_account_config(db, encryption_key, account_id)
                .await?;
            // After clearing, both URLs are None so this branch always runs
            // discovery; pass `true` to persist the freshly discovered values.
            run_caldav_sync_attempt(account_id, db, &refreshed, true).await
        }
        Err(err) => Err(err),
    }
}

async fn run_caldav_sync_attempt(
    account_id: &str,
    db: &ReadDbState,
    config: &super::caldav::CaldavAccountConfig,
    persist_after_build: bool,
) -> Result<(), String> {
    let client = super::caldav::build_client_from_config(config).await?;
    if persist_after_build {
        super::caldav::persist_discovery_results(
            db,
            account_id,
            client.principal_url(),
            client.calendar_home_url(),
        )
        .await;
    }
    rtsk::caldav::sync::sync_caldav_calendars(&client, db, account_id)
        .await
        .map(|_| ())
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
    }
}
