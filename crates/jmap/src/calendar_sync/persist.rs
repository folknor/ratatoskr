use std::collections::HashMap;

use bifrost_jmap::Get;
use bifrost_jmap::calendar_event::CalendarEvent;

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
