use std::collections::HashSet;

use rusqlite::params;

use db::db::DbState;
use sync::state as sync_state;

use super::super::client::GmailClient;
use super::{
    PAGE_SIZE, PEOPLE_API_BASE, PeopleConnectionsResponse, Person, SyncContactsResult,
    extract_avatar_url, extract_display_name, extract_primary_email,
};

const PERSON_FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations,photos,metadata";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Sync Google contacts for an account via the People API.
///
/// Uses incremental sync if a sync token exists, falling back to full sync
/// on 410 Gone or when no token is stored.
pub async fn sync_google_contacts(
    client: &GmailClient,
    account_id: &str,
    db: &DbState,
) -> Result<SyncContactsResult, String> {
    let existing_token = sync_state::load_google_contacts_sync_token(db, account_id).await?;

    match existing_token {
        Some(token) => match incremental_sync(client, account_id, db, &token).await {
            Ok(result) => Ok(result),
            Err(e) if e.contains("410") || e.contains("GONE") || e.contains("syncToken") => {
                log::warn!(
                    "Google contacts sync token expired for {account_id}, falling back to full sync"
                );
                sync_state::delete_google_contacts_sync_token(db, account_id).await?;
                full_sync(client, account_id, db).await
            }
            Err(e) => Err(e),
        },
        None => full_sync(client, account_id, db).await,
    }
}

// ---------------------------------------------------------------------------
// Full sync
// ---------------------------------------------------------------------------

async fn full_sync(
    client: &GmailClient,
    account_id: &str,
    db: &DbState,
) -> Result<SyncContactsResult, String> {
    let mut all_persons = Vec::new();
    let mut page_token: Option<String> = None;
    let mut sync_token: Option<String> = None;

    loop {
        let mut url = format!(
            "{PEOPLE_API_BASE}/people/me/connections?personFields={PERSON_FIELDS}\
             &pageSize={PAGE_SIZE}&requestSyncToken=true"
        );
        if let Some(ref pt) = page_token {
            url.push_str(&format!("&pageToken={pt}"));
        }

        let response: PeopleConnectionsResponse = client.get_absolute(&url, db).await?;

        if let Some(connections) = response.connections {
            all_persons.extend(connections);
        }

        // The final page contains the sync token
        if response.next_sync_token.is_some() {
            sync_token = response.next_sync_token;
        }

        match response.next_page_token {
            Some(pt) => page_token = Some(pt),
            None => break,
        }
    }

    let seen_resource_names: HashSet<String> = all_persons
        .iter()
        .filter_map(|p| p.resource_name.clone())
        .collect();

    let synced = all_persons
        .iter()
        .filter(|p| extract_primary_email(p).is_some())
        .count();

    let aid = account_id.to_string();
    let seen = seen_resource_names;
    db.with_conn(move |conn| {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| format!("begin tx: {e}"))?;
        persist_google_contacts(&tx, &aid, &all_persons)?;
        let pruned = prune_stale_google_contacts(&tx, &aid, &seen)?;
        tx.commit().map_err(|e| format!("commit tx: {e}"))?;
        Ok(pruned)
    })
    .await?;

    // Save sync token for future incremental syncs
    if let Some(ref token) = sync_token {
        sync_state::save_google_contacts_sync_token(db, account_id, token).await?;
    }

    log::info!("Google contacts full sync for {account_id}: {synced} contacts with emails");

    Ok(SyncContactsResult { synced, deleted: 0 })
}

// ---------------------------------------------------------------------------
// Incremental sync
// ---------------------------------------------------------------------------

async fn incremental_sync(
    client: &GmailClient,
    account_id: &str,
    db: &DbState,
    sync_token: &str,
) -> Result<SyncContactsResult, String> {
    let mut upserts = Vec::new();
    let mut deleted_resource_names = Vec::new();
    let mut page_token: Option<String> = None;
    let mut new_sync_token: Option<String> = None;

    loop {
        let mut url = format!(
            "{PEOPLE_API_BASE}/people/me/connections?personFields={PERSON_FIELDS}\
             &pageSize={PAGE_SIZE}&requestSyncToken=true&syncToken={sync_token}"
        );
        if let Some(ref pt) = page_token {
            url.push_str(&format!("&pageToken={pt}"));
        }

        let response: PeopleConnectionsResponse = client.get_absolute(&url, db).await?;

        if let Some(connections) = response.connections {
            for person in connections {
                let is_deleted = person
                    .metadata
                    .as_ref()
                    .and_then(|m| m.deleted)
                    .unwrap_or(false);

                if is_deleted {
                    if let Some(ref rn) = person.resource_name {
                        deleted_resource_names.push(rn.clone());
                    }
                } else {
                    upserts.push(person);
                }
            }
        }

        if response.next_sync_token.is_some() {
            new_sync_token = response.next_sync_token;
        }

        match response.next_page_token {
            Some(pt) => page_token = Some(pt),
            None => break,
        }
    }

    let synced = upserts
        .iter()
        .filter(|p| extract_primary_email(p).is_some())
        .count();
    let deleted_count = deleted_resource_names.len();

    if !upserts.is_empty() || !deleted_resource_names.is_empty() {
        let aid = account_id.to_string();
        let deleted_owned = deleted_resource_names;
        db.with_conn(move |conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| format!("begin tx: {e}"))?;
            persist_google_contacts(&tx, &aid, &upserts)?;
            for resource_name in &deleted_owned {
                delete_google_contact(&tx, &aid, resource_name)?;
            }
            tx.commit().map_err(|e| format!("commit tx: {e}"))?;
            Ok(())
        })
        .await?;
    }

    if let Some(ref token) = new_sync_token {
        sync_state::save_google_contacts_sync_token(db, account_id, token).await?;
    }

    log::info!(
        "Google contacts incremental sync for {account_id}: \
         {synced} upserted, {deleted_count} deleted"
    );

    Ok(SyncContactsResult {
        synced,
        deleted: deleted_count,
    })
}

