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

    /// **Phase 6c-pending write-conn escape hatch.**
    ///
    /// The single writable-connection accessor that survives Phase
    /// 6a's lockdown, and the only allow-listed write-surface call
    /// site in the app crate (gated by the symbol pattern in
    /// `scripts/check_app_write_surface.sh`). Used by the
    /// `cal::actions` ActionContext construction in `app.rs`; Phase
    /// 6c relocates calendar event mutations Service-side and
    /// removes both this accessor and the `ActionContext` itself.
    ///
    /// `Db::with_write_conn`, `Db::with_write_conn_sync`, and the
    /// old `Db::write_db_state` are deleted as of Phase 6a-part-2.
    /// Adding a new caller of this method requires updating the
    /// allow-list in the lockdown script and is a deliberate
    /// regression of the Service-side-write invariant - prefer an
    /// IPC.
    pub fn phase_6c_pending_write_state(&self) -> ReadDbState {
        self.inner.write()
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
