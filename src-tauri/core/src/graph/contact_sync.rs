use std::collections::HashSet;

use rusqlite::params;

use crate::db::DbState;
use crate::sync::state as sync_state;

use super::client::GraphClient;
use super::types::{
    CONTACT_SELECT, GraphContact, GraphContactFolder, ODataCollection,
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Full contact sync: fetch all contact folders, page through all contacts,
/// upsert into contacts table with source='graph', then prune stale entries.
pub(crate) async fn graph_contacts_initial_sync(
    client: &GraphClient,
    account_id: &str,
    db: &DbState,
) -> Result<(), String> {
    let folders = fetch_contact_folders(client, db).await?;
    if folders.is_empty() {
        log::debug!("No contact folders found for account {account_id}");
        return Ok(());
    }

    for folder in &folders {
        full_sync_contact_folder(client, account_id, db, &folder.id).await?;

        // Bootstrap delta token for future delta syncs
        let delta_link = bootstrap_contact_delta_token(client, db, &folder.id).await?;
        sync_state::save_graph_contact_delta_token(db, account_id, &folder.id, &delta_link)
            .await?;
    }

    Ok(())
}

/// Delta contact sync: for each contact folder with a stored delta token,
/// fetch changes and apply creates/updates/deletes.
/// Handles 410 Gone by falling back to full sync for that folder.
pub(crate) async fn graph_contacts_delta_sync(
    client: &GraphClient,
    account_id: &str,
    db: &DbState,
) -> Result<(), String> {
    let tokens = sync_state::load_graph_contact_delta_tokens(db, account_id).await?;
    if tokens.is_empty() {
        // No delta tokens — nothing to do (initial sync hasn't run or was cleared)
        return Ok(());
    }

    for (folder_id, delta_link) in &tokens {
        if let Err(e) =
            sync_contact_folder_delta(client, account_id, db, folder_id, delta_link).await
        {
            if e.contains("410") {
                log::warn!(
                    "Contact delta token expired for folder {folder_id}, falling back to full sync"
                );
                full_sync_contact_folder(client, account_id, db, folder_id).await?;
                let new_token = bootstrap_contact_delta_token(client, db, folder_id).await?;
                sync_state::save_graph_contact_delta_token(db, account_id, folder_id, &new_token)
                    .await?;
            } else {
                return Err(e);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Folder listing
// ---------------------------------------------------------------------------

async fn fetch_contact_folders(
    client: &GraphClient,
    db: &DbState,
) -> Result<Vec<GraphContactFolder>, String> {
    let mut folders = Vec::new();
    let mut next_link: Option<String> = None;

    loop {
        let page: ODataCollection<GraphContactFolder> = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client
                .get_json("/me/contactFolders?$top=250", db)
                .await?
        };

        folders.extend(page.value);

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => break,
        }
    }

    Ok(folders)
}

// ---------------------------------------------------------------------------
// Full sync (initial + 410 fallback)
// ---------------------------------------------------------------------------

/// Fetch all contacts in a folder, upsert them, then prune stale mappings.
async fn full_sync_contact_folder(
    client: &GraphClient,
    account_id: &str,
    db: &DbState,
    folder_id: &str,
) -> Result<(), String> {
    let contacts = fetch_folder_contacts(client, db, folder_id).await?;

    let seen_ids: HashSet<String> = contacts.iter().map(|c| c.id.clone()).collect();

    let aid = account_id.to_string();
    let contacts_owned = contacts;
    let seen_ids_owned = seen_ids;

    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| format!("begin tx: {e}"))?;
        persist_synced_contacts(&tx, &aid, &contacts_owned)?;
        prune_stale_contacts(&tx, &aid, &seen_ids_owned)?;
        tx.commit().map_err(|e| format!("commit tx: {e}"))?;
        Ok(())
    })
    .await
}

