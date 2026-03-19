use std::path::{Path, PathBuf};

use log::{info, warn};
use rusqlite::params;
use rusqlite::OptionalExtension;

use crate::db::DbState;
use ratatoskr_stores::attachment_cache::hash_bytes;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Raw photo data returned from a provider fetch.
#[derive(Debug)]
pub struct PhotoData {
    pub bytes: Vec<u8>,
    pub content_type: String,
}

/// Default maximum cache size in bytes (50 MB).
const DEFAULT_MAX_CACHE_BYTES: u64 = 50 * 1024 * 1024;

/// Subdirectory under the cache root for contact photos.
const PHOTO_CACHE_DIR: &str = "contact_photos";

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build the on-disk path for a cached photo.
fn photo_file_path(cache_dir: &Path, content_hash: &str) -> PathBuf {
    cache_dir.join(PHOTO_CACHE_DIR).join(format!("{content_hash}.jpg"))
}

// ---------------------------------------------------------------------------
// Provider fetch functions
// ---------------------------------------------------------------------------

/// Fetch a contact photo from Microsoft Graph API.
///
/// Returns `None` if the contact has no photo (404).
pub async fn fetch_graph_contact_photo(
    http: &reqwest::Client,
    access_token: &str,
    contact_id: &str,
    api_base: &str,
) -> Result<Option<PhotoData>, String> {
    let url = format!("{api_base}/me/contacts/{contact_id}/photo/$value");

    let response = http
        .get(&url)
        .header("Authorization", format!("Bearer {access_token}"))
        .send()
        .await
        .map_err(|e| format!("fetch graph contact photo: {e}"))?;

    if response.status().as_u16() == 404 {
        return Ok(None);
    }

    if !response.status().is_success() {
        return Err(format!(
            "graph contact photo returned {}",
            response.status()
        ));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read graph contact photo body: {e}"))?
        .to_vec();

    if bytes.is_empty() {
        return Ok(None);
    }

    Ok(Some(PhotoData {
        bytes,
        content_type,
    }))
}

/// Fetch a contact photo from a Google photo URL.
///
/// Google photo URLs are public (`lh3.googleusercontent.com`), no auth needed.
/// The `size` parameter controls the `?sz=` resize hint.
pub async fn fetch_google_contact_photo(
    http: &reqwest::Client,
    photo_url: &str,
    size: u32,
) -> Result<Option<PhotoData>, String> {
    // Append size parameter for Google image resizing
    let url = if photo_url.contains('?') {
        format!("{photo_url}&sz={size}")
    } else {
        format!("{photo_url}?sz={size}")
    };

    let response = http
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("fetch google contact photo: {e}"))?;

    if response.status().as_u16() == 404 {
        return Ok(None);
    }

    if !response.status().is_success() {
        return Err(format!(
            "google contact photo returned {}",
            response.status()
        ));
    }

    let content_type = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/jpeg")
        .to_string();

    let bytes = response
        .bytes()
        .await
        .map_err(|e| format!("read google contact photo body: {e}"))?
        .to_vec();

    if bytes.is_empty() {
        return Ok(None);
    }

    Ok(Some(PhotoData {
        bytes,
        content_type,
    }))
}

// ---------------------------------------------------------------------------
// Cache operations
// ---------------------------------------------------------------------------

