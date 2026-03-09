// tauri::command macro generates code that trips let_underscore_must_use
#![allow(clippy::let_underscore_must_use)]

use rusqlite::params;
use tauri::State;

use super::{SearchDocument, SearchParams, SearchResult, SearchState};

#[tauri::command]
pub async fn search_messages(
    state: State<'_, SearchState>,
    params: SearchParams,
) -> Result<Vec<SearchResult>, String> {
    state.search_with_filters(&params)
}

#[tauri::command]
pub async fn index_message(
    state: State<'_, SearchState>,
    doc: SearchDocument,
) -> Result<(), String> {
    state.index_message(&doc).await
}

#[tauri::command]
pub async fn index_messages_batch(
    state: State<'_, SearchState>,
    docs: Vec<SearchDocument>,
) -> Result<(), String> {
    state.index_messages_batch(&docs).await
}

#[tauri::command]
pub async fn delete_search_document(
    state: State<'_, SearchState>,
    message_id: String,
) -> Result<(), String> {
    state.delete_message(&message_id).await
}

const REBUILD_BATCH_SIZE: usize = 10_000;

#[tauri::command]
pub async fn rebuild_search_index(
    state: State<'_, SearchState>,
    db_state: State<'_, crate::db::DbState>,
) -> Result<u64, String> {
    // Step 1: Clear the existing index
    state.clear_index().await?;

    // Step 2: Count total messages
    let total: u64 = db_state
        .with_conn(|conn| {
            let mut stmt = conn
                .prepare("SELECT COUNT(*) FROM messages")
                .map_err(|e| format!("count messages: {e}"))?;
            let count: i64 = stmt
                .query_row([], |row| row.get(0))
                .map_err(|e| format!("count query: {e}"))?;
            #[allow(clippy::cast_sign_loss)]
            Ok(count as u64)
        })
        .await?;

    if total == 0 {
        return Ok(0);
    }

    // Step 3: Batch-fetch and index
    let mut indexed: u64 = 0;
    let mut offset: u64 = 0;

    loop {
        let batch_offset = offset;
        let docs: Vec<SearchDocument> = db_state
            .with_conn(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT m.id, m.account_id, m.thread_id, m.subject,
                                m.from_name, m.from_address, m.to_addresses,
                                m.body_text, m.snippet, m.date,
                                m.is_read, m.is_starred,
                                (SELECT COUNT(*) FROM attachments a WHERE a.message_id = m.id) > 0 as has_attachment
                         FROM messages m
                         ORDER BY m.rowid
                         LIMIT ?1 OFFSET ?2",
                    )
                    .map_err(|e| format!("prepare batch query: {e}"))?;

                #[allow(clippy::cast_possible_wrap)]
                let rows = stmt
                    .query_map(
                        params![REBUILD_BATCH_SIZE as i64, batch_offset as i64],
                        |row| {
                            Ok(SearchDocument {
                                message_id: row.get("id")?,
                                account_id: row.get("account_id")?,
                                thread_id: row.get("thread_id")?,
                                subject: row.get("subject")?,
                                from_name: row.get("from_name")?,
                                from_address: row.get("from_address")?,
                                to_addresses: row.get("to_addresses")?,
                                body_text: row.get("body_text")?,
                                snippet: row.get("snippet")?,
                                date: row.get::<_, Option<String>>("date")?
                                    .and_then(|d| {
                                        chrono::DateTime::parse_from_rfc3339(&d)
                                            .or_else(|_| chrono::DateTime::parse_from_str(&d, "%Y-%m-%d %H:%M:%S"))
                                            .map(|dt| dt.timestamp())
                                            .ok()
                                    })
                                    .unwrap_or(0),
                                is_read: row.get::<_, i64>("is_read")? != 0,
                                is_starred: row.get::<_, i64>("is_starred")? != 0,
                                has_attachment: row.get::<_, i64>("has_attachment")? != 0,
                            })
                        },
                    )
                    .map_err(|e| format!("query batch: {e}"))?;

                let mut docs = Vec::new();
                for row in rows {
                    docs.push(row.map_err(|e| format!("map row: {e}"))?);
                }
                Ok(docs)
            })
            .await?;

        if docs.is_empty() {
            break;
        }

        let batch_len = docs.len();
        state.index_messages_batch(&docs).await?;

        #[allow(clippy::cast_possible_truncation)]
        {
            indexed += batch_len as u64;
            offset += batch_len as u64;
        }

        log::info!("Search index rebuild: indexed {indexed}/{total} messages");

        #[allow(clippy::cast_possible_truncation)]
        if (batch_len as u64) < REBUILD_BATCH_SIZE as u64 {
            break;
        }
    }

    log::info!("Search index rebuild complete: {indexed} documents indexed");
    Ok(indexed)
}
