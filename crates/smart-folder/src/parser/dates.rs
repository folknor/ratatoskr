use jiff::civil;
use jiff::tz::TimeZone;
use jiff::{Span, Zoned};

/// Build a civil date from the parser's `i32`/`u32` field widths. jiff's civil
/// types are `i16`/`i8`, and out-of-range values are exactly the "reject this
/// input" case every caller already handles.
fn civil_date(year: i32, month: u32, day: u32) -> Option<civil::Date> {
    civil::Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month).ok()?,
        i8::try_from(day).ok()?,
    )
    .ok()
}

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
    if days > 0 {
        // Positive numbers are not valid relative offsets.
        return None;
    }
    let today = Zoned::now().date();
    let target = today.checked_add(Span::new().days(days)).ok()?;
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
            let date = civil_date(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        2 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let date = civil_date(year, month, 1)?;
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
            let date = civil_date(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        2 => {
            let year: i32 = parts[0].parse().ok()?;
            let month: u32 = parts[1].parse().ok()?;
            let date = civil_date(year, month, 1)?;
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
            let date = civil_date(year, 1, 1)?;
            naive_date_to_timestamp(date)
        }
        6 => {
            let year: i32 = s[..4].parse().ok()?;
            let month: u32 = s[4..6].parse().ok()?;
            let date = civil_date(year, month, 1)?;
            naive_date_to_timestamp(date)
        }
        8 => {
            let year: i32 = s[..4].parse().ok()?;
            let month: u32 = s[4..6].parse().ok()?;
            let day: u32 = s[6..8].parse().ok()?;
            let date = civil_date(year, month, day)?;
            naive_date_to_timestamp(date)
        }
        _ => None,
    }
}

/// Convert a civil date to a Unix timestamp at start of day in local time.
///
/// Uses jiff's compatible disambiguation rather than chrono's `.single()?`.
/// That is a deliberate behaviour change: a handful of zones move their clocks
/// AT midnight (Chile, Cuba, Iran, Lebanon, and Brazil historically), so on
/// those dates `single()` returned `None` and the entire date filter silently
/// matched nothing. Resolving past the gap gives the user the filter they
/// asked for, and matches how the calendar path resolves the same situation.
pub(super) fn naive_date_to_timestamp(date: civil::Date) -> Option<i64> {
    TimeZone::system()
        .to_zoned(date.at(0, 0, 0, 0))
        .ok()
        .map(|zoned| zoned.timestamp().as_second())
}
