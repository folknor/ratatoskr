use crate::db::ReadConn;

#[derive(Debug, Clone)]
pub struct SeenAddressStats {
    pub email: String,
    pub display_name: Option<String>,
    pub times_sent_to: i64,
    pub times_received_from: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

pub fn get_seen_address_stats_sync(
    conn: &ReadConn<'_>,
    email: &str,
) -> Result<Option<SeenAddressStats>, String> {
    let normalized = email.to_lowercase();
    match conn.query_row(
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
    ) {
        Ok(stats) => Ok(Some(stats)),
        Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
