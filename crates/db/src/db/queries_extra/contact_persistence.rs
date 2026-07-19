use rusqlite::params;

use crate::db::WriteConn;
use crate::db::WriteTarget;
use crate::db::queries::{get_setting, set_setting};

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

pub fn upsert_contact_sync(conn: &impl WriteTarget, row: &ContactWriteRow) -> Result<(), String> {
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
    conn: &impl WriteTarget,
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
    conn: &impl WriteTarget,
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

/// Record one remote ownership claim for a materialized contact row.
pub fn upsert_contact_claim_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    source: &str,
    server_id: &str,
    email: &str,
    address_book_id: Option<&str>,
    corpus: &str,
) -> Result<(), String> {
    conn.execute(
        "INSERT INTO contact_claims (account_id, source, server_id, email, address_book_id, corpus) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
         ON CONFLICT(account_id, source, server_id, email) DO UPDATE SET \
           address_book_id = excluded.address_book_id, corpus = excluded.corpus",
        params![account_id, source, server_id, email, address_book_id, corpus],
    )
    .map_err(|e| format!("upsert contact claim: {e}"))?;
    Ok(())
}

/// Retire claims absent from a completed snapshot, preserving failed ids.
/// Returns materialized emails that no longer have any remote claim.
pub fn reconcile_contact_claims_sync(
    conn: &impl WriteTarget,
    account_id: &str,
    source: &str,
    fetched_ids: &std::collections::HashSet<String>,
    failed_ids: &std::collections::HashSet<String>,
) -> Result<Vec<String>, String> {
    let mut stale = Vec::new();
    {
        let mut stmt = conn.prepare(
            "SELECT DISTINCT server_id, email FROM contact_claims WHERE account_id = ?1 AND source = ?2",
        ).map_err(|e| format!("prepare contact claim reconcile: {e}"))?;
        let rows = stmt
            .query_map(params![account_id, source], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|e| format!("query contact claims: {e}"))?;
        for row in rows {
            let (server_id, email) = row.map_err(|e| format!("read contact claim: {e}"))?;
            if !fetched_ids.contains(&server_id) && !failed_ids.contains(&server_id) {
                stale.push((server_id, email));
            }
        }
    }
    let mut orphaned = Vec::new();
    for (server_id, email) in stale {
        conn.execute(
            "DELETE FROM contact_claims WHERE account_id = ?1 AND source = ?2 AND server_id = ?3 AND email = ?4",
            params![account_id, source, server_id, email],
        ).map_err(|e| format!("retire contact claim: {e}"))?;
        let remaining: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM contact_claims WHERE email = ?1",
                params![email],
                |row| row.get(0),
            )
            .map_err(|e| format!("count contact claims: {e}"))?;
        if remaining == 0 {
            orphaned.push(email);
        }
    }
    Ok(orphaned)
}

/// Read, then increment, the persisted per-`(account_id, source)` contact-pull
/// cycle counter, returning the value BEFORE the increment.
///
/// Backed by the `settings` table (the same persistence the retired
/// `increment_gmail_sync_cycle` used, § 5.3 / § 5.6), so the cadence survives
/// a service restart - a restart no longer resets every source to cycle 0 and
/// forces an immediate pull for Gmail (~100 min budget) / Graph / CardDAV - and
/// there is no in-memory per-account map to leak on detach. Cycle 0 (a source's
/// first pull after the row is created) always pulls; the divisor gate keys off
/// the returned value.
pub fn next_contact_pull_cycle_sync(
    conn: &WriteConn<'_>,
    account_id: &str,
    source: &str,
) -> Result<u32, String> {
    let key = format!("contact_pull_cycle:{account_id}:{source}");
    let current = match get_setting(conn, &key)? {
        Some(value) => value.parse::<u32>().unwrap_or(0),
        None => 0,
    };
    // Saturate rather than wrap: the divisor gate treats any large value the
    // same modulo N, and saturating avoids a spurious "cycle 0 -> force pull"
    // on overflow.
    let next = current.saturating_add(1);
    set_setting(conn, &key, &next.to_string())?;
    Ok(current)
}
