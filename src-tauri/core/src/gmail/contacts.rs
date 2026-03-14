use std::collections::HashSet;

use rusqlite::params;
use serde::Deserialize;

use crate::db::DbState;
use crate::sync::state as sync_state;

use super::client::GmailClient;

const PEOPLE_API_BASE: &str = "https://people.googleapis.com/v1";
const PERSON_FIELDS: &str = "names,emailAddresses,phoneNumbers,organizations,photos,metadata";
const PAGE_SIZE: u32 = 1000;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Result of a Google contacts sync.
#[derive(Debug)]
pub struct SyncContactsResult {
    pub synced: usize,
    pub deleted: usize,
}

// ---------------------------------------------------------------------------
// API response types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PeopleConnectionsResponse {
    pub connections: Option<Vec<Person>>,
    pub next_page_token: Option<String>,
    pub next_sync_token: Option<String>,
    pub total_people: Option<i32>,
    pub total_items: Option<i32>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Person {
    pub resource_name: Option<String>,
    pub etag: Option<String>,
    pub metadata: Option<PersonMetadata>,
    pub names: Option<Vec<Name>>,
    pub email_addresses: Option<Vec<EmailAddress>>,
    pub phone_numbers: Option<Vec<PhoneNumber>>,
    pub organizations: Option<Vec<Organization>>,
    pub photos: Option<Vec<Photo>>,
}

