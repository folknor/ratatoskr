use std::collections::HashMap;

use db::db::{ReadDbState, ReadError, WriteConn, WriterPool};

fn optional_read<T>(result: Result<T, ReadError>, context: &str) -> Result<Option<T>, String> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
        Err(error) => Err(format!("{context}: {error}")),
    }
}

/// Synchronous version: update account sync state (history_id column).
pub fn update_account_sync_state(
    conn: &WriteConn<'_>,
    account_id: &str,
    history_id: &str,
) -> Result<(), String> {
    db::db::queries_extra::set_account_history_id(conn, account_id, history_id)
}

/// Async version: update account sync state (history_id column).
pub async fn save_account_history_id(
    db: &WriterPool,
    account_id: &str,
    history_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let hid = history_id.to_string();
    db.with_write(move |conn| update_account_sync_state(conn, &aid, &hid))
        .await
}

pub async fn load_account_history_id(
    db: &ReadDbState,
    account_id: &str,
) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    db.with_read(move |conn| db::db::queries_extra::get_account_history_id(conn, &aid))
        .await
}

pub async fn save_jmap_sync_state(
    db: &WriterPool,
    account_id: &str,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    save_jmap_sync_state_for(db, account_id, None, state_type, state).await
}

pub async fn load_jmap_sync_state(
    db: &ReadDbState,
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
    db: &WriterPool,
    account_id: &str,
    shared_account_id: Option<&str>,
    state_type: &str,
    state: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let said = shared_account_id.map(String::from);
    let st = state_type.to_string();
    let sv = state.to_string();

    db.with_write(move |conn| {
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
    db: &ReadDbState,
    account_id: &str,
    shared_account_id: Option<&str>,
    state_type: &str,
) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    let said = shared_account_id.map(String::from);
    let st = state_type.to_string();

    db.with_read(move |conn| {
        optional_read(
            conn.query_row(
                "SELECT state FROM jmap_sync_state \
             WHERE account_id = ?1 AND type = ?2 \
             AND COALESCE(shared_account_id, '') = COALESCE(?3, '')",
                rusqlite::params![aid, st, said],
                |row| row.get::<_, String>("state"),
            ),
            "load jmap sync state",
        )
    })
    .await
}

pub async fn save_graph_delta_token(
    db: &WriterPool,
    account_id: &str,
    folder_id: &str,
    delta_link: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();
    let dl = delta_link.to_string();

    db.with_write(move |conn| {
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
    db: &ReadDbState,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    let aid = account_id.to_string();

    db.with_read(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT folder_id, delta_link FROM graph_folder_delta_tokens \
                 WHERE account_id = ?1",
            )
            .map_err(|e| format!("prepare: {e}"))?;
        stmt.query_map(rusqlite::params![aid], |row| {
            Ok((
                row.get::<_, String>("folder_id")?,
                row.get::<_, String>("delta_link")?,
            ))
        })
        .map_err(|e| format!("query: {e}"))?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|e| format!("collect: {e}"))
    })
    .await
}

pub async fn delete_graph_delta_token(
    db: &WriterPool,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let fid = folder_id.to_string();

    db.with_write(move |conn| {
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

async fn increment_provider_sync_cycle(
    db: &WriterPool,
    account_id: &str,
    provider_key: &'static str,
    provider_label: &'static str,
    overflow_cycle: u32,
) -> Result<u32, String> {
    let key = format!("{provider_key}_sync_cycle:{account_id}");

    db.with_write(move |conn| {
        let tx = conn
            .transaction()
            .map_err(|e| format!("begin {provider_key} sync cycle tx: {e}"))?;
        let stored = db::db::queries::get_setting(&tx, &key)?;
        let current = match stored.as_deref() {
            Some(value) => match value.parse::<u32>() {
                Ok(parsed) => parsed,
                Err(e) => {
                    log::warn!(
                        "Invalid {provider_label} delta cycle value {value:?} for {key}: {e}"
                    );
                    0
                }
            },
            None => 0,
        };
        // Counts delta syncs only. Initial sync runs its own bootstrap work.
        // Overflow lands on the provider's contact-cadence boundary so a wrap
        // still runs the rare contact tier instead of skipping it.
        let next = current.checked_add(1).unwrap_or(overflow_cycle);
        db::db::queries::set_setting(&tx, &key, &next.to_string())?;
        tx.commit()
            .map_err(|e| format!("commit {provider_key} sync cycle tx: {e}"))?;
        Ok(next)
    })
    .await
}

pub async fn increment_graph_sync_cycle(db: &WriterPool, account_id: &str) -> Result<u32, String> {
    increment_provider_sync_cycle(db, account_id, "graph", "Graph", 20).await
}

// ── Shared mailbox sync state management ─────────────────

/// Set the resolved email address for a shared mailbox.
///
/// Used by JMAP principal resolution to associate a JMAP shared account
/// with its owner's email address for send identity auto-selection.
pub async fn set_shared_mailbox_email(
    db: &WriterPool,
    account_id: &str,
    mailbox_id: &str,
    email: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();
    let em = email.to_string();

    db.with_write(move |conn| {
        conn.execute(
            "UPDATE shared_mailboxes \
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
    db: &ReadDbState,
    account_id: &str,
    mailbox_id: &str,
) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    let mid = mailbox_id.to_string();

    db.with_read(move |conn| {
        optional_read(
            conn.query_row(
                "SELECT email_address FROM shared_mailboxes \
             WHERE account_id = ?1 AND mailbox_id = ?2",
                rusqlite::params![aid, mid],
                |row| row.get(0),
            ),
            "get shared mailbox email",
        )
        .map(std::option::Option::flatten)
    })
    .await
}
