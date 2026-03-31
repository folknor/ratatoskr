use std::path::Path;

use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::inline_image_store::InlineImageStoreState;

/// Prepare an account for full resync by deleting all messages, threads,
/// and sync state, then cleaning up orphaned inline images and enforcing
/// the attachment cache limit.
pub async fn prepare_account_resync(
    db: &DbState,
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    app_data_dir: &Path,
    account_id: &str,
) -> Result<(), String> {
    let account_id_owned = account_id.to_string();

    let (message_ids, inline_hashes) = db
        .with_conn({
            let account_id = account_id_owned.clone();
            move |conn| {
                let mut stmt = conn
                    .prepare("SELECT id FROM messages WHERE account_id = ?1")
                    .map_err(|e| format!("prepare resync message query: {e}"))?;
                let msg_ids = stmt
                    .query_map(rusqlite::params![account_id], |row| row.get::<_, String>(0))
                    .map_err(|e| format!("query resync message ids: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect resync message ids: {e}"))?;
                let hashes = crate::inline_image_store::collect_inline_hashes_for_account(
                    conn,
                    &account_id,
                )?;
                Ok((msg_ids, hashes))
            }
        })
        .await?;

    body_store.delete(message_ids).await?;

    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin resync transaction: {e}"))?;
        tx.execute(
            "DELETE FROM threads WHERE account_id = ?1",
            rusqlite::params![account_id_owned],
        )
        .map_err(|e| format!("delete threads for account: {e}"))?;
        crate::sync::pipeline::clear_account_history_id(&tx, &account_id_owned)?;
        crate::sync::pipeline::clear_all_folder_sync_states(&tx, &account_id_owned)?;
        tx.commit()
            .map_err(|e| format!("commit resync transaction: {e}"))?;
        Ok(())
    })
    .await?;

    // Clean up orphaned inline images after messages are gone
    if !inline_hashes.is_empty() {
        let orphaned = db
            .with_conn({
                let hashes = inline_hashes;
                move |conn| crate::inline_image_store::find_unreferenced_hashes(conn, &hashes)
            })
            .await?;
        let _ = inline_images.delete_unreferenced(orphaned).await;
    }

    // Evict file-based attachment cache entries that are now over the limit
    // (cascade-deleted attachment rows freed their cache_size quota)
    let _ = crate::attachment_cache::enforce_cache_limit(db, app_data_dir).await;

    Ok(())
}
