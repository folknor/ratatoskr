use rusqlite::{OptionalExtension, Row, params};

use crate::db::DbState;
use crate::db::types::DbCalendar;
use crate::gmail::client::GmailState;

use super::caldav::{caldav_list_calendars_impl, caldav_sync_events_impl};
use super::google::{google_calendar_list_calendars_impl, google_calendar_sync_events_impl};
use super::types::{
    CalendarEventDto, CalendarEventInput, CalendarInfoDto, CalendarSyncResultDto,
};

pub async fn calendar_sync_account_impl(
    account_id: &str,
    db: &DbState,
    gmail: &GmailState,
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
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    },
                )
                .optional()
                .map_err(|e| e.to_string())
                .and_then(|row| {
                    let Some((provider, calendar_provider, caldav_url)) = row else {
                        return Ok(None);
                    };

                    if calendar_provider.as_deref() == Some("google_api") || provider == "gmail_api"
                    {
                        Ok(Some("google_api"))
                    } else if calendar_provider.as_deref() == Some("caldav")
                        || (provider == "caldav"
                            && caldav_url
                                .as_deref()
                                .is_some_and(|value| !value.trim().is_empty()))
                    {
                        Ok(Some("caldav"))
                    } else {
                        Ok(None)
                    }
                })
            }
        })
        .await?;

    match provider.as_deref() {
        Some("google_api") => sync_google_calendar_account(account_id, db, gmail).await,
        Some("caldav") => {
            sync_caldav_calendar_account(account_id, db, gmail.encryption_key()).await
        }
        _ => Err(format!(
            "No calendar provider configured for account {account_id}"
        )),
    }
}

