//! Seen-address integration for the contacts domain.
//!
//! SQL lives in `db::queries_extra::contacts`. This module re-exports
//! the `seen` crate's public API and provides async wrappers.

use crate::db::DbState;

// Re-export the `seen` crate's public API through core.
pub use seen::{
    AddressObservation, Direction, MessageAddresses, SeenAddressMatch, backfill_seen_addresses,
    ingest_from_messages,
};

// Re-export the stats type from db.
pub use crate::db::queries_extra::contacts::SeenAddressStats;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Promote a seen address to a full contact.
pub async fn promote_seen_to_contact(db: &DbState, email: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        crate::db::queries_extra::contacts::promote_seen_to_contact_sync(conn, &email)
    })
    .await
}

/// Get aggregated statistics for a seen email address.
pub async fn get_seen_address_stats(
    db: &DbState,
    email: String,
) -> Result<Option<SeenAddressStats>, String> {
    db.with_conn(move |conn| {
        crate::db::queries_extra::contacts::get_seen_address_stats_sync(conn, &email)
    })
    .await
}
