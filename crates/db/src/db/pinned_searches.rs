use rusqlite::params;

use super::from_row::query_as;
use super::types::DbThread;
use super::{ReadDbState, WriteTarget, WriteTransactionTarget};

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

/// Create a pinned search or update the existing row for the same query.
///
/// Sync helper called from the Service-side
/// `pinned_search.create_or_update` handler via `WriteDbState::with_conn`.
/// Inside one transaction: query-keyed UPSERT on `pinned_searches`,
/// then full replacement of the row's `pinned_search_threads`.
pub fn db_create_or_update_pinned_search_sync(
    conn: &impl WriteTransactionTarget,
    query: &str,
    thread_ids: &[(String, String)],
    scope_account_id: Option<&str>,
) -> Result<i64, String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let existing_id: Option<i64> = tx
        .query_row(
            "SELECT id FROM pinned_searches WHERE query = ?1",
            params![query],
            |row| row.get(0),
        )
        .ok();

    let pinned_id = if let Some(id) = existing_id {
        tx.execute(
            "UPDATE pinned_searches
             SET updated_at = unixepoch(), scope_account_id = ?1
             WHERE id = ?2",
            params![scope_account_id, id],
        )
        .map_err(|e| e.to_string())?;
        id
    } else {
        tx.execute(
            "INSERT INTO pinned_searches (query, created_at, updated_at, scope_account_id)
             VALUES (?1, unixepoch(), unixepoch(), ?2)",
            params![query, scope_account_id],
        )
        .map_err(|e| e.to_string())?;
        tx.last_insert_rowid()
    };

    tx.execute(
        "DELETE FROM pinned_search_threads WHERE pinned_search_id = ?1",
        params![pinned_id],
    )
    .map_err(|e| e.to_string())?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO pinned_search_threads
                    (pinned_search_id, thread_id, account_id)
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(|e| e.to_string())?;

        for (thread_id, account_id) in thread_ids {
            stmt.execute(params![pinned_id, thread_id, account_id])
                .map_err(|e| e.to_string())?;
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(pinned_id)
}

