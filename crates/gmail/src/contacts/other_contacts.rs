use std::collections::HashSet;
use std::future::Future;

use db::db::{ReadDbState, WriteTxn};
use db::db::queries_extra::{delete_seen_address_google_other, upsert_seen_address_google_other};
use sync::state as sync_state;

use super::super::client::GmailClient;
use super::{
    OtherContactsResponse, PAGE_SIZE, Person, SyncContactsResult, extract_display_name,
    extract_primary_email, people_api_base,
};

const OTHER_CONTACTS_READ_MASK: &str = "names,emailAddresses,phoneNumbers,photos,metadata";

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Sync Google otherContacts (auto-collected contacts from interactions) for an
/// account via the People API. These are inserted into `seen_addresses` with
/// `source = 'google_other'` rather than the `contacts` table, since they are
/// lower-priority autocomplete candidates.
pub async fn sync_google_other_contacts<F, Fut>(
    client: &GmailClient,
    account_id: &str,
    db: &ReadDbState,
    mut write: F,
) -> Result<SyncContactsResult, String>
where
    F: FnMut(GoogleOtherContactsWrite) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let existing_token = sync_state::load_google_other_contacts_sync_token(db, account_id).await?;

    match existing_token {
        Some(token) => match incremental_sync(client, account_id, db, &token, &mut write).await {
            Ok(result) => Ok(result),
            Err(e) if e.contains("410") || e.contains("GONE") || e.contains("syncToken") => {
                log::warn!(
                    "Google otherContacts sync token expired for {account_id}, \
                         falling back to full sync"
                );
                sync_state::delete_google_other_contacts_sync_token(db, account_id).await?;
                full_sync(client, account_id, db, &mut write).await
            }
            Err(e) => Err(e),
        },
        None => full_sync(client, account_id, db, &mut write).await,
    }
}

