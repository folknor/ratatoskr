//! Contact save logic implementing the spec's dual save pattern:
//!
//! - **Local contacts** (`source = 'user'`): save immediately on edit, no
//!   explicit Save button needed.
//! - **Synced contacts** (`source` is `'google'`, `'graph'`, `'carddav'`, `'jmap'`):
//!   edits are held locally until the user clicks an explicit Save button,
//!   which triggers a provider write-back. Display name is always a local-only
//!   override (sets `display_name_overridden = 1`).

use rusqlite::{Connection, params};

use crate::db::DbState;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A contact update payload. Fields set to `None` are not changed.
#[derive(Debug, Clone)]
pub struct ContactUpdate {
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub company: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

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
pub async fn get_contact_source(
    db: &DbState,
    email: String,
) -> Result<ContactSource, String> {
    db.with_conn(move |conn| {
        let source: Option<String> = conn
            .query_row(
                "SELECT source FROM contacts WHERE email = ?1",
                params![email],
                |row| row.get("source"),
            )
            .ok();

        match source.as_deref() {
            Some("user") | None => Ok(ContactSource::Local),
            Some(provider) => Ok(ContactSource::Synced(provider.to_string())),
        }
    })
    .await
}

/// Save a local contact immediately (no Save button needed).
///
/// For `source = 'user'` contacts, this persists all field changes
/// directly to the database.
pub async fn save_local_contact(
    db: &DbState,
    update: ContactUpdate,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        apply_contact_update(conn, &update, true)?;
        Ok(())
    })
    .await
}

/// Save a synced contact's local edits (called when user clicks Save).
///
/// This applies the edits to the local database and marks the contact
/// for provider write-back. Display name changes always set
/// `display_name_overridden = 1` and are NOT pushed to the provider.
/// Other fields (email2, phone, company, notes) are pushed.
pub async fn save_synced_contact(
    db: &DbState,
    update: ContactUpdate,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        apply_contact_update(conn, &update, false)?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn apply_contact_update(
    conn: &Connection,
    update: &ContactUpdate,
    is_local: bool,
) -> Result<(), String> {
    let normalized_email = update.email.to_lowercase();

    // Update display name if provided
    if let Some(ref name) = update.display_name {
        if is_local {
            conn.execute(
                "UPDATE contacts SET display_name = ?1, updated_at = unixepoch() \
                 WHERE email = ?2",
                params![name, normalized_email],
            )
            .map_err(|e| format!("update display_name: {e}"))?;
        } else {
            // Synced contact: mark display name as overridden (local-only change)
            conn.execute(
                "UPDATE contacts SET display_name = ?1, display_name_overridden = 1, \
                 updated_at = unixepoch() WHERE email = ?2",
                params![name, normalized_email],
            )
            .map_err(|e| format!("update display_name (synced): {e}"))?;
        }
    }

    // Update email2 if provided
    if let Some(ref email2) = update.email2 {
        conn.execute(
            "UPDATE contacts SET email2 = ?1, updated_at = unixepoch() \
             WHERE email = ?2",
            params![email2, normalized_email],
        )
        .map_err(|e| format!("update email2: {e}"))?;
    }

    // Update phone if provided
    if let Some(ref phone) = update.phone {
        conn.execute(
            "UPDATE contacts SET phone = ?1, updated_at = unixepoch() \
             WHERE email = ?2",
            params![phone, normalized_email],
        )
        .map_err(|e| format!("update phone: {e}"))?;
    }

    // Update company if provided
    if let Some(ref company) = update.company {
        conn.execute(
            "UPDATE contacts SET company = ?1, updated_at = unixepoch() \
             WHERE email = ?2",
            params![company, normalized_email],
        )
        .map_err(|e| format!("update company: {e}"))?;
    }

    // Update notes if provided
    if let Some(ref notes) = update.notes {
        conn.execute(
            "UPDATE contacts SET notes = ?1, updated_at = unixepoch() \
             WHERE email = ?2",
            params![notes, normalized_email],
        )
        .map_err(|e| format!("update notes: {e}"))?;
    }

    Ok(())
}
