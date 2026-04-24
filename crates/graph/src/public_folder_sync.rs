//! Offline sync for pinned public folders via EWS.
//!
//! Public folders have no delta tokens - all sync is timestamp-based polling.
//! Deletion detection is expensive (full ID scan) and throttled to once per hour.

use crate::ews::{EwsClient, EwsFolder, EwsHeaders, EwsItem};
use crate::parse::parse_iso_datetime;
use db::db::DbState;

/// Result of syncing a single pinned public folder.
#[derive(Debug, Default)]
pub struct PublicFolderSyncResult {
    pub folder_id: String,
    pub new_items: usize,
    pub updated_items: usize,
    pub deleted_items: usize,
}

/// Minimum interval (in seconds) between full deletion scans.
const DELETION_SCAN_INTERVAL_SECS: i64 = 3600; // 1 hour

/// Default page size for `find_items` calls.
const PAGE_SIZE: u32 = 50;

// ── Helpers ──────────────────────────────────────────────────

/// Parse an ISO 8601 date-time string to a Unix timestamp (seconds).
fn parse_iso8601_to_unix(s: &str) -> Option<i64> {
    parse_iso_datetime(s).map(|dt| dt.timestamp())
}

/// Fetch all items from a folder via EWS, paginating until `includes_last`.
/// If `since` is provided, only items received on or after that ISO 8601 timestamp.
async fn fetch_all_items(
    ews: &EwsClient,
    access_token: &str,
    folder_id: &str,
    since: Option<&str>,
    headers: &EwsHeaders,
) -> Result<Vec<EwsItem>, String> {
    let mut all_items = Vec::new();
    let mut offset = 0u32;

    loop {
        let result = ews
            .find_items(
                access_token,
                folder_id,
                since,
                offset,
                PAGE_SIZE,
                Some(headers),
            )
            .await?;

        let batch_len: u32 = result
            .items
            .len()
            .try_into()
            .map_err(|_| "item batch too large for u32 offset".to_string())?;
        all_items.extend(result.items);

        if result.includes_last || batch_len == 0 {
            break;
        }

        offset += batch_len;
    }

    Ok(all_items)
}

/// Fetch just item IDs from a folder (for deletion detection).
/// Reuses `find_items` - every `EwsItem` already includes `item_id`.
async fn fetch_all_item_ids(
    ews: &EwsClient,
    access_token: &str,
    folder_id: &str,
    headers: &EwsHeaders,
) -> Result<Vec<String>, String> {
    let items = fetch_all_items(ews, access_token, folder_id, None, headers).await?;
    Ok(items.into_iter().map(|i| i.item_id).collect())
}

// ── DB helpers ───────────────────────────────────────────────

/// Load sync state for a folder. Returns `(last_sync_timestamp, last_full_scan_at)`.
async fn load_sync_state(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
) -> Result<(Option<i64>, Option<i64>), String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT last_sync_timestamp, last_full_scan_at \
                 FROM public_folder_sync_state \
                 WHERE account_id = ?1 AND folder_id = ?2",
            )
            .map_err(|e| format!("prepare load_sync_state: {e}"))?;

        let result = stmt
            .query_row(rusqlite::params![account_id, folder_id], |row| {
                Ok((
                    row.get::<_, Option<i64>>("last_sync_timestamp")?,
                    row.get::<_, Option<i64>>("last_full_scan_at")?,
                ))
            })
            .ok();

        Ok(result.unwrap_or((None, None)))
    })
    .await
}

/// Save sync state after a sync run.
async fn save_sync_state(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    last_sync_timestamp: i64,
    last_full_scan_at: Option<i64>,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO public_folder_sync_state (account_id, folder_id, last_sync_timestamp, last_full_scan_at) \
             VALUES (?1, ?2, ?3, ?4) \
             ON CONFLICT(account_id, folder_id) DO UPDATE SET \
               last_sync_timestamp = excluded.last_sync_timestamp, \
               last_full_scan_at = COALESCE(excluded.last_full_scan_at, last_full_scan_at)",
            rusqlite::params![account_id, folder_id, last_sync_timestamp, last_full_scan_at],
        )
        .map_err(|e| format!("save_sync_state: {e}"))?;
        Ok(())
    })
    .await
}

