//! Google People API contact sync integration.
//!
//! The actual sync implementation lives in `crates/gmail/src/contacts/`.
//! This module provides the enhanced persistence layer that maps Google
//! People API fields (phone, organization) to the contacts table's
//! phone, company, and account_id columns.
//!
//! The Gmail crate's `sync_google_contacts` function handles:
//! - `GET /v1/people/me/connections` with personFields
//! - Incremental sync via `syncToken`
//! - Full sync with pruning on 410 Gone
//!
//! This module adds:
//! - Phone number extraction from `Person.phoneNumbers`
//! - Organization extraction from `Person.organizations`
//! - `account_id` and `server_id` population on the contacts row

use rusqlite::params;

use crate::db::DbState;

// ---------------------------------------------------------------------------
// Enhanced persistence: post-sync field enrichment
// ---------------------------------------------------------------------------

/// After Google contacts sync completes, enrich the contacts table with
/// phone, company, and account_id from the People API data.
///
/// This is called after `sync_google_contacts()` from the Gmail crate,
/// which already upserts basic fields (email, display_name, avatar_url).
/// We enrich with additional fields that the base sync doesn't populate.
pub async fn enrich_google_contacts(
    db: &DbState,
    account_id: &str,
    persons: &[GoogleContactFields],
) -> Result<usize, String> {
    if persons.is_empty() {
        return Ok(0);
    }

    let aid = account_id.to_string();
    let owned: Vec<GoogleContactFields> = persons.to_vec();

    db.with_conn(move |conn| {
        let mut enriched = 0;

        for person in &owned {
            let changed = conn
                .execute(
                    "UPDATE contacts SET \
                       phone = COALESCE(?1, phone), \
                       company = COALESCE(?2, company), \
                       account_id = COALESCE(?3, account_id), \
                       server_id = COALESCE(?4, server_id), \
                       updated_at = unixepoch() \
                     WHERE email = ?5 AND source IN ('google', 'user')",
                    params![
                        person.phone,
                        person.company,
                        aid,
                        person.resource_name,
                        person.email,
                    ],
                )
                .map_err(|e| format!("enrich google contact: {e}"))?;

            if changed > 0 {
                enriched += 1;
            }
        }

        Ok(enriched)
    })
    .await
}

/// Extracted fields from a Google People API Person for enrichment.
#[derive(Debug, Clone)]
pub struct GoogleContactFields {
    pub email: String,
    pub resource_name: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
}

/// Extract enrichment fields from a Google Person.
///
/// This is a helper intended to be called from the sync pipeline after
/// fetching persons from the People API.
pub fn extract_google_contact_fields(
    email: &str,
    resource_name: Option<&str>,
    phone_numbers: &[GooglePhoneNumber],
    organizations: &[GoogleOrganization],
) -> GoogleContactFields {
    let phone = phone_numbers
        .first()
        .and_then(|p| p.value.clone())
        .filter(|v| !v.is_empty());

    let company = organizations
        .first()
        .and_then(|o| o.name.clone())
        .filter(|v| !v.is_empty());

    GoogleContactFields {
        email: email.to_lowercase(),
        resource_name: resource_name.map(ToString::to_string),
        phone,
        company,
    }
}

/// Phone number from Google People API.
#[derive(Debug, Clone)]
pub struct GooglePhoneNumber {
    pub value: Option<String>,
}

/// Organization from Google People API.
#[derive(Debug, Clone)]
pub struct GoogleOrganization {
    pub name: Option<String>,
}

// ---------------------------------------------------------------------------
// Write-back: push local edits to Google People API
// ---------------------------------------------------------------------------

/// Build the People API update request body for a contact.
///
/// Returns a JSON body suitable for `PATCH /v1/{resourceName}:updateContact`.
/// The `updatePersonFields` mask goes in the query string, not the body.
/// Display name changes are NOT included (they are local-only overrides).
pub fn build_google_contact_update_body(
    phone: Option<&str>,
    company: Option<&str>,
    etag: &str,
) -> serde_json::Value {
    let mut person = serde_json::json!({
        "etag": etag,
    });

    if let Some(phone_val) = phone {
        person["phoneNumbers"] = serde_json::json!([{"value": phone_val}]);
    }

    if let Some(company_val) = company {
        person["organizations"] = serde_json::json!([{"name": company_val}]);
    }

    person
}

/// Look up the Google resource name and current server data for a contact.
pub async fn get_google_contact_server_info(
    db: &DbState,
    email: String,
) -> Result<Option<GoogleServerInfo>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.query_row(
            "SELECT m.resource_name, m.account_id \
             FROM google_contact_map m \
             WHERE m.contact_email = ?1 \
             LIMIT 1",
            params![normalized],
            |row| {
                Ok(GoogleServerInfo {
                    resource_name: row.get("resource_name")?,
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

/// Server-side info for a Google contact.
#[derive(Debug, Clone)]
pub struct GoogleServerInfo {
    pub resource_name: String,
    pub account_id: String,
}
