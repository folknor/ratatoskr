use super::super::DbState;
use super::super::types::{
    ContactAttachmentRow, ContactStats, DbContact, RecentThread, SameDomainContact,
};
use crate::db::FromRow;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone)]
pub struct ExpandedGroupContact {
    pub email: String,
    pub display_name: Option<String>,
}

pub async fn db_get_all_contacts(
    db: &DbState,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbContact>, String> {
    log::debug!("Loading contacts: limit={limit:?}, offset={offset:?}");
    db.with_conn(move |conn| {
        let lim = limit.unwrap_or(crate::db::DEFAULT_QUERY_LIMIT);
        let off = offset.unwrap_or(0);
        let mut stmt = conn
            .prepare(
                "SELECT * FROM contacts ORDER BY frequency DESC, display_name ASC LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![lim, off], DbContact::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_upsert_contact(
    db: &DbState,
    id: String,
    email: String,
    display_name: Option<String>,
) -> Result<(), String> {
    log::info!("Upserting contact: email={email}, display_name={display_name:?}");
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.execute(
            "INSERT INTO contacts (id, email, display_name, last_contacted_at)
                 VALUES (?1, ?2, ?3, unixepoch())
                 ON CONFLICT(email) DO UPDATE SET
                   display_name = COALESCE(?3, display_name),
                   frequency = frequency + 1,
                   last_contacted_at = unixepoch(),
                   updated_at = unixepoch()",
            params![id, normalized, display_name],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_contact(
    db: &DbState,
    id: String,
    display_name: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE contacts SET \
               display_name = ?1, \
               display_name_overridden = CASE \
                 WHEN source IN ('graph', 'google', 'carddav', 'jmap') THEN 1 \
                 ELSE display_name_overridden \
               END, \
               updated_at = unixepoch() \
             WHERE id = ?2",
            params![display_name, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_update_contact_notes(
    db: &DbState,
    email: String,
    notes: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.execute(
            "UPDATE contacts SET notes = ?1, updated_at = unixepoch() WHERE email = ?2",
            params![notes, normalized],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_find_contact_id_by_email(
    db: &DbState,
    email: String,
) -> Result<Option<String>, String> {
    db.with_conn(move |conn| {
        conn.query_row(
            "SELECT id FROM contacts WHERE email = ?1 LIMIT 1",
            params![email],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|e| e.to_string())
    })
    .await
}

/// Upsert a contact with all mutable fields.
///
/// Used by the contact action service. The app-crate `save_contact_inner`
/// has equivalent SQL — this is the core-accessible version.
pub fn db_upsert_contact_full(
    conn: &Connection,
    id: &str,
    email: &str,
    display_name: Option<&str>,
    email2: Option<&str>,
    phone: Option<&str>,
    company: Option<&str>,
    notes: Option<&str>,
    account_id: Option<&str>,
    source: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, email2, phone,
                               company, notes, account_id, source,
                               created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
         ON CONFLICT(id) DO UPDATE SET
             email = excluded.email,
             display_name = excluded.display_name,
             email2 = excluded.email2,
             phone = excluded.phone,
             company = excluded.company,
             notes = excluded.notes,
             account_id = excluded.account_id,
             updated_at = excluded.updated_at",
        params![
            id,
            email,
            display_name,
            email2,
            phone,
            company,
            notes,
            account_id,
            source,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn db_delete_contact(db: &DbState, id: String) -> Result<(), String> {
    log::info!("Deleting contact: id={id}");
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM contacts WHERE id = ?1", params![id])
            .map_err(|e| {
                log::error!("Failed to delete contact {id}: {e}");
                e.to_string()
            })?;
        Ok(())
    })
    .await
}

pub async fn db_get_contact_stats(db: &DbState, email: String) -> Result<ContactStats, String> {
    log::debug!("Loading contact stats: email={email}");
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.query_row(
            "SELECT COUNT(*) as cnt, MIN(date) as first_date, MAX(date) as last_date
                 FROM messages WHERE from_address = ?1",
            params![normalized],
            ContactStats::from_row,
        )
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_contacts_from_same_domain(
    db: &DbState,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<SameDomainContact>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        let domain = normalized
            .split('@')
            .nth(1)
            .map(|d| format!("%@{d}"))
            .unwrap_or_default();
        let lim = limit.unwrap_or(5);
        let mut stmt = conn
            .prepare(
                "SELECT email, display_name, avatar_url FROM contacts
                     WHERE email LIKE ?1 AND email != ?2
                     ORDER BY frequency DESC LIMIT ?3",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(
            params![domain, normalized, lim],
            SameDomainContact::from_row,
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_latest_auth_result(
    db: &DbState,
    email: String,
) -> Result<Option<String>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        let result = conn
            .query_row(
                "SELECT auth_results FROM messages
                     WHERE from_address = ?1 AND auth_results IS NOT NULL
                     ORDER BY date DESC LIMIT 1",
                params![normalized],
                |row| row.get::<_, String>("auth_results"),
            )
            .ok();
        Ok(result)
    })
    .await
}

pub async fn db_get_recent_threads_with_contact(
    db: &DbState,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<RecentThread>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        let lim = limit.unwrap_or(5);
        let mut stmt = conn
            .prepare(
                "SELECT DISTINCT t.id as thread_id, t.subject, t.last_message_at
                     FROM threads t
                     INNER JOIN messages m ON m.account_id = t.account_id AND m.thread_id = t.id
                     WHERE m.from_address = ?1
                     ORDER BY t.last_message_at DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![normalized, lim], RecentThread::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_attachments_from_contact(
    db: &DbState,
    email: String,
    limit: Option<i64>,
) -> Result<Vec<ContactAttachmentRow>, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        let lim = limit.unwrap_or(5);
        let mut stmt = conn
            .prepare(
                "SELECT a.filename, a.mime_type, a.size, m.date
                     FROM attachments a
                     INNER JOIN messages m ON m.account_id = a.account_id AND m.id = a.message_id
                     WHERE m.from_address = ?1 AND a.is_inline = 0 AND a.filename IS NOT NULL
                     ORDER BY m.date DESC LIMIT ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![normalized, lim], ContactAttachmentRow::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

// ── Synchronous contact helpers (for app-layer settings UI) ──

/// A contact entry with extended fields for the settings UI.
#[derive(Debug, Clone)]
pub struct ContactSettingsEntry {
    pub id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub notes: Option<String>,
    pub account_id: Option<String>,
    pub account_color: Option<String>,
    pub groups: Vec<String>,
    pub source: Option<String>,
    pub server_id: Option<String>,
}

/// Load contacts for the settings management list (synchronous).
pub fn load_contacts_for_settings_sync(
    conn: &rusqlite::Connection,
    filter: &str,
) -> Result<Vec<ContactSettingsEntry>, String> {
    let trimmed = filter.trim();
    let escaped = trimmed.replace('%', r"\%").replace('_', r"\_");
    let pattern = format!("%{escaped}%");

    let sql = if trimmed.is_empty() {
        "SELECT c.id, c.email, c.display_name, c.email2, c.phone,
                c.company, c.notes, c.account_id, c.source, c.server_id,
                a.account_color,
                GROUP_CONCAT(g.name, '||') AS group_names
         FROM contacts c
        LEFT JOIN accounts a ON a.id = c.account_id
        LEFT JOIN contact_group_members m
           ON m.member_type = 'email' AND m.member_value = c.email
         LEFT JOIN contact_groups g ON g.id = m.group_id
         WHERE c.source != 'seen'
         GROUP BY c.id
         ORDER BY c.last_contacted_at DESC NULLS LAST,
                  c.display_name ASC
         LIMIT 200"
    } else {
        "SELECT c.id, c.email, c.display_name, c.email2, c.phone,
                c.company, c.notes, c.account_id, c.source, c.server_id,
                a.account_color,
                GROUP_CONCAT(g.name, '||') AS group_names
         FROM contacts c
         LEFT JOIN accounts a ON a.id = c.account_id
         LEFT JOIN contact_group_members m
           ON m.member_type = 'email' AND m.member_value = c.email
         LEFT JOIN contact_groups g ON g.id = m.group_id
         WHERE c.source != 'seen'
           AND (c.email LIKE ?1 ESCAPE '\\'
                OR c.display_name LIKE ?1 ESCAPE '\\'
                OR c.company LIKE ?1 ESCAPE '\\')
         GROUP BY c.id
         ORDER BY c.last_contacted_at DESC NULLS LAST,
                  c.display_name ASC
         LIMIT 200"
    };

    let db_params: &[&dyn rusqlite::types::ToSql] =
        if trimmed.is_empty() { &[] } else { &[&pattern] };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(db_params, |row| {
            let group_names: Option<String> = row.get("group_names")?;
            let groups = group_names
                .map(|s| s.split("||").map(String::from).collect::<Vec<_>>())
                .unwrap_or_default();
            Ok(ContactSettingsEntry {
                id: row.get("id")?,
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                email2: row.get("email2")?,
                phone: row.get("phone")?,
                company: row.get("company")?,
                notes: row.get("notes")?,
                account_id: row.get("account_id")?,
                account_color: row.get("account_color")?,
                groups,
                source: row.get("source")?,
                server_id: row.get("server_id")?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

/// Save (upsert) a contact with extended fields (synchronous).
pub fn save_contact_sync(
    conn: &rusqlite::Connection,
    entry: &ContactSettingsEntry,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    let source = entry.source.as_deref().unwrap_or("user");
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, email2, phone,
                               company, notes, account_id, source,
                               created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)
         ON CONFLICT(id) DO UPDATE SET
             email = excluded.email,
             display_name = excluded.display_name,
             email2 = excluded.email2,
             phone = excluded.phone,
             company = excluded.company,
             notes = excluded.notes,
             account_id = excluded.account_id,
             updated_at = excluded.updated_at",
        params![
            entry.id,
            entry.email,
            entry.display_name,
            entry.email2,
            entry.phone,
            entry.company,
            entry.notes,
            entry.account_id,
            source,
            now,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete a contact by ID (synchronous).
pub fn delete_contact_sync(conn: &rusqlite::Connection, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM contacts WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Contact save (dual local/synced pattern)
// ---------------------------------------------------------------------------

/// A contact update payload. Fields set to `None` are not changed.
/// Double-option fields (`Option<Option<String>>`) use outer None = "no change",
/// inner None = "clear field".
#[derive(Debug, Clone)]
pub struct ContactUpdate {
    pub email: String,
    pub display_name: Option<String>,
    pub email2: Option<Option<String>>,
    pub phone: Option<Option<String>>,
    pub company: Option<Option<String>>,
    pub notes: Option<Option<String>>,
}

/// Save a local contact's fields. Does not set `display_name_overridden`.
pub fn save_local_contact_fields_sync(
    conn: &rusqlite::Connection,
    update: &ContactUpdate,
) -> Result<(), String> {
    apply_contact_update_inner(conn, update, true)
}

/// Save a synced contact's fields. Sets `display_name_overridden = 1`
/// for display name changes (local-only override, not pushed to provider).
pub fn save_synced_contact_fields_sync(
    conn: &rusqlite::Connection,
    update: &ContactUpdate,
) -> Result<(), String> {
    apply_contact_update_inner(conn, update, false)
}

/// Look up the raw source string for a contact by email.
/// Returns `None` if no contact with that email exists.
pub fn get_contact_source_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT source FROM contacts WHERE email = ?1",
        rusqlite::params![email],
        |row| row.get("source"),
    )
    .ok()
    .map_or(Ok(None), |v| Ok(Some(v)))
}

fn apply_contact_update_inner(
    conn: &rusqlite::Connection,
    update: &ContactUpdate,
    is_local: bool,
) -> Result<(), String> {
    let normalized_email = update.email.to_lowercase();

    if let Some(ref name) = update.display_name {
        if is_local {
            conn.execute(
                "UPDATE contacts SET display_name = ?1, updated_at = unixepoch() \
                 WHERE email = ?2",
                rusqlite::params![name, normalized_email],
            )
            .map_err(|e| format!("update display_name: {e}"))?;
        } else {
            conn.execute(
                "UPDATE contacts SET display_name = ?1, display_name_overridden = 1, \
                 updated_at = unixepoch() WHERE email = ?2",
                rusqlite::params![name, normalized_email],
            )
            .map_err(|e| format!("update display_name (synced): {e}"))?;
        }
    }

    if let Some(ref email2) = update.email2 {
        conn.execute(
            "UPDATE contacts SET email2 = ?1, updated_at = unixepoch() WHERE email = ?2",
            rusqlite::params![email2, normalized_email],
        )
        .map_err(|e| format!("update email2: {e}"))?;
    }

    if let Some(ref phone) = update.phone {
        conn.execute(
            "UPDATE contacts SET phone = ?1, updated_at = unixepoch() WHERE email = ?2",
            rusqlite::params![phone, normalized_email],
        )
        .map_err(|e| format!("update phone: {e}"))?;
    }

    if let Some(ref company) = update.company {
        conn.execute(
            "UPDATE contacts SET company = ?1, updated_at = unixepoch() WHERE email = ?2",
            rusqlite::params![company, normalized_email],
        )
        .map_err(|e| format!("update company: {e}"))?;
    }

    if let Some(ref notes) = update.notes {
        conn.execute(
            "UPDATE contacts SET notes = ?1, updated_at = unixepoch() WHERE email = ?2",
            rusqlite::params![notes, normalized_email],
        )
        .map_err(|e| format!("update notes: {e}"))?;
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Seen address helpers
// ---------------------------------------------------------------------------

/// Aggregated stats for a seen address across all accounts.
#[derive(Debug, Clone)]
pub struct SeenAddressStats {
    pub email: String,
    pub display_name: Option<String>,
    pub times_sent_to: i64,
    pub times_received_from: i64,
    pub first_seen_at: i64,
    pub last_seen_at: i64,
}

/// Promote a seen address to a contact with source = 'user'.
/// No-op if a contact with that email already exists.
pub fn promote_seen_to_contact_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<(), String> {
    let normalized = email.to_lowercase();

    // Check if already a contact
    let exists: bool = conn
        .query_row(
            "SELECT COUNT(*) AS cnt FROM contacts WHERE email = ?1",
            rusqlite::params![normalized],
            |row| row.get::<_, i64>("cnt"),
        )
        .map_err(|e| format!("check contact exists: {e}"))?
        > 0;

    if exists {
        return Ok(());
    }

    // Get display name from seen_addresses
    let display_name: Option<String> = conn
        .query_row(
            "SELECT display_name FROM seen_addresses \
             WHERE email = ?1 \
             ORDER BY last_seen_at DESC LIMIT 1",
            rusqlite::params![normalized],
            |row| row.get("display_name"),
        )
        .ok()
        .flatten();

    let id = format!("promoted-{normalized}");
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, source) \
         VALUES (?1, ?2, ?3, 'user')",
        rusqlite::params![id, normalized, display_name],
    )
    .map_err(|e| format!("promote seen to contact: {e}"))?;

    Ok(())
}

/// Get aggregated stats for a seen address across all accounts.
pub fn get_seen_address_stats_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<Option<SeenAddressStats>, String> {
    let normalized = email.to_lowercase();
    conn.query_row(
        "SELECT email, display_name,
                SUM(times_sent_to) AS total_sent,
                SUM(times_received_from) AS total_received,
                MIN(first_seen_at) AS first_seen,
                MAX(last_seen_at) AS last_seen
         FROM seen_addresses
         WHERE email = ?1
         GROUP BY email",
        rusqlite::params![normalized],
        |row| {
            Ok(SeenAddressStats {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                times_sent_to: row.get("total_sent")?,
                times_received_from: row.get("total_received")?,
                first_seen_at: row.get("first_seen")?,
                last_seen_at: row.get("last_seen")?,
            })
        },
    )
    .map_err(|e| e.to_string())
    .map(Some)
    .or_else(|_| Ok(None))
}

// ---------------------------------------------------------------------------
// Google contact enrichment
// ---------------------------------------------------------------------------

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
    .or_else(|_| Ok(None))
}

// ---------------------------------------------------------------------------
// Graph contact enrichment
// ---------------------------------------------------------------------------

/// Server-side info for a Graph contact.
#[derive(Debug, Clone)]
pub struct GraphServerInfo {
    pub graph_contact_id: String,
    pub account_id: String,
}

/// Enrich contacts with Graph account_id and server_id via graph_contact_map.
pub fn enrich_graph_contacts_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<usize, String> {
    conn.execute(
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
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("enrich graph contacts: {e}"))
}

/// Look up the Graph contact ID and account for a contact email.
pub fn get_graph_contact_server_info_sync(
    conn: &rusqlite::Connection,
    email: &str,
) -> Result<Option<GraphServerInfo>, String> {
    let normalized = email.to_lowercase();
    conn.query_row(
        "SELECT m.graph_contact_id, m.account_id \
         FROM graph_contact_map m \
         WHERE m.email = ?1 \
         LIMIT 1",
        rusqlite::params![normalized],
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
}

// ---------------------------------------------------------------------------
// GAL (Global Address List) cache
// ---------------------------------------------------------------------------

/// A GAL entry for bulk cache insert.
#[derive(Debug, Clone)]
pub struct GalEntry {
    pub email: String,
    pub display_name: Option<String>,
    pub phone: Option<String>,
    pub company: Option<String>,
    pub title: Option<String>,
    pub department: Option<String>,
}

/// Clear and refill the GAL cache for an account.
pub fn cache_gal_entries_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
    entries: &[GalEntry],
) -> Result<usize, String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin gal tx: {e}"))?;

    tx.execute(
        "DELETE FROM gal_cache WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| format!("clear gal cache: {e}"))?;

    let mut stmt = tx
        .prepare(
            "INSERT OR REPLACE INTO gal_cache \
             (email, display_name, phone, company, title, department, account_id, cached_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, unixepoch())",
        )
        .map_err(|e| format!("prepare gal insert: {e}"))?;

    for entry in entries {
        stmt.execute(rusqlite::params![
            entry.email,
            entry.display_name,
            entry.phone,
            entry.company,
            entry.title,
            entry.department,
            account_id,
        ])
        .map_err(|e| format!("insert gal entry: {e}"))?;
    }

    drop(stmt);
    tx.commit().map_err(|e| format!("commit gal tx: {e}"))?;
    Ok(entries.len())
}

/// Get the timestamp of the last GAL refresh for an account.
pub fn gal_cache_age_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<Option<i64>, String> {
    let key = format!("gal_refresh_{account_id}");
    conn.query_row(
        "SELECT value FROM settings WHERE key = ?1",
        rusqlite::params![key],
        |row| row.get::<_, String>(0),
    )
    .ok()
    .map(|v| {
        v.parse::<i64>()
            .map_err(|e| format!("parse gal timestamp: {e}"))
    })
    .transpose()
}

/// Record that a GAL refresh was performed for an account.
pub fn record_gal_refresh_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp().to_string();
    let key = format!("gal_refresh_{account_id}");
    conn.execute(
        "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
        rusqlite::params![key, now],
    )
    .map_err(|e| format!("record gal refresh: {e}"))?;
    Ok(())
}

/// Look up the provider type for an account.
pub fn get_account_provider_sync(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<String, String> {
    conn.query_row(
        "SELECT provider FROM accounts WHERE id = ?1",
        rusqlite::params![account_id],
        |row| row.get(0),
    )
    .map_err(|e| format!("lookup provider: {e}"))
}

// ---------------------------------------------------------------------------
// Contact deduplication
// ---------------------------------------------------------------------------

/// A raw duplicate pair row from the database.
#[derive(Debug, Clone)]
pub struct DuplicatePairRow {
    pub contact_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub source: String,
    pub seen_name: Option<String>,
    pub seen_account_id: String,
}

/// Find contacts that also exist in seen_addresses (duplicate candidates).
pub fn find_contact_duplicates_sync(
    conn: &rusqlite::Connection,
    limit: i64,
) -> Result<Vec<DuplicatePairRow>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT c.id, c.email, c.display_name, c.source,
                    s.display_name AS seen_name, s.account_id AS seen_account_id
             FROM contacts c
             INNER JOIN seen_addresses s ON LOWER(c.email) = LOWER(s.email)
             WHERE c.source != 'seen'
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;

    stmt.query_map(rusqlite::params![limit], |row| {
        Ok(DuplicatePairRow {
            contact_id: row.get("id")?,
            email: row.get("email")?,
            display_name: row.get("display_name")?,
            source: row.get("source")?,
            seen_name: row.get("seen_name")?,
            seen_account_id: row.get::<_, String>("seen_account_id").unwrap_or_default(),
        })
    })
    .map_err(|e| e.to_string())?
    .collect::<Result<Vec<_>, _>>()
    .map_err(|e| e.to_string())
}

/// Update a contact's display name from a seen duplicate (auto-merge).
/// Only updates if the contact's current display_name is NULL.
pub fn merge_seen_duplicate_sync(
    conn: &rusqlite::Connection,
    contact_id: &str,
    seen_display_name: &str,
) -> Result<bool, String> {
    let changed = conn
        .execute(
            "UPDATE contacts SET display_name = ?1, updated_at = unixepoch() \
             WHERE id = ?2 AND display_name IS NULL",
            rusqlite::params![seen_display_name, contact_id],
        )
        .map_err(|e| format!("merge display name: {e}"))?;
    Ok(changed > 0)
}

/// Merge two contacts by ID within a single transaction.
/// The keep contact's NULL fields are filled from the merge contact.
/// Group memberships are migrated. The merge contact is deleted.
pub fn merge_contact_pair_sync(
    conn: &rusqlite::Connection,
    keep_id: &str,
    merge_id: &str,
) -> Result<(), String> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("begin merge tx: {e}"))?;

    // Read merge contact's fields
    let merge_row: Option<(
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    )> = tx
        .query_row(
            "SELECT display_name, email2, phone, company, notes, avatar_url \
             FROM contacts WHERE id = ?1",
            rusqlite::params![merge_id],
            |row| {
                Ok((
                    row.get("display_name")?,
                    row.get("email2")?,
                    row.get("phone")?,
                    row.get("company")?,
                    row.get("notes")?,
                    row.get("avatar_url")?,
                ))
            },
        )
        .ok();

    let Some((name, email2, phone, company, notes, avatar_url)) = merge_row else {
        return Err(format!("merge contact {merge_id} not found"));
    };

    // Fill in null fields on the keep contact
    tx.execute(
        "UPDATE contacts SET
           display_name = COALESCE(display_name, ?1),
           email2 = COALESCE(email2, ?2),
           phone = COALESCE(phone, ?3),
           company = COALESCE(company, ?4),
           notes = COALESCE(notes, ?5),
           avatar_url = COALESCE(avatar_url, ?6),
           updated_at = unixepoch()
         WHERE id = ?7",
        rusqlite::params![name, email2, phone, company, notes, avatar_url, keep_id],
    )
    .map_err(|e| format!("merge into keep contact: {e}"))?;

    // Move group memberships from merge to keep
    let keep_email: Option<String> = tx
        .query_row(
            "SELECT email FROM contacts WHERE id = ?1",
            rusqlite::params![keep_id],
            |row| row.get("email"),
        )
        .ok();

    let merge_email: Option<String> = tx
        .query_row(
            "SELECT email FROM contacts WHERE id = ?1",
            rusqlite::params![merge_id],
            |row| row.get("email"),
        )
        .ok();

    if let (Some(ref keep_email), Some(ref merge_email)) = (keep_email, merge_email) {
        tx.execute(
            "UPDATE OR IGNORE contact_group_members \
             SET member_value = ?1 \
             WHERE member_type = 'email' AND member_value = ?2",
            rusqlite::params![keep_email, merge_email],
        )
        .map_err(|e| format!("migrate group memberships: {e}"))?;

        tx.execute(
            "DELETE FROM contact_group_members \
             WHERE member_type = 'email' AND member_value = ?1 \
             AND group_id IN (
               SELECT group_id FROM contact_group_members
               WHERE member_type = 'email' AND member_value = ?2
             )",
            rusqlite::params![merge_email, keep_email],
        )
        .map_err(|e| format!("clean duplicate memberships: {e}"))?;
    }

    // Delete the merge contact
    tx.execute(
        "DELETE FROM contacts WHERE id = ?1",
        rusqlite::params![merge_id],
    )
    .map_err(|e| format!("delete merged contact: {e}"))?;

    tx.commit().map_err(|e| format!("commit merge tx: {e}"))?;
    Ok(())
}

pub async fn db_update_contact_avatar(
    db: &DbState,
    email: String,
    avatar_url: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.execute(
            "UPDATE contacts SET avatar_url = ?1, updated_at = unixepoch() WHERE email = ?2",
            params![avatar_url, normalized],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
