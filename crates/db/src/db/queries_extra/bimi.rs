//! BIMI cache storage: lookup, upsert, and domain-warming query.

use rusqlite::{Connection, OptionalExtension, params};

/// A cached BIMI entry from the database.
#[derive(Debug, Clone)]
pub struct BimiCacheEntry {
    pub domain: String,
    pub has_bimi: bool,
    pub logo_uri: Option<String>,
    pub authority_uri: Option<String>,
    pub fetched_at: i64,
    pub expires_at: i64,
}

/// Retrieve a cached BIMI entry for a domain, returning `None` if absent or
/// expired.
pub fn get_bimi_cache(conn: &Connection, domain: &str) -> Result<Option<BimiCacheEntry>, String> {
    conn.query_row(
        "SELECT domain, has_bimi, logo_uri, authority_uri, fetched_at, expires_at \
         FROM bimi_cache WHERE domain = ?1 AND expires_at > strftime('%s', 'now')",
        params![domain],
        |row| {
            Ok(BimiCacheEntry {
                domain: row.get("domain")?,
                has_bimi: row.get::<_, i32>("has_bimi")? != 0,
                logo_uri: row.get("logo_uri")?,
                authority_uri: row.get("authority_uri")?,
                fetched_at: row.get("fetched_at")?,
                expires_at: row.get("expires_at")?,
            })
        },
    )
    .optional()
    .map_err(|e| format!("bimi cache query: {e}"))
}

/// Insert or update a BIMI cache entry.
pub fn upsert_bimi_cache(
    conn: &Connection,
    domain: &str,
    has_bimi: bool,
    logo_uri: Option<&str>,
    authority_uri: Option<&str>,
    expires_at: i64,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO bimi_cache (domain, has_bimi, logo_uri, authority_uri, fetched_at, expires_at) \
         VALUES (?1, ?2, ?3, ?4, strftime('%s', 'now'), ?5) \
         ON CONFLICT(domain) DO UPDATE SET \
           has_bimi = excluded.has_bimi, \
           logo_uri = excluded.logo_uri, \
           authority_uri = excluded.authority_uri, \
           fetched_at = excluded.fetched_at, \
           expires_at = excluded.expires_at",
        params![domain, has_bimi as i32, logo_uri, authority_uri, expires_at],
    )
    .map_err(|e| format!("bimi cache upsert: {e}"))?;
    Ok(())
}

/// Extract unique sender domains from recent messages that are not already
/// cached (or whose cache has expired).
///
/// `lookback_days` controls how far back to scan messages.
/// `max_domains` limits the result count.
pub fn domains_to_warm(
    conn: &Connection,
    lookback_days: i64,
    max_domains: i64,
) -> Result<Vec<String>, String> {
    let cutoff = chrono::Utc::now().timestamp() - (lookback_days * 24 * 3600);

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT LOWER(SUBSTR(m.from_address, INSTR(m.from_address, '@') + 1)) AS domain \
             FROM messages m \
             WHERE m.from_address IS NOT NULL \
               AND m.from_address LIKE '%@%' \
               AND m.date > ?1 \
             ORDER BY domain \
             LIMIT ?2",
        )
        .map_err(|e| format!("prepare domain query: {e}"))?;

    let rows = stmt
        .query_map(params![cutoff, max_domains], |row| {
            row.get::<_, String>("domain")
        })
        .map_err(|e| format!("query sender domains: {e}"))?;

    let mut domains = Vec::new();
    for row in rows {
        let domain = row.map_err(|e| format!("read domain row: {e}"))?;
        if domain.is_empty() || !domain.contains('.') {
            continue;
        }
        // Check if already cached and not expired
        match get_bimi_cache(conn, &domain) {
            Ok(Some(_)) => continue,
            Ok(None) | Err(_) => domains.push(domain),
        }
    }

    Ok(domains)
}
