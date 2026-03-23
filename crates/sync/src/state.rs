use std::collections::HashMap;

use ratatoskr_db::db::DbState;
use rusqlite::{Connection, OptionalExtension};

/// Synchronous version: update account sync state (history_id column).
pub fn update_account_sync_state(
    conn: &Connection,
    account_id: &str,
    history_id: &str,
) -> Result<(), String> {
    conn.execute(
        "UPDATE accounts SET history_id = ?1, initial_sync_completed = 1 WHERE id = ?2",
        rusqlite::params![history_id, account_id],
    )
    .map_err(|e| format!("update account sync state: {e}"))?;
    Ok(())
}

/// Async version: update account sync state (history_id column).
pub async fn save_account_history_id(
    db: &DbState,
    account_id: &str,
    history_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let hid = history_id.to_string();
    db.with_conn(move |conn| update_account_sync_state(conn, &aid, &hid))
        .await
}

pub async fn load_account_history_id(
    db: &DbState,
    account_id: &str,
) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT history_id FROM accounts WHERE id = ?1",
            rusqlite::params![aid],
            |row| row.get("history_id"),
        )
        .optional()
        .map_err(|e| format!("read history_id: {e}"))
    })
    .await
}

pub async fn save_jmap_sync_state(
    db: &DbState,
    account_id: &str,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    save_jmap_sync_state_for(db, account_id, None, state_type, state).await
}

pub async fn load_jmap_sync_state(
    db: &DbState,
    account_id: &str,
    state_type: &str,
) -> Result<Option<String>, String> {
    load_jmap_sync_state_for(db, account_id, None, state_type).await
}

/// Save JMAP sync state for a specific (possibly shared) account.
///
/// `shared_account_id` is `None` for the primary account, `Some(jmap_id)` for
/// a shared account discovered from the JMAP Session.
pub async fn save_jmap_sync_state_for(
    db: &DbState,
    account_id: &str,
    shared_account_id: Option<&str>,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let said = shared_account_id.map(String::from);
    let st = state_type.to_string();
    let sv = state.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO jmap_sync_state (account_id, shared_account_id, type, state, updated_at) \
             VALUES (?1, ?2, ?3, ?4, strftime('%s', 'now')) \
             ON CONFLICT(account_id, COALESCE(shared_account_id, ''), type) \
             DO UPDATE SET state = ?4, updated_at = strftime('%s', 'now')",
            rusqlite::params![aid, said, st, sv],
        )
        .map_err(|e| format!("save jmap sync state: {e}"))?;
        Ok(())
    })
    .await
}

/// Load JMAP sync state for a specific (possibly shared) account.
pub async fn load_jmap_sync_state_for(
    db: &DbState,
    account_id: &str,
    shared_account_id: Option<&str>,
    state_type: &str,
) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    let said = shared_account_id.map(String::from);
    let st = state_type.to_string();

    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT state FROM jmap_sync_state \
             WHERE account_id = ?1 AND type = ?2 \
             AND COALESCE(shared_account_id, '') = COALESCE(?3, '')",
            rusqlite::params![aid, st, said],
            |row| row.get::<_, String>("state"),
        )
        .optional()
        .map_err(|e| format!("load jmap sync state: {e}"))
    })
    .await
}

pub async fn save_graph_delta_token(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    delta_link: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();
    let dl = delta_link.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO graph_folder_delta_tokens \
             (account_id, folder_id, delta_link, updated_at) \
             VALUES (?1, ?2, ?3, strftime('%s', 'now'))",
            rusqlite::params![aid, fid, dl],
        )
        .map_err(|e| format!("save delta token: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn load_graph_delta_tokens(
    db: &DbState,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    let aid = account_id.to_string();

    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT folder_id, delta_link FROM graph_folder_delta_tokens \
                 WHERE account_id = ?1",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_map(rusqlite::params![aid], |row| {
            Ok((row.get::<_, String>("folder_id")?, row.get::<_, String>("delta_link")?))
        })
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| format!("collect: {e}"))
    })
    .await
}

pub async fn delete_graph_delta_token(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM graph_folder_delta_tokens \
             WHERE account_id = ?1 AND folder_id = ?2",
            rusqlite::params![aid, fid],
        )
        .map_err(|e| format!("delete delta token: {e}"))?;
        Ok(())
    })
    .await
}

// ── Graph contact delta tokens ────────────────────────────

pub async fn save_graph_contact_delta_token(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    delta_link: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();
    let dl = delta_link.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO graph_contact_delta_tokens \
             (account_id, folder_id, delta_link, updated_at) \
             VALUES (?1, ?2, ?3, strftime('%s', 'now'))",
            rusqlite::params![aid, fid, dl],
        )
        .map_err(|e| format!("save contact delta token: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn load_graph_contact_delta_tokens(
    db: &DbState,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    let aid = account_id.to_string();

    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT folder_id, delta_link FROM graph_contact_delta_tokens \
                 WHERE account_id = ?1",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_map(rusqlite::params![aid], |row| {
            Ok((row.get::<_, String>("folder_id")?, row.get::<_, String>("delta_link")?))
        })
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| format!("collect: {e}"))
    })
    .await
}

