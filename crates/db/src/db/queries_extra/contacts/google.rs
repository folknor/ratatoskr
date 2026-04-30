/// Extracted fields from a Google People API Person for enrichment.
#[derive(Debug, Clone)]
pub struct GoogleContactFields {
    pub email: String,
    pub resource_name: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
}

/// Server-side info for a Google contact.
#[derive(Debug, Clone)]
pub struct GoogleServerInfo {
    pub resource_name: String,
    pub account_id: String,
}

/// Enrich contacts with Google People API fields (phone, company, account_id, server_id).
pub fn enrich_google_contacts_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
    persons: &[GoogleContactFields],
) -> Result<usize, String> {
    let mut enriched = 0;
    for person in persons {
        let changed = conn
            .execute(
                "UPDATE contacts SET \
                   phone = COALESCE(?1, phone), \
                   company = COALESCE(?2, company), \
                   account_id = COALESCE(?3, account_id), \
                   server_id = COALESCE(?4, server_id), \
                   updated_at = unixepoch() \
                 WHERE email = ?5 AND source IN ('google', 'user')",
                rusqlite::params![
                    person.phone,
                    person.company,
                    account_id,
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
}

/// Look up the Google resource name and account ID for a contact.
pub fn get_google_contact_server_info_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<Option<GoogleServerInfo>, String> {
    let normalized = email.to_lowercase();
    conn.query_row(
        "SELECT m.resource_name, m.account_id \
         FROM google_contact_map m \
         WHERE m.contact_email = ?1 \
         LIMIT 1",
        rusqlite::params![normalized],
        |row| {
            Ok(GoogleServerInfo {
                resource_name: row.get("resource_name")?,
                account_id: row.get("account_id")?,
            })
        },
    )
    .map_err(|e| e.to_string())
    .map(Some)
    .or(Ok(None))
}
