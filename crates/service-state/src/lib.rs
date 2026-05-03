//! Service-only writer-half state types.
//!
//! Phase 2 of `docs/service/phase-2-plan.md` introduces a read/write split
//! at the type level: `db::DbState` becomes `db::ReadDbState` (UI-visible)
//! and `service_state::WriteDbState` (Service-only). The split is enforced
//! by Cargo dependency graph - the `app` crate does NOT depend on this
//! crate, so `WriteDbState` cannot be reached from UI source files even
//! with `pub` visibility.
//!
//! This commit is the type-level **scaffold**: the crate exists and
//! exports `WriteDbState` with the same shape as today's `DbState`, but
//! no call sites have been moved yet. Task 3 in the plan renames
//! `db::DbState` to `db::ReadDbState`, extracts a private connection
//! pool, and routes the write helpers through this crate. Tasks 6-9
//! relocate the action service onto this writer half.
//!
//! Until task 3 lands, `WriteDbState` is constructible from any
//! `Arc<Mutex<Connection>>` via `from_arc`; the `from_db_state` bridge
//! is provided so the boot path can construct one off the existing
//! `db::DbState` connection pool without touching read-side call sites.

use std::sync::{Arc, Mutex};

use rusqlite::Connection;

/// Service-only writer half of the shared database state.
///
/// Identical in shape to today's `db::DbState`; the rename + pool
/// extraction lands in task 3. Per the plan, the `app` crate must
/// not depend on `service-state`, so reaching this type from UI code
/// is a compile error (missing crate, not a visibility error).
#[derive(Clone)]
pub struct WriteDbState {
    conn: Arc<Mutex<Connection>>,
}

impl WriteDbState {
    /// Construct a writer-half state from an existing connection Arc.
    /// Used by the Service boot path to consume the connection that
    /// Phase 1.5 holds in `BootContext`.
    pub fn from_arc(conn: Arc<Mutex<Connection>>) -> Self {
        Self { conn }
    }

    /// Bridge from `db::DbState` for the boot transition. Will be
    /// removed in task 3 once the connection pool moves into a
    /// private `db::ConnectionPool` type and `WriteDbState` consumes
    /// it directly.
    pub fn from_db_state(state: &db::db::DbState) -> Self {
        Self::from_arc(state.conn())
    }

    /// Run a closure with the database connection on the blocking
    /// thread pool. Mirrors `db::DbState::with_conn` so the action
    /// service can call this directly once relocation lands in task 9.
    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
            f(&conn)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Synchronous variant for boot-path use that already runs inside
    /// `spawn_blocking`. Mirrors `db::DbState::with_conn_sync`.
    pub fn with_conn_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        f(&conn)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;

    fn fresh_state() -> WriteDbState {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        WriteDbState::from_arc(Arc::new(Mutex::new(conn)))
    }

    #[test]
    fn from_arc_is_clone() {
        let state = fresh_state();
        let cloned = state.clone();
        // Both clones share the same underlying connection.
        let result = state.with_conn_sync(|conn| {
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", [])
                .map_err(|e| e.to_string())
        });
        assert!(result.is_ok());

        let count = cloned
            .with_conn_sync(|conn| {
                let mut stmt = conn
                    .prepare("SELECT count(*) FROM sqlite_master WHERE name = 't'")
                    .map_err(|e| e.to_string())?;
                let count: i64 = stmt
                    .query_row([], |row| row.get(0))
                    .map_err(|e| e.to_string())?;
                Ok(count)
            })
            .expect("read after write");
        assert_eq!(count, 1);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn with_conn_dispatches_to_blocking_pool() {
        let state = fresh_state();
        let value = state
            .with_conn(|conn| {
                conn.execute_batch("CREATE TABLE async_t (n INTEGER)")
                    .map_err(|e| e.to_string())?;
                conn.execute("INSERT INTO async_t VALUES (?)", [42])
                    .map_err(|e| e.to_string())?;
                let mut stmt = conn
                    .prepare("SELECT n FROM async_t")
                    .map_err(|e| e.to_string())?;
                let n: i64 = stmt
                    .query_row([], |row| row.get(0))
                    .map_err(|e| e.to_string())?;
                Ok(n)
            })
            .await
            .expect("with_conn ok");
        assert_eq!(value, 42);
    }
}