/// Update an existing pinned search and replace its thread snapshot.
///
/// Sync helper called from the Service-side `pinned_search.update`
/// handler via `WriteDbState::with_conn`. Includes a query-conflict
/// cleanup step that deletes any other row with the same target query
/// before the update fires, preserving the UNIQUE on
/// `pinned_searches.query`.
pub fn db_update_pinned_search_sync(
    conn: &impl WriteTransactionTarget,
    id: i64,
    query: &str,
    thread_ids: &[(String, String)],
    scope_account_id: Option<&str>,
) -> Result<(), String> {
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    let conflict_id: Option<i64> = tx
        .query_row(
            "SELECT id FROM pinned_searches WHERE query = ?1 AND id != ?2",
            params![query, id],
            |row| row.get(0),
        )
        .ok();
    if let Some(cid) = conflict_id {
        tx.execute("DELETE FROM pinned_searches WHERE id = ?1", params![cid])
            .map_err(|e| e.to_string())?;
    }

    tx.execute(
        "UPDATE pinned_searches
         SET query = ?1, updated_at = unixepoch(), scope_account_id = ?2
         WHERE id = ?3",
        params![query, scope_account_id, id],
    )
    .map_err(|e| e.to_string())?;

    tx.execute(
        "DELETE FROM pinned_search_threads WHERE pinned_search_id = ?1",
        params![id],
    )
    .map_err(|e| e.to_string())?;

    {
        let mut stmt = tx
            .prepare(
                "INSERT INTO pinned_search_threads
                    (pinned_search_id, thread_id, account_id)
                 VALUES (?1, ?2, ?3)",
            )
            .map_err(|e| e.to_string())?;

        for (thread_id, account_id) in thread_ids {
            stmt.execute(params![id, thread_id, account_id])
                .map_err(|e| e.to_string())?;
        }
    }

    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete one pinned-search row by id. Idempotent; delete-of-missing
/// is `Ok`. Sync helper called from the Service-side
/// `pinned_search.delete` handler.
pub fn db_delete_pinned_search_sync(
    conn: &impl WriteTarget,
    id: i64,
) -> Result<(), String> {
    conn.execute("DELETE FROM pinned_searches WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn db_list_pinned_searches(db: &ReadDbState) -> Result<Vec<DbPinnedSearch>, String> {
    db.with_conn(|conn| {
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
    db.with_conn(move |conn| {
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

    db.with_conn(move |conn| {
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
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT query FROM pinned_searches
                 ORDER BY updated_at DESC
                 LIMIT ?1",
            )
            .map_err(|e| e.to_string())?;

        stmt.query_map(params![i64::try_from(limit).unwrap_or(i64::MAX)], |row| row.get(0))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

/// Delete every row from `pinned_searches`. Returns the deleted count.
/// Sync helper called from the Service-side `pinned_search.delete_all`
/// handler.
pub fn db_delete_all_pinned_searches_sync(
    conn: &impl WriteTarget,
) -> Result<u64, String> {
    let deleted = conn
        .execute("DELETE FROM pinned_searches", [])
        .map_err(|e| e.to_string())?;
    Ok(deleted as u64)
}

/// Delete pinned searches that were created more than `max_age_secs`
/// ago and never refreshed (`updated_at == created_at`). The DELETE is
/// idempotent; duplicate calls within the same staleness window are
/// no-ops by construction.
///
/// Phase 6a: callable from the Service-side `pinned_search.kick`
/// handler via `WriteDbState::with_conn`. The synchronous shape lets
/// the handler hold the connection for the duration of the DELETE
/// without needing an async wrapper.
pub fn db_expire_stale_pinned_searches_sync(
    conn: &impl WriteTarget,
    max_age_secs: i64,
) -> Result<u64, String> {
    let deleted = conn
        .execute(
            "DELETE FROM pinned_searches
             WHERE updated_at < unixepoch() - ?1
               AND updated_at = created_at",
            params![max_age_secs],
        )
        .map_err(|e| e.to_string())?;
    Ok(deleted as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::migrations;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        migrations::run_all(&conn).expect("migrations");
        conn
    }

    /// Insert a pinned-search row with explicit `created_at`/`updated_at`
    /// so the staleness window can be exercised deterministically.
    fn insert_pinned_search(
        conn: &Connection,
        query: &str,
        created_at: i64,
        updated_at: i64,
    ) -> i64 {
        conn.execute(
            "INSERT INTO pinned_searches (query, created_at, updated_at) \
             VALUES (?1, ?2, ?3)",
            params![query, created_at, updated_at],
        )
        .expect("seed pinned search");
        conn.last_insert_rowid()
    }

    fn count_pinned_searches(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM pinned_searches", [], |row| row.get(0))
            .expect("count")
    }

    #[test]
    fn expire_stale_deletes_old_unrefreshed_rows() {
        let conn = setup_db();
        let now: i64 = conn
            .query_row("SELECT unixepoch()", [], |row| row.get(0))
            .expect("now");
        // 14 days + 1 second old, never refreshed (updated_at == created_at).
        let stale = now - 1_209_600 - 1;
        insert_pinned_search(&conn, "stale", stale, stale);

        let deleted =
            db_expire_stale_pinned_searches_sync(&conn, 1_209_600).expect("expire");
        assert_eq!(deleted, 1);
        assert_eq!(count_pinned_searches(&conn), 0);
    }

    #[test]
    fn expire_stale_keeps_fresh_rows() {
        let conn = setup_db();
        let now: i64 = conn
            .query_row("SELECT unixepoch()", [], |row| row.get(0))
            .expect("now");
        // Created within the staleness window - must survive.
        insert_pinned_search(&conn, "fresh", now - 60, now - 60);

        let deleted =
            db_expire_stale_pinned_searches_sync(&conn, 1_209_600).expect("expire");
        assert_eq!(deleted, 0);
        assert_eq!(count_pinned_searches(&conn), 1);
    }

    #[test]
    fn expire_stale_keeps_old_but_refreshed_rows() {
        let conn = setup_db();
        let now: i64 = conn
            .query_row("SELECT unixepoch()", [], |row| row.get(0))
            .expect("now");
        // Created long ago but refreshed recently
        // (updated_at != created_at). Must survive.
        let created = now - 1_209_600 - 100;
        let updated = now - 60;
        insert_pinned_search(&conn, "refreshed", created, updated);

        let deleted =
            db_expire_stale_pinned_searches_sync(&conn, 1_209_600).expect("expire");
        assert_eq!(deleted, 0);
        assert_eq!(count_pinned_searches(&conn), 1);
    }

    #[test]
    fn expire_stale_is_idempotent() {
        let conn = setup_db();
        let now: i64 = conn
            .query_row("SELECT unixepoch()", [], |row| row.get(0))
            .expect("now");
        let stale = now - 1_209_600 - 1;
        insert_pinned_search(&conn, "stale", stale, stale);

        let first =
            db_expire_stale_pinned_searches_sync(&conn, 1_209_600).expect("first");
        let second =
            db_expire_stale_pinned_searches_sync(&conn, 1_209_600).expect("second");
        assert_eq!(first, 1);
        assert_eq!(second, 0);
    }
}
