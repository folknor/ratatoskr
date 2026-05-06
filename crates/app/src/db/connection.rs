use std::path::Path;
use rtsk::db::{Connection, ReadDbState, ReadWriteDb};

// ── DB connection ───────────────────────────────────────

pub struct Db {
    inner: ReadWriteDb,
}

impl Db {
    /// Open the UI's view of the DB after the Service has signaled
    /// `boot.ready`. Routes through `ReadWriteDb::open_existing` (no rename
    /// reconciliation, no migrations) since the Service owns those as part of
    /// the boot sequence. Calling `ReadWriteDb::init` here would re-run the
    /// rename and the migration runner from the UI process - both correct
    /// (idempotent) but contradicting "the Service is the only writer" and
    /// adding a redundant migration check on every boot.
    pub fn open(app_data_dir: &Path) -> Result<Self, String> {
        let db_path = app_data_dir.join("ratatoskr.db");
        if !db_path.exists() {
            return Err(format!("database not found: {}", db_path.display()));
        }
        Ok(Self {
            inner: ReadWriteDb::open_existing(app_data_dir)?,
        })
    }

    pub fn read_db_state(&self) -> ReadDbState {
        self.inner.read()
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
