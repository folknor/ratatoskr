/// A contact update payload. Fields set to `None` are not changed.
/// Double-option fields (`Option<Option<String>>`) use outer None = "no change",
/// inner None = "clear field".
#[derive(Debug, Clone)]
pub struct ContactUpdate {
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub company: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

/// Save a local contact's fields. Does not set `display_name_overridden`.
pub fn save_local_contact_fields_sync(
    conn: &rusqlite::Connection,
    update: &ContactUpdate,
) -> Result<(), String> {
    apply_contact_update_inner(conn, update, true)
}

/// Save a synced contact's fields. Sets `display_name_overridden = 1`
/// for display name changes (local-only override, not pushed to provider).
pub fn save_synced_contact_fields_sync(
    conn: &rusqlite::Connection,
    update: &ContactUpdate,
) -> Result<(), String> {
    apply_contact_update_inner(conn, update, false)
}

/// Look up the raw source string for a contact by email.
/// Returns `None` if no contact with that email exists.
pub fn get_contact_source_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT source FROM contacts WHERE email = ?1",
        rusqlite::params![email],
        |row| row.get("source"),
    )
    .ok()
    .map_or(Ok(None), |v| Ok(Some(v)))
}

fn apply_contact_update_inner(
    conn: &rusqlite::Connection,
    update: &ContactUpdate,
    is_local: bool,
) -> Result<(), String> {
    let normalized_email = update.email.to_lowercase();

    if let Some(ref name) = update.display_name {
        if is_local {
            conn.execute(
                "UPDATE contacts SET display_name = ?1, updated_at = unixepoch() \
                 WHERE email = ?2",
                rusqlite::params![name, normalized_email],
            )
            .map_err(|e| format!("update display_name: {e}"))?;
        } else {
            conn.execute(
                "UPDATE contacts SET display_name = ?1, display_name_overridden = 1, \
                 updated_at = unixepoch() WHERE email = ?2",
                rusqlite::params![name, normalized_email],
            )
            .map_err(|e| format!("update display_name (synced): {e}"))?;
        }
    }

    if let Some(ref email2) = update.email2 {
        conn.execute(
            "UPDATE contacts SET email2 = ?1, updated_at = unixepoch() WHERE email = ?2",
            rusqlite::params![email2, normalized_email],
        )
        .map_err(|e| format!("update email2: {e}"))?;
    }

    if let Some(ref phone) = update.phone {
        conn.execute(
            "UPDATE contacts SET phone = ?1, updated_at = unixepoch() WHERE email = ?2",
            rusqlite::params![phone, normalized_email],
        )
        .map_err(|e| format!("update phone: {e}"))?;
    }

    if let Some(ref company) = update.company {
        conn.execute(
            "UPDATE contacts SET company = ?1, updated_at = unixepoch() WHERE email = ?2",
            rusqlite::params![company, normalized_email],
        )
        .map_err(|e| format!("update company: {e}"))?;
    }

    if let Some(ref notes) = update.notes {
        conn.execute(
            "UPDATE contacts SET notes = ?1, updated_at = unixepoch() WHERE email = ?2",
            rusqlite::params![notes, normalized_email],
        )
        .map_err(|e| format!("update notes: {e}"))?;
    }

    Ok(())
}
