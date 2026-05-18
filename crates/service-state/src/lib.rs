//! Service-only writer-half state types.
//!
//! The Service boundary uses a read/write split at the type level:
//! `db::ReadDbState` is UI-visible, while `service_state::WriteDbState`
//! is Service-only. The split is enforced by the Cargo dependency graph:
//! the `app` crate does not depend on this crate, so `WriteDbState`
//! cannot be reached from UI source files even with `pub` visibility.
//!
//! `WriteDbState` wraps `db::WriterPool`; callers receive only typed
//! `WriteConn` or `ReadConn` capabilities.

pub mod body_store_write;
pub mod inline_image_store_write;
pub mod search_write;
pub use body_store_write::BodyStoreWriteState;
pub use inline_image_store_write::InlineImageStoreWriteState;
pub use search_write::{SearchWriteHandle, WriterCommand};

/// Service-only writer half of the shared database state.
///
/// Per the plan, the `app` crate must not depend on `service-state`,
/// so reaching this type from UI code is a compile error (missing
/// crate, not a visibility error).
#[derive(Clone)]
pub struct WriteDbState {
    pool: db::db::WriterPool,
}

impl WriteDbState {
    pub fn from_pool(pool: db::db::WriterPool) -> Self {
        Self { pool }
    }

    pub async fn with_write<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&db::db::WriteConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        self.pool.with_write(f).await
    }

    pub async fn with_write_mapped<F, T, E, M>(&self, f: F, map_error: M) -> Result<T, E>
    where
        F: FnOnce(&db::db::WriteConn<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        M: Fn(String) -> E + Copy + Send + 'static,
    {
        self.pool.with_write_mapped(f, map_error).await
    }

    pub async fn with_read<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&db::db::ReadConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        self.pool.with_read(f).await
    }

    pub fn with_write_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&db::db::WriteConn<'_>) -> Result<T, String>,
    {
        self.pool.with_write_sync(f)
    }

    pub fn with_read_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&db::db::ReadConn<'_>) -> Result<T, String>,
    {
        self.pool.with_read_sync(f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fresh_state() -> (WriteDbState, tempfile::TempDir) {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let pool = db::db::open_writer_pool(tmp.path()).expect("open writer pool");
        (WriteDbState::from_pool(pool), tmp)
    }

    #[test]
    fn from_pool_is_clone() {
        let (state, _tmp) = fresh_state();
        let cloned = state.clone();
        // Both clones share the same underlying connection.
        let result = state.with_write_sync(|conn| {
            conn.execute("CREATE TABLE t (id INTEGER PRIMARY KEY)", [])
                .map_err(|e| e.to_string())
        });
        assert!(result.is_ok());

        let count = cloned
            .with_write_sync(|conn| {
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
    async fn with_write_dispatches_to_blocking_pool() {
        let (state, _tmp) = fresh_state();
        let value = state
            .with_write(|conn| {
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
            .expect("with_write ok");
        assert_eq!(value, 42);
    }
}
