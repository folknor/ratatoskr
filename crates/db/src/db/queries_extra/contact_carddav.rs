//! CardDAV contact persistence: upsert with source-priority ON CONFLICT,
//! cascading orphan-checked delete, CTag/ETag state management.

use std::collections::HashMap;

use rusqlite::{Connection, params};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A parsed CardDAV contact ready for database upsert.
#[derive(Debug, Clone)]
pub struct CarddavContactUpsert {
    pub uri: String,
    pub etag: String,
    pub email: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
}

// ---------------------------------------------------------------------------
// Batch persist + delete (one transaction)
// ---------------------------------------------------------------------------

/// Persist upserted contacts and delete removed ones within a single transaction.
///
/// The upsert uses ON CONFLICT with source-priority CASE logic:
/// - If the existing row is user-created, user fields are preserved.
/// - If display_name_overridden is set, the user's display name is kept.
/// - Otherwise, the CardDAV value wins via COALESCE.
///
/// The delete uses a 3-way orphan check (carddav, google, graph mapping tables)
/// to only remove contacts that no provider still claims.
pub fn persist_carddav_contacts_sync(
    conn: &Connection,
    account_id: &str,
    contacts: &[CarddavContactUpsert],
    deleted_uris: &[String],
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin carddav tx: {e}"))?;

    for contact in contacts {
        persist_one(&tx, account_id, contact)?;
    }

    for uri in deleted_uris {
        delete_one(&tx, account_id, uri)?;
    }

    tx.commit().map_err(|e| format!("commit carddav tx: {e}"))?;
    Ok(())
}

fn persist_one(
    conn: &Connection,
    account_id: &str,
    contact: &CarddavContactUpsert,
) -> Result<(), String> {
    let local_id = format!("carddav-{account_id}-{}", contact.email);

    conn.execute(
        "INSERT INTO contacts (id, email, display_name, avatar_url, phone, company,
                               source, account_id, server_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'carddav', ?7, ?8) \
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
             ELSE 'carddav' \
           END, \
           account_id = COALESCE(excluded.account_id, contacts.account_id), \
           server_id = COALESCE(excluded.server_id, contacts.server_id), \
           updated_at = unixepoch()",
        params![
            local_id,
            contact.email,
            contact.display_name,
            contact.avatar_url,
            contact.phone,
            contact.company,
            account_id,
            contact.uri,
        ],
    )
    .map_err(|e| format!("upsert carddav contact: {e}"))?;

    conn.execute(
        "INSERT OR REPLACE INTO carddav_contact_map \
         (uri, account_id, contact_email, etag) \
         VALUES (?1, ?2, ?3, ?4)",
        params![contact.uri, account_id, contact.email, contact.etag],
    )
    .map_err(|e| format!("upsert carddav contact map: {e}"))?;

    Ok(())
}

fn delete_one(conn: &Connection, account_id: &str, uri: &str) -> Result<(), String> {
    let email: Option<String> = conn
        .query_row(
            "SELECT contact_email FROM carddav_contact_map \
             WHERE uri = ?1 AND account_id = ?2",
            params![uri, account_id],
            |row| row.get("contact_email"),
        )
        .ok();

    conn.execute(
        "DELETE FROM carddav_contact_map \
         WHERE uri = ?1 AND account_id = ?2",
        params![uri, account_id],
    )
    .map_err(|e| format!("delete carddav contact map: {e}"))?;

    if let Some(ref email) = email {
        let carddav_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM carddav_contact_map WHERE contact_email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .map_err(|e| format!("count remaining carddav mappings: {e}"))?;

        let google_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM google_contact_map WHERE contact_email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .unwrap_or(0);

        let graph_remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) AS cnt FROM graph_contact_map WHERE email = ?1",
                params![email],
                |row| row.get("cnt"),
            )
            .unwrap_or(0);

        if carddav_remaining == 0 && google_remaining == 0 && graph_remaining == 0 {
            conn.execute(
                "DELETE FROM contacts WHERE email = ?1 AND source = 'carddav'",
                params![email],
            )
            .map_err(|e| format!("delete orphaned carddav contact: {e}"))?;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// CTag / ETag persistence
// ---------------------------------------------------------------------------

/// Load the stored CTag for an account's CardDAV addressbook.
pub fn load_carddav_ctag_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<Option<String>, String> {
    let key = format!("carddav_ctag:{account_id}");
    crate::db::queries::get_setting(conn, &key)
}

/// Save the CTag after a successful sync.
pub fn save_carddav_ctag_sync(
    conn: &Connection,
    account_id: &str,
    ctag: &str,
) -> Result<(), String> {
    let key = format!("carddav_ctag:{account_id}");
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        params![key, ctag],
    )
    .map_err(|e| format!("save carddav ctag: {e}"))?;
    Ok(())
}

/// Load all stored ETags for an account's CardDAV contacts.
pub fn load_carddav_etags_sync(
    conn: &Connection,
    account_id: &str,
) -> Result<HashMap<String, String>, String> {
    let mut stmt = conn
        .prepare("SELECT uri, etag FROM carddav_contact_map WHERE account_id = ?1")
        .map_err(|e| format!("prepare etag query: {e}"))?;

    let rows = stmt
        .query_map(params![account_id], |row| {
            Ok((
                row.get::<_, String>("uri")?,
                row.get::<_, Option<String>>("etag")?,
            ))
        })
        .map_err(|e| format!("query etags: {e}"))?;

    let mut map = HashMap::new();
    for row in rows {
        let (uri, etag) = row.map_err(|e| format!("read etag row: {e}"))?;
        if let Some(etag) = etag {
            map.insert(uri, etag);
        }
    }

    Ok(map)
}
