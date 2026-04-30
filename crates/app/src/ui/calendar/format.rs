use chrono::Weekday;

use super::types::CalendarEventData;

/// Format time range like "10:00 - 11:30" or "All day".
pub(super) fn format_event_time_range(event: &CalendarEventData) -> String {
    if event.all_day {
        return "All day".to_string();
    }
    format!(
        "{:02}:{:02} \u{2013} {:02}:{:02}",
        event.start_hour_u32(),
        event.start_minute_u32(),
        event.end_hour_u32(),
        event.end_minute_u32(),
    )
}

pub(super) fn weekday_short(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

pub(super) fn month_short(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    }
}

/// Format a recurrence rule for display.
pub(super) fn format_recurrence_rule(rrule: &str) -> String {
    let rule = rrule.strip_prefix("RRULE:").unwrap_or(rrule);
    let mut freq = "";
    let mut interval = 1u32;
    for part in rule.split(';') {
        if let Some(val) = part.strip_prefix("FREQ=") {
            freq = val;
        }
        if let Some(val) = part.strip_prefix("INTERVAL=") {
            interval = val.parse().unwrap_or(1);
        }
    }
    let freq_label = match freq {
        "DAILY" => "day",
        "WEEKLY" => "week",
        "MONTHLY" => "month",
        "YEARLY" => "year",
        _ => return rule.to_string(),
    };
    if interval <= 1 {
        format!("Every {freq_label}")
    } else {
        format!("Every {interval} {freq_label}s")
    }
}

/// Format a reminder as human-readable text.
pub(super) fn format_reminder(minutes_before: i64) -> String {
    if minutes_before <= 0 {
        "At time of event".to_string()
    } else if minutes_before < 60 {
        format!("{minutes_before} min before")
    } else if minutes_before < 1440 {
        let hours = minutes_before / 60;
        if hours == 1 {
            "1 hour before".to_string()
        } else {
            format!("{hours} hours before")
        }
    } else {
        let days = minutes_before / 1440;
        if days == 1 {
            "1 day before".to_string()
        } else {
            format!("{days} days before")
        }
    }
}

/// Parse a hex color string (e.g. "#4285f4") to an iced Color.
pub(super) fn parse_hex_color(hex: &str) -> iced::Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(100);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(100);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(100);
        iced::Color::from_rgb8(r, g, b)
    } else {
        iced::Color::from_rgb8(100, 100, 200)
    }
}
