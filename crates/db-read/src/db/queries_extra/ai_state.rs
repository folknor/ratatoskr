use super::super::ReadDbState;
use rusqlite::params;

pub async fn db_get_ai_cache(
    db: &ReadDbState,
    account_id: String,
    thread_id: String,
    cache_type: String,
) -> Result<Option<String>, String> {
    db.with_read(move |conn| {
        match conn.query_row(
            "SELECT content FROM ai_cache
             WHERE account_id = ?1 AND thread_id = ?2 AND cache_type = ?3",
            params![account_id, thread_id, cache_type],
            |row| row.get::<_, String>(0),
        ) {
            Ok(content) => Ok(Some(content)),
            Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

pub async fn db_get_cached_scan_result(
    db: &ReadDbState,
    account_id: String,
    thread_id: String,
) -> Result<Option<String>, String> {
    db.with_read(move |conn| {
        match conn.query_row(
            "SELECT result_json FROM ai_scan_results WHERE account_id = ?1 AND thread_id = ?2",
            params![account_id, thread_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(result) => Ok(Some(result)),
            Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}

pub async fn db_get_writing_style_profile(
    db: &ReadDbState,
    account_id: String,
) -> Result<Option<String>, String> {
    db.with_read(move |conn| {
        match conn.query_row(
            "SELECT profile_text FROM writing_style_profiles WHERE account_id = ?1",
            params![account_id],
            |row| row.get::<_, String>(0),
        ) {
            Ok(profile) => Ok(Some(profile)),
            Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => Ok(None),
            Err(e) => Err(e.to_string()),
        }
    })
    .await
}