async fn fetch_folder_contacts(
    client: &GraphClient,
    db: &DbState,
    folder_id: &str,
) -> Result<Vec<GraphContact>, String> {
    let mut contacts = Vec::new();
    let enc_folder_id = urlencoding::encode(folder_id);
    let mut next_link: Option<String> = None;

    loop {
        let page: ODataCollection<GraphContact> = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client
                .get_json(
                    &format!(
                        "/me/contactFolders/{enc_folder_id}/contacts?$select={CONTACT_SELECT}&$top=999"
                    ),
                    db,
                )
                .await?
        };

        contacts.extend(page.value);

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => break,
        }
    }

    Ok(contacts)
}

// ---------------------------------------------------------------------------
// Delta sync
// ---------------------------------------------------------------------------

async fn sync_contact_folder_delta(
    client: &GraphClient,
    account_id: &str,
    db: &DbState,
    folder_id: &str,
    delta_link: &str,
) -> Result<(), String> {
    let mut current_link = delta_link.to_string();

    loop {
        let page: ODataCollection<serde_json::Value> =
            client.get_absolute(&current_link, db).await?;

        let mut upserts = Vec::new();
        let mut deleted_ids = Vec::new();

        for item in &page.value {
            let Some(id) = item.get("id").and_then(|v| v.as_str()) else {
                continue;
            };

            if item.get("@removed").is_some() {
                deleted_ids.push(id.to_string());
            } else {
                match serde_json::from_value::<GraphContact>(item.clone()) {
                    Ok(contact) => upserts.push(contact),
                    Err(e) => {
                        log::warn!("Failed to deserialize Graph contact {id}: {e}");
                    }
                }
            }
        }

        if !upserts.is_empty() || !deleted_ids.is_empty() {
            let aid = account_id.to_string();
            let upserts_owned = upserts;
            let deleted_owned = deleted_ids;

            db.with_conn(move |conn| {
                let tx = conn.unchecked_transaction().map_err(|e| format!("begin tx: {e}"))?;
                persist_synced_contacts(&tx, &aid, &upserts_owned)?;
                for graph_contact_id in &deleted_owned {
                    delete_synced_contact(&tx, &aid, graph_contact_id)?;
                }
                tx.commit().map_err(|e| format!("commit tx: {e}"))?;
                Ok(())
            })
            .await?;
        }

        // Follow pagination or store new delta link
        if let Some(ref next) = page.next_link {
            current_link = next.clone();
        } else if let Some(ref new_delta) = page.delta_link {
            sync_state::save_graph_contact_delta_token(db, account_id, folder_id, new_delta)
                .await?;
            break;
        } else {
            log::warn!(
                "Graph contact delta for folder {folder_id} has no nextLink or deltaLink"
            );
            break;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Delta token bootstrap
// ---------------------------------------------------------------------------

async fn bootstrap_contact_delta_token(
    client: &GraphClient,
    db: &DbState,
    folder_id: &str,
) -> Result<String, String> {
    let enc_folder_id = urlencoding::encode(folder_id);
    let initial_url = format!(
        "/me/contactFolders/{enc_folder_id}/contacts/delta?$select=id"
    );
    let mut next_link: Option<String> = None;

    loop {
        let page: ODataCollection<serde_json::Value> = if let Some(ref link) = next_link {
            client.get_absolute(link, db).await?
        } else {
            client.get_json(&initial_url, db).await?
        };

        if let Some(ref delta) = page.delta_link {
            return Ok(delta.clone());
        }

        match page.next_link {
            Some(link) => next_link = Some(link),
            None => {
                return Err(format!(
                    "Contact delta bootstrap for folder {folder_id} ended without a deltaLink"
                ));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Persistence helpers (run inside a transaction)
// ---------------------------------------------------------------------------

fn persist_synced_contacts(
    conn: &rusqlite::Connection,
    account_id: &str,
    contacts: &[GraphContact],
) -> Result<(), String> {
    for contact in contacts {
        let emails = extract_emails(contact);
        if emails.is_empty() {
            continue;
        }

        for email in &emails {
            let local_id = format!("graph-{account_id}-{email}");

            conn.execute(
                "INSERT INTO contacts (id, email, display_name, source) \
                 VALUES (?1, ?2, ?3, 'graph') \
                 ON CONFLICT(email) DO UPDATE SET \
                   display_name = CASE \
                     WHEN contacts.source = 'user' THEN contacts.display_name \
                     WHEN contacts.display_name_overridden = 1 THEN contacts.display_name \
                     ELSE COALESCE(excluded.display_name, contacts.display_name) \
                   END, \
                   source = CASE \
                     WHEN contacts.source = 'user' THEN 'user' \
                     ELSE 'graph' \
                   END, \
                   updated_at = unixepoch()",
                params![local_id, email, contact.display_name],
            )
            .map_err(|e| format!("upsert synced contact: {e}"))?;

            conn.execute(
                "INSERT OR REPLACE INTO graph_contact_map \
                 (account_id, graph_contact_id, email) \
                 VALUES (?1, ?2, ?3)",
                params![account_id, contact.id, email],
            )
            .map_err(|e| format!("upsert contact map: {e}"))?;
        }
    }

    Ok(())
}

fn delete_synced_contact(
    conn: &rusqlite::Connection,
    account_id: &str,
    graph_contact_id: &str,
) -> Result<(), String> {
    // 1. Look up all emails for this graph contact
    let mut stmt = conn
        .prepare(
            "SELECT email FROM graph_contact_map \
             WHERE account_id = ?1 AND graph_contact_id = ?2",
        )
        .map_err(|e| format!("prepare contact map lookup: {e}"))?;

    let emails: Vec<String> = stmt
        .query_map(params![account_id, graph_contact_id], |row| {
            row.get::<_, String>("email")
        })
        .map_err(|e| format!("query contact map: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect contact map: {e}"))?;

    drop(stmt);

    // 2. Remove this graph contact's mapping rows
    conn.execute(
        "DELETE FROM graph_contact_map \
         WHERE account_id = ?1 AND graph_contact_id = ?2",
        params![account_id, graph_contact_id],
    )
    .map_err(|e| format!("delete contact map: {e}"))?;

    // 3. Only delete the contacts row if no other mapping references that email
    for email in &emails {
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM graph_contact_map WHERE email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("count remaining mappings: {e}"))?;

        if remaining == 0 {
            conn.execute(
                "DELETE FROM contacts WHERE email = ?1 AND source = 'graph'",
                params![email],
            )
            .map_err(|e| format!("delete orphaned contact: {e}"))?;
        }
    }

    Ok(())
}

/// After a full sync, remove mapping rows for graph_contact_ids not seen
/// in the fetch, then delete orphaned source='graph' contacts.
fn prune_stale_contacts(
    conn: &rusqlite::Connection,
    account_id: &str,
    seen_ids: &HashSet<String>,
) -> Result<(), String> {
    // Find all graph_contact_ids we have mapped for this account
    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT graph_contact_id FROM graph_contact_map \
             WHERE account_id = ?1",
        )
        .map_err(|e| format!("prepare stale lookup: {e}"))?;

    let all_mapped: Vec<String> = stmt
        .query_map(params![account_id], |row| row.get::<_, String>("graph_contact_id"))
        .map_err(|e| format!("query stale: {e}"))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("collect stale: {e}"))?;

    drop(stmt);

    // Delete mappings and orphaned contacts for IDs not in the current fetch
    for graph_contact_id in &all_mapped {
        if !seen_ids.contains(graph_contact_id) {
            delete_synced_contact(conn, account_id, graph_contact_id)?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Extract valid, lowercased email addresses from a Graph contact.
fn extract_emails(contact: &GraphContact) -> Vec<String> {
    let Some(ref addrs) = contact.email_addresses else {
        return Vec::new();
    };

    addrs
        .iter()
        .filter_map(|e| {
            e.address
                .as_deref()
                .filter(|a| !a.is_empty())
                .map(|a| a.to_lowercase())
        })
        .collect()
}
