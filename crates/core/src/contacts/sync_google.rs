//! Google People API contact sync integration.
//!
//! SQL lives in `db::queries_extra::contacts`. This module keeps
//! HTTP/JSON helpers and provides async wrappers.

use crate::db::DbState;

// Re-export types from db.
pub use crate::db::queries_extra::contacts::{GoogleContactFields, GoogleServerInfo};

/// After Google contacts sync completes, enrich with phone, company, etc.
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
        crate::db::queries_extra::contacts::enrich_google_contacts_sync(conn, &aid, &owned)
    })
    .await
}

/// Extract enrichment fields from a Google Person.
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

/// Build the People API update request body for a contact.
pub fn build_google_contact_update_body(
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
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
    if let Some(notes_val) = notes {
        person["biographies"] = serde_json::json!([{"value": notes_val}]);
    }
    person
}

/// Look up the Google resource name and current server data for a contact.
pub async fn get_google_contact_server_info(
    db: &DbState,
    email: String,
) -> Result<Option<GoogleServerInfo>, String> {
    db.with_conn(move |conn| {
        crate::db::queries_extra::contacts::get_google_contact_server_info_sync(conn, &email)
    })
    .await
}