/// Upsert items into `public_folder_items`. Returns `(new_count, updated_count)`.
async fn upsert_items(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    items: Vec<EwsItem>,
) -> Result<(usize, usize), String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    db.with_conn(move |conn| {
        let mut new_count = 0usize;
        let mut updated_count = 0usize;

        let mut insert_stmt = conn
            .prepare(
                "INSERT INTO public_folder_items \
                 (account_id, folder_id, item_id, change_key, subject, sender_email, sender_name, received_at, body_preview, is_read, item_class) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                 ON CONFLICT(account_id, item_id) DO UPDATE SET \
                   change_key = excluded.change_key, \
                   subject = excluded.subject, \
                   is_read = excluded.is_read, \
                   body_preview = excluded.body_preview",
            )
            .map_err(|e| format!("prepare upsert_items: {e}"))?;

        let mut exists_stmt = conn
            .prepare(
                "SELECT change_key FROM public_folder_items WHERE account_id = ?1 AND item_id = ?2",
            )
            .map_err(|e| format!("prepare exists check: {e}"))?;

        for item in &items {
            let received_ts = item
                .received_at
                .as_deref()
                .and_then(parse_iso8601_to_unix);

            // Check if this item already exists (and if change_key differs)
            let existing_ck: Option<Option<String>> = exists_stmt
                .query_row(rusqlite::params![account_id, item.item_id], |row| {
                    row.get::<_, Option<String>>("change_key")
                })
                .ok();

            let is_update = match &existing_ck {
                Some(ck) => ck.as_deref() != item.change_key.as_deref(),
                None => false,
            };
            let is_new = existing_ck.is_none();

            insert_stmt
                .execute(rusqlite::params![
                    account_id,
                    folder_id,
                    item.item_id,
                    item.change_key,
                    item.subject,
                    item.sender_email,
                    item.sender_name,
                    received_ts,
                    item.body_preview,
                    item.is_read as i32,
                    item.item_class,
                ])
                .map_err(|e| format!("upsert item {}: {e}", item.item_id))?;

            if is_new {
                new_count += 1;
            } else if is_update {
                updated_count += 1;
            }
        }

        Ok((new_count, updated_count))
    })
    .await
}

/// Delete local items not present on the server. Returns deletion count.
async fn delete_stale_items(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    server_item_ids: &[String],
) -> Result<usize, String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    let server_ids: Vec<String> = server_item_ids.to_vec();

    db.with_conn(move |conn| {
        if server_ids.is_empty() {
            // If server returns 0 items, delete everything local for this folder
            let deleted = conn
                .execute(
                    "DELETE FROM public_folder_items WHERE account_id = ?1 AND folder_id = ?2",
                    rusqlite::params![account_id, folder_id],
                )
                .map_err(|e| format!("delete_stale_items (all): {e}"))?;
            return Ok(deleted);
        }

        // Build a parameterized IN clause
        let placeholders: Vec<String> = (0..server_ids.len())
            .map(|i| format!("?{}", i + 3))
            .collect();
        let sql = format!(
            "DELETE FROM public_folder_items \
             WHERE account_id = ?1 AND folder_id = ?2 AND item_id NOT IN ({})",
            placeholders.join(", ")
        );

        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
        params.push(Box::new(account_id));
        params.push(Box::new(folder_id));
        for id in &server_ids {
            params.push(Box::new(id.clone()));
        }

        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(AsRef::as_ref).collect();

        let deleted = conn
            .execute(&sql, param_refs.as_slice())
            .map_err(|e| format!("delete_stale_items: {e}"))?;
        Ok(deleted)
    })
    .await
}

/// Load sync_depth_days from `public_folder_pins` for a folder. Returns default 30 if missing.
async fn load_sync_depth_days(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
) -> Result<i32, String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    db.with_conn(move |conn| {
        let depth = conn
            .query_row(
                "SELECT sync_depth_days FROM public_folder_pins \
                 WHERE account_id = ?1 AND folder_id = ?2",
                rusqlite::params![account_id, folder_id],
                |row| row.get::<_, i32>("sync_depth_days"),
            )
            .unwrap_or(30);
        Ok(depth)
    })
    .await
}

