use chrono::Local;

/// Resolve dynamic date tokens in a query string to `YYYY/MM/DD` format.
///
/// Supported tokens:
/// - `__LAST_7_DAYS__`  - date 7 days ago
/// - `__LAST_30_DAYS__` - date 30 days ago
/// - `__TODAY__`        - today's date
pub fn resolve_query_tokens(query: &str) -> String {
    let now = Local::now().date_naive();
    let mut result = query.to_owned();

    if result.contains("__LAST_7_DAYS__") {
        let d = now - chrono::Duration::days(7);
        result = result.replace("__LAST_7_DAYS__", &format_date(d));
    }

    if result.contains("__LAST_30_DAYS__") {
        let d = now - chrono::Duration::days(30);
        result = result.replace("__LAST_30_DAYS__", &format_date(d));
    }

    if result.contains("__TODAY__") {
        result = result.replace("__TODAY__", &format_date(now));
    }

    result
}

fn format_date(d: chrono::NaiveDate) -> String {
    d.format("%Y/%m/%d").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_today_token() {
        let result = resolve_query_tokens("after:__TODAY__");
        assert!(!result.contains("__TODAY__"));
        assert!(result.starts_with("after:"));
        // Should be YYYY/MM/DD format
        let date_part = &result["after:".len()..];
        assert_eq!(date_part.len(), 10);
    }

    #[test]
    fn leaves_plain_text_unchanged() {
        let input = "is:unread from:alice";
        assert_eq!(resolve_query_tokens(input), input);
    }
}
