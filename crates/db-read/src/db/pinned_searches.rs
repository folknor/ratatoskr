use rusqlite::params;

use super::from_row::query_as;
use super::types::DbThread;
use super::ReadDbState;

/// Stored pinned-search metadata and snapshot ownership.
#[derive(Debug, Clone)]
pub struct DbPinnedSearch {
    pub id: i64,
    pub query: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub scope_account_id: Option<String>,
    pub thread_ids: Option<Vec<(String, String)>>,
}

pub async fn db_list_pinned_searches(db: &ReadDbState) -> Result<Vec<DbPinnedSearch>, String> {
    db.with_read(|conn| {
        let mut stmt = conn
            .prepare(
                "SELECT id, query, created_at, updated_at, scope_account_id
                 FROM pinned_searches
                 ORDER BY updated_at DESC",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map([], |row| {
            Ok(DbPinnedSearch {
                id: row.get("id")?,
                query: row.get("query")?,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
                scope_account_id: row.get("scope_account_id")?,
                thread_ids: None,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_pinned_search_thread_ids(
    db: &ReadDbState,
    pinned_search_id: i64,
) -> Result<Vec<(String, String)>, String> {
    db.with_read(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT thread_id, account_id
                 FROM pinned_search_threads
                 WHERE pinned_search_id = ?1",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map(params![pinned_search_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_threads_by_ids(
    db: &ReadDbState,
    ids: Vec<(String, String)>,
) -> Result<Vec<DbThread>, String> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }

    db.with_read(move |conn| {
        let chunk_size = 400;
        let mut results = Vec::with_capacity(ids.len());

        for chunk in ids.chunks(chunk_size) {
            let placeholders: Vec<String> = chunk
                .iter()
                .enumerate()
                .map(|(i, _)| {
                    let p1 = i * 2 + 1;
                    let p2 = i * 2 + 2;
                    format!("(?{p1}, ?{p2})")
                })
                .collect();
            let values_clause = placeholders.join(", ");

            let sql = format!(
                "WITH target_ids(tid, aid) AS (VALUES {values_clause})
                 SELECT t.*, m.from_name, m.from_address
                 FROM target_ids ti
                 JOIN threads t ON t.id = ti.tid
                     AND t.account_id = ti.aid
                 LEFT JOIN messages m
                     ON m.account_id = t.account_id
                     AND m.thread_id = t.id
                     AND m.date = (
                         SELECT MAX(m2.date) FROM messages m2
                         WHERE m2.account_id = t.account_id
                           AND m2.thread_id = t.id
                     )
                 GROUP BY t.account_id, t.id
                 ORDER BY t.last_message_at DESC"
            );

            let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
                .iter()
                .flat_map(|(tid, aid)| {
                    vec![
                        Box::new(tid.clone()) as Box<dyn rusqlite::types::ToSql>,
                        Box::new(aid.clone()) as Box<dyn rusqlite::types::ToSql>,
                    ]
                })
                .collect();
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(std::convert::AsRef::as_ref).collect();

            results.extend(query_as::<DbThread>(conn, &sql, param_refs.as_slice())?);
        }

        Ok(results)
    })
    .await
}

pub async fn db_get_recent_search_queries(
    db: &ReadDbState,
    limit: usize,
) -> Result<Vec<String>, String> {
    db.with_read(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT query FROM pinned_searches
                 ORDER BY updated_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            row.get(0)
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<String>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}
