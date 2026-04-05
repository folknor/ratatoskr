//! Microsoft Graph contact sync integration.
//!
//! SQL lives in `db::queries_extra::contacts`. This module keeps
//! HTTP/JSON helpers and provides async wrappers.

use crate::db::DbState;

// Re-export types from db.
pub use crate::db::queries_extra::contacts::GraphServerInfo;

/// After Graph contacts sync completes, enrich with account_id and server_id.
pub async fn enrich_graph_contacts(db: &DbState, account_id: &str) -> Result<usize, String> {
    let aid = account_id.to_string();
    db.with_conn(move |conn| {
        crate::db::queries_extra::contacts::enrich_graph_contacts_sync(conn, &aid)
    })
    .await
}

/// Build the Graph API update request body for a contact.
pub fn build_graph_contact_update_body(
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
) -> serde_json::Value {
    let mut body = serde_json::json!({});
    if let Some(phone_val) = phone {
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
        crate::db::queries_extra::contacts::get_graph_contact_server_info_sync(conn, &email)
    })
    .await
}
