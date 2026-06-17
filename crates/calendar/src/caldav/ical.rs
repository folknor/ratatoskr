use super::super::types::CalendarEventDto;

pub(super) fn parse_caldav_event_input(
    value: &serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    let mut input = value
        .as_object()
        .cloned()
        .ok_or_else(|| "invalid CalDAV event payload".to_string())?;
    normalize_event_time_field(&mut input, "start", "startTime")?;
    normalize_event_time_field(&mut input, "end", "endTime")?;
    if !input.contains_key("isAllDay")
        && let Some(value) = input.get("is_all_day").cloned()
    {
        input.insert("isAllDay".to_string(), value);
    }
    Ok(input)
}

fn normalize_event_time_field(
    input: &mut serde_json::Map<String, serde_json::Value>,
    generic_key: &str,
    caldav_key: &str,
) -> Result<(), String> {
    if input.contains_key(caldav_key) {
        return Ok(());
    }
    let Some(value) = input.get(generic_key) else {
        return Ok(());
    };
    let normalized = match value {
        serde_json::Value::String(value) => Some(value.clone()),
        serde_json::Value::Number(value) => {
            let timestamp = value
                .as_i64()
                .ok_or_else(|| format!("{generic_key} must be an integer timestamp"))?;
            chrono::DateTime::<chrono::Utc>::from_timestamp(timestamp, 0)
                .map(|value| value.to_rfc3339_opts(chrono::SecondsFormat::Secs, true))
        }
        serde_json::Value::Null => None,
        _ => {
            return Err(format!(
                "{generic_key} must be an integer timestamp or string"
            ));
        }
    };
    if let Some(normalized) = normalized {
        input.insert(
            caldav_key.to_string(),
            serde_json::Value::String(normalized),
        );
    }
    Ok(())
}

pub(super) fn merge_caldav_event_input(
    existing: &CalendarEventDto,
    updates: &serde_json::Map<String, serde_json::Value>,
) -> serde_json::Map<String, serde_json::Value> {
    let mut merged = serde_json::Map::new();
    merged.insert(
        "summary".to_string(),
        serde_json::Value::String(
            updates
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| existing.summary.clone().unwrap_or_default()),
        ),
    );
    if let Some(description) = updates
        .get("description")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .or_else(|| existing.description.clone())
    {
        merged.insert(
            "description".to_string(),
            serde_json::Value::String(description),
        );
    }
    if let Some(location) = updates
        .get("location")
        .and_then(serde_json::Value::as_str)
        .map(ToString::to_string)
        .or_else(|| existing.location.clone())
    {
        merged.insert("location".to_string(), serde_json::Value::String(location));
    }
    merged.insert(
        "startTime".to_string(),
        serde_json::Value::String(
            updates
                .get("startTime")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(existing.start_time, 0)
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_default()
                }),
        ),
    );
    merged.insert(
        "endTime".to_string(),
        serde_json::Value::String(
            updates
                .get("endTime")
                .and_then(serde_json::Value::as_str)
                .map(ToString::to_string)
                .unwrap_or_else(|| {
                    chrono::DateTime::<chrono::Utc>::from_timestamp(existing.end_time, 0)
                        .map(|value| value.to_rfc3339())
                        .unwrap_or_default()
                }),
        ),
    );
    merged.insert(
        "isAllDay".to_string(),
        serde_json::Value::Bool(
            updates
                .get("isAllDay")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(existing.is_all_day),
        ),
    );
    merged
}

pub(super) fn build_caldav_ical_event(
    input: &serde_json::Map<String, serde_json::Value>,
    uid: Option<&str>,
) -> String {
    let event_uid = uid
        .map(ToString::to_string)
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let now = chrono::Utc::now().format("%Y%m%dT%H%M%SZ").to_string();
    let mut lines = vec![
        "BEGIN:VCALENDAR".to_string(),
        "VERSION:2.0".to_string(),
        "PRODID:-//Ratatoskr//CalDAV Client//EN".to_string(),
        "BEGIN:VEVENT".to_string(),
        format!("UID:{event_uid}"),
        format!("DTSTAMP:{now}"),
    ];

    if let Some(summary) = input.get("summary").and_then(serde_json::Value::as_str) {
        lines.push(format!("SUMMARY:{}", escape_ical_text(summary)));
    }

    let start_time = input.get("startTime").and_then(serde_json::Value::as_str);
    let end_time = input.get("endTime").and_then(serde_json::Value::as_str);
    let is_all_day = input
        .get("isAllDay")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if let (Some(start), Some(end)) = (start_time, end_time) {
        if is_all_day {
            lines.push(format!("DTSTART;VALUE=DATE:{}", format_ical_date(start)));
            lines.push(format!("DTEND;VALUE=DATE:{}", format_ical_date(end)));
        } else {
            lines.push(format!("DTSTART:{}", format_ical_datetime(start)));
            lines.push(format!("DTEND:{}", format_ical_datetime(end)));
        }
    }

    if let Some(description) = input.get("description").and_then(serde_json::Value::as_str) {
        lines.push(format!("DESCRIPTION:{}", escape_ical_text(description)));
    }
    if let Some(location) = input.get("location").and_then(serde_json::Value::as_str) {
        lines.push(format!("LOCATION:{}", escape_ical_text(location)));
    }
    if let Some(attendees) = input.get("attendees").and_then(serde_json::Value::as_array) {
        for attendee in attendees {
            if let Some(email) = attendee.get("email").and_then(serde_json::Value::as_str) {
                lines.push(format!("ATTENDEE;RSVP=TRUE:mailto:{email}"));
            }
        }
    }

    lines.push("END:VEVENT".to_string());
    lines.push("END:VCALENDAR".to_string());
    lines.join("\r\n")
}

// ---------------------------------------------------------------------------
// iCal serialization helpers (write side)
// ---------------------------------------------------------------------------

fn escape_ical_text(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace(';', "\\;")
        .replace(',', "\\,")
        .replace('\n', "\\n")
}

fn format_ical_datetime(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| {
            date.with_timezone(&chrono::Utc)
                .format("%Y%m%dT%H%M%SZ")
                .to_string()
        })
        .unwrap_or_else(|_| value.to_string())
}

fn format_ical_date(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| date.format("%Y%m%d").to_string())
        .or_else(|_| {
            chrono::NaiveDate::parse_from_str(value, "%Y-%m-%d")
                .map(|date| date.format("%Y%m%d").to_string())
        })
        .unwrap_or_else(|_| value.replace('-', ""))
}

#[cfg(test)]
mod tests {
    use super::{build_caldav_ical_event, parse_caldav_event_input};

    #[test]
    fn parse_input_accepts_generic_unix_times() {
        let input = parse_caldav_event_input(&serde_json::json!({
            "summary": "Standup moved",
            "start": 1768471200,
            "end": 1768473900,
            "isAllDay": false,
        }))
        .expect("parse");

        assert_eq!(
            input.get("startTime").and_then(serde_json::Value::as_str),
            Some("2026-01-15T10:00:00Z")
        );
        assert_eq!(
            input.get("endTime").and_then(serde_json::Value::as_str),
            Some("2026-01-15T10:45:00Z")
        );
        let ical = build_caldav_ical_event(&input, Some("ev-001"));
        assert!(ical.contains("DTSTART:20260115T100000Z"));
        assert!(ical.contains("DTEND:20260115T104500Z"));
    }
}