/// Load all pinned folder IDs with sync_enabled = 1.
async fn load_pinned_folder_ids(db: &DbState, account_id: &str) -> Result<Vec<String>, String> {
    let account_id = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT folder_id FROM public_folder_pins \
                 WHERE account_id = ?1 AND sync_enabled = 1",
            )
            .map_err(|e| format!("prepare load_pinned_folder_ids: {e}"))?;

        let rows = stmt
            .query_map(rusqlite::params![account_id], |row| {
                row.get::<_, String>("folder_id")
            })
            .map_err(|e| format!("query load_pinned_folder_ids: {e}"))?;

        let mut ids = Vec::new();
        for row in rows {
            ids.push(row.map_err(|e| format!("row error: {e}"))?);
        }
        Ok(ids)
    })
    .await
}

// ── Public API ───────────────────────────────────────────────

/// Sync a single pinned public folder.
///
/// Runs initial sync (fetch last N days) if no prior sync state exists,
/// otherwise runs incremental sync from the last sync timestamp.
/// Deletion detection runs at most once per hour.
pub async fn sync_pinned_public_folder(
    ews: &EwsClient,
    access_token: &str,
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    headers: &EwsHeaders,
) -> Result<PublicFolderSyncResult, String> {
    let (last_sync_ts, last_full_scan) = load_sync_state(db, account_id, folder_id).await?;
    let now = chrono::Utc::now().timestamp();

    // Determine the `since` filter for find_items
    let since_str = match last_sync_ts {
        Some(ts) => {
            log::info!("Public folder {folder_id}: incremental sync from timestamp {ts}");
            chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.to_rfc3339())
        }
        None => {
            let depth_days = load_sync_depth_days(db, account_id, folder_id).await?;
            log::info!("Public folder {folder_id}: initial sync, looking back {depth_days} days");
            let since = chrono::Utc::now() - chrono::Duration::days(i64::from(depth_days));
            Some(since.to_rfc3339())
        }
    };

    // Fetch items (new or changed since last sync)
    let items =
        fetch_all_items(ews, access_token, folder_id, since_str.as_deref(), headers).await?;

    log::info!(
        "Public folder {folder_id}: fetched {} items from server",
        items.len()
    );

    // Upsert into DB
    let (new_items, updated_items) = upsert_items(db, account_id, folder_id, items).await?;

    // Deletion detection - throttled to once per hour
    let mut deleted_items = 0usize;
    let should_scan = match last_full_scan {
        Some(scan_ts) => (now - scan_ts) >= DELETION_SCAN_INTERVAL_SECS,
        None => last_sync_ts.is_some(), // Skip deletion scan on very first sync
    };

    let full_scan_ts = if should_scan {
        log::info!("Public folder {folder_id}: running deletion scan");
        let server_ids = fetch_all_item_ids(ews, access_token, folder_id, headers).await?;
        deleted_items = delete_stale_items(db, account_id, folder_id, &server_ids).await?;
        if deleted_items > 0 {
            log::info!("Public folder {folder_id}: deleted {deleted_items} stale items");
        }
        Some(now)
    } else {
        None
    };

    // Save sync state
    save_sync_state(db, account_id, folder_id, now, full_scan_ts).await?;

    Ok(PublicFolderSyncResult {
        folder_id: folder_id.to_string(),
        new_items,
        updated_items,
        deleted_items,
    })
}

/// Sync all pinned public folders for an account.
///
/// Each folder syncs independently - one failure does not block others.
/// Returns `(folder_id, result)` pairs.
pub async fn sync_all_pinned_folders(
    ews: &EwsClient,
    access_token: &str,
    db: &DbState,
    account_id: &str,
    headers: &EwsHeaders,
) -> Result<Vec<(String, Result<PublicFolderSyncResult, String>)>, String> {
    let folder_ids = load_pinned_folder_ids(db, account_id).await?;

    if folder_ids.is_empty() {
        log::info!("No pinned public folders for account {account_id}");
        return Ok(Vec::new());
    }

    log::info!(
        "Syncing {} pinned public folder(s) for account {account_id}",
        folder_ids.len()
    );

    let mut results = Vec::with_capacity(folder_ids.len());

    for fid in &folder_ids {
        let result =
            sync_pinned_public_folder(ews, access_token, db, account_id, fid, headers).await;

        match &result {
            Ok(sr) => {
                log::info!(
                    "Public folder {fid}: sync complete ({} new, {} updated, {} deleted)",
                    sr.new_items,
                    sr.updated_items,
                    sr.deleted_items
                );
            }
            Err(e) => {
                log::warn!("Public folder {fid}: sync failed: {e}");
            }
        }

        results.push((fid.clone(), result));
    }

    Ok(results)
}

