//! JMAP contacts sync using `ContactCard/get`, `ContactCard/changes`,
//! and `ContactCard/set` from the jmap-client crate.
//!
//! Uses JSContact (RFC 9553) `ContactCard` objects. The jmap-client crate
//! stores all JSContact properties in a `serde_json::Map` — we extract
//! display name, emails, phones, organizations, notes, and addresses from
//! the raw JSON for persistence into the local contacts DB.
//!
//! ## Sync strategy
//!
//! - **Initial sync**: `AddressBook/get` (all) → `ContactCard/get` (all) →
//!   upsert into local contacts table with `source = 'jmap'`.
//! - **Delta sync**: `ContactCard/changes` with saved state string →
//!   fetch created/updated → upsert; remove destroyed.
//! - **Write-back**: `ContactCard/set` for local edits on synced contacts.

use rusqlite::params;

use ratatoskr_db::db::DbState;
use ratatoskr_sync::state as sync_state;

use super::client::JmapClient;

const CONTACT_BATCH_SIZE: usize = 50;

// ---------------------------------------------------------------------------
// JSContact field extraction
// ---------------------------------------------------------------------------

/// Extracted contact fields from a JSContact `ContactCard`.
#[derive(Debug, Clone)]
struct ExtractedContact {
    /// JMAP server-side id.
    server_id: String,
    /// Primary email address (lowercased).
    email: String,
    /// Display name derived from fullName or name components.
    display_name: Option<String>,
    /// Secondary email address, if any.
    email2: Option<String>,
    /// First phone number found.
    phone: Option<String>,
    /// First organization name found.
    company: Option<String>,
    /// Concatenated notes text.
    notes: Option<String>,
}

/// Extract contact fields from a JSContact `ContactCard`.
///
/// Returns `None` if the contact has no email address (we cannot create
/// a meaningful contact row without one).
fn extract_contact(card: &jmap_client::contact_card::ContactCard) -> Option<ExtractedContact> {
    let server_id = card.id()?.to_string();

    // Extract emails — the `emails` property is a map of id → { address, ... }
    let mut email_addresses: Vec<String> = Vec::new();
    if let Some(emails_map) = card.emails() {
        for (_key, entry) in emails_map {
            if let Some(addr) = entry.get("address").and_then(|v| v.as_str()) {
                let normalized = addr.to_lowercase();
                if !normalized.is_empty() {
                    email_addresses.push(normalized);
                }
            }
        }
    }

    let primary_email = email_addresses.first()?.clone();
    let email2 = email_addresses.get(1).cloned();

    // Extract display name from the `name` property (JSContact NameComponents)
    let display_name = extract_display_name(card);

    // Extract first phone number
    let phone = extract_first_phone(card);

    // Extract first organization name
    let company = extract_first_organization(card);

    // Extract notes
    let notes = extract_notes(card);

    Some(ExtractedContact {
        server_id,
        email: primary_email,
        display_name,
        email2,
        phone,
        company,
        notes,
    })
}

/// Extract a display name from the JSContact `name` property.
///
/// Priority:
/// 1. `name.full` (RFC 9553 `NameComponent` with `full` key)
/// 2. Concatenation of `name.given` + `name.surname`
/// 3. Fall back to `None`
fn extract_display_name(card: &jmap_client::contact_card::ContactCard) -> Option<String> {
    let name_obj = card.name()?;

    // Try `full` first (a string or an array of components — check both)
    if let Some(full) = name_obj.get("full") {
        if let Some(s) = full.as_str() {
            let trimmed = s.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }

    // Try `components` array — look for given + surname
    if let Some(components) = name_obj.get("components").and_then(|v| v.as_array()) {
        let mut given = String::new();
        let mut surname = String::new();
        for comp in components {
            let kind = comp.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            let value = comp.get("value").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "given" => given = value.to_string(),
                "surname" => surname = value.to_string(),
                _ => {}
            }
        }
        let full = format!("{given} {surname}").trim().to_string();
        if !full.is_empty() {
            return Some(full);
        }
    }

    None
}

