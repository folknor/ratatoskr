/// Reusable SQL fragments to avoid duplication across query modules.
/// Subquery that picks the latest message per (account_id, thread_id),
/// returning `id`, `account_id`, `thread_id`, `from_name`, `from_address`.
///
/// Usage: embed inside a `LEFT JOIN (…) m ON m.account_id = t.account_id AND m.thread_id = t.id`.
pub const LATEST_MESSAGE_SUBQUERY: &str = "\
SELECT id, account_id, thread_id, from_name, from_address FROM (
  SELECT id, account_id, thread_id, from_name, from_address,
         ROW_NUMBER() OVER (
           PARTITION BY account_id, thread_id
           ORDER BY date DESC, id DESC
         ) AS rn
  FROM messages
) WHERE rn = 1";

/// Scoring formula for `seen_addresses` contact search relevance.
///
/// Weights: sent_to * 3, sent_cc * 1.5, received_from * 1, received_cc * 0.5.
/// Decays with time (90-day half-life).
pub const SEEN_ADDRESS_SCORE_EXPR: &str = "\
CAST(
  (sa.times_sent_to * 3.0 + sa.times_sent_cc * 1.5
   + sa.times_received_from * 1.0 + sa.times_received_cc * 0.5)
  / (1.0 + CAST((unixepoch() * 1000 - sa.last_seen_at) AS REAL)
     / 86400000.0 / 90.0)
AS INTEGER)";

// ---------------------------------------------------------------------------
// FTS5 / LIKE helpers
// ---------------------------------------------------------------------------

/// Build a `LIKE` pattern from a search query, escaping SQL wildcards.
///
/// Short queries (1-2 chars) use prefix matching (`pattern%`) which can use
/// a B-tree index. Longer queries use substring matching (`%pattern%`).
pub fn make_like_pattern(trimmed: &str) -> String {
    let escaped = trimmed.replace('%', r"\%").replace('_', r"\_");
    if trimmed.len() <= 2 {
        format!("{escaped}%")
    } else {
        format!("%{escaped}%")
    }
}

/// Build an FTS5 prefix query from raw user input.
///
/// Each whitespace-separated token is cleaned, quoted, and given a `*`
/// suffix for prefix matching. Empty tokens are dropped.
pub fn build_fts_query(raw: &str) -> String {
    raw.split_whitespace()
        .map(|token| {
            let clean: String = token
                .chars()
                .filter(|c| c.is_alphanumeric() || *c == '@' || *c == '.' || *c == '-' || *c == '_')
                .collect();
            format!("\"{clean}\"*")
        })
        .filter(|t| t.len() > 3)
        .collect::<Vec<_>>()
        .join(" ")
}

/// Valid boolean columns on the `threads` table that can be toggled.
/// Used by [`super::queries::set_thread_bool_field`] to prevent SQL injection.
const VALID_THREAD_BOOL_COLUMNS: &[&str] = &["is_read", "is_starred", "is_pinned", "is_muted"];

/// Validate that a column name is an allowed boolean field on `threads`.
///
/// Returns `Ok(column)` if valid, `Err` if the column is not in the allowlist.
pub fn validate_thread_bool_column(column: &str) -> Result<&str, String> {
    if VALID_THREAD_BOOL_COLUMNS.contains(&column) {
        Ok(column)
    } else {
        Err(format!("invalid thread boolean column: {column}"))
    }
}
