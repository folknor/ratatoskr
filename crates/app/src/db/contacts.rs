use rusqlite::{Connection, params};

use rtsk::db::{build_fts_query, make_like_pattern};
use rtsk::db::queries_extra::{
    ContactSettingsEntry, GroupSettingsEntry, delete_group_sync, load_contacts_for_settings_sync,
    load_group_member_emails_sync, load_groups_for_settings_sync, save_contact_sync,
    save_group_sync,
};

use super::connection::Db;

// ── Contact search types ─────────────────────────────────────

/// A contact result from the autocomplete search.
#[derive(Debug, Clone)]
pub struct ContactMatch {
    pub email: String,
    pub display_name: Option<String>,
    /// Whether this is a group result.
    pub is_group: bool,
    /// Group ID (only set for group results).
    pub group_id: Option<String>,
    /// Member count (only set for group results).
    pub member_count: Option<i64>,
}

/// Search contacts, seen addresses, and groups for autocomplete.
///
/// Uses FTS5 prefix matching for the contacts and seen_addresses tables
/// (with LIKE fallback if the FTS5 tables are unavailable). For GAL cache,
/// short queries (1-2 chars) use prefix LIKE (`pattern%`) which can use
/// a B-tree index; longer queries use substring LIKE (`%pattern%`) for
/// mid-word matches. Returns up to `limit` results.
pub fn search_contacts_for_autocomplete(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<ContactMatch>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let like_pattern = make_like_pattern(trimmed);
    let mut results = Vec::new();
    let mut seen_emails: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Search contacts table first (higher priority) — FTS5 with LIKE fallback.
    search_contacts_fts_or_like(
        conn,
        trimmed,
        &like_pattern,
        limit,
        &mut seen_emails,
        &mut results,
    )?;

    // Search GAL cache (second priority, after synced contacts).
    #[allow(clippy::cast_possible_wrap)]
    let gal_remaining = limit - results.len() as i64;
    if gal_remaining > 0 {
        search_gal_cache(
            conn,
            &like_pattern,
            gal_remaining,
            &mut seen_emails,
            &mut results,
        )?;
    }

    // Search seen_addresses table (lower priority, fills remaining) — FTS5 with LIKE fallback.
    #[allow(clippy::cast_possible_wrap)]
    let remaining = limit - results.len() as i64;
    if remaining > 0 {
        search_seen_addresses_fts_or_like(
            conn,
            trimmed,
            &like_pattern,
            remaining,
            &mut seen_emails,
            &mut results,
        )?;
    }

    // Search contact groups by name.
    #[allow(clippy::cast_possible_wrap)]
    let group_remaining = limit - results.len() as i64;
    if group_remaining > 0 {
        search_groups(conn, &like_pattern, group_remaining, &mut results)?;
    }

    Ok(results)
}

