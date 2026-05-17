use std::collections::HashMap;

use bifrost_jmap::Get;
use bifrost_jmap::calendar_event::CalendarEvent;

use db::db::ReadDbState;
use db::db::queries_extra::{
    CalendarAttendeeWriteRow, CalendarReminderWriteRow, UpsertCalendarEventParams,
    delete_event_by_account_remote_id_sync, replace_event_attendees_sync,
    replace_event_reminders_sync, upsert_calendar_event_sync, upsert_calendar_sync,
};

use super::payload::{
    extract_attendee_rows, extract_attendees_json, extract_location, extract_organizer_email,
    extract_reminder_rows, parse_jscalendar_times, resolve_calendar_id,
};

#[derive(Debug, Clone)]
pub struct JmapCalendarAttendeeRecord {
    pub email: String,
    pub name: Option<String>,
    pub rsvp_status: Option<String>,
    pub is_organizer: bool,
}

#[derive(Debug, Clone)]
pub struct JmapCalendarReminderRecord {
    pub minutes_before: i64,
    pub method: Option<String>,
}

#[derive(Debug, Clone)]
pub struct JmapCalendarEventRecord {
    pub google_event_id: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_time: i64,
    pub end_time: i64,
    pub is_all_day: bool,
    pub status: String,
    pub organizer_email: Option<String>,
    pub attendees_json: Option<String>,
    pub calendar_id: Option<String>,
    pub remote_event_id: String,
    pub ical_data: Option<String>,
    pub uid: Option<String>,
    pub recurrence_rule: Option<String>,
    pub attendees: Vec<JmapCalendarAttendeeRecord>,
    pub reminders: Vec<JmapCalendarReminderRecord>,
}

/// Persist a JMAP CalendarEvent into the local database.
///
/// Extracts JSCalendar properties and maps them to the DB schema.
pub(super) async fn persist_jmap_event(
    db: &ReadDbState,
    account_id: &str,
    event: &CalendarEvent<Get>,
    cal_map: &HashMap<&str, &str>,
) -> Result<(), String> {
    let Some(record) = jmap_event_record(event, cal_map) else {
        return Ok(());
    };

    persist_jmap_event_record(db, account_id, record).await
}

pub(super) async fn persist_jmap_event_record(
    db: &ReadDbState,
    account_id: &str,
    record: JmapCalendarEventRecord,
) -> Result<(), String> {
    let aid = account_id.to_string();

    db.with_conn(move |conn| {
        let local_event_id = upsert_calendar_event_sync(
            conn,
            &UpsertCalendarEventParams {
                account_id: aid.clone(),
                google_event_id: record.google_event_id.clone(),
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
                calendar_id: record.calendar_id.clone(),
                remote_event_id: Some(record.remote_event_id.clone()),
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
                // JMAP's CalendarEvent type carries `recurrenceOverrides` as
                // a JSCalendar map keyed by RECURRENCE-ID; the sync path
                // currently flattens those into the master row. Leave None
                // until the override-as-row path lands here too.
                recurrence_id: None,
            },
        )?;

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
        replace_event_attendees_sync(conn, &aid, &local_event_id, &attendee_rows)?;

        let reminder_rows: Vec<CalendarReminderWriteRow> = record
            .reminders
            .iter()
            .map(|rem| CalendarReminderWriteRow {
                minutes_before: rem.minutes_before,
                method: rem.method.clone(),
            })
            .collect();
        replace_event_reminders_sync(conn, &aid, &local_event_id, &reminder_rows)?;
        Ok(())
    })
    .await
}

pub fn jmap_event_record(
    event: &CalendarEvent<Get>,
    cal_map: &HashMap<&str, &str>,
) -> Option<JmapCalendarEventRecord> {
    let event_id = event.id()?;
    let ical_data = event
        .recurrence_rules()
        .and_then(|rules| serde_json::to_string(rules).ok());
    let recurrence_rule = event
        .recurrence_rules()
        .and_then(|rules| serde_json::to_value(rules).ok())
        .and_then(|value| jmap_recurrence_rules_to_rrule(&value));
    let (start_time, end_time, is_all_day) = parse_jscalendar_times(event);
    let attendees = extract_attendee_rows(event)
        .into_iter()
        .map(|att| JmapCalendarAttendeeRecord {
            email: att.email,
            name: att.name,
            rsvp_status: att.rsvp_status,
            is_organizer: att.is_organizer,
        })
        .collect();
    let reminders = extract_reminder_rows(event)
        .into_iter()
        .map(|rem| JmapCalendarReminderRecord {
            minutes_before: rem.minutes_before,
            method: rem.method,
        })
        .collect();

    Some(JmapCalendarEventRecord {
        google_event_id: event_id.to_string(),
        summary: event.title().map(String::from),
        description: event.description().map(String::from),
        location: extract_location(event),
        start_time,
        end_time,
        is_all_day,
        status: event.status().unwrap_or("confirmed").to_string(),
        organizer_email: extract_organizer_email(event),
        attendees_json: extract_attendees_json(event),
        calendar_id: resolve_calendar_id(event, cal_map),
        remote_event_id: event_id.to_string(),
        ical_data,
        uid: event.uid().map(String::from),
        recurrence_rule,
        attendees,
        reminders,
    })
}

