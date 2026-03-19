use std::collections::{HashMap, HashSet};

use rusqlite::params;

use crate::db::DbState;

use super::client::CardDavClient;
use super::parse::{self, ParsedVCard};

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a CardDAV contact sync.
#[derive(Debug)]
pub struct SyncResult {
    pub upserted: usize,
    pub deleted: usize,
    pub skipped_no_email: usize,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Sync CardDAV contacts for an account.
///
/// Uses CTag-based change detection:
/// - If the CTag is unchanged since last sync, skip (no changes).
/// - If the CTag changed (or first sync), compare ETags to find changed/new
///   contacts, fetch only those, and prune contacts that disappeared.
pub async fn sync_carddav_contacts(
    client: &CardDavClient,
    db: &DbState,
    account_id: &str,
) -> Result<SyncResult, String> {
    // Check CTag for quick change detection
    let remote_ctag = client.get_ctag().await?;
    let stored_ctag = load_ctag(db, account_id).await?;

    if let (Some(remote), Some(stored)) = (&remote_ctag, &stored_ctag)
        && remote == stored
    {
        log::info!("CardDAV ctag unchanged for {account_id}, skipping sync");
        return Ok(SyncResult {
            upserted: 0,
            deleted: 0,
            skipped_no_email: 0,
        });
    }

    // List all contacts on the server
    let remote_entries = client.list_contacts().await?;

    // Load stored ETags for comparison
    let stored_etags = load_stored_etags(db, account_id).await?;

    // Determine which contacts are new or changed
    let mut fetch_uris: Vec<String> = Vec::new();
    let remote_uri_set: HashSet<String> = remote_entries.iter().map(|e| e.uri.clone()).collect();

    for entry in &remote_entries {
        match stored_etags.get(&entry.uri) {
            Some(old_etag) if *old_etag == entry.etag => {
                // ETag unchanged, skip
            }
            _ => {
                // New or changed
                fetch_uris.push(entry.uri.clone());
            }
        }
    }

    // Determine which contacts were deleted on the server
    let deleted_uris: Vec<String> = stored_etags
        .keys()
        .filter(|uri| !remote_uri_set.contains(*uri))
        .cloned()
        .collect();

    log::info!(
        "CardDAV sync for {account_id}: {} to fetch, {} unchanged, {} deleted",
        fetch_uris.len(),
        remote_entries.len() - fetch_uris.len(),
        deleted_uris.len()
    );

    // Fetch changed/new vCards
    let uri_refs: Vec<&str> = fetch_uris.iter().map(String::as_str).collect();
    let fetched_vcards = client.fetch_vcards(&uri_refs).await?;

    // Build ETag lookup from remote entries
    let etag_map: HashMap<&str, &str> = remote_entries
        .iter()
        .map(|e| (e.uri.as_str(), e.etag.as_str()))
        .collect();

    // Parse vCards and collect results
    let mut parsed_contacts: Vec<(String, String, ParsedVCard)> = Vec::new();
    let mut skipped_no_email = 0;

    for (uri, vcard_data) in &fetched_vcards {
        match parse::parse_vcard(vcard_data) {
            Ok(parsed) => {
                if parsed.email.is_some() {
                    let etag = etag_map
                        .get(uri.as_str())
                        .unwrap_or(&"")
                        .to_string();
                    parsed_contacts.push((uri.clone(), etag, parsed));
                } else {
                    skipped_no_email += 1;
                }
            }
            Err(e) => {
                log::warn!("Failed to parse vCard at {uri}: {e}");
                skipped_no_email += 1;
            }
        }
    }

    let upserted = parsed_contacts.len();
    let deleted_count = deleted_uris.len();

    // Persist to database
    let aid = account_id.to_string();
    let deleted_owned = deleted_uris;
    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;

        // Upsert changed/new contacts
        for (uri, etag, parsed) in &parsed_contacts {
            persist_carddav_contact(&tx, &aid, uri, etag, parsed)?;
        }

        // Delete removed contacts
        for uri in &deleted_owned {
            delete_carddav_contact(&tx, &aid, uri)?;
        }

        tx.commit().map_err(|e| format!("commit tx: {e}"))?;
        Ok(())
    })
    .await?;

    // Save the new CTag
    if let Some(ref ctag) = remote_ctag {
        save_ctag(db, account_id, ctag).await?;
    }

    log::info!(
        "CardDAV sync complete for {account_id}: {upserted} upserted, \
         {deleted_count} deleted, {skipped_no_email} skipped (no email)"
    );

    Ok(SyncResult {
        upserted,
        deleted: deleted_count,
        skipped_no_email,
    })
}

// ---------------------------------------------------------------------------
// Persistence helpers
// ---------------------------------------------------------------------------

