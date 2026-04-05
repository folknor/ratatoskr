use rusqlite::{Connection, params};

pub struct ContactWriteRow {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub avatar_url: Option<String>,
    pub source: String,
    pub account_id: String,
    pub server_id: Option<String>,
}

pub fn upsert_contact_sync(conn: &Connection, row: &ContactWriteRow) -> Result<(), String> {
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, email2, phone, company, notes,
                               avatar_url, source, account_id, server_id) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) \
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
           avatar_url = CASE \
             WHEN contacts.source = 'user' THEN contacts.avatar_url \
             ELSE COALESCE(excluded.avatar_url, contacts.avatar_url) \
           END, \
           source = CASE \
             WHEN contacts.source = 'user' THEN 'user' \
             ELSE excluded.source \
           END, \
           account_id = COALESCE(excluded.account_id, contacts.account_id), \
           server_id = COALESCE(excluded.server_id, contacts.server_id), \
           updated_at = unixepoch()",
        params![
            row.id,
            row.email,
            row.display_name,
            row.email2,
            row.phone,
            row.company,
            row.notes,
            row.avatar_url,
            row.source,
            row.account_id,
            row.server_id,
        ],
    )
    .map_err(|e| format!("upsert contact: {e}"))?;
    Ok(())
}

pub fn delete_contact_by_email_and_source_sync(
    conn: &Connection,
    email: &str,
    source: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM contacts WHERE email = ?1 AND source = ?2",
        params![email, source],
    )
    .map_err(|e| format!("delete contact by email/source: {e}"))?;
    Ok(())
}

pub fn delete_contact_by_server_id_and_source_sync(
    conn: &Connection,
    account_id: &str,
    server_id: &str,
    source: &str,
) -> Result<(), String> {
    conn.execute(
        "DELETE FROM contacts WHERE server_id = ?1 AND account_id = ?2 AND source = ?3",
        params![server_id, account_id, source],
    )
    .map_err(|e| format!("delete contact by server id/source: {e}"))?;
    Ok(())
}
