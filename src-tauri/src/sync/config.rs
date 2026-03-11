use rusqlite::Connection;

/// Account record read from the DB (minimal fields needed for sync).
pub struct SyncAccount {
    pub provider: String,
}

/// Read an account from the DB.
pub fn get_account(conn: &Connection, account_id: &str) -> Result<SyncAccount, String> {
    conn.query_row(
        "SELECT provider FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| Ok(SyncAccount { provider: row.get(0)? }),
    )
    .map_err(|e| format!("get account {account_id}: {e}"))
}

/// Read the `sync_period_days` setting from DB, defaulting to 365.
pub fn get_sync_period_days(conn: &Connection) -> i64 {
    conn.query_row(
        "SELECT value FROM settings WHERE key = 'sync_period_days'",
        [],
        |row| {
            let val: String = row.get(0)?;
            Ok(val.parse::<i64>().unwrap_or(365))
        },
    )
    .unwrap_or(365)
}