pub async fn upsert_discovered_calendars_impl(
    db: &DbState,
    account_id: &str,
    provider: &str,
    calendars: Vec<CalendarInfoDto>,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let provider = provider.to_string();
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for calendar in calendars {
            let existing_id: Option<String> = tx
                .query_row(
                    "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                    params![&account_id, &calendar.remote_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| e.to_string())?;
            let id = existing_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
            tx.execute(
                "INSERT INTO calendars (id, account_id, provider, remote_id, display_name, color, is_primary)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
                 ON CONFLICT(account_id, remote_id) DO UPDATE SET
                   display_name = ?5, color = ?6, is_primary = ?7, updated_at = unixepoch()",
                params![
                    &id,
                    &account_id,
                    &provider,
                    &calendar.remote_id,
                    &calendar.display_name,
                    &calendar.color,
                    calendar.is_primary as i64,
                ],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn apply_calendar_sync_result_impl(
    db: &DbState,
    account_id: &str,
    calendar_remote_id: &str,
    sync_result: CalendarSyncResultDto,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    db.with_conn(move |conn| {
        let calendar_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, calendar_remote_id],
                |row| row.get(0),
            )
            .map_err(|e| e.to_string())?;

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

        for event in sync_result.created.into_iter().chain(sync_result.updated) {
            let event = calendar_dto_to_input(event);
            upsert_calendar_event(&tx, &account_id, &calendar_id, &event)?;
        }

        for remote_event_id in sync_result.deleted_remote_ids {
            tx.execute(
                "DELETE FROM calendar_events WHERE calendar_id = ?1 AND remote_event_id = ?2",
                params![calendar_id, remote_event_id],
            )
            .map_err(|e| e.to_string())?;
        }

        if sync_result.new_sync_token.is_some() || sync_result.new_ctag.is_some() {
            tx.execute(
                "UPDATE calendars
                 SET sync_token = COALESCE(?1, sync_token),
                     ctag = COALESCE(?2, ctag),
                     updated_at = unixepoch()
                 WHERE id = ?3",
                params![
                    sync_result.new_sync_token,
                    sync_result.new_ctag,
                    calendar_id
                ],
            )
            .map_err(|e| e.to_string())?;
        }

        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn upsert_provider_events_impl(
    db: &DbState,
    account_id: &str,
    calendar_remote_id: &str,
    events: Vec<CalendarEventInput>,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    db.with_conn(move |conn| {
        let calendar_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, calendar_remote_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?
            .ok_or_else(|| {
                format!("Calendar with remote_id '{calendar_remote_id}' not found for account '{account_id}'")
            })?;

        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for event in events {
            upsert_calendar_event(&tx, &account_id, &calendar_id, &event)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn delete_provider_event_impl(
    db: &DbState,
    account_id: &str,
    calendar_remote_id: &str,
    remote_event_id: &str,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let calendar_remote_id = calendar_remote_id.to_string();
    let remote_event_id = remote_event_id.to_string();
    db.with_conn(move |conn| {
        let calendar_id: String = conn
            .query_row(
                "SELECT id FROM calendars WHERE account_id = ?1 AND remote_id = ?2",
                params![account_id, calendar_remote_id],
                |row| row.get(0),
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

pub async fn load_visible_calendars(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<DbCalendar>, String> {
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM calendars WHERE account_id = ?1 AND is_visible = 1
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
    })
}

pub fn calendar_input_to_dto(event: CalendarEventInput) -> CalendarEventDto {
    CalendarEventDto {
        remote_event_id: event.remote_event_id,
        uid: event.uid,
        etag: event.etag,
        summary: event.summary,
        description: event.description,
        location: event.location,
        start_time: event.start_time,
        end_time: event.end_time,
        is_all_day: event.is_all_day,
        status: event.status,
        organizer_email: event.organizer_email,
        attendees_json: event.attendees_json,
        html_link: event.html_link,
        ical_data: event.ical_data,
    }
}

fn calendar_dto_to_input(event: CalendarEventDto) -> CalendarEventInput {
    CalendarEventInput {
        remote_event_id: event.remote_event_id,
        uid: event.uid,
        etag: event.etag,
        summary: event.summary,
        description: event.description,
        location: event.location,
        start_time: event.start_time,
        end_time: event.end_time,
        is_all_day: event.is_all_day,
        status: event.status,
        organizer_email: event.organizer_email,
        attendees_json: event.attendees_json,
        html_link: event.html_link,
        ical_data: event.ical_data,
    }
}

async fn sync_google_calendar_account(
    account_id: &str,
    db: &DbState,
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

async fn sync_caldav_calendar_account(
    account_id: &str,
    db: &DbState,
    encryption_key: &[u8; 32],
) -> Result<(), String> {
    let calendars = caldav_list_calendars_impl(account_id, db, encryption_key).await?;
    upsert_discovered_calendars_impl(db, account_id, "caldav", calendars).await?;
    let visible_calendars = load_visible_calendars(db, account_id).await?;

    for calendar in visible_calendars {
        let sync_result =
            caldav_sync_events_impl(account_id, &calendar.remote_id, db, encryption_key).await?;
        apply_calendar_sync_result_impl(db, account_id, &calendar.remote_id, sync_result).await?;
    }

    Ok(())
}

pub fn upsert_calendar_event(
    tx: &rusqlite::Transaction<'_>,
    account_id: &str,
    calendar_id: &str,
    event: &CalendarEventInput,
) -> Result<(), String> {
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute(
        "INSERT INTO calendar_events (id, account_id, google_event_id, summary, description, location, start_time, end_time, is_all_day, status, organizer_email, attendees_json, html_link, calendar_id, remote_event_id, etag, ical_data, uid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)
         ON CONFLICT(account_id, google_event_id) DO UPDATE SET
           summary = ?4, description = ?5, location = ?6, start_time = ?7, end_time = ?8,
           is_all_day = ?9, status = ?10, organizer_email = ?11, attendees_json = ?12,
           html_link = ?13, calendar_id = ?14, remote_event_id = ?15, etag = ?16,
           ical_data = ?17, uid = ?18, updated_at = unixepoch()",
        params![
            id,
            account_id,
            event.remote_event_id,
            event.summary,
            event.description,
            event.location,
            event.start_time,
            event.end_time,
            event.is_all_day as i64,
            event.status,
            event.organizer_email,
            event.attendees_json,
            event.html_link,
            calendar_id,
            event.remote_event_id,
            event.etag,
            event.ical_data,
            event.uid,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