/// Pin a public folder for offline sync.
pub async fn pin_public_folder(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
    sync_depth_days: Option<i32>,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();
    let depth = sync_depth_days.unwrap_or(30);

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO public_folder_pins (account_id, folder_id, sync_enabled, sync_depth_days) \
             VALUES (?1, ?2, 1, ?3) \
             ON CONFLICT(account_id, folder_id) DO UPDATE SET \
               sync_enabled = 1, \
               sync_depth_days = excluded.sync_depth_days",
            rusqlite::params![account_id, folder_id, depth],
        )
        .map_err(|e| format!("pin_public_folder: {e}"))?;
        Ok(())
    })
    .await
}

/// Unpin a public folder - removes pin, local items, and sync state.
pub async fn unpin_public_folder(
    db: &DbState,
    account_id: &str,
    folder_id: &str,
) -> Result<(), String> {
    let account_id = account_id.to_string();
    let folder_id = folder_id.to_string();

    db.with_conn(move |conn| {
        conn.execute(
            "DELETE FROM public_folder_pins WHERE account_id = ?1 AND folder_id = ?2",
            rusqlite::params![account_id, folder_id],
        )
        .map_err(|e| format!("unpin delete pins: {e}"))?;

        conn.execute(
            "DELETE FROM public_folder_items WHERE account_id = ?1 AND folder_id = ?2",
            rusqlite::params![account_id, folder_id],
        )
        .map_err(|e| format!("unpin delete items: {e}"))?;

        conn.execute(
            "DELETE FROM public_folder_sync_state WHERE account_id = ?1 AND folder_id = ?2",
            rusqlite::params![account_id, folder_id],
        )
        .map_err(|e| format!("unpin delete sync_state: {e}"))?;

        Ok(())
    })
    .await
}

