mod parser;
mod sql_builder;

pub use parser::{CursorContext, ParsedQuery, analyze_cursor_context, parse_query};
pub use sql_builder::{count_matching_read, query_thread_keys_read, query_threads_read};

use db_read::db::ReadConn;
use db_read::db::types::AccountScope;

/// Return the count of unread threads matching a smart folder query. Wired
/// from `get_navigation_state()` to populate per-smart-folder unread counts
/// in the sidebar.
pub fn count_smart_folder_unread(
    conn: &ReadConn<'_>,
    query: &str,
    scope: &AccountScope,
) -> Result<i64, String> {
    let mut parsed = parse_query(query);
    parsed.is_unread = Some(true);
    let result = sql_builder::count_matching_read(conn, &parsed, scope);
    if let Err(ref e) = result {
        log::error!("Smart folder unread count failed: {e}");
    }
    result
}
