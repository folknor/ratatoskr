mod parser;
mod sql_builder;
mod tokens;

pub use parser::{ParsedQuery, parse_query};
pub use tokens::resolve_query_tokens;

use rusqlite::Connection;

use crate::db::types::{AccountScope, DbThread};

/// Parameters for a smart folder query, packed to stay under the 7-arg limit.
pub struct SmartFolderParams<'a> {
    pub query: &'a str,
    pub scope: &'a AccountScope,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

/// Execute a smart folder query string against the database.
///
/// Resolves date tokens, parses operators, builds parameterized SQL,
/// and returns matching threads (deduplicated by thread).
pub fn execute_smart_folder_query(
    conn: &Connection,
    params: &SmartFolderParams<'_>,
) -> Result<Vec<DbThread>, String> {
    let resolved = resolve_query_tokens(params.query);
    let parsed = parse_query(&resolved);
    sql_builder::query_threads(conn, &parsed, params.scope, params.limit, params.offset)
}

/// Return the count of unread threads matching a smart folder query.
pub fn count_smart_folder_unread(
    conn: &Connection,
    query: &str,
    scope: &AccountScope,
) -> Result<i64, String> {
    let resolved = resolve_query_tokens(query);
    let mut parsed = parse_query(&resolved);
    parsed.is_unread = Some(true);
    sql_builder::count_matching(conn, &parsed, scope)
}
