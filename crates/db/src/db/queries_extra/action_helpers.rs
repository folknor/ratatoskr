//! Lightweight DB helpers for the email action pipeline.
//!
//! These are small query/mutation primitives used by `core::actions::*` modules.
//! Keeping them here avoids inline SQL in the action handlers.

use rusqlite::{Connection, params};

/// Check whether a thread exists.
pub fn thread_exists_sync(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM threads WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| e.to_string())
}

/// Get the label_kind for a label ("tag" or "container"). Returns None if not found.
pub fn get_label_kind_sync(
    conn: &Connection,
    label_id: &str,
    account_id: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT label_kind FROM labels WHERE id = ?1 AND account_id = ?2 LIMIT 1",
        params![label_id, account_id],
        |row| row.get(0),
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        _ => Err(e.to_string()),
    })
}

/// Look up contact metadata by ID (source, server_id, account_id).
pub fn get_contact_meta_by_id_sync(
    conn: &Connection,
    contact_id: &str,
) -> Result<Option<(Option<String>, Option<String>, Option<String>)>, String> {
    conn.query_row(
        "SELECT source, server_id, account_id FROM contacts WHERE id = ?1",
        params![contact_id],
        |row| {
            Ok((
                row.get::<_, Option<String>>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        },
    )
    .map(Some)
    .or_else(|e| match e {
        rusqlite::Error::QueryReturnedNoRows => Ok(None),
        _ => Err(e.to_string()),
    })
}

/// Set snooze state on a thread.
pub fn snooze_thread_sync(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
    until: i64,
) -> Result<(), String> {
    conn.execute(
        "UPDATE threads SET is_snoozed = 1, snooze_until = ?3 \
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id, until],
    )
    .map_err(|e| format!("snooze: {e}"))?;
    Ok(())
}

/// Clear snooze state on a thread.
pub fn unsnooze_thread_sync(
    conn: &Connection,
    account_id: &str,
    thread_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE threads SET is_snoozed = 0, snooze_until = NULL \
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, thread_id],
    )
    .map_err(|e| format!("unsnooze: {e}"))?;
    Ok(())
}

/// Upsert a folder/label row from a provider mutation result.
#[allow(clippy::too_many_arguments)]
pub fn upsert_folder_from_mutation_sync(
    conn: &Connection,
    label_id: &str,
    account_id: &str,
    name: &str,
    folder_type: &str,
    color_bg: Option<&str>,
    color_fg: Option<&str>,
    path: Option<&str>,
    special_use: Option<&str>,
    parent_label_id: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO labels (id, account_id, name, type, color_bg, color_fg, \
         imap_folder_path, imap_special_use, parent_label_id, label_kind) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, 'container') \
         ON CONFLICT(account_id, id) DO UPDATE SET \
           name = ?3, type = ?4, color_bg = ?5, color_fg = ?6, \
           imap_folder_path = ?7, imap_special_use = ?8, \
           parent_label_id = ?9, label_kind = 'container'",
        params![
            label_id, account_id, name, folder_type, color_bg, color_fg, path, special_use,
            parent_label_id,
        ],
    )
    .map_err(|e| format!("upsert folder: {e}"))?;
    Ok(())
}

/// Delete a folder/label and its thread_labels associations.
pub fn delete_folder_sync(
    conn: &Connection,
    account_id: &str,
    label_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM thread_labels WHERE account_id = ?1 AND label_id = ?2",
        params![account_id, label_id],
    )
    .map_err(|e| format!("delete folder thread_labels: {e}"))?;
    conn.execute(
        "DELETE FROM labels WHERE account_id = ?1 AND id = ?2",
        params![account_id, label_id],
    )
    .map_err(|e| format!("delete folder label: {e}"))?;
    Ok(())
}