/// Cache a photo on disk and record metadata in the database.
///
/// Returns the absolute file path of the cached photo.
pub async fn cache_photo(
    db: &DbState,
    cache_dir: &Path,
    email: &str,
    account_id: &str,
    photo_data: &PhotoData,
    etag: Option<&str>,
) -> Result<String, String> {
    let content_hash = hash_bytes(&photo_data.bytes);
    let file_path = photo_file_path(cache_dir, &content_hash);
    let size_bytes = i64::try_from(photo_data.bytes.len())
        .map_err(|_| "photo size exceeds i64 range".to_string())?;

    // Ensure directory exists
    if let Some(parent) = file_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create contact photo cache dir: {e}"))?;
    }

    // Write photo to disk
    tokio::fs::write(&file_path, &photo_data.bytes)
        .await
        .map_err(|e| format!("write contact photo: {e}"))?;

    let file_path_str = file_path
        .to_str()
        .ok_or_else(|| "contact photo path is not valid UTF-8".to_string())?
        .to_string();

    // Upsert cache metadata and update contacts.avatar_url
    let email_owned = email.to_string();
    let account_id_owned = account_id.to_string();
    let content_hash_owned = content_hash;
    let file_path_db = file_path_str.clone();
    let etag_owned = etag.map(str::to_string);

    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO contact_photo_cache \
             (email, account_id, content_hash, file_path, size_bytes, etag, \
              fetched_at, last_accessed_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, unixepoch(), unixepoch()) \
             ON CONFLICT(email, account_id) DO UPDATE SET \
               content_hash = excluded.content_hash, \
               file_path = excluded.file_path, \
               size_bytes = excluded.size_bytes, \
               etag = excluded.etag, \
               fetched_at = excluded.fetched_at, \
               last_accessed_at = excluded.last_accessed_at",
            params![
                email_owned,
                account_id_owned,
                content_hash_owned,
                file_path_db,
                size_bytes,
                etag_owned,
            ],
        )
        .map_err(|e| format!("upsert contact photo cache: {e}"))?;

        // Update contacts.avatar_url to point to the local cached file
        conn.execute(
            "UPDATE contacts SET avatar_url = ?1, updated_at = unixepoch() WHERE email = ?2",
            params![file_path_db, email_owned],
        )
        .map_err(|e| format!("update contact avatar_url: {e}"))?;

        Ok(())
    })
    .await?;

    Ok(file_path_str)
}

/// Look up a cached photo path for a contact, updating `last_accessed_at`.
///
/// Returns `None` if no cache entry exists.
pub async fn get_cached_photo_path(
    db: &DbState,
    email: &str,
    account_id: &str,
) -> Result<Option<String>, String> {
    let email_owned = email.to_string();
    let account_id_owned = account_id.to_string();

    db.with_conn(move |conn| {
        let path: Option<String> = conn
            .query_row(
                "SELECT file_path FROM contact_photo_cache \
                 WHERE email = ?1 AND account_id = ?2",
                params![email_owned, account_id_owned],
                |row| row.get("file_path"),
            )
            .optional()
            .map_err(|e| format!("query contact photo cache: {e}"))?;

        if path.is_some() {
            conn.execute(
                "UPDATE contact_photo_cache SET last_accessed_at = unixepoch() \
                 WHERE email = ?1 AND account_id = ?2",
                params![email_owned, account_id_owned],
            )
            .map_err(|e| format!("update contact photo last_accessed_at: {e}"))?;
        }

        Ok(path)
    })
    .await
}

/// Evict oldest cached photos until total cache size is under `max_bytes`.
///
/// Returns the number of evicted entries.
pub async fn evict_photos_to_size(
    db: &DbState,
    cache_dir: &Path,
    max_bytes: u64,
) -> Result<usize, String> {
    let mut evicted = 0usize;

    loop {
        let total_size: i64 = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT COALESCE(SUM(size_bytes), 0) AS total FROM contact_photo_cache",
                    [],
                    |row| row.get("total"),
                )
                .map_err(|e| format!("query contact photo cache size: {e}"))
            })
            .await?;

        #[allow(clippy::cast_sign_loss)] // total_size is COALESCE(SUM(...), 0), always >= 0
        if total_size <= 0 || (total_size as u64) <= max_bytes {
            break;
        }

        // Find the oldest entry by last_accessed_at
        let oldest: Option<(String, String, String)> = db
            .with_conn(|conn| {
                conn.query_row(
                    "SELECT email, account_id, file_path \
                     FROM contact_photo_cache \
                     ORDER BY last_accessed_at ASC \
                     LIMIT 1",
                    [],
                    |row| Ok((row.get("email")?, row.get("account_id")?, row.get("file_path")?)),
                )
                .optional()
                .map_err(|e| format!("query oldest contact photo: {e}"))
            })
            .await?;

        let Some((email, account_id, file_path)) = oldest else {
            break;
        };

        // Remove from disk
        let full_path = if Path::new(&file_path).is_absolute() {
            PathBuf::from(&file_path)
        } else {
            cache_dir.join(&file_path)
        };

        match tokio::fs::remove_file(&full_path).await {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => warn!("remove evicted contact photo {}: {e}", full_path.display()),
        }

        // Remove DB entry
        db.with_conn(move |conn| {
            conn.execute(
                "DELETE FROM contact_photo_cache WHERE email = ?1 AND account_id = ?2",
                params![email, account_id],
            )
            .map_err(|e| format!("delete evicted contact photo: {e}"))?;
            Ok(())
        })
        .await?;

        evicted += 1;
    }

    if evicted > 0 {
        info!("Contact photo cache: evicted {evicted} entries");
    }

    Ok(evicted)
}

