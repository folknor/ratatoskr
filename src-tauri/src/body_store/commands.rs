// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use rusqlite::params;
use tauri::State;

use super::{BodyStoreState, BodyStoreStats, MessageBody};

/// Store a single message body (zstd-compressed).
#[tauri::command]
pub async fn body_store_put(
    state: State<'_, BodyStoreState>,
    message_id: String,
    body_html: Option<String>,
    body_text: Option<String>,
) -> Result<(), String> {
    state.put(message_id, body_html, body_text).await
}

/// Store multiple message bodies in a single transaction.
#[tauri::command]
pub async fn body_store_put_batch(
    state: State<'_, BodyStoreState>,
    bodies: Vec<MessageBody>,
) -> Result<(), String> {
    state.put_batch(bodies).await
}

/// Retrieve a single message body (decompressed).
#[tauri::command]
pub async fn body_store_get(
    state: State<'_, BodyStoreState>,
    message_id: String,
) -> Result<Option<MessageBody>, String> {
    state.get(message_id).await
}

/// Retrieve multiple message bodies (decompressed).
#[tauri::command]
pub async fn body_store_get_batch(
    state: State<'_, BodyStoreState>,
    message_ids: Vec<String>,
) -> Result<Vec<MessageBody>, String> {
    state.get_batch(message_ids).await
}

/// Delete bodies for given message IDs.
#[tauri::command]
pub async fn body_store_delete(
    state: State<'_, BodyStoreState>,
    message_ids: Vec<String>,
) -> Result<u64, String> {
    state.delete(message_ids).await
}

/// Get body store statistics (count, compressed sizes).
#[tauri::command]
pub async fn body_store_stats(state: State<'_, BodyStoreState>) -> Result<BodyStoreStats, String> {
    state.stats().await
}

/// Migrate existing bodies from the metadata DB into the body store.
///
/// Reads body_html/body_text from messages table in batches,
/// compresses and inserts into body store, then NULLs the columns
/// in the metadata DB to reclaim space.
const MIGRATE_BATCH_SIZE: i64 = 1000;

#[tauri::command]
#[allow(clippy::too_many_lines)]
pub async fn body_store_migrate(
    state: State<'_, BodyStoreState>,
    db_state: State<'_, crate::db::DbState>,
) -> Result<u64, String> {
    // Count messages with bodies still in metadata DB
    let total: u64 = db_state
        .with_conn(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE body_html IS NOT NULL OR body_text IS NOT NULL",
                    [],
                    |row| row.get(0),
                )
                .map_err(|e| format!("count for migration: {e}"))?;
            #[allow(clippy::cast_sign_loss)]
            Ok(count as u64)
        })
        .await?;

    if total == 0 {
        log::info!("Body store migration: no bodies to migrate");
        return Ok(0);
    }

    log::info!("Body store migration: {total} messages with bodies to migrate");

    let mut migrated: u64 = 0;

    loop {
        // Fetch a batch of message IDs + bodies from metadata DB
        let batch: Vec<(String, Option<String>, Option<String>)> = db_state
            .with_conn(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, body_html, body_text FROM messages
                         WHERE body_html IS NOT NULL OR body_text IS NOT NULL
                         LIMIT ?1",
                    )
                    .map_err(|e| format!("prepare migration fetch: {e}"))?;

                let rows = stmt
                    .query_map(params![MIGRATE_BATCH_SIZE], |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, Option<String>>(1)?,
                            row.get::<_, Option<String>>(2)?,
                        ))
                    })
                    .map_err(|e| format!("query migration batch: {e}"))?;

                let mut batch = Vec::new();
                for row in rows {
                    batch.push(row.map_err(|e| format!("map migration row: {e}"))?);
                }
                Ok(batch)
            })
            .await?;

        if batch.is_empty() {
            break;
        }

        let batch_len = batch.len();

        // Build MessageBody vec for body store
        let bodies: Vec<MessageBody> = batch
            .iter()
            .map(|(id, html, text)| MessageBody {
                message_id: id.clone(),
                body_html: html.clone(),
                body_text: text.clone(),
            })
            .collect();

        // Insert into body store
        state.put_batch(bodies).await?;

        // NULL out the body columns in metadata DB for these message IDs
        let ids_to_null: Vec<String> = batch.into_iter().map(|(id, _, _)| id).collect();
        db_state
            .with_conn(move |conn| {
                let tx = conn
                    .unchecked_transaction()
                    .map_err(|e| format!("null tx: {e}"))?;

                for chunk in ids_to_null.chunks(100) {
                    let placeholders: String = chunk
                        .iter()
                        .enumerate()
                        .map(|(i, _)| format!("?{}", i + 1))
                        .collect::<Vec<_>>()
                        .join(", ");

                    let sql = format!(
                        "UPDATE messages SET body_html = NULL, body_text = NULL WHERE id IN ({placeholders})"
                    );

                    let mut stmt = tx.prepare(&sql).map_err(|e| format!("prepare null: {e}"))?;
                    let param_values: Vec<Box<dyn rusqlite::types::ToSql>> = chunk
                        .iter()
                        .map(|id| Box::new(id.clone()) as Box<dyn rusqlite::types::ToSql>)
                        .collect();
                    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                        param_values.iter().map(AsRef::as_ref).collect();

                    stmt.execute(param_refs.as_slice())
                        .map_err(|e| format!("null bodies: {e}"))?;
                }

                tx.commit().map_err(|e| format!("null commit: {e}"))?;
                Ok(())
            })
            .await?;

        #[allow(clippy::cast_possible_truncation)]
        {
            migrated += batch_len as u64;
        }

        log::info!("Body store migration: {migrated}/{total} messages migrated");

        let batch_len_i64 = i64::try_from(batch_len).unwrap_or(i64::MAX);
        if batch_len_i64 < MIGRATE_BATCH_SIZE {
            break;
        }
    }

    log::info!("Body store migration complete: {migrated} messages migrated");
    Ok(migrated)
}
