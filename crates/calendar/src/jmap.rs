//! JMAP arm for `calendar_sync_account_impl`.
//!
//! Wraps `jmap::calendar_sync::sync_calendars` so that the JMAP path
//! lives behind the same `CalendarRuntime` boundary as Google, Graph,
//! and CalDAV. Before Phase 5 the JMAP email-sync pipeline called
//! `sync_calendars` directly from inside email sync (bypassing the
//! runtime); that bypass is removed in the same commit that introduces
//! this arm.
//!
//! Cancellation is a coarse entry-point check today: the underlying
//! `jmap::calendar_sync::sync_calendars` does not accept a token.
//! Threading per-batch cancellation through the JMAP calendar pipeline
//! is tracked with the Gmail/Graph entry-only checks.

use std::collections::HashMap;

use db::db::queries_extra::calendar_contacts_writes::{
    CalendarAttendeeWriteRow, CalendarEventRow, CalendarReminderWriteRow,
    delete_event_by_account_remote_id, replace_event_attendees, replace_event_reminders,
    upsert_calendar_event_row,
};
use jmap::client::{JmapClient, JmapState};
use jmap::calendar_sync::{JmapCalendarEventRecord, JmapDiscoveredCalendar};
use rusqlite::params;
use rtsk::db::ReadDbState;
use service_state::WriteDbState;
use tokio_util::sync::CancellationToken;

pub(crate) async fn sync_jmap_calendar_account(
    account_id: &str,
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    jmap: &JmapState,
    cancellation_token: &CancellationToken,
    mutated: &mut bool,
) -> Result<(), String> {
    if cancellation_token.is_cancelled() {
        return Err("calendar sync cancelled".to_string());
    }
    let client = jmap
        .get_or_try_insert_with(account_id, || {
            JmapClient::from_account(read_db, account_id, jmap.encryption_key())
        })
        .await?;
    // Calendar list and event fetches are provider RPCs, but every local
    // mutation is applied through the Service writer state below. Flip
    // `mutated` before the first write because provider failures after a
    // partial commit still need to drive a UI reload.
    *mutated = true;

    let calendar_list = jmap::calendar_sync::fetch_calendar_list(&client).await?;
    let calendars = calendar_list
        .calendars
        .iter()
        .map(jmap_calendar_info)
        .collect();
    super::sync::upsert_discovered_calendars_impl(write_db, account_id, "jmap", calendars)
        .await?;
    save_jmap_calendar_sync_state(write_db, account_id, "Calendar", &calendar_list.state).await?;

    let visible_calendars = super::sync::load_visible_calendars(read_db, account_id).await?;
    let cal_map: HashMap<&str, &str> = visible_calendars
        .iter()
        .map(|calendar| (calendar.remote_id.as_str(), calendar.id.as_str()))
        .collect();

    let event_state = load_jmap_calendar_sync_state(read_db, account_id, "CalendarEvent").await?;
    let event_sync = if let Some(since_state) = event_state {
        jmap::calendar_sync::fetch_events_delta(&client, account_id, &cal_map, since_state).await?
    } else {
        jmap::calendar_sync::fetch_all_events(&client, account_id, &cal_map).await?
    };

    for record in event_sync.events {
        persist_jmap_calendar_event(write_db, account_id, record).await?;
    }
    for remote_event_id in event_sync.deleted_remote_ids {
        let aid = account_id.to_string();
        write_db
            .with_write(move |conn| delete_event_by_account_remote_id(conn, &aid, &remote_event_id))
            .await?;
    }

    save_jmap_calendar_sync_state(write_db, account_id, "CalendarEvent", &event_sync.state).await
}

fn jmap_calendar_info(calendar: &JmapDiscoveredCalendar) -> super::types::CalendarInfoDto {
    super::types::CalendarInfoDto {
        remote_id: calendar.remote_id.clone(),
        display_name: calendar
            .display_name
            .clone()
            .unwrap_or_else(|| calendar.remote_id.clone()),
        color: calendar.color.clone(),
        is_primary: calendar.is_primary,
        can_edit: true,
    }
}

async fn persist_jmap_calendar_event(
    db: &WriteDbState,
    account_id: &str,
    record: JmapCalendarEventRecord,
) -> Result<(), String> {
    let aid = account_id.to_string();
    db.with_write(move |conn| {
        let row = CalendarEventRow {
            account_id: aid.clone(),
            google_event_id: record.google_event_id.clone(),
            remote_event_id: record.remote_event_id.clone(),
            calendar_id: record.calendar_id.clone().unwrap_or_default(),
            summary: record.summary.clone(),
            description: record.description.clone(),
            location: record.location.clone(),
            start_time: record.start_time,
            end_time: record.end_time,
            is_all_day: record.is_all_day,
            status: record.status.clone(),
            organizer_email: record.organizer_email.clone(),
            attendees_json: record.attendees_json.clone(),
            html_link: None,
            etag: None,
            ical_data: record.ical_data.clone(),
            uid: record.uid.clone(),
            title: None,
            timezone: None,
            recurrence_rule: record.recurrence_rule.clone(),
            organizer_name: None,
            rsvp_status: None,
            availability: None,
            visibility: None,
            recurrence_id: None,
        };
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let local_event_id = upsert_calendar_event_row(&tx, &row)?;
        tx.commit().map_err(|e| e.to_string())?;

        let attendee_rows: Vec<CalendarAttendeeWriteRow> = record
            .attendees
            .iter()
            .map(|att| CalendarAttendeeWriteRow {
                email: att.email.clone(),
                name: att.name.clone(),
                rsvp_status: att.rsvp_status.clone(),
                is_organizer: att.is_organizer,
            })
            .collect();
        replace_event_attendees(conn, &aid, &local_event_id, &attendee_rows)?;

        let reminder_rows: Vec<CalendarReminderWriteRow> = record
            .reminders
            .iter()
            .map(|rem| CalendarReminderWriteRow {
                minutes_before: rem.minutes_before,
                method: rem.method.clone(),
            })
            .collect();
        replace_event_reminders(conn, &aid, &local_event_id, &reminder_rows)?;
        Ok(())
    })
    .await
}

async fn save_jmap_calendar_sync_state(
    db: &WriteDbState,
    account_id: &str,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let state_type = state_type.to_string();
    let state = state.to_string();
    db.with_write(move |conn| {
        conn.execute(
            "INSERT INTO jmap_sync_state (account_id, shared_account_id, type, state, updated_at) \
             VALUES (?1, NULL, ?2, ?3, strftime('%s', 'now')) \
             ON CONFLICT(account_id, COALESCE(shared_account_id, ''), type) \
             DO UPDATE SET state = ?3, updated_at = strftime('%s', 'now')",
            params![account_id, state_type, state],
        )
        .map_err(|e| format!("save jmap calendar sync state: {e}"))?;
        Ok(())
    })
    .await
}

async fn load_jmap_calendar_sync_state(
    db: &ReadDbState,
    account_id: &str,
    state_type: &str,
) -> Result<Option<String>, String> {
    let account_id = account_id.to_string();
    let state_type = state_type.to_string();
    db.with_read(move |conn| {
        match conn.query_row(
            "SELECT state FROM jmap_sync_state \
             WHERE account_id = ?1 AND type = ?2 \
             AND COALESCE(shared_account_id, '') = ''",
            params![account_id, state_type],
            |row| row.get::<_, String>("state"),
        ) {
            Ok(state) => Ok(Some(state)),
            Err(db::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(format!("load jmap calendar sync state: {e}")),
        }
    })
    .await
}
