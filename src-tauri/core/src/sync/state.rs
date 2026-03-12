use std::collections::HashMap;

use crate::db::DbState;
use rusqlite::OptionalExtension;

pub async fn save_account_history_id(
    db: &DbState,
    account_id: &str,
    history_id: &str,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let hid = history_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE accounts SET history_id = ?1, initial_sync_completed = 1 WHERE id = ?2",
            rusqlite::params![hid, aid],
        )
        .map_err(|e| format!("update history_id: {e}"))?;
        Ok(())
    })
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
            |row| row.get(0),
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
    let aid = account_id.to_string();
    let st = state_type.to_string();
    let sv = state.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO jmap_sync_state (account_id, type, state, updated_at) \
             VALUES (?1, ?2, ?3, strftime('%s', 'now'))",
            rusqlite::params![aid, st, sv],
        )
        .map_err(|e| format!("save jmap sync state: {e}"))?;
        Ok(())
    })
    .await
}

pub async fn load_jmap_sync_state(
    db: &DbState,
    account_id: &str,
    state_type: &str,
) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    let st = state_type.to_string();

    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT state FROM jmap_sync_state WHERE account_id = ?1 AND type = ?2",
            rusqlite::params![aid, st],
            |row| row.get::<_, String>(0),
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
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
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
