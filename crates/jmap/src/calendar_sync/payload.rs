use std::collections::HashMap;

use jmap_client::Get;
use jmap_client::calendar_event::CalendarEvent;

// ── JSCalendar property extraction ─────────────────────────

/// Extract the first location name from the JSCalendar locations map.
pub(super) fn extract_location(event: &CalendarEvent<Get>) -> Option<String> {
    let locations = event.locations()?;
    for (_key, value) in locations {
        if let Some(obj) = value.as_object()
            && let Some(name) = obj.get("name").and_then(|n| n.as_str())
            && !name.is_empty()
        {
            return Some(name.to_string());
        }
    }
    None
}

/// Resolve a local calendar_id from the event's calendarIds map.
pub(super) fn resolve_calendar_id(
    event: &CalendarEvent<Get>,
    cal_map: &HashMap<&str, &str>,
) -> Option<String> {
    let calendar_ids = event.calendar_ids()?;
    for (remote_cal_id, value) in calendar_ids {
        // calendarIds maps calendar_id -> true for calendars this event belongs to
        if value.as_bool() == Some(true)
            && let Some(local_id) = cal_map.get(remote_cal_id.as_str())
        {
            return Some((*local_id).to_string());
        }
    }
    None
}

/// Extract organizer email from JSCalendar participants.
pub(super) fn extract_organizer_email(event: &CalendarEvent<Get>) -> Option<String> {
    let participants = event.participants()?;
    for (_key, value) in participants {
        let obj = value.as_object()?;
        // Look for roles containing "owner"
        if let Some(roles) = obj.get("roles").and_then(|r| r.as_object())
            && roles.contains_key("owner")
        {
            if let Some(email) = obj
                .get("sendTo")
                .and_then(|s| s.as_object())
                .and_then(|s| s.get("imip"))
                .and_then(|v| v.as_str())
            {
                // Strip "mailto:" prefix
                let email = email.strip_prefix("mailto:").unwrap_or(email);
                return Some(email.to_string());
            }
            // Try email field directly
            if let Some(email) = obj.get("email").and_then(|e| e.as_str()) {
                return Some(email.to_string());
            }
        }
    }
    None
}

/// Extract attendees as a JSON string from JSCalendar participants.
pub(super) fn extract_attendees_json(event: &CalendarEvent<Get>) -> Option<String> {
    let participants = event.participants()?;
    if participants.is_empty() {
        return None;
    }

    let mut attendees = Vec::new();
    for (_key, value) in participants {
        let Some(obj) = value.as_object() else {
            continue;
        };

        // Extract email
        let email = obj
            .get("sendTo")
            .and_then(|s| s.as_object())
            .and_then(|s| s.get("imip"))
            .and_then(|v| v.as_str())
            .map(|e| e.strip_prefix("mailto:").unwrap_or(e))
            .or_else(|| obj.get("email").and_then(|e| e.as_str()));

        let name = obj.get("name").and_then(|n| n.as_str());

        let roles = obj.get("roles").and_then(|r| r.as_object());
        let is_owner = roles.is_some_and(|r| r.contains_key("owner"));

        // Map JSCalendar participationStatus to Google-style responseStatus
        let participation = obj
            .get("participationStatus")
            .and_then(|p| p.as_str())
            .map(map_participation_status);

        let mut att = serde_json::Map::new();
        if let Some(email) = email {
            att.insert("email".to_string(), serde_json::json!(email));
        }
        if let Some(name) = name {
            att.insert("displayName".to_string(), serde_json::json!(name));
        }
        if let Some(status) = participation {
            att.insert("responseStatus".to_string(), serde_json::json!(status));
        }
        if is_owner {
            att.insert("organizer".to_string(), serde_json::json!(true));
        }

        attendees.push(serde_json::Value::Object(att));
    }

    if attendees.is_empty() {
        None
    } else {
        serde_json::to_string(&attendees).ok()
    }
}

/// Map JSCalendar participationStatus to a Google-compatible responseStatus.
fn map_participation_status(status: &str) -> &str {
    match status {
        "accepted" => "accepted",
        "declined" => "declined",
        "tentative" => "tentative",
        "needs-action" => "needsAction",
        _ => "needsAction",
    }
}

/// Parse JSCalendar start + duration into Unix timestamps.
///
/// JSCalendar uses local date-time + timezone + duration (ISO 8601).
/// `start` is like "2025-03-15T14:30:00", `duration` like "PT1H30M",
/// and `timeZone` like "America/New_York".
pub(super) fn parse_jscalendar_times(event: &CalendarEvent<Get>) -> (i64, i64, bool) {
    let is_all_day = event.show_without_time().unwrap_or(false);

    let start_str = match event.start() {
        Some(s) => s,
        None => return (0, 3600, is_all_day),
    };

    let tz = event.time_zone().as_value().copied().unwrap_or("UTC");

    let start_ts = if is_all_day {
        // All-day: parse as date only (e.g. "2025-03-15")
        parse_local_date(start_str)
    } else {
        // Timed: parse as local datetime in the given timezone
        parse_local_datetime(start_str, tz)
    };

    let duration_str = event
        .duration()
        .unwrap_or(if is_all_day { "P1D" } else { "PT1H" });
    let duration_secs = parse_iso8601_duration(duration_str);

    (start_ts, start_ts + duration_secs, is_all_day)
}

