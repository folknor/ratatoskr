//! Contact search types and unified search across contacts, seen addresses,
//! and contact groups.
//!
//! `ContactSearchResult` is the canonical search result type used by both
//! compose autocomplete and calendar attendee fields. It lives here in core
//! (not in the app crate) so that all layers can share it.

use rusqlite::{Connection, params};

use crate::db::build_fts_query;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// The kind of search result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactSearchKind {
    /// A regular contact (synced or local).
    Contact,
    /// An address observed in message headers (lower priority).
    SeenAddress,
    /// A contact group (can be expanded into individual addresses).
    Group {
        group_id: String,
        member_count: i64,
    },
}

/// A single result from a contact/autocomplete search.
///
/// Used by compose recipient fields and calendar attendee fields.
/// Deduplicates by email across contacts, seen addresses, and groups.
#[derive(Debug, Clone)]
pub struct ContactSearchResult {
    /// Email address (empty for group results).
    pub email: String,
    /// Display name.
    pub display_name: Option<String>,
    /// What kind of result this is.
    pub kind: ContactSearchKind,
    /// Source identifier (e.g. "google", "graph", "carddav", "user", "seen").
    pub source: Option<String>,
}

// ---------------------------------------------------------------------------
// Unified search
// ---------------------------------------------------------------------------

/// Search contacts, seen addresses, and groups for autocomplete.
///
/// Uses FTS5 prefix matching for the contacts table (with LIKE fallback
/// if the FTS5 table is unavailable). For seen_addresses, short queries
/// (1-2 chars) use prefix LIKE (`pattern%`) which can use a B-tree index;
/// longer queries use substring LIKE (`%pattern%`) for mid-word matches.
/// Deduplicates by email (contacts take priority over seen addresses).
/// Returns up to `limit` results.
pub fn search_contacts_unified(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<ContactSearchResult>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let like_pattern = make_like_pattern(trimmed);
    let mut results = Vec::new();
    let mut seen_emails: std::collections::HashSet<String> = std::collections::HashSet::new();

    // 1. Search contacts table (highest priority) — FTS5 with LIKE fallback.
    search_contacts_fts_or_like(conn, trimmed, &like_pattern, limit, &mut seen_emails, &mut results)?;

    // 2. Search seen_addresses table (lower priority, fills remaining).
    let remaining = limit - i64::try_from(results.len()).unwrap_or(i64::MAX);
    if remaining > 0 {
        search_seen_addresses_table(conn, &like_pattern, remaining, &mut seen_emails, &mut results)?;
    }

    // 3. Search contact groups by name.
    let group_remaining = limit - i64::try_from(results.len()).unwrap_or(i64::MAX);
    if group_remaining > 0 {
        search_groups_table(conn, &like_pattern, group_remaining, &mut results)?;
    }

    Ok(results)
}

/// Build LIKE pattern: short queries (1-2 chars) use prefix match
/// (`pattern%`) which can use a B-tree index; longer queries use
/// substring match (`%pattern%`) for mid-word hits.
fn make_like_pattern(trimmed: &str) -> String {
    if trimmed.len() <= 2 {
        format!("{trimmed}%")
    } else {
        format!("%{trimmed}%")
    }
}

/// Search contacts via FTS5 prefix matching, falling back to LIKE if
/// the FTS5 table is unavailable.
fn search_contacts_fts_or_like(
    conn: &Connection,
    raw_query: &str,
    like_pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let fts_query = build_fts_query(raw_query);
    if !fts_query.is_empty() {
        let fts_sql = "SELECT c.email, c.display_name, c.source
                       FROM contacts c
                       INNER JOIN contacts_fts ON contacts_fts.rowid = c.rowid
                       WHERE contacts_fts MATCH ?1
                       ORDER BY c.last_contacted_at DESC NULLS LAST,
                                c.display_name ASC
                       LIMIT ?2";
        match conn.prepare(fts_sql) {
            Ok(mut stmt) => {
                let rows = stmt
                    .query_map(params![&fts_query, limit], |row| {
                        Ok(ContactSearchResult {
                            email: row.get("email")?,
                            display_name: row.get("display_name")?,
                            kind: ContactSearchKind::Contact,
                            source: row.get("source")?,
                        })
                    })
                    .map_err(|e| e.to_string())?;
                for row in rows {
                    let result = row.map_err(|e| e.to_string())?;
                    let key = result.email.to_lowercase();
                    if seen_emails.insert(key) {
                        results.push(result);
                    }
                }
                return Ok(());
            }
            Err(_) => { /* FTS5 table missing — fall through to LIKE */ }
        }
    }

    // LIKE fallback
    search_contacts_table(conn, like_pattern, limit, seen_emails, results)
}

// ---------------------------------------------------------------------------
// Internal search helpers
// ---------------------------------------------------------------------------

fn search_contacts_table(
    conn: &Connection,
    pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let sql = "SELECT email, display_name, source FROM contacts
               WHERE email LIKE ?1 OR display_name LIKE ?1
               ORDER BY last_contacted_at DESC NULLS LAST, display_name ASC
               LIMIT ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            Ok(ContactSearchResult {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                kind: ContactSearchKind::Contact,
                source: row.get("source")?,
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let result = row.map_err(|e| e.to_string())?;
        let key = result.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(result);
        }
    }
    Ok(())
}

fn search_seen_addresses_table(
    conn: &Connection,
    pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let sql = "SELECT email, display_name FROM seen_addresses
               WHERE email LIKE ?1 OR display_name LIKE ?1
               ORDER BY last_seen_at DESC
               LIMIT ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            Ok(ContactSearchResult {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
                kind: ContactSearchKind::SeenAddress,
                source: Some("seen".to_string()),
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        let result = row.map_err(|e| e.to_string())?;
        let key = result.email.to_lowercase();
        if seen_emails.insert(key) {
            results.push(result);
        }
    }
    Ok(())
}

fn search_groups_table(
    conn: &Connection,
    pattern: &str,
    limit: i64,
    results: &mut Vec<ContactSearchResult>,
) -> Result<(), String> {
    let sql = "SELECT g.id, g.name,
                      (SELECT COUNT(*) FROM contact_group_members m
                       WHERE m.group_id = g.id) AS member_count
               FROM contact_groups g
               WHERE g.name LIKE ?1
               ORDER BY g.name ASC
               LIMIT ?2";
    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![pattern, limit], |row| {
            let id: String = row.get("id")?;
            let name: String = row.get("name")?;
            let count: i64 = row.get("member_count")?;
            Ok(ContactSearchResult {
                email: String::new(),
                display_name: Some(name),
                kind: ContactSearchKind::Group {
                    group_id: id,
                    member_count: count,
                },
                source: None,
            })
        })
        .map_err(|e| e.to_string())?;
    for row in rows {
        results.push(row.map_err(|e| e.to_string())?);
    }
    Ok(())
}
