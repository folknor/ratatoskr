use std::path::Path;
use std::sync::{Arc, Mutex};

use rusqlite::{Connection, OpenFlags};

/// Error returned by `ReadConn` SQL methods.
#[derive(Debug, thiserror::Error)]
pub enum ReadError {
    #[error("{0}")]
    Sql(#[from] rusqlite::Error),
    #[error("SQL is not read-only: {0}")]
    NotReadOnly(String),
}

pub struct ReadConn<'a> {
    raw: &'a Connection,
}

impl<'a> ReadConn<'a> {
    #[doc(hidden)]
    pub fn from_raw(raw: &'a Connection) -> Self {
        Self { raw }
    }

    pub fn prepare<'b>(&'b self, sql: &str) -> Result<ReadStatement<'b>, ReadError> {
        let stmt = self.raw.prepare(sql)?;
        if !stmt.readonly() {
            return Err(ReadError::NotReadOnly(sql.to_string()));
        }
        Ok(ReadStatement { raw: stmt })
    }

    pub fn prepare_cached<'b>(
        &'b self,
        sql: &str,
    ) -> Result<ReadCachedStatement<'b>, ReadError> {
        let stmt = self.raw.prepare_cached(sql)?;
        if !stmt.readonly() {
            return Err(ReadError::NotReadOnly(sql.to_string()));
        }
        Ok(ReadCachedStatement { raw: stmt })
    }

    /// Route through validated `prepare` so a mutating SQL string supplied to
    /// `query_row` cannot bypass the readonly check.
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, f: F) -> Result<T, ReadError>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        let mut stmt = self.prepare(sql)?;
        stmt.query_row(params, f)
    }
}

pub struct ReadStatement<'a> {
    raw: rusqlite::Statement<'a>,
}

impl<'a> ReadStatement<'a> {
    pub fn query<P>(&mut self, params: P) -> Result<rusqlite::Rows<'_>, ReadError>
    where
        P: rusqlite::Params,
    {
        Ok(self.raw.query(params)?)
    }

    pub fn query_map<T, P, F>(
        &mut self,
        params: P,
        f: F,
    ) -> Result<rusqlite::MappedRows<'_, F>, ReadError>
    where
        P: rusqlite::Params,
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.raw.query_map(params, f)?)
    }

    pub fn query_row<T, P, F>(&mut self, params: P, f: F) -> Result<T, ReadError>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.raw.query_row(params, f)?)
    }
}

pub struct ReadCachedStatement<'a> {
    raw: rusqlite::CachedStatement<'a>,
}

impl<'a> ReadCachedStatement<'a> {
    pub fn query<P>(&mut self, params: P) -> Result<rusqlite::Rows<'_>, ReadError>
    where
        P: rusqlite::Params,
    {
        Ok(self.raw.query(params)?)
    }

    pub fn query_map<T, P, F>(
        &mut self,
        params: P,
        f: F,
    ) -> Result<rusqlite::MappedRows<'_, F>, ReadError>
    where
        P: rusqlite::Params,
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.raw.query_map(params, f)?)
    }

    pub fn query_row<T, P, F>(&mut self, params: P, f: F) -> Result<T, ReadError>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.raw.query_row(params, f)?)
    }
}

#[derive(Clone)]
pub struct ReadDbState {
    conn: Arc<Mutex<Connection>>,
}

impl ReadDbState {
    pub fn open_existing(app_data_dir: &Path) -> Result<Self, String> {
        let db_path = app_data_dir.join("ratatoskr.db");
        let conn = Connection::open_with_flags(
            &db_path,
            OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )
        .map_err(|e| format!("open read db {}: {e}", db_path.display()))?;
        apply_reader_pragmas(&conn)?;
        Ok(Self {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    pub async fn with_read<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, String> + Send + 'static,
        T: Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock poisoned: {e}"))?;
            let read = ReadConn::from_raw(&conn);
            f(&read)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    pub async fn with_read_mapped<F, T, E, M>(&self, f: F, map_error: M) -> Result<T, E>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, E> + Send + 'static,
        T: Send + 'static,
        E: Send + 'static,
        M: Fn(String) -> E + Copy + Send + 'static,
    {
        let conn = Arc::clone(&self.conn);
        tokio::task::spawn_blocking(move || {
            let conn = conn
                .lock()
                .map_err(|e| map_error(format!("db lock poisoned: {e}")))?;
            let read = ReadConn::from_raw(&conn);
            f(&read)
        })
        .await
        .map_err(|e| map_error(format!("spawn_blocking: {e}")))?
    }

    pub fn with_read_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(&ReadConn<'_>) -> Result<T, String>,
    {
        let conn = self
            .conn
            .lock()
            .map_err(|e| format!("db lock poisoned: {e}"))?;
        let read = ReadConn::from_raw(&conn);
        f(&read)
    }

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

pub fn apply_reader_pragmas(conn: &Connection) -> Result<(), String> {
    conn.execute_batch(
        "PRAGMA busy_timeout = 15000;
         PRAGMA query_only = ON;
         PRAGMA foreign_keys = ON;
         PRAGMA temp_store = MEMORY;",
    )
    .map_err(|e| format!("reader pragmas: {e}"))
}

pub fn open_reader_pool(app_data_dir: &Path) -> Result<ReadDbState, String> {
    ReadDbState::open_existing(app_data_dir)
}
