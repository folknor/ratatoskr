//! Microsoft Graph contact sync integration.
//!
//! The actual sync implementation lives in `crates/graph/src/contact_sync.rs`.
//! This module provides the enhanced persistence layer and write-back support.
//!
//! The Graph crate's `graph_contacts_initial_sync` / `graph_contacts_delta_sync`
//! functions handle:
//! - `GET /me/contacts` with `$select` fields
//! - Delta sync via `deltaLink`
//! - Full sync fallback on 410 Gone
//!
//! This module adds:
//! - `account_id` and `server_id` population on the contacts row
//! - Write-back: `PATCH /me/contacts/{id}` for local edits to synced contacts

use rusqlite::params;

use crate::db::DbState;

// ---------------------------------------------------------------------------
// Enhanced persistence: post-sync field enrichment
// ---------------------------------------------------------------------------

/// After Graph contacts sync completes, set the `account_id` and `server_id`
/// on contacts that were synced from this account.
///
/// This is called after `graph_contacts_initial_sync()` or
/// `graph_contacts_delta_sync()` from the Graph crate.
pub async fn enrich_graph_contacts(
    db: &DbState,
    account_id: &str,
) -> Result<usize, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        let changed = conn
            .execute(
                "UPDATE contacts SET \
                   account_id = ?1, \
                   server_id = (
                     SELECT m.graph_contact_id FROM graph_contact_map m
                     WHERE m.email = contacts.email AND m.account_id = ?1
                     LIMIT 1
                   ) \
                 WHERE source = 'graph' \
                   AND email IN (
                     SELECT m2.email FROM graph_contact_map m2 WHERE m2.account_id = ?1
                   ) \
                   AND (account_id IS NULL OR account_id = ?1)",
                params![aid],
            )
            .map_err(|e| format!("enrich graph contacts: {e}"))?;

        Ok(changed)
    })
    .await
}

// ---------------------------------------------------------------------------
// Write-back: push local edits to Microsoft Graph
// ---------------------------------------------------------------------------

/// Build the Graph API update request body for a contact.
///
/// Returns a JSON body suitable for `PATCH /me/contacts/{id}`.
/// Display name changes are NOT included (they are local-only overrides).
pub fn build_graph_contact_update_body(
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({});

    if let Some(phone_val) = phone {
        // Graph uses `businessPhones` and `mobilePhone` — set homePhones as fallback
        body["homePhones"] = serde_json::json!([phone_val]);
    }

    if let Some(company_val) = company {
        body["companyName"] = serde_json::Value::String(company_val.to_string());
    }

    if let Some(notes_val) = notes {
        body["personalNotes"] = serde_json::Value::String(notes_val.to_string());
    }

    body
}

/// Look up the Graph contact ID and account for a contact email.
pub async fn get_graph_contact_server_info(
    db: &DbState,
    email: String,
) -> Result<Option<GraphServerInfo>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.query_row(
            "SELECT m.graph_contact_id, m.account_id \
             FROM graph_contact_map m \
             WHERE m.email = ?1 \
             LIMIT 1",
            params![normalized],
            |row| {
                Ok(GraphServerInfo {
                    graph_contact_id: row.get("graph_contact_id")?,
                    account_id: row.get("account_id")?,
                })
            },
        )
        .map_err(|e| e.to_string())
        .map(Some)
        .or_else(|_| Ok(None))
    })
    .await
}

/// Server-side info for a Graph contact.
#[derive(Debug, Clone)]
pub struct GraphServerInfo {
    pub graph_contact_id: String,
    pub account_id: String,
}