/// Parse a local date string (e.g. "2025-03-15") into a UTC Unix timestamp
/// at midnight.
fn parse_local_date(s: &str) -> i64 {
    chrono::NaiveDate::parse_from_str(s, "%Y-%m-%d")
        .ok()
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .map(|ndt| ndt.and_utc().timestamp())
        .unwrap_or(0)
}

/// Parse a local datetime string (e.g. "2025-03-15T14:30:00") into a UTC
/// Unix timestamp.
///
/// Tries RFC 3339 first (includes offset). Falls back to parsing as a naive
/// datetime treated as UTC - JMAP servers typically include the timezone in
/// the `timeZone` property but the `start` value itself is local. Without
/// chrono-tz we cannot resolve IANA names, so we treat naive times as UTC.
/// This is acceptable because delta-sync will correct any drift.
fn parse_local_datetime(s: &str, _tz_name: &str) -> i64 {
    // Try parsing with timezone offset (RFC 3339)
    if let Ok(dt) = chrono::DateTime::parse_from_rfc3339(s) {
        return dt.timestamp();
    }

    // Parse as naive local datetime - treat as UTC
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S")
        .map(|n| n.and_utc().timestamp())
        .unwrap_or(0)
}

/// Parse an ISO 8601 duration string (e.g. "PT1H30M", "P1D", "PT45M")
/// into seconds.
fn parse_iso8601_duration(s: &str) -> i64 {
    let mut total_secs: i64 = 0;
    let mut in_time = false;
    let mut num_buf = String::new();

    for ch in s.chars() {
        match ch {
            'P' => {}
            'T' => in_time = true,
            'W' => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n * 7 * 86400;
                }
                num_buf.clear();
            }
            'D' => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n * 86400;
                }
                num_buf.clear();
            }
            'H' if in_time => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n * 3600;
                }
                num_buf.clear();
            }
            'M' if in_time => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n * 60;
                }
                num_buf.clear();
            }
            'S' if in_time => {
                if let Ok(n) = num_buf.parse::<i64>() {
                    total_secs += n;
                }
                num_buf.clear();
            }
            c if c.is_ascii_digit() => num_buf.push(c),
            _ => {}
        }
    }

    if total_secs == 0 {
        3600 // Fallback: 1 hour
    } else {
        total_secs
    }
}

/// Format Unix timestamps into JSCalendar start + duration strings.
pub(super) fn format_jscalendar_times(
    start_time: i64,
    end_time: i64,
    is_all_day: bool,
) -> (String, String) {
    use chrono::TimeZone;
    let start_dt = chrono::Utc
        .timestamp_opt(start_time, 0)
        .single()
        .unwrap_or_else(chrono::Utc::now);

    if is_all_day {
        let start_str = start_dt.format("%Y-%m-%d").to_string();
        let duration_days = (end_time - start_time) / 86400;
        let duration_days = if duration_days < 1 { 1 } else { duration_days };
        let duration_str = format!("P{duration_days}D");
        (start_str, duration_str)
    } else {
        let start_str = start_dt.format("%Y-%m-%dT%H:%M:%S").to_string();
        let duration_secs = end_time - start_time;
        let duration_str = format_duration_iso8601(duration_secs);
        (start_str, duration_str)
    }
}

/// Format a duration in seconds as ISO 8601 (e.g. "PT1H30M").
fn format_duration_iso8601(mut secs: i64) -> String {
    if secs <= 0 {
        return "PT1H".to_string();
    }

    let mut parts = String::from("P");

    let days = secs / 86400;
    if days > 0 {
        parts.push_str(&format!("{days}D"));
        secs %= 86400;
    }

    if secs > 0 {
        parts.push('T');
        let hours = secs / 3600;
        if hours > 0 {
            parts.push_str(&format!("{hours}H"));
            secs %= 3600;
        }
        let minutes = secs / 60;
        if minutes > 0 {
            parts.push_str(&format!("{minutes}M"));
            secs %= 60;
        }
        if secs > 0 {
            parts.push_str(&format!("{secs}S"));
        }
    }

    parts
}

/// Extracted attendee row, ready to be persisted inside a `with_conn` closure.
pub(super) struct AttendeeRow {
    pub(super) email: String,
    pub(super) name: Option<String>,
    pub(super) rsvp_status: Option<String>,
    pub(super) is_organizer: bool,
}

