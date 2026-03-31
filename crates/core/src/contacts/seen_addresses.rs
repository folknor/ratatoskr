//! Seen-address integration for the contacts domain.
//!
//! This module re-exports and wraps the `seen` crate,
//! providing a unified interface for the contacts system. The `seen`
//! crate handles the low-level address observation tracking; this module
//! bridges it into the contacts domain.
//!
//! The seen crate shares the same DB tables and domain as contacts,
//! so it is logically part of the contacts module even though it remains a
//! separate crate for compilation speed.

// Re-export the core types for consumers.
pub use seen::{
    AddressObservation, Direction, MessageAddresses, SeenAddressMatch, backfill_seen_addresses,
    ingest_from_messages,
};

use rusqlite::{Connection, params};

use crate::db::DbState;

// ---------------------------------------------------------------------------
// Promotion: seen address -> contact
// ---------------------------------------------------------------------------

/// Promote a seen address to a full contact.
///
/// Copies the display name from `seen_addresses` into the `contacts` table
/// with `source = 'user'`. If the email already exists in contacts, this is
/// a no-op (the contact already has higher priority).
pub async fn promote_seen_to_contact(db: &DbState, email: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();

        // Check if already a contact
        let exists: bool = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM contacts WHERE email = ?1",
                params![normalized],
                |row| row.get::<_, i64>("cnt"),
            )
            .unwrap_or(0)
            > 0;

        if exists {
            return Ok(());
        }

        // Get seen address display name
        let display_name: Option<String> = conn
            .query_row(
                "SELECT display_name FROM seen_addresses \
                 WHERE email = ?1 \
                 ORDER BY last_seen_at DESC LIMIT 1",
                params![normalized],
                |row| row.get("display_name"),
            )
            .ok()
            .flatten();

        let id = format!("promoted-{normalized}");
        conn.execute(
            "INSERT INTO contacts (id, email, display_name, source) \
             VALUES (?1, ?2, ?3, 'user')",
            params![id, normalized, display_name],
        )
        .map_err(|e| format!("promote seen to contact: {e}"))?;

        Ok(())
    })
    .await
}

/// Get aggregate stats for a seen address across all accounts.
pub async fn get_seen_address_stats(
    db: &DbState,
    email: String,
) -> Result<Option<SeenAddressStats>, String> {
    db.with_conn(move |conn| get_seen_stats_inner(conn, &email))
        .await
}

/// Aggregate stats for a seen address.
#[derive(Debug, Clone)]
pub struct SeenAddressStats {
    pub email: String,
    pub display_name: Option<String>,
    pub times_sent_to: i64,
    pub times_received_from: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

fn get_seen_stats_inner(
    conn: &Connection,
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
        params![normalized],
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
    .or_else(|_| Ok(None))
}