pub async fn delete_graph_contact_delta_token(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM graph_contact_delta_tokens \
             WHERE account_id = ?1 AND folder_id = ?2",
            rusqlite::params![aid, fid],
        )
        .map_err(|e| format!("delete contact delta token: {e}"))?;
        Ok(())
    })
    .await
}

// ── Google People API contact sync tokens ────────────────

pub async fn save_google_contacts_sync_token(
    db: &DbState,
    account_id: &str,
    sync_token: &str,
) -> Result<(), String> {
    let key = format!("google_contacts_sync_token:{account_id}");
    let val = sync_token.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, val],
        )
        .map_err(|e| format!("save google contacts sync token: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn load_google_contacts_sync_token(
    db: &DbState,
    account_id: &str,
) -> Result<Option<String>, String> {
    let key = format!("google_contacts_sync_token:{account_id}");

    db.with_conn(move |conn| ratatoskr_db::db::queries::get_setting(conn, &key))
        .await
}

pub async fn delete_google_contacts_sync_token(
    db: &DbState,
    account_id: &str,
) -> Result<(), String> {
    let key = format!("google_contacts_sync_token:{account_id}");

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM settings WHERE key = ?1",
            rusqlite::params![key],
        )
        .map_err(|e| format!("delete google contacts sync token: {e}"))?;
        Ok(())
    })
    .await
}

// ── Google People API otherContacts sync tokens ──────────

pub async fn save_google_other_contacts_sync_token(
    db: &DbState,
    account_id: &str,
    sync_token: &str,
) -> Result<(), String> {
    let key = format!("google_other_contacts_sync_token:{account_id}");
    let val = sync_token.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            rusqlite::params![key, val],
        )
        .map_err(|e| format!("save google other contacts sync token: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn load_google_other_contacts_sync_token(
    db: &DbState,
    account_id: &str,
) -> Result<Option<String>, String> {
    let key = format!("google_other_contacts_sync_token:{account_id}");

    db.with_conn(move |conn| ratatoskr_db::db::queries::get_setting(conn, &key))
        .await
}

pub async fn delete_google_other_contacts_sync_token(
    db: &DbState,
    account_id: &str,
) -> Result<(), String> {
    let key = format!("google_other_contacts_sync_token:{account_id}");

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM settings WHERE key = ?1",
            rusqlite::params![key],
        )
        .map_err(|e| format!("delete google other contacts sync token: {e}"))?;
        Ok(())
    })
    .await
}

// ── Graph shared mailbox delta tokens ────────────────────

pub async fn save_shared_mailbox_delta_token(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
    folder_id: &str,
    delta_link: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();
    let fid = folder_id.to_string();
    let dl = delta_link.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO graph_shared_mailbox_delta_tokens \
             (account_id, mailbox_id, folder_id, delta_link, updated_at) \
             VALUES (?1, ?2, ?3, ?4, strftime('%s', 'now'))",
            rusqlite::params![aid, mid, fid, dl],
        )
        .map_err(|e| format!("save shared mailbox delta token: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn load_shared_mailbox_delta_tokens(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
) -> Result<HashMap<String, String>, String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();

    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT folder_id, delta_link FROM graph_shared_mailbox_delta_tokens \
                 WHERE account_id = ?1 AND mailbox_id = ?2",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_map(rusqlite::params![aid, mid], |row| {
            Ok((row.get::<_, String>("folder_id")?, row.get::<_, String>("delta_link")?))
        })
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| format!("collect: {e}"))
    })
    .await
}

pub async fn delete_shared_mailbox_delta_tokens(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM graph_shared_mailbox_delta_tokens \
             WHERE account_id = ?1 AND mailbox_id = ?2",
            rusqlite::params![aid, mid],
        )
        .map_err(|e| format!("delete shared mailbox delta tokens: {e}"))?;
        Ok(())
    })
    .await
}

/// Delete a single delta token for a specific folder within a shared mailbox.
pub async fn delete_shared_mailbox_delta_token(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();
    let fid = folder_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM graph_shared_mailbox_delta_tokens \
             WHERE account_id = ?1 AND mailbox_id = ?2 AND folder_id = ?3",
            rusqlite::params![aid, mid, fid],
        )
        .map_err(|e| format!("delete shared mailbox delta token: {e}"))?;
        Ok(())
    })
    .await
}

// ── Shared mailbox sync state management ─────────────────

/// A shared mailbox that is tracked for sync.
#[derive(Debug, Clone)]
pub struct SharedMailboxSyncEntry {
    pub mailbox_id: String,
    pub display_name: Option<String>,
    pub last_synced_at: Option<i64>,
    pub sync_error: Option<String>,
}