#[derive(Debug, Deserialize)]
pub struct PersonMetadata {
    pub deleted: Option<bool>,
    pub sources: Option<Vec<Source>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    #[serde(rename = "type")]
    pub source_type: Option<String>,
    pub id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Name {
    pub display_name: Option<String>,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EmailAddress {
    pub value: Option<String>,
    #[serde(rename = "type")]
    pub email_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PhoneNumber {
    pub value: Option<String>,
    #[serde(rename = "type")]
    pub phone_type: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Organization {
    pub name: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct Photo {
    pub url: Option<String>,
}

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

    log::info!(
        "Google contacts full sync for {account_id}: {synced} contacts with emails"
    );

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

        conn.execute(
            "INSERT INTO contacts (id, email, display_name, avatar_url, source) \
             VALUES (?1, ?2, ?3, ?4, 'google') \
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
                 ELSE 'google' \
               END, \
               updated_at = unixepoch()",
            params![local_id, email, display_name, avatar_url],
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
            |row| row.get(0),
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
                "SELECT COUNT(*) FROM google_contact_map WHERE contact_email = ?1",
                params![email],
                |row| row.get(0),
            )
            .map_err(|e| format!("count remaining google mappings: {e}"))?;

        // Also check graph_contact_map to not delete contacts synced from Exchange
        let graph_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM graph_contact_map WHERE email = ?1",
                params![email],
                |row| row.get(0),
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
        .query_map(params![account_id], |row| row.get::<_, String>(0))
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

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract the first valid, lowercased email address from a Person.
fn extract_primary_email(person: &Person) -> Option<String> {
    person
        .email_addresses
        .as_ref()?
        .iter()
        .find_map(|e| e.value.as_deref().filter(|v| !v.is_empty()))
        .map(str::to_lowercase)
}

/// Extract display name, falling back to the email.
fn extract_display_name(person: &Person, fallback_email: &str) -> String {
    person
        .names
        .as_ref()
        .and_then(|names| names.first())
        .and_then(|n| n.display_name.as_deref())
        .filter(|n| !n.is_empty())
        .unwrap_or(fallback_email)
        .to_string()
}

/// Extract avatar URL from the first photo.
fn extract_avatar_url(person: &Person) -> Option<String> {
    person
        .photos
        .as_ref()?
        .first()
        .and_then(|p| p.url.clone())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_person(
        resource_name: &str,
        email: Option<&str>,
        display_name: Option<&str>,
        photo_url: Option<&str>,
        deleted: bool,
    ) -> Person {
        Person {
            resource_name: Some(resource_name.to_string()),
            etag: None,
            metadata: if deleted {
                Some(PersonMetadata {
                    deleted: Some(true),
                    sources: None,
                })
            } else {
                None
            },
            names: display_name.map(|n| {
                vec![Name {
                    display_name: Some(n.to_string()),
                    given_name: None,
                    family_name: None,
                }]
            }),
            email_addresses: email.map(|e| {
                vec![EmailAddress {
                    value: Some(e.to_string()),
                    email_type: Some("home".to_string()),
                }]
            }),
            phone_numbers: None,
            organizations: None,
            photos: photo_url.map(|u| vec![Photo { url: Some(u.to_string()) }]),
        }
    }

    #[test]
    fn test_deserialize_people_response() {
        let json = r#"{
            "connections": [
                {
                    "resourceName": "people/c12345",
                    "etag": "abc",
                    "names": [{"displayName": "Alice Smith"}],
                    "emailAddresses": [{"value": "alice@example.com", "type": "home"}],
                    "photos": [{"url": "https://photo.example.com/alice.jpg"}]
                }
            ],
            "nextSyncToken": "sync_token_abc",
            "totalPeople": 1,
            "totalItems": 1
        }"#;

        let response: PeopleConnectionsResponse = serde_json::from_str(json).expect("deserialize");
        assert!(response.connections.is_some());
        let connections = response.connections.as_ref().expect("connections");
        assert_eq!(connections.len(), 1);
        assert_eq!(
            connections[0].resource_name.as_deref(),
            Some("people/c12345")
        );
        assert_eq!(response.next_sync_token.as_deref(), Some("sync_token_abc"));
    }

    #[test]
    fn test_extract_primary_email() {
        let person = make_person("people/1", Some("Alice@Example.COM"), None, None, false);
        assert_eq!(
            extract_primary_email(&person),
            Some("alice@example.com".to_string())
        );
    }

    #[test]
    fn test_extract_primary_email_none() {
        let person = make_person("people/1", None, None, None, false);
        assert_eq!(extract_primary_email(&person), None);
    }

    #[test]
    fn test_extract_primary_email_empty() {
        let person = Person {
            resource_name: Some("people/1".to_string()),
            etag: None,
            metadata: None,
            names: None,
            email_addresses: Some(vec![EmailAddress {
                value: Some(String::new()),
                email_type: None,
            }]),
            phone_numbers: None,
            organizations: None,
            photos: None,
        };
        assert_eq!(extract_primary_email(&person), None);
    }

    #[test]
    fn test_extract_display_name_with_name() {
        let person = make_person("people/1", Some("a@b.com"), Some("Alice"), None, false);
        assert_eq!(extract_display_name(&person, "a@b.com"), "Alice");
    }

    #[test]
    fn test_extract_display_name_fallback() {
        let person = make_person("people/1", Some("a@b.com"), None, None, false);
        assert_eq!(extract_display_name(&person, "a@b.com"), "a@b.com");
    }

    #[test]
    fn test_extract_avatar_url() {
        let person = make_person(
            "people/1",
            Some("a@b.com"),
            None,
            Some("https://photo.example.com/a.jpg"),
            false,
        );
        assert_eq!(
            extract_avatar_url(&person),
            Some("https://photo.example.com/a.jpg".to_string())
        );
    }

    #[test]
    fn test_extract_avatar_url_none() {
        let person = make_person("people/1", Some("a@b.com"), None, None, false);
        assert_eq!(extract_avatar_url(&person), None);
    }

    #[test]
    fn test_deleted_contact_metadata() {
        let person = make_person("people/1", Some("a@b.com"), None, None, true);
        let is_deleted = person
            .metadata
            .as_ref()
            .and_then(|m| m.deleted)
            .unwrap_or(false);
        assert!(is_deleted);
    }

    #[test]
    fn test_not_deleted_contact() {
        let person = make_person("people/1", Some("a@b.com"), None, None, false);
        let is_deleted = person
            .metadata
            .as_ref()
            .and_then(|m| m.deleted)
            .unwrap_or(false);
        assert!(!is_deleted);
    }

    #[test]
    fn test_deserialize_incremental_with_deleted() {
        let json = r#"{
            "connections": [
                {
                    "resourceName": "people/c999",
                    "metadata": {"deleted": true}
                },
                {
                    "resourceName": "people/c888",
                    "names": [{"displayName": "Bob"}],
                    "emailAddresses": [{"value": "bob@test.com"}]
                }
            ],
            "nextSyncToken": "new_token"
        }"#;

        let response: PeopleConnectionsResponse = serde_json::from_str(json).expect("deserialize");
        let connections = response.connections.expect("connections");

        // First entry is deleted
        let deleted = connections[0]
            .metadata
            .as_ref()
            .and_then(|m| m.deleted)
            .unwrap_or(false);
        assert!(deleted);

        // Second entry is not deleted and has email
        let email = extract_primary_email(&connections[1]);
        assert_eq!(email, Some("bob@test.com".to_string()));
    }

    #[test]
    fn test_contacts_with_no_email_skipped() {
        let persons = vec![
            make_person("people/1", Some("valid@test.com"), Some("Valid"), None, false),
            make_person("people/2", None, Some("No Email"), None, false),
            make_person("people/3", Some("also@valid.com"), None, None, false),
        ];

        let with_email: Vec<_> = persons
            .iter()
            .filter(|p| extract_primary_email(p).is_some())
            .collect();
        assert_eq!(with_email.len(), 2);
    }
}
