use super::super::types::CalendarEventDto;

pub(super) fn parse_caldav_event_input(
    value: &serde_json::Value,
) -> Result<serde_json::Map<String, serde_json::Value>, String> {
    value
        .as_object()
        .cloned()
        .ok_or_else(|| "invalid CalDAV event payload".to_string())
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
