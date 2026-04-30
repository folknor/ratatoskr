/// A GAL entry for bulk cache insert.
#[derive(Debug, Clone)]
pub struct GalEntry {
    pub email: String,
    pub display_name: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub title: Option<String>,
    pub department: Option<String>,
}

/// Clear and refill the GAL cache for an account.
pub fn cache_gal_entries_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
    entries: &[GalEntry],
) -> Result<usize, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin gal tx: {e}"))?;

    tx.execute(
        "DELETE FROM gal_cache WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("clear gal cache: {e}"))?;

    let mut stmt = tx
        .prepare(
            "INSERT OR REPLACE INTO gal_cache \
             (email, display_name, phone, company, title, department, account_id, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch())",
        )
        .map_err(|e| format!("prepare gal insert: {e}"))?;

    for entry in entries {
        stmt.execute(rusqlite::params![
            entry.email,
            entry.display_name,
            entry.phone,
            entry.company,
            entry.title,
            entry.department,
            account_id,
        ])
        .map_err(|e| format!("insert gal entry: {e}"))?;
    }

    drop(stmt);
    tx.commit().map_err(|e| format!("commit gal tx: {e}"))?;
    Ok(entries.len())
}

/// Get the timestamp of the last GAL refresh for an account.
pub fn gal_cache_age_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<Option<i64>, String> {
    let key = format!("gal_refresh_{account_id}");
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .map(|v| {
        v.parse::<i64>()
            .map_err(|e| format!("parse gal timestamp: {e}"))
    })
    .transpose()
}

/// Record that a GAL refresh was performed for an account.
pub fn record_gal_refresh_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp().to_string();
    let key = format!("gal_refresh_{account_id}");
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        rusqlite::params![key, now],
    )
    .map_err(|e| format!("record gal refresh: {e}"))?;
    Ok(())
}

/// Look up the provider type for an account.
pub fn get_account_provider_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<String, String> {
    conn.query_row(
        "SELECT provider FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| row.get(0),
    )
    .map_err(|e| format!("lookup provider: {e}"))
}
