//! Lightweight DB helpers for the email action pipeline.
//!
//! These are small query/mutation primitives used by `core::actions::*` modules.
//! Keeping them here avoids inline SQL in the action handlers.

use crate::db::{ReadConn, WriteTarget};
use rusqlite::params;

/// Check whether a thread exists.
pub fn thread_exists_sync(
    conn: &ReadConn<'_>,
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

/// Check whether a tag label exists for an account.
pub fn label_exists_sync(
    conn: &ReadConn<'_>,
    label_id: &str,
    account_id: &str,
) -> Result<bool, String> {
    conn.query_row(
        "SELECT COUNT(*) FROM labels WHERE id = ?1 AND account_id = ?2",
        params![label_id, account_id],
        |row| row.get::<_, i64>(0),
    )
    .map(|n| n > 0)
    .map_err(|e| e.to_string())
}

/// Identity fields needed to route a contact mutation through the right provider.
pub struct ContactMeta {
    pub source: Option<String>,
    pub server_id: Option<String>,
    pub account_id: Option<String>,
}

/// Look up contact metadata by ID. Returns `None` when the contact row is missing.
pub fn get_contact_meta_by_id_sync(
    conn: &ReadConn<'_>,
    contact_id: &str,
) -> Result<Option<ContactMeta>, String> {
    conn.query_row(
        "SELECT source, server_id, account_id FROM contacts WHERE id = ?1",
        params![contact_id],
        |row| {
            Ok(ContactMeta {
                source: row.get(0)?,
                server_id: row.get(1)?,
                account_id: row.get(2)?,
            })
        },
    )
    .map(Some)
    .or_else(|e| match e {
        crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        _ => Err(e.to_string()),
    })
}

/// Set snooze state on a thread.
pub fn snooze_thread_sync(
    conn: &impl WriteTarget,
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
    conn: &impl WriteTarget,
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

// `upsert_folder_from_mutation_sync` retired with the `ProviderOps` folder
// CRUD surface (B6). The folder CRUD action handlers now upsert their
// `folders` row directly through `insert_folders_batch` from the engine's
// returned `ContainerId`, and the list sync writes rows via
// `bifrost::containers::sync_containers`.

/// Get all message IDs for an account.
pub fn get_message_ids_for_account_sync(
    conn: &ReadConn<'_>,
    account_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare("SELECT id FROM messages WHERE account_id = ?1")
        .map_err(|e| format!("prepare resync message query: {e}"))?;
    stmt.query_map(params![account_id], |row| row.get::<_, String>(0))
        .map_err(|e| format!("query resync message ids: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect resync message ids: {e}"))
}

/// Delete all threads for an account within a transaction.
pub fn delete_threads_for_account_sync(
    conn: &impl WriteTarget,
    account_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM threads WHERE account_id = ?1",
        params![account_id],
    )
    .map_err(|e| format!("delete threads for account: {e}"))?;
    Ok(())
}

/// Write back a folder's new parent after a container move succeeded on the
/// provider. A targeted UPDATE of `parent_id` only, rather than a full
/// `insert_folders_batch` upsert, so it does not clobber the row's `name`
/// and other columns the move handler does not carry. A move to root passes
/// `parent_id = None`. Mirrors the create/rename/delete local write-backs so
/// a reparent lands locally instead of waiting on a resync delta that does
/// not reconcile a container `updated` into `folders.parent_id`.
pub fn update_folder_parent_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    folder_id: &str,
    parent_id: Option<&str>,
) -> Result<(), String> {
    conn.execute(
        "UPDATE folders SET parent_id = ?3 WHERE account_id = ?1 AND id = ?2",
        params![account_id, folder_id, parent_id],
    )
    .map_err(|e| format!("update folder parent: {e}"))?;
    Ok(())
}

/// Re-key a path-based (IMAP) folder row after a server-side rename moved its
/// mailbox path.
///
/// IMAP folder storage ids are `folder-{path}`, so a rename (which changes the
/// path) changes the id. The legacy in-place name update left the row keyed by
/// the OLD id, so a later lookup-by-id (delete / move) missed with `NotFound`.
/// This repoints the row and its membership (`message_folders`,
/// `thread_folders`) plus any child folders (`folders.parent_id`) from `old_id`
/// to `new_id`, and refreshes `imap_folder_path` / `name`.
///
/// `PRAGMA defer_foreign_keys` holds FK validation until commit, so the parent
/// PK update can precede the child repoints inside one transaction even though
/// the FKs are not declared `ON UPDATE CASCADE`; by commit every row is
/// consistent. No-op-safe when `old_id == new_id` (only the name is refreshed).
/// The caller owns the surrounding transaction.
pub fn rekey_imap_folder_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    old_id: &str,
    new_id: &str,
    new_native_path: &str,
    new_name: &str,
) -> Result<(), String> {
    if old_id == new_id {
        conn.execute(
            "UPDATE folders SET name = ?3 WHERE account_id = ?1 AND id = ?2",
            params![account_id, old_id, new_name],
        )
        .map_err(|e| format!("rename folder name in place: {e}"))?;
        return Ok(());
    }
    conn.execute("PRAGMA defer_foreign_keys = ON", [])
        .map_err(|e| format!("defer foreign keys for folder rekey: {e}"))?;
    conn.execute(
        "UPDATE folders SET id = ?3, imap_folder_path = ?4, name = ?5 \
         WHERE account_id = ?1 AND id = ?2",
        params![account_id, old_id, new_id, new_native_path, new_name],
    )
    .map_err(|e| format!("rekey folder row: {e}"))?;
    conn.execute(
        "UPDATE message_folders SET folder_id = ?3 WHERE account_id = ?1 AND folder_id = ?2",
        params![account_id, old_id, new_id],
    )
    .map_err(|e| format!("rekey message_folders: {e}"))?;
    conn.execute(
        "UPDATE thread_folders SET folder_id = ?3 WHERE account_id = ?1 AND folder_id = ?2",
        params![account_id, old_id, new_id],
    )
    .map_err(|e| format!("rekey thread_folders: {e}"))?;
    conn.execute(
        "UPDATE folders SET parent_id = ?3 WHERE account_id = ?1 AND parent_id = ?2",
        params![account_id, old_id, new_id],
    )
    .map_err(|e| format!("rekey child folder parents: {e}"))?;
    Ok(())
}

/// Delete a folder and its thread_folders associations.
pub fn delete_folder_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM thread_folders WHERE account_id = ?1 AND folder_id = ?2",
        params![account_id, folder_id],
    )
    .map_err(|e| format!("delete folder thread_folders: {e}"))?;
    conn.execute(
        "DELETE FROM folders WHERE account_id = ?1 AND id = ?2",
        params![account_id, folder_id],
    )
    .map_err(|e| format!("delete folder: {e}"))?;
    Ok(())
}