/// Get all enabled shared mailboxes for an account.
pub async fn get_enabled_shared_mailboxes(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<SharedMailboxSyncEntry>, String> {
    let aid = account_id.to_string();

    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT mailbox_id, display_name, last_synced_at, sync_error \
                 FROM shared_mailbox_sync_state \
                 WHERE account_id = ?1 AND is_sync_enabled = 1",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_map(rusqlite::params![aid], |row| {
            Ok(SharedMailboxSyncEntry {
                mailbox_id: row.get("mailbox_id")?,
                display_name: row.get("display_name")?,
                last_synced_at: row.get("last_synced_at")?,
                sync_error: row.get("sync_error")?,
            })
        })
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect: {e}"))
    })
    .await
}

/// Update the sync status for a shared mailbox after a sync attempt.
pub async fn update_shared_mailbox_sync_status(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
    last_synced_at: i64,
    sync_error: Option<&str>,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();
    let err = sync_error.map(String::from);

    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE shared_mailbox_sync_state \
             SET last_synced_at = ?1, sync_error = ?2 \
             WHERE account_id = ?3 AND mailbox_id = ?4",
            rusqlite::params![last_synced_at, err, aid, mid],
        )
        .map_err(|e| format!("update shared mailbox sync status: {e}"))?;
        Ok(())
    })
    .await
}

/// Enable sync for a shared mailbox, inserting a row if it doesn't exist.
pub async fn enable_shared_mailbox_sync(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
    display_name: Option<&str>,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();
    let dn = display_name.map(String::from);

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO shared_mailbox_sync_state \
             (account_id, mailbox_id, display_name, is_sync_enabled) \
             VALUES (?1, ?2, ?3, 1) \
             ON CONFLICT(account_id, mailbox_id) DO UPDATE \
             SET is_sync_enabled = 1, display_name = COALESCE(?3, display_name)",
            rusqlite::params![aid, mid, dn],
        )
        .map_err(|e| format!("enable shared mailbox sync: {e}"))?;
        Ok(())
    })
    .await
}

/// Disable sync for a shared mailbox.
pub async fn disable_shared_mailbox_sync(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE shared_mailbox_sync_state \
             SET is_sync_enabled = 0 \
             WHERE account_id = ?1 AND mailbox_id = ?2",
            rusqlite::params![aid, mid],
        )
        .map_err(|e| format!("disable shared mailbox sync: {e}"))?;
        Ok(())
    })
    .await
}

/// Disable sync for a shared mailbox and record an error message.
pub async fn disable_shared_mailbox_sync_with_error(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
    error: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();
    let err = error.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE shared_mailbox_sync_state \
             SET is_sync_enabled = 0, sync_error = ?3 \
             WHERE account_id = ?1 AND mailbox_id = ?2",
            rusqlite::params![aid, mid, err],
        )
        .map_err(|e| format!("disable shared mailbox sync with error: {e}"))?;
        Ok(())
    })
    .await
}

/// Get all shared mailbox IDs for an account (enabled and disabled).
///
/// Used to detect revoked access — compare against currently-available
/// shared accounts to find entries that should be disabled.
pub async fn get_all_shared_mailbox_ids(
    db: &DbState,
    account_id: &str,
) -> Result<Vec<String>, String> {
    let aid = account_id.to_string();

    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT mailbox_id FROM shared_mailbox_sync_state \
                 WHERE account_id = ?1",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_map(rusqlite::params![aid], |row| row.get(0))
            .map_err(|e| format!("query: {e}"))?
            .collect::<Result<Vec<String>, _>>()
            .map_err(|e| format!("collect: {e}"))
    })
    .await
}

/// Set the resolved email address for a shared mailbox.
///
/// Used by JMAP principal resolution to associate a JMAP shared account
/// with its owner's email address for send identity auto-selection.
pub async fn set_shared_mailbox_email(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
    email: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();
    let em = email.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE shared_mailbox_sync_state \
             SET email_address = ?3 \
             WHERE account_id = ?1 AND mailbox_id = ?2",
            rusqlite::params![aid, mid, em],
        )
        .map_err(|e| format!("set shared mailbox email: {e}"))?;
        Ok(())
    })
    .await
}

/// Get the email address for a shared mailbox, if resolved.
pub async fn get_shared_mailbox_email(
    db: &DbState,
    account_id: &str,
    mailbox_id: &str,
) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();

    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT email_address FROM shared_mailbox_sync_state \
             WHERE account_id = ?1 AND mailbox_id = ?2",
            rusqlite::params![aid, mid],
            |row| row.get(0),
        )
        .optional()
        .map_err(|e| format!("get shared mailbox email: {e}"))
        .map(std::option::Option::flatten)
    })
    .await
}
