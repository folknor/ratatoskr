//! GAL (Global Address List) / organization directory caching.
//!
//! SQL lives in `db::queries_extra::contacts`. This module keeps
//! HTTP fetch logic and cache-age orchestration.

use crate::db::DbState;

// Re-export the entry type from db.
pub use crate::db::queries_extra::contacts::GalEntry;

/// Store fetched GAL entries in the cache for a given account (full refresh).
pub async fn cache_gal_entries(
    db: &DbState,
    account_id: String,
    entries: Vec<GalEntry>,
) -> Result<usize, String> {
    let aid = account_id;
    db.with_conn(move |conn| {
        crate::db::queries_extra::contacts::cache_gal_entries_sync(conn, &aid, &entries)
    })
    .await
}

// ── Provider fetch functions ────────────────────────────────

/// Fetch the organization directory from Microsoft Graph (`/users`).
///
/// Requires the `User.ReadBasic.All` or `User.Read.All` Graph permission.
/// Paginates using `@odata.nextLink` until all users are fetched.
pub async fn fetch_graph_gal(
    client: &graph::client::GraphClient,
    db: &DbState,
) -> Result<Vec<GalEntry>, String> {
    let select = "displayName,mail,businessPhones,companyName,jobTitle,department";
    let mut entries = Vec::new();
    let mut url = format!("/users?$select={select}&$top=999");

    loop {
        let resp: serde_json::Value = client
            .get_json(&url, db)
            .await
            .map_err(|e| format!("Graph /users: {e}"))?;

        if let Some(users) = resp["value"].as_array() {
            for user in users {
                let email = user["mail"]
                    .as_str()
                    .filter(|s| !s.is_empty())
                    .map(str::to_string);
                let Some(email) = email else { continue };
                entries.push(GalEntry {
                    email,
                    display_name: user["displayName"].as_str().map(str::to_string),
                    phone: user["businessPhones"]
                        .as_array()
                        .and_then(|a| a.first())
                        .and_then(|v| v.as_str())
                        .map(str::to_string),
                    company: user["companyName"].as_str().map(str::to_string),
                    title: user["jobTitle"].as_str().map(str::to_string),
                    department: user["department"].as_str().map(str::to_string),
                });
            }
        }

        // Follow pagination
        match resp["@odata.nextLink"].as_str() {
            Some(next) => {
                // nextLink is an absolute URL — strip the Graph base to get the path
                url = next
                    .strip_prefix("https://graph.microsoft.com/v1.0")
                    .unwrap_or(next)
                    .to_string();
            }
            None => break,
        }
    }

    log::info!("[Graph-GAL] Fetched {} directory entries", entries.len());
    Ok(entries)
}

/// Fetch the organization directory from Google People API
/// (`people.listDirectoryPeople`).
///
/// Requires the `https://www.googleapis.com/auth/directory.readonly` scope.
/// Returns an empty vec if the scope is not granted (403).
pub async fn fetch_google_gal(
    client: &gmail::client::GmailClient,
    db: &DbState,
) -> Result<Vec<GalEntry>, String> {
    let mut entries = Vec::new();
    let mut page_token: Option<String> = None;
    let read_mask = "names,emailAddresses,phoneNumbers,organizations";

    loop {
        let mut url = format!(
            "https://people.googleapis.com/v1/people:listDirectoryPeople\
             ?readMask={read_mask}&sources=DIRECTORY_SOURCE_TYPE_DOMAIN_PROFILE\
             &pageSize=1000"
        );
        if let Some(ref token) = page_token {
            url.push_str(&format!("&pageToken={token}"));
        }

        let resp: serde_json::Value = match client.get_absolute(&url, db).await {
            Ok(r) => r,
            Err(e) => {
                // 403 = scope not granted (personal Gmail, not Workspace)
                if e.contains("403") || e.contains("PERMISSION_DENIED") {
                    log::debug!(
                        "[Google-GAL] Directory access not available (likely personal account)"
                    );
                    return Ok(Vec::new());
                }
                return Err(format!("Google listDirectoryPeople: {e}"));
            }
        };

        if let Some(people) = resp["people"].as_array() {
            for person in people {
                let email = person["emailAddresses"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|e| e["value"].as_str())
                    .map(str::to_string);
                let Some(email) = email else { continue };

                let display_name = person["names"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|n| n["displayName"].as_str())
                    .map(str::to_string);

                let phone = person["phoneNumbers"]
                    .as_array()
                    .and_then(|a| a.first())
                    .and_then(|p| p["value"].as_str())
                    .map(str::to_string);

                let org = person["organizations"].as_array().and_then(|a| a.first());
                let company = org.and_then(|o| o["name"].as_str()).map(str::to_string);
                let title = org.and_then(|o| o["title"].as_str()).map(str::to_string);
                let department = org
                    .and_then(|o| o["department"].as_str())
                    .map(str::to_string);

                entries.push(GalEntry {
                    email,
                    display_name,
                    phone,
                    company,
                    title,
                    department,
                });
            }
        }

        match resp["nextPageToken"].as_str() {
            Some(token) => page_token = Some(token.to_string()),
            None => break,
        }
    }

    log::info!("[Google-GAL] Fetched {} directory entries", entries.len());
    Ok(entries)
}

/// Refresh the GAL cache for a single account if stale (>24h).
/// Determines the provider type from the DB and dispatches to the
/// appropriate fetch function. Returns the number of entries cached,
/// or 0 if the cache was fresh or the provider doesn't support GAL.
pub async fn refresh_gal_for_account(
    db: &DbState,
    account_id: &str,
    encryption_key: [u8; 32],
) -> Result<usize, String> {
    // Check cache age
    let now = chrono::Utc::now().timestamp();
    let stale_threshold = now - 86400; // 24 hours
    if let Some(cached_at) = gal_cache_age(db, account_id.to_string()).await? {
        if cached_at > stale_threshold {
            return Ok(0); // cache is fresh
        }
    }

    // Look up provider type via db
    let aid = account_id.to_string();
    let provider: String = db
        .with_conn(move |conn| {
            crate::db::queries_extra::contacts::get_account_provider_sync(conn, &aid)
        })
        .await?;

    let entries = match provider.as_str() {
        "graph" => {
            let client =
                crate::graph::client::GraphClient::from_account(db, account_id, encryption_key)
                    .await?;
            fetch_graph_gal(&client, db).await?
        }
        "gmail_api" => {
            let client =
                crate::gmail::client::GmailClient::from_account(db, account_id, encryption_key)
                    .await?;
            fetch_google_gal(&client, db).await?
        }
        _ => return Ok(0), // IMAP/JMAP don't have organization directories
    };

    let count = entries.len();
    cache_gal_entries(db, account_id.to_string(), entries).await?;
    record_gal_refresh(db, account_id.to_string()).await?;
    Ok(count)
}

/// Record that a GAL refresh was performed for an account.
async fn record_gal_refresh(db: &DbState, account_id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        crate::db::queries_extra::contacts::record_gal_refresh_sync(conn, &account_id)
    })
    .await
}

/// Get the timestamp of the last GAL refresh attempt for an account.
pub async fn gal_cache_age(db: &DbState, account_id: String) -> Result<Option<i64>, String> {
    db.with_conn(move |conn| {
        crate::db::queries_extra::contacts::gal_cache_age_sync(conn, &account_id)
    })
    .await
}