pub enum GoogleOtherContactsWrite {
    Full {
        account_id: String,
        persons: Vec<Person>,
        seen_resource_names: HashSet<String>,
    },
    Delta {
        account_id: String,
        upserts: Vec<Person>,
        deleted_resource_names: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Full sync
// ---------------------------------------------------------------------------

async fn full_sync<F, Fut>(
    client: &GmailClient,
    account_id: &str,
    db: &ReadDbState,
    write: &mut F,
) -> Result<SyncContactsResult, String>
where
    F: FnMut(GoogleOtherContactsWrite) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let mut all_persons = Vec::new();
    let mut page_token: Option<String> = None;
    let mut sync_token: Option<String> = None;
    let api_base = people_api_base();

    loop {
        let mut url = format!(
            "{api_base}/otherContacts?readMask={OTHER_CONTACTS_READ_MASK}\
             &pageSize={PAGE_SIZE}&requestSyncToken=true"
        );
        if let Some(ref pt) = page_token {
            url.push_str(&format!("&pageToken={pt}"));
        }

        let response: OtherContactsResponse = client.get_absolute(&url, db).await?;

        if let Some(contacts) = response.other_contacts {
            all_persons.extend(contacts);
        }

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

    write(GoogleOtherContactsWrite::Full {
        account_id: account_id.to_string(),
        persons: all_persons,
        seen_resource_names,
    })
    .await?;

    if let Some(ref token) = sync_token {
        sync_state::save_google_other_contacts_sync_token(db, account_id, token).await?;
    }

    log::info!("Google otherContacts full sync for {account_id}: {synced} contacts with emails");

    Ok(SyncContactsResult { synced, deleted: 0 })
}

// ---------------------------------------------------------------------------
// Incremental sync
// ---------------------------------------------------------------------------

async fn incremental_sync<F, Fut>(
    client: &GmailClient,
    account_id: &str,
    db: &ReadDbState,
    sync_token: &str,
    write: &mut F,
) -> Result<SyncContactsResult, String>
where
    F: FnMut(GoogleOtherContactsWrite) -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let mut upserts = Vec::new();
    let mut deleted_resource_names = Vec::new();
    let mut page_token: Option<String> = None;
    let mut new_sync_token: Option<String> = None;
    let api_base = people_api_base();

    loop {
        let mut url = format!(
            "{api_base}/otherContacts?readMask={OTHER_CONTACTS_READ_MASK}\
             &pageSize={PAGE_SIZE}&requestSyncToken=true&syncToken={sync_token}"
        );
        if let Some(ref pt) = page_token {
            url.push_str(&format!("&pageToken={pt}"));
        }

        let response: OtherContactsResponse = client.get_absolute(&url, db).await?;

        if let Some(contacts) = response.other_contacts {
            for person in contacts {
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
        write(GoogleOtherContactsWrite::Delta {
            account_id: account_id.to_string(),
            upserts,
            deleted_resource_names,
        })
        .await?;
    }

    if let Some(ref token) = new_sync_token {
        sync_state::save_google_other_contacts_sync_token(db, account_id, token).await?;
    }

    log::info!(
        "Google otherContacts incremental sync for {account_id}: \
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

pub fn persist_google_other_contacts_write(
    tx: &WriteTxn<'_>,
    write: GoogleOtherContactsWrite,
) -> Result<(), String> {
    match write {
        GoogleOtherContactsWrite::Full {
            account_id,
            persons,
            seen_resource_names,
        } => {
            persist_other_contacts(tx, &account_id, &persons)?;
            let _ = prune_stale_other_contacts(tx, &account_id, &seen_resource_names)?;
        }
        GoogleOtherContactsWrite::Delta {
            account_id,
            upserts,
            deleted_resource_names,
        } => {
            persist_other_contacts(tx, &account_id, &upserts)?;
            for resource_name in &deleted_resource_names {
                delete_other_contact(tx, &account_id, resource_name)?;
            }
        }
    }
    Ok(())
}

fn persist_other_contacts(
    conn: &WriteTxn<'_>,
    account_id: &str,
    persons: &[Person],
) -> Result<(), String> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs().cast_signed())
        .unwrap_or(0);

    for person in persons {
        let Some(ref resource_name) = person.resource_name else {
            continue;
        };

        let Some(email) = extract_primary_email(person) else {
            continue;
        };

        let display_name = extract_display_name(person, &email);

        // Upsert into seen_addresses - don't overwrite locally-observed data
        upsert_seen_address_google_other(conn, &email, account_id, Some(&display_name), now)?;

        // Track the mapping for deletion handling
        conn.execute(
            "INSERT OR REPLACE INTO google_other_contact_map \
             (resource_name, account_id, contact_email) \
             VALUES (?1, ?2, ?3)",
            rusqlite::params![resource_name, account_id, email],
        )
        .map_err(|e| format!("upsert google other contact map: {e}"))?;
    }

    Ok(())
}

fn delete_other_contact(
    conn: &WriteTxn<'_>,
    account_id: &str,
    resource_name: &str,
) -> Result<(), String> {
    // Look up the email for this otherContact
    let email: Option<String> = conn
        .query_row(
            "SELECT contact_email FROM google_other_contact_map \
             WHERE resource_name = ?1 AND account_id = ?2",
            rusqlite::params![resource_name, account_id],
            |row| row.get("contact_email"),
        )
        .ok();

    // Remove the mapping row
    conn.execute(
        "DELETE FROM google_other_contact_map \
         WHERE resource_name = ?1 AND account_id = ?2",
        rusqlite::params![resource_name, account_id],
    )
    .map_err(|e| format!("delete google other contact map: {e}"))?;

    // Only delete the seen_addresses row if no other otherContact mapping
    // references that email AND the source is 'google_other' (don't delete
    // locally-observed addresses)
    if let Some(ref email) = email {
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM google_other_contact_map WHERE contact_email = ?1",
                rusqlite::params![email],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("count remaining other contact mappings: {e}"))?;

        if remaining == 0 {
            delete_seen_address_google_other(conn, email, account_id)?;
        }
    }

    Ok(())
}

/// After a full otherContacts sync, remove mapping rows for resource_names
/// not seen in the fetch, then delete orphaned source='google_other' addresses.
fn prune_stale_other_contacts(
    conn: &WriteTxn<'_>,
    account_id: &str,
    seen_resource_names: &HashSet<String>,
) -> Result<usize, String> {
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT resource_name FROM google_other_contact_map \
             WHERE account_id = ?1",
        )
        .map_err(|e| format!("prepare stale other contacts: {e}"))?;

    let all_mapped: Vec<String> = stmt
        .query_map(rusqlite::params![account_id], |row| {
            row.get::<_, String>("resource_name")
        })
        .map_err(|e| format!("query stale other contacts: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect stale other contacts: {e}"))?;

    drop(stmt);

    let mut pruned = 0;
    for resource_name in &all_mapped {
        if !seen_resource_names.contains(resource_name) {
            delete_other_contact(conn, account_id, resource_name)?;
            pruned += 1;
        }
    }

    Ok(pruned)
}
