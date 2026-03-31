//! Unified search pipeline that routes queries through SQL, Tantivy, or both.
//!
//! Entry point: [`search()`] parses the query string and dispatches to the
//! appropriate backend based on whether the query contains free text,
//! structured operators, or both.

use std::collections::{HashMap, HashSet};

use db::db::types::{AccountScope, DbThread};
use rusqlite::Connection;
use search::{SearchParams, SearchResult as TantivyResult, SearchState};
use smart_folder::{ParsedQuery, parse_query, query_threads};

// ── Result type ─────────────────────────────────────────────

/// A unified search result that works for all three search paths.
#[derive(Debug, Clone)]
pub struct UnifiedSearchResult {
    pub thread_id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub date: Option<i64>,
    pub is_read: bool,
    pub is_starred: bool,
    pub message_count: Option<i64>,
    /// BM25 score from Tantivy, or 0.0 for SQL-only results.
    pub rank: f32,
}

// ── Public entry point ──────────────────────────────────────

/// Parse a query string and route it through the appropriate search backend(s).
///
/// - Empty query returns no results.
/// - Operators only (e.g. `is:unread from:alice`) routes through SQL.
/// - Free text only (e.g. `meeting notes`) routes through Tantivy.
/// - Both operators and free text intersects SQL candidates with Tantivy scores.
pub fn search(
    query: &str,
    search_state: &SearchState,
    conn: &Connection,
) -> Result<Vec<UnifiedSearchResult>, String> {
    let parsed = parse_query(query);

    let has_free_text = !parsed.free_text.is_empty();
    let has_operators = parsed.has_any_operator();

    let path_name = match (has_free_text, has_operators) {
        (false, false) => "empty",
        (false, true) => "sql_only",
        (true, false) => "tantivy_only",
        (true, true) => "combined",
    };
    log::debug!("Search pipeline routing: path={path_name}, query={query:?}");

    let result = match (has_free_text, has_operators) {
        (false, false) => Ok(vec![]),
        (false, true) => search_sql_only(&parsed, conn),
        (true, false) => search_tantivy_only(&parsed, search_state),
        (true, true) => search_combined(&parsed, search_state, conn),
    };

    match &result {
        Ok(results) => {
            log::info!(
                "Search executed via {path_name} path, returned {} results",
                results.len()
            );
        }
        Err(e) => {
            log::error!("Search failed via {path_name} path: {e}");
        }
    }

    result
}

// ── Path 1: SQL only ────────────────────────────────────────

/// Operators without free text: run SQL query, return date-sorted results.
fn search_sql_only(
    parsed: &ParsedQuery,
    conn: &Connection,
) -> Result<Vec<UnifiedSearchResult>, String> {
    let scope = build_scope(parsed);
    let threads = query_threads(conn, parsed, &scope, Some(200), Some(0))?;
    Ok(threads.into_iter().map(db_thread_to_unified).collect())
}

// ── Path 2: Tantivy only ────────────────────────────────────

