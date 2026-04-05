use std::path::Path;
use rtsk::db::{Connection, DbState, ReadWriteDb};

// ── DB connection ───────────────────────────────────────

pub struct Db {
    inner: ReadWriteDb,
}

impl Db {
    pub fn open(app_data_dir: &Path) -> Result<Self, String> {
        let db_path = app_data_dir.join("ratatoskr.db");
        if !db_path.exists() {
            return Err(format!("database not found: {}", db_path.display()));
        }
        Ok(Self {
            inner: ReadWriteDb::init(app_data_dir)?,
        })
    }

    pub fn read_db_state(&self) -> DbState {
        self.inner.read()
    }

    pub fn write_db_state(&self) -> DbState {
        self.inner.write()
    }

    /// Execute a closure on the writable connection.
    pub async fn with_write_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        self.inner.write().with_conn(f).await
    }

    /// Synchronous access to the writable connection.
    pub fn with_write_conn_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        self.inner.write().with_conn_sync(f)
    }

    pub async fn with_conn<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        self.inner.read().with_conn(f).await
    }

    /// Synchronous DB access for use inside `spawn_blocking`.
    pub fn with_conn_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&Connection) -> Result<T, String>,
    {
        self.inner.read().with_conn_sync(f)
    }
}