/// Browse public folders under a parent folder via EWS.
///
/// Results are persisted to the `public_folders` table for offline access.
/// Use `"publicfoldersroot"` as `parent_folder_id` for the top level.
pub async fn browse_public_folders(
    ews: &EwsClient,
    access_token: &str,
    db: &DbState,
    account_id: &str,
    parent_folder_id: &str,
    headers: &EwsHeaders,
) -> Result<Vec<EwsFolder>, String> {
    let folders = ews
        .find_folder(access_token, parent_folder_id, Some(headers))
        .await?;

    // Persist to DB for offline browsing
    let account_id_owned = account_id.to_string();
    let parent_id_owned = parent_folder_id.to_string();
    let folders_clone = folders.clone();

    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "INSERT INTO public_folders \
                 (account_id, folder_id, parent_id, display_name, folder_class, \
                  unread_count, total_count, can_create_items, can_modify, can_delete, can_read) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
                 ON CONFLICT(account_id, folder_id) DO UPDATE SET \
                   parent_id = excluded.parent_id, \
                   display_name = excluded.display_name, \
                   folder_class = excluded.folder_class, \
                   unread_count = excluded.unread_count, \
                   total_count = excluded.total_count, \
                   can_create_items = excluded.can_create_items, \
                   can_modify = excluded.can_modify, \
                   can_delete = excluded.can_delete, \
                   can_read = excluded.can_read",
            )
            .map_err(|e| format!("prepare browse_public_folders: {e}"))?;

        for f in &folders_clone {
            stmt.execute(rusqlite::params![
                account_id_owned,
                f.folder_id,
                parent_id_owned,
                f.display_name,
                f.folder_class,
                f.unread_count,
                f.total_count,
                f.effective_rights.create_contents as i32,
                f.effective_rights.modify as i32,
                f.effective_rights.delete as i32,
                f.effective_rights.read as i32,
            ])
            .map_err(|e| format!("upsert folder {}: {e}", f.folder_id))?;
        }

        Ok(())
    })
    .await?;

    Ok(folders)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_result_default() {
        let r = PublicFolderSyncResult::default();
        assert_eq!(r.folder_id, "");
        assert_eq!(r.new_items, 0);
        assert_eq!(r.updated_items, 0);
        assert_eq!(r.deleted_items, 0);
    }

    #[test]
    fn sync_result_construction() {
        let r = PublicFolderSyncResult {
            folder_id: "folder123".to_string(),
            new_items: 10,
            updated_items: 3,
            deleted_items: 2,
        };
        assert_eq!(r.folder_id, "folder123");
        assert_eq!(r.new_items, 10);
        assert_eq!(r.updated_items, 3);
        assert_eq!(r.deleted_items, 2);
    }

    #[test]
    fn parse_iso8601_rfc3339() {
        let ts = parse_iso8601_to_unix("2024-06-15T10:30:00Z");
        assert!(ts.is_some());
        // 2024-06-15T10:30:00Z = 1718444200 (approx)
        let v = ts.expect("should parse");
        assert!(v > 1_700_000_000);
        assert!(v < 1_800_000_000);
    }

    #[test]
    fn parse_iso8601_naive() {
        let ts = parse_iso8601_to_unix("2024-06-15T10:30:00");
        assert!(ts.is_some());
    }

    #[test]
    fn parse_iso8601_with_offset() {
        let ts = parse_iso8601_to_unix("2024-06-15T10:30:00+05:00");
        assert!(ts.is_some());
    }

    #[test]
    fn parse_iso8601_invalid() {
        let ts = parse_iso8601_to_unix("not-a-date");
        assert!(ts.is_none());
    }

    #[test]
    fn deletion_scan_interval_is_one_hour() {
        assert_eq!(DELETION_SCAN_INTERVAL_SECS, 3600);
    }

    // Integration-style tests using in-memory SQLite

    fn setup_test_db() -> rusqlite::Connection {
        let conn = rusqlite::Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch(
            r#"
            CREATE TABLE public_folder_items (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                item_id TEXT NOT NULL,
                change_key TEXT,
                subject TEXT,
                sender_email TEXT,
                sender_name TEXT,
                received_at INTEGER,
                body_preview TEXT,
                is_read INTEGER NOT NULL DEFAULT 0,
                item_class TEXT NOT NULL DEFAULT 'IPM.Note',
                UNIQUE(account_id, item_id)
            );
            CREATE TABLE public_folder_pins (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                sync_enabled INTEGER NOT NULL DEFAULT 1,
                sync_depth_days INTEGER NOT NULL DEFAULT 30,
                last_sync_at INTEGER,
                UNIQUE(account_id, folder_id)
            );
            CREATE TABLE public_folder_sync_state (
                account_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                last_sync_timestamp INTEGER,
                last_full_scan_at INTEGER,
                PRIMARY KEY(account_id, folder_id)
            );
            "#,
        )
        .expect("create tables");
        conn
    }

    #[test]
    fn upsert_new_item() {
        let conn = setup_test_db();
        let received_ts = parse_iso8601_to_unix("2024-06-15T10:30:00Z");

        conn.execute(
            "INSERT INTO public_folder_items \
             (account_id, folder_id, item_id, change_key, subject, sender_email, sender_name, received_at, body_preview, is_read, item_class) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
             ON CONFLICT(account_id, item_id) DO UPDATE SET \
               change_key = excluded.change_key, \
               subject = excluded.subject, \
               is_read = excluded.is_read, \
               body_preview = excluded.body_preview",
            rusqlite::params![
                "acc1", "folder1", "item1", "ck1", "Test Subject",
                "sender@example.com", "Sender", received_ts, "Preview text", 0, "IPM.Note"
            ],
        )
        .expect("insert");

        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM public_folder_items WHERE account_id = 'acc1'",
                [],
                |row| row.get("cnt"),
            )
            .expect("count");
        assert_eq!(count, 1);
    }

    #[test]
    fn upsert_updates_on_change_key_change() {
        let conn = setup_test_db();

        // Insert original
        conn.execute(
            "INSERT INTO public_folder_items \
             (account_id, folder_id, item_id, change_key, subject, is_read, item_class) \
             VALUES ('acc1', 'f1', 'item1', 'ck1', 'Original Subject', 0, 'IPM.Note')",
            [],
        )
        .expect("insert original");

        // Upsert with changed change_key
        conn.execute(
            "INSERT INTO public_folder_items \
             (account_id, folder_id, item_id, change_key, subject, is_read, item_class) \
             VALUES ('acc1', 'f1', 'item1', 'ck2', 'Updated Subject', 1, 'IPM.Note') \
             ON CONFLICT(account_id, item_id) DO UPDATE SET \
               change_key = excluded.change_key, \
               subject = excluded.subject, \
               is_read = excluded.is_read, \
               body_preview = excluded.body_preview",
            [],
        )
        .expect("upsert");

        let (ck, subj, read): (String, String, i32) = conn
            .query_row(
                "SELECT change_key, subject, is_read FROM public_folder_items WHERE item_id = 'item1'",
                [],
                |row| Ok((row.get("change_key")?, row.get("subject")?, row.get("is_read")?)),
            )
            .expect("query");

        assert_eq!(ck, "ck2");
        assert_eq!(subj, "Updated Subject");
        assert_eq!(read, 1);
    }

    #[test]
    fn deletion_detection_removes_stale_items() {
        let conn = setup_test_db();

        // Insert 3 items
        for id in &["item1", "item2", "item3"] {
            conn.execute(
                "INSERT INTO public_folder_items \
                 (account_id, folder_id, item_id, item_class) \
                 VALUES ('acc1', 'f1', ?1, 'IPM.Note')",
                rusqlite::params![id],
            )
            .expect("insert");
        }

        // Server only has item1 and item3
        let server_ids = ["item1", "item3"];
        let placeholders: Vec<String> = (0..server_ids.len())
            .map(|i| format!("?{}", i + 3))
            .collect();
        let sql = format!(
            "DELETE FROM public_folder_items \
             WHERE account_id = ?1 AND folder_id = ?2 AND item_id NOT IN ({})",
            placeholders.join(", ")
        );

        let deleted = conn
            .execute(&sql, rusqlite::params!["acc1", "f1", "item1", "item3"])
            .expect("delete stale");

        assert_eq!(deleted, 1); // item2 deleted

        let remaining: i32 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM public_folder_items WHERE account_id = 'acc1'",
                [],
                |row| row.get("cnt"),
            )
            .expect("count");
        assert_eq!(remaining, 2);
    }

    #[test]
    fn pin_unpin_operations() {
        let conn = setup_test_db();

        // Pin
        conn.execute(
            "INSERT INTO public_folder_pins (account_id, folder_id, sync_enabled, sync_depth_days) \
             VALUES ('acc1', 'f1', 1, 60) \
             ON CONFLICT(account_id, folder_id) DO UPDATE SET \
               sync_enabled = 1, \
               sync_depth_days = excluded.sync_depth_days",
            [],
        )
        .expect("pin");

        let depth: i32 = conn
            .query_row(
                "SELECT sync_depth_days FROM public_folder_pins WHERE account_id = 'acc1' AND folder_id = 'f1'",
                [],
                |row| row.get("sync_depth_days"),
            )
            .expect("query depth");
        assert_eq!(depth, 60);

        // Add some items
        conn.execute(
            "INSERT INTO public_folder_items (account_id, folder_id, item_id, item_class) \
             VALUES ('acc1', 'f1', 'item1', 'IPM.Note')",
            [],
        )
        .expect("add item");

        // Add sync state
        conn.execute(
            "INSERT INTO public_folder_sync_state (account_id, folder_id, last_sync_timestamp) \
             VALUES ('acc1', 'f1', 1700000000)",
            [],
        )
        .expect("add sync state");

        // Unpin - should remove pin, items, and sync state
        conn.execute(
            "DELETE FROM public_folder_pins WHERE account_id = 'acc1' AND folder_id = 'f1'",
            [],
        )
        .expect("unpin");
        conn.execute(
            "DELETE FROM public_folder_items WHERE account_id = 'acc1' AND folder_id = 'f1'",
            [],
        )
        .expect("delete items");
        conn.execute(
            "DELETE FROM public_folder_sync_state WHERE account_id = 'acc1' AND folder_id = 'f1'",
            [],
        )
        .expect("delete sync state");

        let pin_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM public_folder_pins WHERE account_id = 'acc1'",
                [],
                |row| row.get("cnt"),
            )
            .expect("count pins");
        assert_eq!(pin_count, 0);

        let item_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM public_folder_items WHERE account_id = 'acc1'",
                [],
                |row| row.get("cnt"),
            )
            .expect("count items");
        assert_eq!(item_count, 0);

        let sync_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM public_folder_sync_state WHERE account_id = 'acc1'",
                [],
                |row| row.get("cnt"),
            )
            .expect("count sync state");
        assert_eq!(sync_count, 0);
    }
}