/// Extract the first phone number from the `phones` map.
fn extract_first_phone(card: &jmap_client::contact_card::ContactCard) -> Option<String> {
    let phones = card.phones()?;
    for (_key, entry) in phones {
        if let Some(number) = entry.get("number").and_then(|v| v.as_str()) {
            let trimmed = number.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Extract the first organization name from the `organizations` map.
fn extract_first_organization(card: &jmap_client::contact_card::ContactCard) -> Option<String> {
    let orgs = card.organizations()?;
    for (_key, entry) in orgs {
        if let Some(name) = entry.get("name").and_then(|v| v.as_str()) {
            let trimmed = name.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Extract notes text from the `notes` map.
fn extract_notes(card: &jmap_client::contact_card::ContactCard) -> Option<String> {
    let notes_map = card.notes()?;
    let mut parts: Vec<&str> = Vec::new();
    for (_key, entry) in notes_map {
        if let Some(note) = entry.get("note").and_then(|v| v.as_str()) {
            let trimmed = note.trim();
            if !trimmed.is_empty() {
                parts.push(trimmed);
            }
        }
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

// ---------------------------------------------------------------------------
// Initial sync
// ---------------------------------------------------------------------------

/// Run initial JMAP contacts sync for an account.
///
/// 1. Fetch all address books via `AddressBook/get` (no IDs → returns all).
/// 2. Fetch all contact cards via `ContactCard/get` (no IDs → returns all).
/// 3. Upsert into local contacts table.
/// 4. Save the `ContactCard` state string for future delta syncs.
pub async fn jmap_contacts_initial_sync(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
) -> Result<usize, String> {
    log::info!("[JMAP-Contacts] Starting initial sync for account {account_id}");

    // Fetch all contact cards (no IDs = return all)
    let inner = client.inner();
    let mut request = inner.build();
    request.get_contact_card();
    let response = request
        .send_get_contact_card()
        .await
        .map_err(|e| format!("ContactCard/get (initial): {e}"))?;

    let state = response.state().to_string();
    let mut response = response;
    let cards = response.take_list();

    log::info!(
        "[JMAP-Contacts] Fetched {} contact cards for account {account_id}",
        cards.len()
    );

    // Extract and persist
    let extracted: Vec<ExtractedContact> = cards.iter().filter_map(extract_contact).collect();
    let count = extracted.len();

    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for contact in &extracted {
            persist_jmap_contact(&tx, &aid, contact)?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await?;

    // Save state for delta sync
    sync_state::save_jmap_sync_state(db, account_id, "ContactCard", &state).await?;

    log::info!(
        "[JMAP-Contacts] Initial sync complete for account {account_id}: {count} contacts"
    );

    Ok(count)
}

// ---------------------------------------------------------------------------
// Delta sync
// ---------------------------------------------------------------------------

/// Run delta JMAP contacts sync for an account.
///
/// Uses `ContactCard/changes` to find created/updated/destroyed contacts
/// since the last sync, then fetches changed contacts and applies updates.
///
/// Returns the number of contacts affected (created + updated + destroyed).
pub async fn jmap_contacts_delta_sync(
    client: &JmapClient,
    account_id: &str,
    db: &DbState,
) -> Result<usize, String> {
    let state = sync_state::load_jmap_sync_state(db, account_id, "ContactCard").await?;
    let Some(mut since_state) = state else {
        log::warn!(
            "[JMAP-Contacts] No ContactCard state for account {account_id} — running initial sync"
        );
        return jmap_contacts_initial_sync(client, account_id, db).await;
    };

    log::info!("[JMAP-Contacts] Starting delta sync for account {account_id}");
    let mut total_affected: usize = 0;

    loop {
        let inner = client.inner();
        let changes = inner
            .contact_card_changes(&since_state, Some(500))
            .await
            .map_err(|e| {
                let msg = e.to_string();
                if msg.contains("cannotCalculateChanges") {
                    log::warn!(
                        "[JMAP-Contacts] State expired for account {account_id}, full re-sync needed"
                    );
                    return "JMAP_CONTACTS_STATE_EXPIRED".to_string();
                }
                format!("ContactCard/changes: {msg}")
            })?;

        let created = changes.created();
        let updated = changes.updated();
        let destroyed = changes.destroyed();

        let ids_to_fetch: Vec<&str> = created
            .iter()
            .chain(updated.iter())
            .map(String::as_str)
            .collect();

        // Fetch created + updated contacts in batches
        if !ids_to_fetch.is_empty() {
            for chunk in ids_to_fetch.chunks(CONTACT_BATCH_SIZE) {
                let cards = fetch_contact_batch(client, chunk).await?;
                let extracted: Vec<ExtractedContact> =
                    cards.iter().filter_map(extract_contact).collect();

                let aid = account_id.to_string();
                let batch_count = extracted.len();
                db.with_conn(move |conn| {
                    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
                    for contact in &extracted {
                        persist_jmap_contact(&tx, &aid, contact)?;
                    }
                    tx.commit().map_err(|e| e.to_string())?;
                    Ok(())
                })
                .await?;

                total_affected += batch_count;
            }
        }

        // Delete destroyed contacts
        if !destroyed.is_empty() {
            let destroyed_ids: Vec<String> = destroyed.to_vec();
            let aid = account_id.to_string();
            let destroy_count = destroyed_ids.len();
            db.with_conn(move |conn| {
                for server_id in &destroyed_ids {
                    delete_jmap_contact(conn, &aid, server_id)?;
                }
                Ok(())
            })
            .await?;
            total_affected += destroy_count;
        }

        since_state = changes.new_state().to_string();

        if !changes.has_more_changes() {
            break;
        }
    }

    // Save updated state
    sync_state::save_jmap_sync_state(db, account_id, "ContactCard", &since_state).await?;

    log::info!(
        "[JMAP-Contacts] Delta sync complete for account {account_id}: {total_affected} affected"
    );

    Ok(total_affected)
}

// ---------------------------------------------------------------------------
// Write-back: push local edits to server
// ---------------------------------------------------------------------------

/// Push a local contact edit to the JMAP server via `ContactCard/set`.
///
/// Only pushes phone, company, and notes — display name changes are
/// local-only overrides (consistent with Google/Graph providers).
pub async fn jmap_contacts_push_update(
    client: &JmapClient,
    server_id: &str,
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
) -> Result<(), String> {
    let inner = client.inner();
    let mut request = inner.build();
    let update = request.set_contact_card().update(server_id);

    // Build phone property
    if let Some(phone_val) = phone {
        let mut phones_map = serde_json::Map::new();
        let mut phone_entry = serde_json::Map::new();
        phone_entry.insert("number".into(), serde_json::Value::String(phone_val.to_string()));
        phones_map.insert("ph1".into(), serde_json::Value::Object(phone_entry));
        update.phones(phones_map);
    }

    // Build organization property
    if let Some(company_val) = company {
        let mut orgs_map = serde_json::Map::new();
        let mut org_entry = serde_json::Map::new();
        org_entry.insert("name".into(), serde_json::Value::String(company_val.to_string()));
        orgs_map.insert("org1".into(), serde_json::Value::Object(org_entry));
        update.organizations(orgs_map);
    }

    // Build notes property
    if let Some(notes_val) = notes {
        let mut notes_map = serde_json::Map::new();
        let mut note_entry = serde_json::Map::new();
        note_entry.insert("note".into(), serde_json::Value::String(notes_val.to_string()));
        notes_map.insert("n1".into(), serde_json::Value::Object(note_entry));
        update.notes(notes_map);
    }

    request
        .send_set_contact_card()
        .await
        .map_err(|e| format!("ContactCard/set (update): {e}"))?;

    log::info!("[JMAP-Contacts] Pushed update for contact {server_id}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Server info lookup (for write-back coordination)
// ---------------------------------------------------------------------------

/// Server-side info for a JMAP contact.
#[derive(Debug, Clone)]
pub struct JmapContactServerInfo {
    pub server_id: String,
    pub account_id: String,
}

/// Look up the JMAP server ID and account for a contact email.
pub async fn get_jmap_contact_server_info(
    db: &DbState,
    email: String,
) -> Result<Option<JmapContactServerInfo>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.query_row(
            "SELECT server_id, account_id FROM contacts \
             WHERE email = ?1 AND source = 'jmap' AND server_id IS NOT NULL",
            params![normalized],
            |row| {
                Ok(JmapContactServerInfo {
                    server_id: row.get("server_id")?,
                    account_id: row.get("account_id")?,
                })
            },
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(format!("lookup jmap contact server info: {other}")),
        })
    })
    .await
}

// ---------------------------------------------------------------------------
// Batch fetch helper
// ---------------------------------------------------------------------------

/// Fetch a batch of contact cards by ID.
async fn fetch_contact_batch(
    client: &JmapClient,
    ids: &[&str],
) -> Result<Vec<jmap_client::contact_card::ContactCard>, String> {
    let inner = client.inner();
    let mut request = inner.build();
    request.get_contact_card().ids(ids.iter().copied());
    let response = request
        .send_get_contact_card()
        .await
        .map_err(|e| format!("ContactCard/get batch: {e}"))?;

    let mut response = response;
    Ok(response.take_list())
}

// ---------------------------------------------------------------------------
// DB persistence
// ---------------------------------------------------------------------------

/// Upsert a JMAP contact into the local contacts table.
///
/// Follows the same conflict-resolution pattern as CardDAV:
/// - Never overwrite `source = 'user'` contacts
/// - Respect `display_name_overridden` flag
/// - Track `server_id` and `account_id` for write-back
fn persist_jmap_contact(
    conn: &rusqlite::Connection,
    account_id: &str,
    contact: &ExtractedContact,
) -> Result<(), String> {
    let local_id = format!("jmap-{account_id}-{}", contact.email);
    let display_name = contact
        .display_name
        .as_deref()
        .unwrap_or(contact.email.as_str());

    conn.execute(
        "INSERT INTO contacts (id, email, display_name, email2, phone, company, notes,
                               source, account_id, server_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'jmap', ?8, ?9) \
         ON CONFLICT(email) DO UPDATE SET \
           display_name = CASE \
             WHEN contacts.source = 'user' THEN contacts.display_name \
             WHEN contacts.display_name_overridden = 1 THEN contacts.display_name \
             ELSE COALESCE(excluded.display_name, contacts.display_name) \
           END, \
           email2 = CASE \
             WHEN contacts.source = 'user' THEN contacts.email2 \
             ELSE COALESCE(excluded.email2, contacts.email2) \
           END, \
           phone = CASE \
             WHEN contacts.source = 'user' THEN contacts.phone \
             ELSE COALESCE(excluded.phone, contacts.phone) \
           END, \
           company = CASE \
             WHEN contacts.source = 'user' THEN contacts.company \
             ELSE COALESCE(excluded.company, contacts.company) \
           END, \
           notes = CASE \
             WHEN contacts.source = 'user' THEN contacts.notes \
             ELSE COALESCE(excluded.notes, contacts.notes) \
           END, \
           source = CASE \
             WHEN contacts.source = 'user' THEN 'user' \
             ELSE 'jmap' \
           END, \
           account_id = COALESCE(excluded.account_id, contacts.account_id), \
           server_id = COALESCE(excluded.server_id, contacts.server_id), \
           updated_at = unixepoch()",
        params![
            local_id,
            contact.email,
            display_name,
            contact.email2,
            contact.phone,
            contact.company,
            contact.notes,
            account_id,
            contact.server_id,
        ],
    )
    .map_err(|e| format!("upsert jmap contact: {e}"))?;

    Ok(())
}

/// Delete a JMAP contact that was destroyed on the server.
///
/// Only deletes if the contact is still sourced from JMAP (don't remove
/// user-owned contacts).
fn delete_jmap_contact(
    conn: &rusqlite::Connection,
    account_id: &str,
    server_id: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM contacts \
         WHERE server_id = ?1 AND account_id = ?2 AND source = 'jmap'",
        params![server_id, account_id],
    )
    .map_err(|e| format!("delete jmap contact: {e}"))?;
    Ok(())
}