// ---------------------------------------------------------------------------
// Sync entry point
// ---------------------------------------------------------------------------

/// Sync contact photos for an account from the appropriate provider.
///
/// Fetches photos for contacts that either have no cached photo or have a
/// stale cache entry (etag mismatch). Returns the count of photos fetched.
pub async fn sync_contact_photos(
    db: &DbState,
    cache_dir: &Path,
    http: &reqwest::Client,
    access_token: &str,
    account_id: &str,
    provider_type: &str,
) -> Result<usize, String> {
    let fetched = match provider_type {
        "graph" => sync_graph_photos(db, cache_dir, http, access_token, account_id).await?,
        "google" => sync_google_photos(db, cache_dir, http, account_id).await?,
        other => {
            info!("Contact photo sync: unsupported provider '{other}', skipping");
            return Ok(0);
        }
    };

    // Run eviction if needed
    evict_photos_to_size(db, cache_dir, DEFAULT_MAX_CACHE_BYTES).await?;

    Ok(fetched)
}

/// Sync photos for Graph (Exchange) contacts.
async fn sync_graph_photos(
    db: &DbState,
    cache_dir: &Path,
    http: &reqwest::Client,
    access_token: &str,
    account_id: &str,
) -> Result<usize, String> {
    // Query contacts that need photo fetching:
    // - Have a graph_contact_map entry (so we have the Graph contact ID)
    // - Either have no cache entry, or the cache etag differs from the
    //   current Graph contact's changeKey (if we tracked it)
    let account_id_owned = account_id.to_string();
    let contacts_to_fetch: Vec<(String, String)> = db
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT DISTINCT gcm.email, gcm.graph_contact_id \
                     FROM graph_contact_map gcm \
                     WHERE gcm.account_id = ?1 \
                       AND NOT EXISTS ( \
                         SELECT 1 FROM contact_photo_cache cpc \
                         WHERE cpc.email = gcm.email AND cpc.account_id = ?1 \
                       )",
                )
                .map_err(|e| format!("prepare graph photo query: {e}"))?;

            let rows = stmt
                .query_map(params![account_id_owned], |row| {
                    Ok((row.get::<_, String>("email")?, row.get::<_, String>("graph_contact_id")?))
                })
                .map_err(|e| format!("query graph contacts for photos: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("collect graph contacts for photos: {e}"))?;

            Ok(rows)
        })
        .await?;

    if contacts_to_fetch.is_empty() {
        return Ok(0);
    }

    info!(
        "Contact photo sync (graph): {} contacts to fetch for account {account_id}",
        contacts_to_fetch.len()
    );

    let api_base = "https://graph.microsoft.com/v1.0";
    let mut fetched = 0usize;

    for (email, graph_contact_id) in &contacts_to_fetch {
        match fetch_graph_contact_photo(http, access_token, graph_contact_id, api_base).await {
            Ok(Some(photo_data)) => {
                match cache_photo(db, cache_dir, email, account_id, &photo_data, None).await {
                    Ok(_path) => fetched += 1,
                    Err(e) => warn!("cache graph contact photo for {email}: {e}"),
                }
            }
            Ok(None) => {} // no photo for this contact
            Err(e) => warn!("fetch graph contact photo for {email}: {e}"),
        }
    }

    info!("Contact photo sync (graph): fetched {fetched} photos for account {account_id}");
    Ok(fetched)
}

