//! GAL (Global Address List) / organization directory caching.
//!
//! Pre-fetches organization directory at startup for providers that
//! support it (Microsoft Graph `/users`, Google Directory API).
//! The cache is stored in the `gal_cache` table and refreshed by polling.
//!
//! Autocomplete searches include GAL entries via the app-level
//! `search_gal_cache()` function, so autocomplete is always local.

use rusqlite::params;

use crate::db::DbState;

/// A single GAL entry to cache.
#[derive(Debug, Clone)]
pub struct GalEntry {
    pub email: String,
    pub display_name: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub title: Option<String>,
    pub department: Option<String>,
}

/// Store fetched GAL entries in the cache for a given account.
///
/// This replaces all existing entries for the account (full refresh).
pub async fn cache_gal_entries(
    db: &DbState,
    account_id: String,
    entries: Vec<GalEntry>,
) -> Result<usize, String> {
    let count = entries.len();
    db.with_conn(move |conn| {
        // Clear existing entries for this account
        conn.execute(
            "DELETE FROM gal_cache WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| format!("clear gal_cache: {e}"))?;

        // Insert new entries
        let mut stmt = conn
            .prepare(
                "INSERT OR REPLACE INTO gal_cache
                 (email, display_name, phone, company, title, department, account_id, cached_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch())",
            )
            .map_err(|e| format!("prepare gal insert: {e}"))?;

        for entry in &entries {
            stmt.execute(params![
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

        Ok(count)
    })
    .await
}

/// Get the timestamp of the last GAL cache refresh for an account.
/// Returns None if no cache exists.
pub async fn gal_cache_age(
    db: &DbState,
    account_id: String,
) -> Result<Option<i64>, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT MAX(cached_at) FROM gal_cache WHERE account_id = ?1",
            params![account_id],
            |row| row.get::<_, Option<i64>>(0),
        )
        .map_err(|e| format!("query gal_cache age: {e}"))
    })
    .await
}