fn persist_carddav_contact(
    conn: &rusqlite::Connection,
    account_id: &str,
    uri: &str,
    etag: &str,
    parsed: &ParsedVCard,
) -> Result<(), String> {
    let email = parsed
        .email
        .as_ref()
        .ok_or("contact has no email")?;

    let local_id = format!("carddav-{account_id}-{email}");
    let display_name = parsed
        .display_name
        .as_deref()
        .unwrap_or(email.as_str());
    let avatar_url = parsed.photo_url.as_deref();

    // Upsert into contacts table — don't overwrite user-edited or higher-priority sources
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, avatar_url, source) \
         VALUES (?1, ?2, ?3, ?4, 'carddav') \
         ON CONFLICT(email) DO UPDATE SET \
           display_name = CASE \
             WHEN contacts.source = 'user' THEN contacts.display_name \
             WHEN contacts.display_name_overridden = 1 THEN contacts.display_name \
             ELSE COALESCE(excluded.display_name, contacts.display_name) \
           END, \
           avatar_url = CASE \
             WHEN contacts.source = 'user' THEN contacts.avatar_url \
             ELSE COALESCE(excluded.avatar_url, contacts.avatar_url) \
           END, \
           source = CASE \
             WHEN contacts.source = 'user' THEN 'user' \
             ELSE 'carddav' \
           END, \
           updated_at = unixepoch()",
        params![local_id, email, display_name, avatar_url],
    )
    .map_err(|e| format!("upsert carddav contact: {e}"))?;

    // Track in mapping table
    conn.execute(
        "INSERT OR REPLACE INTO carddav_contact_map \
         (uri, account_id, contact_email, etag) \
         VALUES (?1, ?2, ?3, ?4)",
        params![uri, account_id, email, etag],
    )
    .map_err(|e| format!("upsert carddav contact map: {e}"))?;

    Ok(())
}

fn delete_carddav_contact(
    conn: &rusqlite::Connection,
    account_id: &str,
    uri: &str,
) -> Result<(), String> {
    // Look up the email for this CardDAV contact
    let email: Option<String> = conn
        .query_row(
            "SELECT contact_email FROM carddav_contact_map \
             WHERE uri = ?1 AND account_id = ?2",
            params![uri, account_id],
            |row| row.get("contact_email"),
        )
        .ok();

    // Remove the mapping row
    conn.execute(
        "DELETE FROM carddav_contact_map \
         WHERE uri = ?1 AND account_id = ?2",
        params![uri, account_id],
    )
    .map_err(|e| format!("delete carddav contact map: {e}"))?;

    // Only delete the contacts row if no other mapping references that email
    if let Some(ref email) = email {
        let carddav_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM carddav_contact_map WHERE contact_email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("count remaining carddav mappings: {e}"))?;

        // Also check other providers' mappings
        let google_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM google_contact_map WHERE contact_email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .unwrap_or(0);

        let graph_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM graph_contact_map WHERE email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .unwrap_or(0);

        if carddav_remaining == 0 && google_remaining == 0 && graph_remaining == 0 {
            conn.execute(
                "DELETE FROM contacts WHERE email = ?1 AND source = 'carddav'",
                params![email],
            )
            .map_err(|e| format!("delete orphaned carddav contact: {e}"))?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// CTag persistence (settings table)
// ---------------------------------------------------------------------------

async fn load_ctag(db: &DbState, account_id: &str) -> Result<Option<String>, String> {
    let key = format!("carddav_ctag:{account_id}");
    db.with_conn(move |conn| crate::db::get_setting(conn, &key))
        .await
}

async fn save_ctag(db: &DbState, account_id: &str, ctag: &str) -> Result<(), String> {
    let key = format!("carddav_ctag:{account_id}");
    let ctag_owned = ctag.to_string();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
            params![key, ctag_owned],
        )
        .map_err(|e| format!("save carddav ctag: {e}"))?;
        Ok(())
    })
    .await
}

// ---------------------------------------------------------------------------
// ETag loading for incremental sync
// ---------------------------------------------------------------------------

async fn load_stored_etags(
    db: &DbState,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT uri, etag FROM carddav_contact_map WHERE account_id = ?1",
            )
            .map_err(|e| format!("prepare etag query: {e}"))?;

        let rows = stmt
            .query_map(params![aid], |row| {
                Ok((
                    row.get::<_, String>("uri")?,
                    row.get::<_, Option<String>>("etag")?,
                ))
            })
            .map_err(|e| format!("query etags: {e}"))?;

        let mut map = HashMap::new();
        for row in rows {
            let (uri, etag) = row.map_err(|e| format!("read etag row: {e}"))?;
            if let Some(etag) = etag {
                map.insert(uri, etag);
            }
        }

        Ok(map)
    })
    .await
}