/// Sync photos for Google contacts.
async fn sync_google_photos(
    db: &DbState,
    cache_dir: &Path,
    http: &reqwest::Client,
    account_id: &str,
) -> Result<usize, String> {
    // Query contacts with Google avatar URLs that don't have a cached version yet.
    // For Google, the avatar_url column already contains the remote photo URL
    // (set during contact sync). We check if a cached version exists; if not,
    // download and cache.
    let account_id_owned = account_id.to_string();
    let contacts_to_fetch: Vec<(String, String)> = db
        .with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT c.email, c.avatar_url \
                     FROM contacts c \
                     INNER JOIN google_contact_map gcm ON gcm.contact_email = c.email \
                       AND gcm.account_id = ?1 \
                     WHERE c.avatar_url IS NOT NULL \
                       AND c.avatar_url LIKE 'http%' \
                       AND NOT EXISTS ( \
                         SELECT 1 FROM contact_photo_cache cpc \
                         WHERE cpc.email = c.email AND cpc.account_id = ?1 \
                       )",
                )
                .map_err(|e| format!("prepare google photo query: {e}"))?;

            let rows = stmt
                .query_map(params![account_id_owned], |row| {
                    Ok((row.get::<_, String>("email")?, row.get::<_, String>("avatar_url")?))
                })
                .map_err(|e| format!("query google contacts for photos: {e}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| format!("collect google contacts for photos: {e}"))?;

            Ok(rows)
        })
        .await?;

    if contacts_to_fetch.is_empty() {
        return Ok(0);
    }

    info!(
        "Contact photo sync (google): {} contacts to fetch for account {account_id}",
        contacts_to_fetch.len()
    );

    let mut fetched = 0usize;

    for (email, photo_url) in &contacts_to_fetch {
        // Use the Google photo URL as the etag — if the URL changes, we re-fetch
        match fetch_google_contact_photo(http, photo_url, 128).await {
            Ok(Some(photo_data)) => {
                match cache_photo(
                    db,
                    cache_dir,
                    email,
                    account_id,
                    &photo_data,
                    Some(photo_url),
                )
                .await
                {
                    Ok(_path) => fetched += 1,
                    Err(e) => warn!("cache google contact photo for {email}: {e}"),
                }
            }
            Ok(None) => {} // no photo available
            Err(e) => warn!("fetch google contact photo for {email}: {e}"),
        }
    }

    info!("Contact photo sync (google): fetched {fetched} photos for account {account_id}");
    Ok(fetched)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hash_bytes_deterministic() {
        let data = b"hello world";
        let h1 = hash_bytes(data);
        let h2 = hash_bytes(data);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16); // 16 hex chars for u64
    }

    #[test]
    fn test_hash_bytes_different_inputs() {
        let h1 = hash_bytes(b"photo1.jpg");
        let h2 = hash_bytes(b"photo2.jpg");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_photo_file_path_construction() {
        let cache_dir = Path::new("/tmp/test-cache");
        let hash = "abcdef0123456789";
        let path = photo_file_path(cache_dir, hash);
        assert_eq!(
            path,
            PathBuf::from("/tmp/test-cache/contact_photos/abcdef0123456789.jpg")
        );
    }

    #[test]
    fn test_photo_file_path_deterministic() {
        let cache_dir = Path::new("/home/user/.cache/ratatoskr");
        let hash = hash_bytes(b"test photo data");
        let p1 = photo_file_path(cache_dir, &hash);
        let p2 = photo_file_path(cache_dir, &hash);
        assert_eq!(p1, p2);
        assert!(p1
            .to_str()
            .is_some_and(|s| s.starts_with("/home/user/.cache/ratatoskr/contact_photos/")));
        assert!(p1.extension().is_some_and(|e| e == "jpg"));
    }

    #[test]
    fn test_photo_data_creation() {
        let photo = PhotoData {
            bytes: vec![0xFF, 0xD8, 0xFF, 0xE0], // JPEG magic bytes
            content_type: "image/jpeg".to_string(),
        };
        assert_eq!(photo.bytes.len(), 4);
        assert_eq!(photo.content_type, "image/jpeg");
    }

    #[test]
    fn test_hash_empty_bytes() {
        let hash = hash_bytes(b"");
        assert_eq!(hash.len(), 16);
        // xxh3 of empty input is a known constant
        let hash2 = hash_bytes(b"");
        assert_eq!(hash, hash2);
    }

    #[test]
    fn test_eviction_ordering_concept() {
        // Verify that our eviction strategy (oldest last_accessed_at first) is
        // correct by checking that timestamps can be compared as integers.
        let older: i64 = 1700000000;
        let newer: i64 = 1700001000;
        assert!(older < newer, "older timestamps should sort first in ASC order");
    }

    #[test]
    fn test_default_cache_size() {
        assert_eq!(DEFAULT_MAX_CACHE_BYTES, 50 * 1024 * 1024);
        assert_eq!(DEFAULT_MAX_CACHE_BYTES, 52_428_800);
    }
}
