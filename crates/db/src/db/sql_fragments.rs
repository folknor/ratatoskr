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
