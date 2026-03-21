//! Pinned search CRUD — app-local UI state.
//!
//! These queries intentionally stay in the app crate rather than moving
//! to `ratatoskr-core` because:
//!
//! 1. **Pinned searches are local UI state**, not domain/sync data.
//!    The `pinned_searches` and `pinned_search_threads` tables are
//!    created by the app's write connection (see `db/connection.rs`)
//!    and don't exist in the core schema.
//!
//! 2. **Thread snapshot queries** (`get_threads_by_ids`) join against
//!    the app's `Thread` type and `row_to_thread` mapper, which are
//!    app-specific display types.
//!
//! 3. **No sync involvement** — pinned searches are never synced to
//!    any provider. They're purely a local bookmark mechanism.

use rusqlite::params;

use super::connection::Db;
use super::types::{Thread, row_to_thread};

// ── Pinned search type ───────────────────────────────────────

/// A pinned search with its stored thread snapshot.
#[derive(Debug, Clone)]
pub struct PinnedSearch {
    pub id: i64,
    pub query: String,
    pub created_at: i64,
    pub updated_at: i64,
}

// ── Pinned search CRUD ───────────────────────────────────────

impl Db {
    /// Creates a pinned search, or updates the existing one if
    /// `query` already exists. Returns the pinned search ID.
    pub async fn create_or_update_pinned_search(
        &self,
        query: String,
        thread_ids: Vec<(String, String)>,
    ) -> Result<i64, String> {
        self.with_write_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let now = chrono::Utc::now().timestamp();

            let existing_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM pinned_searches WHERE query = ?1",
                    params![query],
                    |row| row.get(0),
                )
                .ok();

            let pinned_id = if let Some(id) = existing_id {
                tx.execute(
                    "UPDATE pinned_searches SET updated_at = ?1 WHERE id = ?2",
                    params![now, id],
                )
                .map_err(|e| e.to_string())?;
                id
            } else {
                tx.execute(
                    "INSERT INTO pinned_searches (query, created_at, updated_at)
                     VALUES (?1, ?2, ?2)",
                    params![query, now],
                )
                .map_err(|e| e.to_string())?;
                tx.last_insert_rowid()
            };

            // Replace thread snapshot
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

                for (thread_id, account_id) in &thread_ids {
                    stmt.execute(params![pinned_id, thread_id, account_id])
                        .map_err(|e| e.to_string())?;
                }
            }

            tx.commit().map_err(|e| e.to_string())?;
            Ok(pinned_id)
        })
        .await
    }

    /// Updates a pinned search's query string and thread snapshot.
    /// If the new query conflicts with another pinned search, the
    /// conflicting row is deleted (merge behavior).
    pub async fn update_pinned_search(
        &self,
        id: i64,
        query: String,
        thread_ids: Vec<(String, String)>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
            let now = chrono::Utc::now().timestamp();

            // Check for a different pinned search with this query
            let conflict_id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM pinned_searches WHERE query = ?1 AND id != ?2",
                    params![query, id],
                    |row| row.get(0),
                )
                .ok();
            if let Some(cid) = conflict_id {
                tx.execute(
                    "DELETE FROM pinned_searches WHERE id = ?1",
                    params![cid],
                )
                .map_err(|e| e.to_string())?;
            }

            tx.execute(
                "UPDATE pinned_searches
                 SET query = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![query, now, id],
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

                for (thread_id, account_id) in &thread_ids {
                    stmt.execute(params![id, thread_id, account_id])
                        .map_err(|e| e.to_string())?;
                }
            }

            tx.commit().map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }

    /// Deletes a pinned search. CASCADE handles thread cleanup.
    pub async fn delete_pinned_search(&self, id: i64) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "DELETE FROM pinned_searches WHERE id = ?1",
                params![id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }

    /// Returns all pinned searches ordered by most recently updated.
    pub async fn list_pinned_searches(&self) -> Result<Vec<PinnedSearch>, String> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, query, created_at, updated_at
                     FROM pinned_searches
                     ORDER BY updated_at DESC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map([], |row| {
                Ok(PinnedSearch {
                    id: row.get("id")?,
                    query: row.get("query")?,
                    created_at: row.get("created_at")?,
                    updated_at: row.get("updated_at")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Loads the thread ID snapshot for a specific pinned search.
    pub async fn get_pinned_search_thread_ids(
        &self,
        pinned_search_id: i64,
    ) -> Result<Vec<(String, String)>, String> {
        self.with_conn(move |conn| {
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

    /// Fetches threads by (thread_id, account_id) pairs with current
    /// metadata. Threads that no longer exist are silently omitted.
    pub async fn get_threads_by_ids(
        &self,
        ids: Vec<(String, String)>,
    ) -> Result<Vec<Thread>, String> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }
        self.with_conn(move |conn| {
            let chunk_size = 400; // 2 params per ID, stay under 999
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

                let mut stmt =
                    conn.prepare(&sql).map_err(|e| e.to_string())?;

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
                    param_values.iter().map(|p| p.as_ref()).collect();

                let rows = stmt
                    .query_map(param_refs.as_slice(), row_to_thread)
                    .map_err(|e| e.to_string())?;

                for row in rows {
                    results.push(row.map_err(|e| e.to_string())?);
                }
            }

            Ok(results)
        })
        .await
    }

    /// Deletes all pinned searches. Used for the "Clear all" action.
    pub async fn delete_all_pinned_searches(&self) -> Result<u64, String> {
        self.with_write_conn(move |conn| {
            let deleted = conn
                .execute("DELETE FROM pinned_searches", [])
                .map_err(|e| e.to_string())?;
            #[allow(clippy::cast_sign_loss)]
            Ok(deleted as u64)
        })
        .await
    }

    /// Removes pinned searches older than `max_age_secs` that haven't
    /// been accessed (updated_at == created_at).
    pub async fn expire_stale_pinned_searches(
        &self,
        max_age_secs: i64,
    ) -> Result<u64, String> {
        self.with_write_conn(move |conn| {
            let cutoff = chrono::Utc::now().timestamp() - max_age_secs;
            let deleted = conn
                .execute(
                    "DELETE FROM pinned_searches
                     WHERE updated_at < ?1
                       AND updated_at = created_at",
                    params![cutoff],
                )
                .map_err(|e| e.to_string())?;
            #[allow(clippy::cast_sign_loss)]
            Ok(deleted as u64)
        })
        .await
    }
}
