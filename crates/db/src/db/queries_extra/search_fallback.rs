//! Free-text SQL fallback search for threads.
//!
//! Used when no Tantivy search index is available. Matches against
//! thread subject and snippet via LIKE.

use rusqlite::{Connection, params_from_iter};

use super::super::types::AccountScope;

/// A thread row from the free-text SQL fallback search.
#[derive(Debug, Clone)]
pub struct SearchFallbackRow {
    pub thread_id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub snippet: Option<String>,
    pub last_message_at: Option<i64>,
    pub message_count: i64,
    pub is_read: bool,
    pub is_starred: bool,
    pub has_attachments: bool,
    pub from_name: Option<String>,
    pub from_address: Option<String>,
}

/// Search threads by free-text LIKE on subject and snippet,
/// scoped to the given accounts.
pub fn search_threads_freetext_sync(
    conn: &Connection,
    pattern: &str,
    scope: &AccountScope,
    limit: i64,
) -> Result<Vec<SearchFallbackRow>, String> {
    let (scope_clause, scope_params): (String, Vec<String>) = match scope {
        AccountScope::All => (String::new(), vec![]),
        AccountScope::Single(id) => ("AND t.account_id = ?2".to_string(), vec![id.clone()]),
        AccountScope::Multiple(ids) => {
            let placeholders: Vec<String> =
                (0..ids.len()).map(|i| format!("?{}", i + 2)).collect();
            (
                format!("AND t.account_id IN ({})", placeholders.join(",")),
                ids.clone(),
            )
        }
    };

    let sql = format!(
        "SELECT t.id, t.account_id, t.subject, t.snippet,
                t.last_message_at, t.message_count,
                t.is_read, t.is_starred, t.has_attachments,
                t.from_name, t.from_address
         FROM threads t
         WHERE (t.subject LIKE ?1 OR t.snippet LIKE ?1)
         {scope_clause}
         ORDER BY t.last_message_at DESC
         LIMIT {limit}"
    );

    let mut stmt = conn
        .prepare(&sql)
        .map_err(|e| format!("prepare search: {e}"))?;

    let mut params: Vec<String> = Vec::with_capacity(1 + scope_params.len());
    params.push(pattern.to_string());
    params.extend(scope_params);

    let rows = stmt
        .query_map(params_from_iter(params.iter()), |row| {
            Ok(SearchFallbackRow {
                thread_id: row.get(0)?,
                account_id: row.get(1)?,
                subject: row.get(2)?,
                snippet: row.get(3)?,
                last_message_at: row.get(4)?,
                message_count: row.get(5)?,
                is_read: row.get(6)?,
                is_starred: row.get(7)?,
                has_attachments: row.get(8)?,
                from_name: row.get(9)?,
                from_address: row.get(10)?,
            })
        })
        .map_err(|e| format!("search query: {e}"))?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("search row: {e}"))
}
