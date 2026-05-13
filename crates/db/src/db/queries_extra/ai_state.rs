use super::super::ReadDbState;
use super::super::types::{DbFolderSyncState, DbWritingStyleProfile};
use crate::db::{query_as, query_one};
use rusqlite::params;

pub async fn db_get_ai_cache(
    db: &ReadDbState,
    account_id: String,
    thread_id: String,
    cache_type: String,
) -> Result<Option<String>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT content FROM ai_cache WHERE account_id = ?1 AND thread_id = ?2 AND type = ?3")
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![account_id, thread_id, cache_type], |row| {
                row.get::<_, String>("content")
            })
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(content)) => Ok(Some(content)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    })
    .await
}

pub async fn db_set_ai_cache(
    db: &ReadDbState,
    account_id: String,
    thread_id: String,
    cache_type: String,
    content: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO ai_cache (id, account_id, thread_id, type, content)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(account_id, thread_id, type) DO UPDATE SET
                   content = ?5, created_at = unixepoch()",
            params![id, account_id, thread_id, cache_type, content],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_ai_cache(
    db: &ReadDbState,
    account_id: String,
    thread_id: String,
    cache_type: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM ai_cache WHERE account_id = ?1 AND thread_id = ?2 AND type = ?3",
            params![account_id, thread_id, cache_type],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_cached_scan_result(
    db: &ReadDbState,
    account_id: String,
    message_id: String,
) -> Result<Option<String>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT result_json FROM link_scan_results WHERE account_id = ?1 AND message_id = ?2 LIMIT 1",
            )
            .map_err(|e| e.to_string())?;
        let mut rows = stmt
            .query_map(params![account_id, message_id], |row| row.get::<_, String>("result_json"))
            .map_err(|e| e.to_string())?;
        match rows.next() {
            Some(Ok(val)) => Ok(Some(val)),
            Some(Err(e)) => Err(e.to_string()),
            None => Ok(None),
        }
    })
    .await
}

pub async fn db_cache_scan_result(
    db: &ReadDbState,
    account_id: String,
    message_id: String,
    result_json: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO link_scan_results (account_id, message_id, result_json) VALUES (?1, ?2, ?3)",
            params![account_id, message_id, result_json],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_scan_results(db: &ReadDbState, account_id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM link_scan_results WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_writing_style_profile(
    db: &ReadDbState,
    account_id: String,
) -> Result<Option<DbWritingStyleProfile>, String> {
    db.with_conn(move |conn| {
        query_one::<DbWritingStyleProfile>(
            conn,
            "SELECT id, account_id, profile_text, sample_count, created_at, updated_at
                 FROM writing_style_profiles WHERE account_id = ?1",
            &[&account_id],
        )
    })
    .await
}

pub async fn db_upsert_writing_style_profile(
    db: &ReadDbState,
    account_id: String,
    profile_text: String,
    sample_count: i64,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO writing_style_profiles (id, account_id, profile_text, sample_count)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id) DO UPDATE SET
                   profile_text = ?3, sample_count = ?4, updated_at = unixepoch()",
            params![id, account_id, profile_text, sample_count],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_writing_style_profile(
    db: &ReadDbState,
    account_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM writing_style_profiles WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_folder_sync_state(
    db: &ReadDbState,
    account_id: String,
    folder_path: String,
) -> Result<Option<DbFolderSyncState>, String> {
    db.with_conn(move |conn| {
        query_one::<DbFolderSyncState>(
            conn,
            "SELECT account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at
                 FROM folder_sync_state WHERE account_id = ?1 AND folder_path = ?2",
            &[&account_id, &folder_path],
        )
    })
    .await
}

pub async fn db_upsert_folder_sync_state(
    db: &ReadDbState,
    account_id: String,
    folder_path: String,
    uidvalidity: Option<i64>,
    last_uid: i64,
    modseq: Option<i64>,
    last_sync_at: Option<i64>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO folder_sync_state (account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(account_id, folder_path) DO UPDATE SET
                   uidvalidity = ?3, last_uid = ?4, modseq = ?5, last_sync_at = ?6",
            params![account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_folder_sync_state(
    db: &ReadDbState,
    account_id: String,
    folder_path: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM folder_sync_state WHERE account_id = ?1 AND folder_path = ?2",
            params![account_id, folder_path],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_clear_all_folder_sync_states(
    db: &ReadDbState,
    account_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM folder_sync_state WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_all_folder_sync_states(
    db: &ReadDbState,
    account_id: String,
) -> Result<Vec<DbFolderSyncState>, String> {
    db.with_conn(move |conn| {
        query_as::<DbFolderSyncState>(
            conn,
            "SELECT account_id, folder_path, uidvalidity, last_uid, modseq, last_sync_at
                 FROM folder_sync_state WHERE account_id = ?1 ORDER BY folder_path ASC",
            &[&account_id],
        )
    })
    .await
}