/// Search contacts via FTS5 prefix matching, falling back to LIKE if
/// the FTS5 table is unavailable (e.g. old DB without migration 32).
fn search_contacts_fts_or_like(
    conn: &Connection,
    raw_query: &str,
    like_pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactMatch>,
) -> Result<(), String> {
    let fts_query = build_fts_query(raw_query);
    if !fts_query.is_empty() {
        let fts_sql = "SELECT c.email, c.display_name
                       FROM contacts c
                       INNER JOIN contacts_fts ON contacts_fts.rowid = c.rowid
                       WHERE contacts_fts MATCH ?1
                       ORDER BY c.last_contacted_at DESC NULLS LAST,
                                c.display_name ASC
                       LIMIT ?2";
        match conn.prepare(fts_sql).and_then(|mut stmt| {
            stmt.query_map(params![&fts_query, limit], |row| {
                Ok(ContactMatch {
                    email: row.get("email")?,
                    display_name: row.get("display_name")?,
                    is_group: false,
                    group_id: None,
                    member_count: None,
                })
            })
            .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
        }) {
            Ok(contacts) => {
                for contact in contacts {
                    let key = contact.email.to_lowercase();
                    if seen_emails.insert(key) {
                        results.push(contact);
                    }
                }
                return Ok(());
            }
            Err(_) => { /* FTS5 table missing — fall through to LIKE */ }
        }
    }

    // LIKE fallback
    let like_sql = "SELECT email, display_name FROM contacts
                    WHERE email LIKE ?1 ESCAPE '\\' OR display_name LIKE ?1 ESCAPE '\\'
                    ORDER BY last_contacted_at DESC NULLS LAST,
                             display_name ASC
                    LIMIT ?2";
    let mut stmt = conn.prepare(like_sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![like_pattern, limit], |row| {
            Ok(ContactMatch {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                is_group: false,
                group_id: None,
                member_count: None,
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let contact = row.map_err(|e| e.to_string())?;
        let key = contact.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(contact);
        }
    }
    Ok(())
}

/// Search the GAL cache for autocomplete matches.
fn search_gal_cache(
    conn: &Connection,
    pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactMatch>,
) -> Result<(), String> {
    let sql = "SELECT email, display_name FROM gal_cache
               WHERE email LIKE ?1 ESCAPE '\\' OR display_name LIKE ?1 ESCAPE '\\'
               ORDER BY display_name ASC
               LIMIT ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            Ok(ContactMatch {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                is_group: false,
                group_id: None,
                member_count: None,
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let contact = row.map_err(|e| e.to_string())?;
        let key = contact.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(contact);
        }
    }
    Ok(())
}

/// Search seen_addresses via FTS5 prefix matching, falling back to LIKE if
/// the FTS5 table is unavailable (e.g. old DB without migration 79).
fn search_seen_addresses_fts_or_like(
    conn: &Connection,
    raw_query: &str,
    like_pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactMatch>,
) -> Result<(), String> {
    let fts_query = build_fts_query(raw_query);
    if !fts_query.is_empty() {
        let fts_sql = "SELECT s.email, s.display_name
                       FROM seen_addresses s
                       INNER JOIN seen_addresses_fts ON seen_addresses_fts.rowid = s.rowid
                       WHERE seen_addresses_fts MATCH ?1
                       ORDER BY s.last_seen_at DESC
                       LIMIT ?2";
        match conn.prepare(fts_sql).and_then(|mut stmt| {
            stmt.query_map(params![&fts_query, limit], |row| {
                Ok(ContactMatch {
                    email: row.get("email")?,
                    display_name: row.get("display_name")?,
                    is_group: false,
                    group_id: None,
                    member_count: None,
                })
            })
            .map(|rows| rows.filter_map(Result::ok).collect::<Vec<_>>())
        }) {
            Ok(matches) => {
                for contact in matches {
                    let key = contact.email.to_lowercase();
                    if seen_emails.insert(key) {
                        results.push(contact);
                    }
                }
                return Ok(());
            }
            Err(_) => { /* FTS5 table missing — fall through to LIKE */ }
        }
    }

    // LIKE fallback
    let seen_sql = "SELECT email, display_name FROM seen_addresses
                    WHERE email LIKE ?1 ESCAPE '\\' OR display_name LIKE ?1 ESCAPE '\\'
                    ORDER BY last_seen_at DESC
                    LIMIT ?2";
    let mut seen_stmt = conn.prepare(seen_sql).map_err(|e| e.to_string())?;
    let seen_rows = seen_stmt
        .query_map(params![like_pattern, limit], |row| {
            Ok(ContactMatch {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                is_group: false,
                group_id: None,
                member_count: None,
            })
        })
        .map_err(|e| e.to_string())?;
    for row in seen_rows {
        let contact = row.map_err(|e| e.to_string())?;
        let key = contact.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(contact);
        }
    }
    Ok(())
}

/// Search contact groups by name for autocomplete.
fn search_groups(
    conn: &Connection,
    pattern: &str,
    limit: i64,
    results: &mut Vec<ContactMatch>,
) -> Result<(), String> {
    let groups_sql = "SELECT g.id, g.name,
                (SELECT COUNT(*) FROM contact_group_members m
                 WHERE m.group_id = g.id) AS member_count
         FROM contact_groups g
         WHERE g.name LIKE ?1 ESCAPE '\\'
         ORDER BY g.name ASC
         LIMIT ?2";
    let mut stmt = conn.prepare(groups_sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            let id: String = row.get("id")?;
            let name: String = row.get("name")?;
            let count: i64 = row.get("member_count")?;
            Ok(ContactMatch {
                email: String::new(),
                display_name: Some(name),
                is_group: true,
                group_id: Some(id),
                member_count: Some(count),
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        results.push(row.map_err(|e| e.to_string())?);
    }
    Ok(())
}

// ── Async autocomplete wrapper for Db ─────────────────────────

impl Db {
    /// Async wrapper for autocomplete search, suitable for
    /// `Task::perform`.
    pub async fn search_autocomplete(
        &self,
        query: String,
        limit: i64,
    ) -> Result<Vec<ContactMatch>, String> {
        self.with_conn(move |conn| search_contacts_for_autocomplete(conn, &query, limit))
            .await
    }
}

// ── Contact management types ─────────────────────────────────

/// A contact entry for the settings management UI.
#[derive(Debug, Clone)]
pub struct ContactEntry {
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
    /// Contact source: "user", "google", "graph", "carddav".
    /// Used to determine save behavior: local contacts save immediately,
    /// synced contacts use explicit Save with provider write-back.
    pub source: Option<String>,
    /// Provider-assigned server ID for synced contacts. Used by the action
    /// service for write-back dispatch without ambiguous email-based lookups.
    pub server_id: Option<String>,
}

/// A contact group entry for the settings management UI.
#[derive(Debug, Clone)]
pub struct GroupEntry {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub created_at: i64,
    pub updated_at: i64,
}

// ── Contact management CRUD ──────────────────────────────────

impl Db {
    pub async fn find_contact_id_by_email(&self, email: String) -> Result<Option<String>, String> {
        let db = self.read_db_state();
        rtsk::db::queries_extra::db_find_contact_id_by_email(&db, email).await
    }

    pub async fn find_group_id_by_name(&self, name: String) -> Result<Option<String>, String> {
        let db = self.read_db_state();
        rtsk::db::queries_extra::db_find_contact_group_id_by_name(&db, name).await
    }

    /// Load contacts for the settings management list, optionally
    /// filtered.
    pub async fn get_contacts_for_settings(
        &self,
        filter: String,
    ) -> Result<Vec<ContactEntry>, String> {
        self.with_conn(move |conn| {
            Ok(load_contacts_for_settings_sync(conn, &filter)?
                .into_iter()
                .map(|row| ContactEntry {
                    id: row.id,
                    email: row.email,
                    display_name: row.display_name,
                    email2: row.email2,
                    phone: row.phone,
                    company: row.company,
                    notes: row.notes,
                    account_id: row.account_id,
                    account_color: row.account_color,
                    groups: row.groups,
                    source: row.source,
                    server_id: row.server_id,
                })
                .collect())
        })
        .await
    }

    /// Load contact groups for the settings management list.
    pub async fn get_groups_for_settings(&self, filter: String) -> Result<Vec<GroupEntry>, String> {
        self.with_conn(move |conn| {
            Ok(load_groups_for_settings_sync(conn, &filter)?
                .into_iter()
                .map(|row| GroupEntry {
                    id: row.id,
                    name: row.name,
                    member_count: row.member_count,
                    created_at: row.created_at,
                    updated_at: row.updated_at,
                })
                .collect())
        })
        .await
    }

    /// Get member emails for a group.
    pub async fn get_group_member_emails(&self, group_id: String) -> Result<Vec<String>, String> {
        self.with_conn(move |conn| load_group_member_emails_sync(conn, &group_id))
            .await
    }

    /// Expand a contact group into individual (email, display_name) pairs.
    /// Recursively expands nested groups with cycle detection.
    pub async fn expand_contact_group(
        &self,
        group_id: String,
    ) -> Result<Vec<(String, Option<String>)>, String> {
        let db = self.read_db_state();
        Ok(rtsk::db::queries_extra::db_expand_contact_group_with_names(&db, group_id)
            .await?
            .into_iter()
            .map(|row| (row.email, row.display_name))
            .collect())
    }

    /// Insert or update a contact.
    pub async fn save_contact(&self, entry: ContactEntry) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            save_contact_sync(
                conn,
                &ContactSettingsEntry {
                    id: entry.id,
                    email: entry.email,
                    display_name: entry.display_name,
                    email2: entry.email2,
                    phone: entry.phone,
                    company: entry.company,
                    notes: entry.notes,
                    account_id: entry.account_id,
                    account_color: entry.account_color,
                    groups: entry.groups,
                    source: entry.source,
                    server_id: entry.server_id,
                },
            )
        })
        .await
    }

    /// Insert or update a contact group.
    pub async fn save_group(
        &self,
        group: GroupEntry,
        member_emails: Vec<String>,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            save_group_sync(
                conn,
                &GroupSettingsEntry {
                    id: group.id,
                    name: group.name,
                    member_count: group.member_count,
                    created_at: group.created_at,
                    updated_at: group.updated_at,
                },
                &member_emails,
            )
        })
        .await
    }

    /// Delete a contact group by ID.
    pub async fn delete_group(&self, group_id: String) -> Result<(), String> {
        self.with_write_conn(move |conn| delete_group_sync(conn, &group_id))
        .await
    }
}
