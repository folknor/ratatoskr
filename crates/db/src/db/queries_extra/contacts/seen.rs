/// Aggregated stats for a seen address across all accounts.
#[derive(Debug, Clone)]
pub struct SeenAddressStats {
    pub email: String,
    pub display_name: Option<String>,
    pub times_sent_to: i64,
    pub times_received_from: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// Promote a seen address to a contact with source = 'user'.
/// No-op if a contact with that email already exists.
pub fn promote_seen_to_contact_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<(), String> {
    let normalized = email.to_lowercase();

    // Check if already a contact
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) AS cnt FROM contacts WHERE email = ?1",
            rusqlite::params![normalized],
            |row| row.get::<_, i64>("cnt"),
        )
        .map_err(|e| format!("check contact exists: {e}"))?
        > 0;

    if exists {
        return Ok(());
    }

    // Get display name from seen_addresses
    let display_name: Option<String> = conn
        .query_row(
            "SELECT display_name FROM seen_addresses \
             WHERE email = ?1 \
             ORDER BY last_seen_at DESC LIMIT 1",
            rusqlite::params![normalized],
            |row| row.get("display_name"),
        )
        .ok()
        .flatten();

    let id = format!("promoted-{normalized}");
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, source) \
         VALUES (?1, ?2, ?3, 'user')",
        rusqlite::params![id, normalized, display_name],
    )
    .map_err(|e| format!("promote seen to contact: {e}"))?;

    Ok(())
}

/// Get aggregated stats for a seen address across all accounts.
pub fn get_seen_address_stats_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<Option<SeenAddressStats>, String> {
    let normalized = email.to_lowercase();
    conn.query_row(
        "SELECT email, display_name,
                SUM(times_sent_to) AS total_sent,
                SUM(times_received_from) AS total_received,
                MIN(first_seen_at) AS first_seen,
                MAX(last_seen_at) AS last_seen
         FROM seen_addresses
         WHERE email = ?1
         GROUP BY email",
        rusqlite::params![normalized],
        |row| {
            Ok(SeenAddressStats {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                times_sent_to: row.get("total_sent")?,
                times_received_from: row.get("total_received")?,
                first_seen_at: row.get("first_seen")?,
                last_seen_at: row.get("last_seen")?,
            })
        },
    )
    .map_err(|e| e.to_string())
    .map(Some)
    .or(Ok(None))
}