/// Free text without operators: run Tantivy search, group by thread.
fn search_tantivy_only(
    parsed: &ParsedQuery,
    search_state: &SearchState,
) -> Result<Vec<UnifiedSearchResult>, String> {
    let params = build_tantivy_params(parsed);
    let results = search_state.search_with_filters(&params)?;
    let mut grouped = group_by_thread_unified(results);
    grouped.sort_by(|a, b| {
        b.rank
            .partial_cmp(&a.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(grouped)
}

// ── Path 3: Combined ────────────────────────────────────────

/// Both operators and free text: SQL narrows candidates, Tantivy scores them.
fn search_combined(
    parsed: &ParsedQuery,
    search_state: &SearchState,
    conn: &Connection,
) -> Result<Vec<UnifiedSearchResult>, String> {
    // Step 1: SQL generates candidate thread IDs.
    let scope = build_scope(parsed);
    let sql_threads = query_threads(
        conn,
        parsed,
        &scope,
        Some(crate::constants::DEFAULT_QUERY_LIMIT),
        Some(0),
    )?;
    let candidate_ids: HashSet<String> = sql_threads.iter().map(|t| t.id.clone()).collect();

    // Build a lookup map for enrichment from SQL results.
    let thread_map: HashMap<String, &DbThread> =
        sql_threads.iter().map(|t| (t.id.clone(), t)).collect();

    // Step 2: Tantivy searches free text (no account filter — SQL handles it
    // via intersection, and account: values are display names, not IDs).
    let mut params = build_tantivy_params(parsed);
    params.account_ids = None;
    let tantivy_results = search_state.search_with_filters(&params)?;

    // Step 3: Intersect — keep only Tantivy hits in the SQL candidate set.
    let filtered: Vec<TantivyResult> = tantivy_results
        .into_iter()
        .filter(|r| candidate_ids.contains(&r.thread_id))
        .collect();

    // Step 4: Group by thread, take max score.
    let grouped = group_by_thread_unified(filtered);

    // Step 5: Enrich with SQL metadata where available.
    let mut enriched: Vec<UnifiedSearchResult> = grouped
        .into_iter()
        .map(|r| enrich_from_sql(r, &thread_map))
        .collect();

    // Step 6: Sort by rank descending.
    enriched.sort_by(|a, b| {
        b.rank
            .partial_cmp(&a.rank)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    Ok(enriched)
}

// ── Helpers ─────────────────────────────────────────────────

/// Determine the account scope from parsed query operators.
///
/// Always returns `AccountScope::All` because account narrowing is handled
/// internally by the SQL builder when `account:` operators are present.
fn build_scope(_parsed: &ParsedQuery) -> AccountScope {
    AccountScope::All
}

/// Build Tantivy `SearchParams` from a parsed query.
fn build_tantivy_params(parsed: &ParsedQuery) -> SearchParams {
    let account_ids = if parsed.account.is_empty() {
        None
    } else {
        Some(parsed.account.clone())
    };

    SearchParams {
        account_ids,
        free_text: Some(parsed.free_text.clone()),
        from: parsed.from.clone(),
        to: parsed.to.clone(),
        subject: None,
        has_attachment: if parsed.has_attachment {
            Some(true)
        } else {
            None
        },
        is_unread: parsed.is_unread,
        is_starred: parsed.is_starred,
        before: parsed.before,
        after: parsed.after,
        limit: Some(200),
    }
}

/// Convert a `DbThread` (from SQL) into a `UnifiedSearchResult` with rank 0.0.
fn db_thread_to_unified(t: DbThread) -> UnifiedSearchResult {
    UnifiedSearchResult {
        thread_id: t.id,
        account_id: t.account_id,
        subject: t.subject,
        snippet: t.snippet,
        from_name: t.from_name,
        from_address: t.from_address,
        date: t.last_message_at,
        is_read: t.is_read,
        is_starred: t.is_starred,
        message_count: Some(t.message_count),
        rank: 0.0,
    }
}

/// Group message-level Tantivy results by thread_id, taking the highest
/// score per thread. Delegates to `search::group_by_thread`
/// for the grouping logic, then converts to `UnifiedSearchResult`.
fn group_by_thread_unified(results: Vec<TantivyResult>) -> Vec<UnifiedSearchResult> {
    let grouped = search::group_by_thread(results);
    grouped
        .into_iter()
        .map(|r| tantivy_result_to_unified(&r))
        .collect()
}

/// Convert a single Tantivy result into a `UnifiedSearchResult`.
fn tantivy_result_to_unified(r: &TantivyResult) -> UnifiedSearchResult {
    UnifiedSearchResult {
        thread_id: r.thread_id.clone(),
        account_id: r.account_id.clone(),
        subject: r.subject.clone(),
        snippet: r.snippet.clone(),
        from_name: r.from_name.clone(),
        from_address: r.from_address.clone(),
        date: Some(r.date),
        is_read: false,
        is_starred: false,
        message_count: None,
        rank: r.rank,
    }
}

/// Enrich a unified result with metadata from the SQL thread map.
fn enrich_from_sql(
    mut result: UnifiedSearchResult,
    thread_map: &HashMap<String, &DbThread>,
) -> UnifiedSearchResult {
    if let Some(t) = thread_map.get(&result.thread_id) {
        result.subject = result.subject.or_else(|| t.subject.clone());
        result.snippet = result.snippet.or_else(|| t.snippet.clone());
        result.from_name = result.from_name.or_else(|| t.from_name.clone());
        result.from_address = result.from_address.or_else(|| t.from_address.clone());
        result.is_read = t.is_read;
        result.is_starred = t.is_starred;
        result.message_count = Some(t.message_count);
        if result.date.is_none() {
            result.date = t.last_message_at;
        }
    }
    result
}

/// Parse a date string (ISO 8601 or unix timestamp) into epoch seconds.
fn parse_date_string(s: &str) -> Option<i64> {
    s.parse::<i64>().ok()
}

// ── Tests ───────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // -- Routing logic --

    #[test]
    fn empty_query_returns_empty() {
        let parsed = parse_query("");
        assert!(parsed.free_text.is_empty());
        assert!(!parsed.has_any_operator());
    }

    #[test]
    fn free_text_only_detected() {
        let parsed = parse_query("meeting notes");
        assert!(!parsed.free_text.is_empty());
        assert!(!parsed.has_any_operator());
    }

    #[test]
    fn operators_only_detected() {
        let parsed = parse_query("is:unread from:alice");
        assert!(parsed.free_text.is_empty());
        assert!(parsed.has_any_operator());
    }

    #[test]
    fn combined_detected() {
        let parsed = parse_query("meeting from:alice");
        assert!(!parsed.free_text.is_empty());
        assert!(parsed.has_any_operator());
    }

    // -- Scope building --

    #[test]
    fn scope_is_all_when_no_account() {
        let parsed = parse_query("is:unread");
        let scope = build_scope(&parsed);
        assert!(matches!(scope, AccountScope::All));
    }

    #[test]
    fn scope_is_all_when_account_present() {
        // account: operators are handled by the SQL builder internally,
        // so we always pass All.
        let parsed = parse_query("account:work");
        let scope = build_scope(&parsed);
        assert!(matches!(scope, AccountScope::All));
    }

    // -- Tantivy params building --

    #[test]
    fn tantivy_params_basic() {
        let parsed = parse_query("hello world");
        let params = build_tantivy_params(&parsed);
        assert_eq!(params.free_text.as_deref(), Some("hello world"));
        assert!(params.account_ids.is_none());
        assert!(params.from.is_empty());
    }

    #[test]
    fn tantivy_params_with_account() {
        let parsed = parse_query("hello account:work");
        let params = build_tantivy_params(&parsed);
        assert_eq!(params.free_text.as_deref(), Some("hello"));
        assert_eq!(params.account_ids, Some(vec!["work".to_owned()]));
    }

    #[test]
    fn tantivy_params_with_filters() {
        let parsed = parse_query("hello from:alice has:attachment is:unread");
        let params = build_tantivy_params(&parsed);
        assert_eq!(params.free_text.as_deref(), Some("hello"));
        assert_eq!(params.from.first().map(String::as_str), Some("alice"));
        assert_eq!(params.has_attachment, Some(true));
        assert_eq!(params.is_unread, Some(true));
    }

    // -- DbThread to UnifiedSearchResult conversion --

    #[test]
    fn db_thread_converts_correctly() {
        let thread = DbThread {
            id: "t1".to_owned(),
            account_id: "acc1".to_owned(),
            subject: Some("Test subject".to_owned()),
            snippet: Some("Test snippet".to_owned()),
            last_message_at: Some(1_700_000_000),
            message_count: 3,
            is_read: true,
            is_starred: false,
            is_important: false,
            has_attachments: true,
            is_snoozed: false,
            snooze_until: None,
            is_pinned: false,
            is_muted: false,
            from_name: Some("Alice".to_owned()),
            from_address: Some("alice@test.com".to_owned()),
        };

        let result = db_thread_to_unified(thread);
        assert_eq!(result.thread_id, "t1");
        assert_eq!(result.account_id, "acc1");
        assert_eq!(result.subject.as_deref(), Some("Test subject"));
        assert_eq!(result.date, Some(1_700_000_000));
        assert!(result.is_read);
        assert!(!result.is_starred);
        assert_eq!(result.message_count, Some(3));
        assert_eq!(result.rank, 0.0);
    }

    // -- group_by_thread --

    #[test]
    fn group_by_thread_takes_max_score() {
        let results = vec![
            TantivyResult {
                message_id: "m1".to_owned(),
                account_id: "acc1".to_owned(),
                thread_id: "t1".to_owned(),
                subject: Some("A".to_owned()),
                from_name: None,
                from_address: None,
                snippet: None,
                date: 1000,
                rank: 2.5,
            },
            TantivyResult {
                message_id: "m2".to_owned(),
                account_id: "acc1".to_owned(),
                thread_id: "t1".to_owned(),
                subject: Some("B".to_owned()),
                from_name: None,
                from_address: None,
                snippet: None,
                date: 2000,
                rank: 5.0,
            },
            TantivyResult {
                message_id: "m3".to_owned(),
                account_id: "acc2".to_owned(),
                thread_id: "t2".to_owned(),
                subject: Some("C".to_owned()),
                from_name: None,
                from_address: None,
                snippet: None,
                date: 3000,
                rank: 1.0,
            },
        ];

        let grouped = group_by_thread_unified(results);
        assert_eq!(grouped.len(), 2);

        let t1 = grouped.iter().find(|r| r.thread_id == "t1");
        let t2 = grouped.iter().find(|r| r.thread_id == "t2");
        assert!(t1.is_some());
        assert!(t2.is_some());

        let t1 = t1.expect("t1 should exist");
        assert_eq!(t1.rank, 5.0);
        assert_eq!(t1.subject.as_deref(), Some("B"));

        let t2 = t2.expect("t2 should exist");
        assert_eq!(t2.rank, 1.0);
    }

    // -- enrichment --

    #[test]
    fn enrich_fills_missing_fields() {
        let thread = DbThread {
            id: "t1".to_owned(),
            account_id: "acc1".to_owned(),
            subject: Some("SQL subject".to_owned()),
            snippet: Some("SQL snippet".to_owned()),
            last_message_at: Some(1_700_000_000),
            message_count: 5,
            is_read: true,
            is_starred: true,
            is_important: false,
            has_attachments: false,
            is_snoozed: false,
            snooze_until: None,
            is_pinned: false,
            is_muted: false,
            from_name: Some("Alice".to_owned()),
            from_address: Some("alice@test.com".to_owned()),
        };
        let mut map = HashMap::new();
        map.insert("t1".to_owned(), &thread);

        let result = UnifiedSearchResult {
            thread_id: "t1".to_owned(),
            account_id: "acc1".to_owned(),
            subject: Some("Tantivy subject".to_owned()),
            snippet: None,
            from_name: None,
            from_address: None,
            date: Some(1000),
            is_read: false,
            is_starred: false,
            message_count: None,
            rank: 3.5,
        };

        let enriched = enrich_from_sql(result, &map);
        // Subject was already set from Tantivy, so it stays.
        assert_eq!(enriched.subject.as_deref(), Some("Tantivy subject"));
        // Snippet was None, so it gets filled from SQL.
        assert_eq!(enriched.snippet.as_deref(), Some("SQL snippet"));
        // Flags and counts come from SQL.
        assert!(enriched.is_read);
        assert!(enriched.is_starred);
        assert_eq!(enriched.message_count, Some(5));
        // Rank stays from Tantivy.
        assert_eq!(enriched.rank, 3.5);
    }

    // -- parse_date_string --

    #[test]
    fn parse_date_string_unix_timestamp() {
        assert_eq!(parse_date_string("1700000000"), Some(1_700_000_000));
    }

    #[test]
    fn parse_date_string_non_numeric() {
        assert_eq!(parse_date_string("not-a-number"), None);
    }
}
