use std::collections::{HashMap, HashSet};

use crate::db::DbState;
use crate::db::queries_extra::contact_carddav::CarddavContactUpsert;

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
                    let etag = etag_map.get(uri.as_str()).unwrap_or(&"").to_string();
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

    // Map parsed vCards to db upsert structs
    let mut upserts = Vec::with_capacity(parsed_contacts.len());
    for (uri, etag, parsed) in &parsed_contacts {
        let email = match &parsed.email {
            Some(e) => e.clone(),
            None => continue,
        };
        let display_name = parsed
            .display_name
            .clone()
            .unwrap_or_else(|| email.clone());
        upserts.push(CarddavContactUpsert {
            uri: uri.clone(),
            etag: etag.clone(),
            email,
            display_name,
            avatar_url: parsed.photo_url.clone(),
            phone: parsed.phone.clone(),
            company: parsed.organization.clone(),
        });
    }
    let upserted = upserts.len();
    let deleted_count = deleted_uris.len();

    // Persist to database via db layer (single transaction)
    let aid = account_id.to_string();
    let deleted_owned = deleted_uris;
    db.with_conn(move |conn| {
        crate::db::queries_extra::contact_carddav::persist_carddav_contacts_sync(
            conn,
            &aid,
            &upserts,
            &deleted_owned,
        )
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
// CTag / ETag / persist / delete — delegated to db
// ---------------------------------------------------------------------------

async fn load_ctag(db: &DbState, account_id: &str) -> Result<Option<String>, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        crate::db::queries_extra::contact_carddav::load_carddav_ctag_sync(conn, &aid)
    })
    .await
}

async fn save_ctag(db: &DbState, account_id: &str, ctag: &str) -> Result<(), String> {
    let aid = account_id.to_string();
    let ctag_owned = ctag.to_string();
    db.with_conn(move |conn| {
        crate::db::queries_extra::contact_carddav::save_carddav_ctag_sync(
            conn,
            &aid,
            &ctag_owned,
        )
    })
    .await
}

async fn load_stored_etags(
    db: &DbState,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        crate::db::queries_extra::contact_carddav::load_carddav_etags_sync(conn, &aid)
    })
    .await
}