fn jmap_recurrence_rules_to_rrule(rules: &serde_json::Value) -> Option<String> {
    let rule = rules.as_array()?.first()?.as_object()?;
    let frequency = match rule.get("frequency")?.as_str()?.to_ascii_lowercase().as_str() {
        "daily" => "DAILY",
        "weekly" => "WEEKLY",
        "monthly" => "MONTHLY",
        "yearly" => "YEARLY",
        _ => return None,
    };
    let mut parts = vec![format!("FREQ={frequency}")];

    if let Some(interval) = rule
        .get("interval")
        .and_then(serde_json::Value::as_u64)
        .filter(|interval| *interval > 1)
    {
        parts.push(format!("INTERVAL={interval}"));
    }
    if let Some(days) = rule.get("byDay").and_then(serde_json::Value::as_array) {
        let by_day: Vec<String> = days
            .iter()
            .filter_map(jmap_by_day_to_rrule)
            .collect();
        if !by_day.is_empty() {
            parts.push(format!("BYDAY={}", by_day.join(",")));
        }
    }
    if let Some(days) = rule.get("byMonthDay").and_then(serde_json::Value::as_array) {
        let by_month_day: Vec<String> = days
            .iter()
            .filter_map(serde_json::Value::as_i64)
            .map(|day| day.to_string())
            .collect();
        if !by_month_day.is_empty() {
            parts.push(format!("BYMONTHDAY={}", by_month_day.join(",")));
        }
    }
    if let Some(months) = rule.get("byMonth").and_then(serde_json::Value::as_array) {
        // RFC 8984 byMonth is an array of strings ("1".."12", optionally
        // suffixed with "L" for leap months on non-Gregorian calendars).
        // Real JMAP servers and the saehrimnir mock both emit the string
        // form; the numeric branch is a tolerance for older mocks.
        let by_month: Vec<String> = months
            .iter()
            .filter_map(|value| {
                value
                    .as_u64()
                    .map(|month| month.to_string())
                    .or_else(|| {
                        value
                            .as_str()
                            .and_then(|s| s.trim_end_matches('L').parse::<u32>().ok())
                            .map(|month| month.to_string())
                    })
            })
            .collect();
        if !by_month.is_empty() {
            parts.push(format!("BYMONTH={}", by_month.join(",")));
        }
    }
    if let Some(count) = rule.get("count").and_then(serde_json::Value::as_u64) {
        parts.push(format!("COUNT={count}"));
    } else if let Some(until) = rule
        .get("until")
        .and_then(serde_json::Value::as_str)
        .and_then(jmap_until_to_rrule)
    {
        parts.push(format!("UNTIL={until}"));
    }

    Some(format!("RRULE:{}", parts.join(";")))
}

fn jmap_by_day_to_rrule(day: &serde_json::Value) -> Option<String> {
    let obj = day.as_object()?;
    let weekday = match obj.get("day")?.as_str()?.to_ascii_lowercase().as_str() {
        "su" => "SU",
        "mo" => "MO",
        "tu" => "TU",
        "we" => "WE",
        "th" => "TH",
        "fr" => "FR",
        "sa" => "SA",
        _ => return None,
    };
    let ordinal = obj
        .get("nthOfPeriod")
        .and_then(serde_json::Value::as_i64)
        .map(|nth| nth.to_string())
        .unwrap_or_default();
    Some(format!("{ordinal}{weekday}"))
}

fn jmap_until_to_rrule(until: &str) -> Option<String> {
    chrono::NaiveDateTime::parse_from_str(until.trim_end_matches('Z'), "%Y-%m-%dT%H:%M:%S")
        .ok()
        .map(|dt| format!("{}Z", dt.format("%Y%m%dT%H%M%S")))
        .or_else(|| {
            chrono::NaiveDate::parse_from_str(until, "%Y-%m-%d")
                .ok()
                .and_then(|date| date.and_hms_opt(23, 59, 59))
                .map(|dt| format!("{}Z", dt.format("%Y%m%dT%H%M%S")))
        })
}

/// Delete a calendar event by its JMAP event ID.
pub(super) async fn delete_event_by_jmap_id(
    db: &ReadDbState,
    account_id: &str,
    jmap_event_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let eid = jmap_event_id.to_string();

    db.with_conn(move |conn| delete_event_by_account_remote_id_sync(conn, &aid, &eid))
        .await
}

// ── Sync state persistence ─────────────────────────────────

/// Save a JMAP sync state for calendar objects.
///
/// Reuses the existing `jmap_sync_state` table via sync.
pub(super) async fn save_calendar_sync_state(
    db: &ReadDbState,
    account_id: &str,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    sync::state::save_jmap_sync_state(db, account_id, state_type, state).await
}

/// Load a JMAP sync state for calendar objects.
pub(super) async fn load_calendar_sync_state(
    db: &ReadDbState,
    account_id: &str,
    state_type: &str,
) -> Result<Option<String>, String> {
    sync::state::load_jmap_sync_state(db, account_id, state_type).await
}

// ── Calendar DB helpers ────────────────────────────────────

/// Upsert a calendar entry. Returns the local UUID.
pub(super) async fn upsert_calendar(
    db: &ReadDbState,
    account_id: &str,
    remote_id: &str,
    display_name: Option<&str>,
    color: Option<&str>,
    is_primary: bool,
) -> Result<String, String> {
    let aid = account_id.to_string();
    let rid = remote_id.to_string();
    let dname = display_name.map(String::from);
    let col = color.map(String::from);

    db.with_conn(move |conn| {
        // JMAP calendars discovered via Calendar/get are owned by the
        // authenticated user; sharing/permissions land via JMAP Sharing
        // (already wired for mailboxes) and would override this in a future
        // pass.
        upsert_calendar_sync(
            conn,
            &aid,
            "jmap",
            &rid,
            dname.as_deref(),
            col.as_deref(),
            is_primary,
            true,
        )
    })
    .await
}
