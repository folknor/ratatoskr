//! Contact save logic implementing the spec's dual save pattern:
//!
//! - **Local contacts** (`source = 'user'`): save immediately on edit.
//! - **Synced contacts**: edits held locally until explicit Save;
//!   display name is always a local-only override.
//!
//! SQL lives in `db::queries_extra::contacts`. This module provides
//! async wrappers and domain types.

use crate::db::DbState;

// Re-export the storage parameter type so existing callers keep working.
pub use db::db::queries_extra::contacts::ContactUpdate;

// ---------------------------------------------------------------------------
// Domain types (stay in core)
// ---------------------------------------------------------------------------

/// Whether a contact is local or synced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactSource {
    Local,
    Synced(String), // the provider source: "google", "graph", "carddav", "jmap"
}

/// The result of a synced contact write-back attempt.
#[derive(Debug, Clone)]
pub enum WriteBackResult {
    /// Provider accepted the update.
    Success,
    /// Provider rejected the update (e.g. read-only GAL entry).
    Rejected(String),
    /// No write-back needed (local contact or display-name-only change).
    NotNeeded,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Determine whether a contact is local or synced.
pub async fn get_contact_source(db: &DbState, email: String) -> Result<ContactSource, String> {
    db.with_conn(move |conn| {
        let source = db::db::queries_extra::contacts::get_contact_source_sync(conn, &email)?;
        match source.as_deref() {
            Some("user") | None => Ok(ContactSource::Local),
            Some(provider) => Ok(ContactSource::Synced(provider.to_string())),
        }
    })
    .await
}

/// Save a local contact immediately (no Save button needed).
pub async fn save_local_contact(db: &DbState, update: ContactUpdate) -> Result<(), String> {
    db.with_conn(move |conn| {
        db::db::queries_extra::contacts::save_local_contact_fields_sync(conn, &update)
    })
    .await
}

/// Save a synced contact's local edits (called when user clicks Save).
pub async fn save_synced_contact(db: &DbState, update: ContactUpdate) -> Result<(), String> {
    db.with_conn(move |conn| {
        db::db::queries_extra::contacts::save_synced_contact_fields_sync(conn, &update)
    })
    .await
}
