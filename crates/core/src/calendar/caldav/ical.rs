use super::super::types::CalendarEventDto;

/// Parse raw iCalendar data into a `CalendarEventDto`.
pub(super) fn parse_caldav_ical_event(
    ical_data: &str,
    href: &str,
) -> Result<CalendarEventDto, String> {
    let lines = unfold_ical_lines(ical_data);
    let mut uid = None;
    let mut summary = None;
    let mut description = None;
    let mut location = None;
    let mut dtstart: Option<String> = None;
    let mut dtstart_tzid: Option<String> = None;
    let mut dtend: Option<String> = None;
    let mut dtend_tzid: Option<String> = None;
    let mut status = "confirmed".to_string();
    let mut organizer_email = None;
    let mut is_all_day = false;
    let mut attendees = Vec::<serde_json::Value>::new();

    for line in lines {
        let mut parts = line.splitn(2, ':');
        let Some(name_with_params) = parts.next() else {
            continue;
        };
        let value = parts.next().unwrap_or_default();
        let mut name_parts = name_with_params.split(';');
        let prop_name = name_parts.next().unwrap_or_default().to_uppercase();
        let params = name_parts.collect::<Vec<_>>().join(";").to_uppercase();

        match prop_name.as_str() {
            "UID" => uid = Some(value.to_string()),
            "SUMMARY" => summary = Some(unescape_ical_text(value)),
            "DESCRIPTION" => description = Some(unescape_ical_text(value)),
            "LOCATION" => location = Some(unescape_ical_text(value)),
            "DTSTART" => {
                dtstart = Some(value.to_string());
                dtstart_tzid = extract_param_value(name_with_params, "TZID");
                if params.contains("VALUE=DATE") && !params.contains("VALUE=DATE-TIME") {
                    is_all_day = true;
                }
            }
            "DTEND" => {
                dtend = Some(value.to_string());
                dtend_tzid = extract_param_value(name_with_params, "TZID");
            }
            "STATUS" => status = value.to_lowercase(),
            "ORGANIZER" => {
                if let Some(email) = value
                    .strip_prefix("mailto:")
                    .or_else(|| value.strip_prefix("MAILTO:"))
                {
                    organizer_email = Some(email.to_string());
                }
            }
            "ATTENDEE" => {
                if let Some(email) = value
                    .strip_prefix("mailto:")
                    .or_else(|| value.strip_prefix("MAILTO:"))
                {
                    let display_name = extract_param_value(name_with_params, "CN");
                    let response_status = extract_param_value(name_with_params, "PARTSTAT")
                        .map(|value| value.to_lowercase());
                    attendees.push(serde_json::json!({
                        "email": email,
                        "displayName": display_name,
                        "responseStatus": response_status,
                    }));
                }
            }
            _ => {}
        }
    }

    let _ = dtstart_tzid;
    let _ = dtend_tzid;

    let start_time = dtstart
        .as_deref()
        .map(|value| parse_ical_datetime(value, is_all_day, dtstart_tzid.as_deref()))
        .transpose()?
        .unwrap_or(0);
    let end_time = dtend
        .as_deref()
        .map(|value| parse_ical_datetime(value, is_all_day, dtend_tzid.as_deref()))
        .transpose()?
        .unwrap_or(start_time + 3600);

    Ok(CalendarEventDto {
        remote_event_id: href.to_string(),
        uid,
        etag: None,
        summary,
        description,
        location,
        start_time,
        end_time,
        is_all_day,
        status,
        organizer_email,
        attendees_json: if attendees.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&attendees)
                    .map_err(|e| format!("serialize CalDAV attendees: {e}"))?,
            )
        },
        html_link: None,
        ical_data: Some(ical_data.to_string()),
    })
}

pub(super) fn parse_caldav_event_input(
    value: serde_json::Value,
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
// iCal helpers
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

fn unfold_ical_lines(ical_data: &str) -> Vec<String> {
    ical_data
        .replace("\r\n ", "")
        .replace("\r\n\t", "")
        .replace("\n ", "")
        .replace("\n\t", "")
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

fn unescape_ical_text(value: &str) -> String {
    value
        .replace("\\n", "\n")
        .replace("\\N", "\n")
        .replace("\\,", ",")
        .replace("\\;", ";")
        .replace("\\\\", "\\")
}

fn extract_param_value(name_with_params: &str, key: &str) -> Option<String> {
    for param in name_with_params.split(';').skip(1) {
        let mut parts = param.splitn(2, '=');
        let param_name = parts.next()?.trim();
        let param_value = parts.next()?.trim();
        if param_name.eq_ignore_ascii_case(key) {
            return Some(param_value.trim_matches('"').to_string());
        }
    }
    None
}

fn parse_ical_datetime(value: &str, is_all_day: bool, _tzid: Option<&str>) -> Result<i64, String> {
    if is_all_day {
        let date = chrono::NaiveDate::parse_from_str(value, "%Y%m%d")
            .map_err(|e| format!("invalid all-day CalDAV date {value}: {e}"))?;
        return date
            .and_hms_opt(0, 0, 0)
            .ok_or_else(|| "invalid all-day CalDAV time".to_string())
            .map(|date_time| date_time.and_utc().timestamp());
    }

    if let Some(cleaned) = value.strip_suffix('Z') {
        return chrono::NaiveDateTime::parse_from_str(cleaned, "%Y%m%dT%H%M%S")
            .map_err(|e| format!("invalid UTC CalDAV datetime {value}: {e}"))
            .map(|date_time| date_time.and_utc().timestamp());
    }

    chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
        .map_err(|e| format!("invalid CalDAV datetime {value}: {e}"))
        .map(|dt| {
            dt.and_local_timezone(chrono::Local)
                .single()
                .map(|dt| dt.timestamp())
                .unwrap_or_else(|| dt.and_utc().timestamp())
        })
}