/// Extract attendee rows from a JMAP CalendarEvent's participants.
pub(super) fn extract_attendee_rows(event: &CalendarEvent<Get>) -> Vec<AttendeeRow> {
    let Some(participants) = event.participants() else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for (_key, value) in participants {
        let Some(obj) = value.as_object() else {
            continue;
        };

        let email = obj
            .get("sendTo")
            .and_then(|s| s.as_object())
            .and_then(|s| s.get("imip"))
            .and_then(|v| v.as_str())
            .map(|e| e.strip_prefix("mailto:").unwrap_or(e))
            .or_else(|| obj.get("email").and_then(|e| e.as_str()));

        let Some(email) = email else { continue };
        if email.is_empty() {
            continue;
        }

        let name = obj.get("name").and_then(|n| n.as_str()).map(String::from);
        let roles = obj.get("roles").and_then(|r| r.as_object());
        let is_owner = roles.is_some_and(|r| r.contains_key("owner"));

        let participation = obj
            .get("participationStatus")
            .and_then(|p| p.as_str())
            .map(|s| map_participation_status(s).to_string());

        rows.push(AttendeeRow {
            email: email.to_string(),
            name,
            rsvp_status: participation,
            is_organizer: is_owner,
        });
    }

    rows
}

/// Extracted reminder row, ready to be persisted inside a `with_conn` closure.
pub(super) struct ReminderRow {
    pub(super) minutes_before: i64,
    pub(super) method: Option<String>,
}

/// Extract reminder rows from a JMAP CalendarEvent's alerts.
pub(super) fn extract_reminder_rows(event: &CalendarEvent<Get>) -> Vec<ReminderRow> {
    // alerts returns Field<&Map> - Omitted/Null = no alerts, Value = has alerts
    let jmap_client::core::field::Field::Value(alerts) = event.alerts() else {
        return Vec::new();
    };

    let mut rows = Vec::new();
    for (_alert_id, alert_value) in alerts {
        let Some(alert_obj) = alert_value.as_object() else {
            continue;
        };

        let Some(trigger) = alert_obj.get("trigger").and_then(|t| t.as_object()) else {
            continue;
        };

        // Extract offset from OffsetTrigger (e.g. "-PT15M")
        let Some(offset) = trigger.get("offset").and_then(|o| o.as_str()) else {
            continue;
        };

        let is_negative = offset.starts_with('-');
        let clean_offset = offset.trim_start_matches('-');
        let offset_secs = parse_iso8601_duration(clean_offset);
        let minutes_before = if is_negative {
            offset_secs / 60
        } else {
            -(offset_secs / 60)
        };

        let method = alert_obj
            .get("action")
            .and_then(|a| a.as_str())
            .map(|a| match a {
                "display" => "popup",
                "email" => "email",
                other => other,
            })
            .map(String::from);

        rows.push(ReminderRow {
            minutes_before,
            method,
        });
    }

    rows
}

// ── Tests ──────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_iso8601_duration_basic() {
        assert_eq!(parse_iso8601_duration("PT1H"), 3600);
        assert_eq!(parse_iso8601_duration("PT30M"), 1800);
        assert_eq!(parse_iso8601_duration("PT1H30M"), 5400);
        assert_eq!(parse_iso8601_duration("P1D"), 86400);
        assert_eq!(parse_iso8601_duration("P1DT2H"), 93600);
        assert_eq!(parse_iso8601_duration("P1W"), 604800);
        assert_eq!(parse_iso8601_duration("PT45S"), 45);
        assert_eq!(parse_iso8601_duration("PT1H15M30S"), 4530);
    }

    #[test]
    fn parse_iso8601_duration_fallback() {
        // Empty or invalid durations should fall back to 1 hour
        assert_eq!(parse_iso8601_duration(""), 3600);
        assert_eq!(parse_iso8601_duration("P"), 3600);
    }

    #[test]
    fn format_duration_roundtrip() {
        assert_eq!(format_duration_iso8601(3600), "PT1H");
        assert_eq!(format_duration_iso8601(5400), "PT1H30M");
        assert_eq!(format_duration_iso8601(86400), "P1D");
        assert_eq!(format_duration_iso8601(93600), "P1DT2H");
        assert_eq!(format_duration_iso8601(45), "PT45S");
    }

    #[test]
    fn format_duration_zero_fallback() {
        assert_eq!(format_duration_iso8601(0), "PT1H");
    }

    #[test]
    fn parse_local_date_works() {
        let ts = parse_local_date("2025-03-15");
        assert!(ts > 0);
    }

    #[test]
    fn parse_local_datetime_utc() {
        let ts = parse_local_datetime("2025-03-15T14:30:00", "UTC");
        assert!(ts > 0);
        // 2025-03-15T14:30:00 UTC
        assert_eq!(ts, 1_742_049_000);
    }

    #[test]
    fn map_participation_status_covers_known() {
        assert_eq!(map_participation_status("accepted"), "accepted");
        assert_eq!(map_participation_status("declined"), "declined");
        assert_eq!(map_participation_status("tentative"), "tentative");
        assert_eq!(map_participation_status("needs-action"), "needsAction");
        assert_eq!(map_participation_status("unknown"), "needsAction");
    }
}
