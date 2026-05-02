use chrono::TimeZone as _;

// -- Date parsing --

/// Parse a date string into a Unix timestamp (seconds, start of day in local time).
///
/// Supported formats:
/// - Relative offsets: `-7` (7 days ago), `0` (today)
/// - Year only: `2025` -> January 1, 2025
/// - Year+month: `202603` -> March 1, 2026
/// - Full date: `20260311` -> March 11, 2026
/// - Slash-separated: `2026/03/11`
/// - Dash-separated: `2026-03-11`
/// - Space-separated: `2026 03 11` (from greedy consumption)
pub(super) fn parse_date_to_timestamp(date_str: &str) -> Option<i64> {
    let trimmed = date_str.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Relative offset: starts with `-` or is `0`.
    if trimmed == "0" || trimmed.starts_with('-') {
        return parse_relative_offset(trimmed);
    }

    // Contains separator -> split on it.
    if trimmed.contains('/') || trimmed.contains('-') {
        return parse_separated_date(trimmed);
    }

    // Contains spaces -> split on space (from greedy consumption).
    if trimmed.contains(' ') {
        return parse_space_separated_date(trimmed);
    }

    // Pure digits -> length determines interpretation.
    parse_compact_date(trimmed)
}

/// Parse a relative offset like `-7` or `0` into a timestamp.
fn parse_relative_offset(s: &str) -> Option<i64> {
    let days: i64 = s.parse().ok()?;
    let today = chrono::Local::now().date_naive();
    let target = if days <= 0 {
        today + chrono::Duration::days(days)
    } else {
        // Positive numbers are not valid relative offsets.
        return None;
    };
    naive_date_to_timestamp(target)
}

/// Parse a date with `/` or `-` separators.
fn parse_separated_date(s: &str) -> Option<i64> {
    let sep = if s.contains('/') { '/' } else { '-' };
    let parts: Vec<&str> = s.split(sep).collect();
    match parts.len() {
        3 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let day: u32 = parts[2].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        2 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
            naive_date_to_timestamp(date)
        }
        _ => None,
    }
}

/// Parse a space-separated date like `2026 03 11`.
fn parse_space_separated_date(s: &str) -> Option<i64> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    match parts.len() {
        3 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let day: u32 = parts[2].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        2 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
            naive_date_to_timestamp(date)
        }
        _ => None,
    }
}

/// Parse a compact digit-only date string.
fn parse_compact_date(s: &str) -> Option<i64> {
    if !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match s.len() {
        4 => {
            let year: i32 = s.parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, 1, 1)?;
            naive_date_to_timestamp(date)
        }
        6 => {
            let year: i32 = s[..4].parse().ok()?;
            let month: u32 = s[4..6].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, 1)?;
            naive_date_to_timestamp(date)
        }
        8 => {
            let year: i32 = s[..4].parse().ok()?;
            let month: u32 = s[4..6].parse().ok()?;
            let day: u32 = s[6..8].parse().ok()?;
            let date = chrono::NaiveDate::from_ymd_opt(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        _ => None,
    }
}

/// Convert a `NaiveDate` to a Unix timestamp at start of day in local time.
pub(super) fn naive_date_to_timestamp(date: chrono::NaiveDate) -> Option<i64> {
    let datetime = date.and_hms_opt(0, 0, 0)?;
    let local = chrono::Local.from_local_datetime(&datetime).single()?;
    Some(local.timestamp())
}