// ---------------------------------------------------------------------------
// Persistence helpers (run inside a transaction)
// ---------------------------------------------------------------------------

fn persist_google_contacts(
    conn: &rusqlite::Connection,
    account_id: &str,
    persons: &[Person],
) -> Result<(), String> {
    for person in persons {
        let Some(ref resource_name) = person.resource_name else {
            continue;
        };

        let Some(email) = extract_primary_email(person) else {
            continue;
        };

        let display_name = extract_display_name(person, &email);
        let avatar_url = extract_avatar_url(person);
        let local_id = format!("google-{account_id}-{email}");

        let phone = person
            .phone_numbers
            .as_ref()
            .and_then(|nums| nums.first())
            .and_then(|p| p.value.as_deref())
            .filter(|v| !v.is_empty());
        let company = person
            .organizations
            .as_ref()
            .and_then(|orgs| orgs.first())
            .and_then(|o| o.name.as_deref())
            .filter(|v| !v.is_empty());

        conn.execute(
            "INSERT INTO contacts (id, email, display_name, avatar_url, phone, company,
                                   source, account_id, server_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'google', ?7, ?8) \
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
               phone = CASE \
                 WHEN contacts.source = 'user' THEN contacts.phone \
                 ELSE COALESCE(excluded.phone, contacts.phone) \
               END, \
               company = CASE \
                 WHEN contacts.source = 'user' THEN contacts.company \
                 ELSE COALESCE(excluded.company, contacts.company) \
               END, \
               source = CASE \
                 WHEN contacts.source = 'user' THEN 'user' \
                 ELSE 'google' \
               END, \
               account_id = COALESCE(excluded.account_id, contacts.account_id), \
               server_id = COALESCE(excluded.server_id, contacts.server_id), \
               updated_at = unixepoch()",
            params![
                local_id,
                email,
                display_name,
                avatar_url,
                phone,
                company,
                account_id,
                resource_name
            ],
        )
        .map_err(|e| format!("upsert google contact: {e}"))?;

        conn.execute(
            "INSERT OR REPLACE INTO google_contact_map \
             (resource_name, account_id, contact_email) \
             VALUES (?1, ?2, ?3)",
            params![resource_name, account_id, email],
        )
        .map_err(|e| format!("upsert google contact map: {e}"))?;
    }

    Ok(())
}

fn delete_google_contact(
    conn: &rusqlite::Connection,
    account_id: &str,
    resource_name: &str,
) -> Result<(), String> {
    // Look up the email for this Google contact
    let email: Option<String> = conn
        .query_row(
            "SELECT contact_email FROM google_contact_map \
             WHERE resource_name = ?1 AND account_id = ?2",
            params![resource_name, account_id],
            |row| row.get("contact_email"),
        )
        .ok();

    // Remove the mapping row
    conn.execute(
        "DELETE FROM google_contact_map \
         WHERE resource_name = ?1 AND account_id = ?2",
        params![resource_name, account_id],
    )
    .map_err(|e| format!("delete google contact map: {e}"))?;

    // Only delete the contacts row if no other mapping references that email
    if let Some(ref email) = email {
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM google_contact_map WHERE contact_email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("count remaining google mappings: {e}"))?;

        // Also check graph_contact_map to not delete contacts synced from Exchange
        let graph_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM graph_contact_map WHERE email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .unwrap_or(0);

        if remaining == 0 && graph_remaining == 0 {
            conn.execute(
                "DELETE FROM contacts WHERE email = ?1 AND source = 'google'",
                params![email],
            )
            .map_err(|e| format!("delete orphaned google contact: {e}"))?;
        }
    }

    Ok(())
}

/// After a full sync, remove mapping rows for resource_names not seen
/// in the fetch, then delete orphaned source='google' contacts.
fn prune_stale_google_contacts(
    conn: &rusqlite::Connection,
    account_id: &str,
    seen_resource_names: &HashSet<String>,
) -> Result<usize, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT resource_name FROM google_contact_map \
             WHERE account_id = ?1",
        )
        .map_err(|e| format!("prepare stale google contacts: {e}"))?;

    let all_mapped: Vec<String> = stmt
        .query_map(params![account_id], |row| {
            row.get::<_, String>("resource_name")
        })
        .map_err(|e| format!("query stale google contacts: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect stale google contacts: {e}"))?;

    drop(stmt);

    let mut pruned = 0;
    for resource_name in &all_mapped {
        if !seen_resource_names.contains(resource_name) {
            delete_google_contact(conn, account_id, resource_name)?;
            pruned += 1;
        }
    }

    Ok(pruned)
}
