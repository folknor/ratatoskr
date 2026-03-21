mod parser;
mod sql_builder;
mod tokens;

pub use parser::{CursorContext, ParsedQuery, analyze_cursor_context, parse_query};
pub use sql_builder::{count_matching, query_threads};

use rusqlite::Connection;

use ratatoskr_db::db::types::{AccountScope, DbThread};

/// Parameters for a smart folder query, packed to stay under the 7-arg limit.
pub struct SmartFolderParams<'a> {
    pub query: &'a str,
    pub scope: &'a AccountScope,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Execute a smart folder query string against the database.
///
/// The parser handles relative date offsets natively (`-7`, `-30`, `0`),
/// so the old token system (`__LAST_7_DAYS__` etc.) is no longer needed
/// for new queries. Legacy queries using tokens are still supported via
/// a backward-compatibility shim during the migration period.
pub fn execute_smart_folder_query(
    conn: &Connection,
    params: &SmartFolderParams<'_>,
) -> Result<Vec<DbThread>, String> {
    let query = migrate_legacy_tokens(params.query);
    let parsed = parse_query(&query);
    log::debug!("Smart folder query parsed: {:?}", parsed);
    let result = sql_builder::query_threads(conn, &parsed, params.scope, params.limit, params.offset);
    if let Err(ref e) = result {
        log::error!("Smart folder query execution failed: {e}");
    }
    result
}

/// Return the count of unread threads matching a smart folder query.
pub fn count_smart_folder_unread(
    conn: &Connection,
    query: &str,
    scope: &AccountScope,
) -> Result<i64, String> {
    let query = migrate_legacy_tokens(query);
    let mut parsed = parse_query(&query);
    parsed.is_unread = Some(true);
    let result = sql_builder::count_matching(conn, &parsed, scope);
    if let Err(ref e) = result {
        log::error!("Smart folder unread count failed: {e}");
    }
    result
}

/// Translate legacy date tokens to the parser's native offset syntax.
///
/// This is a backward-compatibility shim. New smart folder queries
/// should use offset syntax directly (e.g. `after:-7` instead of
/// `after:__LAST_7_DAYS__`).
fn migrate_legacy_tokens(query: &str) -> String {
    if !query.contains("__") {
        return query.to_owned();
    }
    query
        .replace("__LAST_7_DAYS__", "-7")
        .replace("__LAST_30_DAYS__", "-30")
        .replace("__TODAY__", "0")
}
