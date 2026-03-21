use rusqlite::{Connection, params};

use ratatoskr_core::db::queries_extra::contacts::{
    ContactSettingsEntry, delete_contact_sync,
    load_contacts_for_settings_sync, save_contact_sync,
};
use ratatoskr_core::db::queries_extra::contact_groups::{
    GroupSettingsEntry, delete_group_sync,
    load_group_member_emails_sync, load_groups_for_settings_sync,
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
/// Searches the `contacts` table and `seen_addresses` table using LIKE
/// matching, plus `contact_groups` by name. Deduplicates by email
/// (contacts take priority over seen addresses). Results ranked by
/// recency (last_contacted_at / last_seen_at) as spec requires.
/// Returns up to `limit` results.
pub fn search_contacts_for_autocomplete(
    conn: &Connection,
    query: &str,
    limit: i64,
) -> Result<Vec<ContactMatch>, String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        return Ok(Vec::new());
    }
    let pattern = format!("%{trimmed}%");
    let mut results = Vec::new();
    let mut seen_emails: std::collections::HashSet<String> =
        std::collections::HashSet::new();

    // Search contacts table first (higher priority).
    // Order by recency (last_contacted_at DESC), not frequency.
    let contacts_sql = "SELECT email, display_name FROM contacts
                        WHERE email LIKE ?1 OR display_name LIKE ?1
                        ORDER BY last_contacted_at DESC NULLS LAST,
                                 display_name ASC
                        LIMIT ?2";
    let mut stmt =
        conn.prepare(contacts_sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![&pattern, limit], |row| {
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

    // Search seen_addresses table (lower priority, fills remaining).
    // Order by last_seen_at DESC for recency.
    let remaining = limit - results.len() as i64;
    if remaining > 0 {
        search_seen_addresses(
            conn,
            &pattern,
            remaining,
            &mut seen_emails,
            &mut results,
        )?;
    }

    // Search contact groups by name.
    let group_remaining = limit - results.len() as i64;
    if group_remaining > 0 {
        search_groups(conn, &pattern, group_remaining, &mut results)?;
    }

    Ok(results)
}

/// Search seen_addresses table for autocomplete matches.
fn search_seen_addresses(
    conn: &Connection,
    pattern: &str,
    limit: i64,
    seen_emails: &mut std::collections::HashSet<String>,
    results: &mut Vec<ContactMatch>,
) -> Result<(), String> {
    let seen_sql = "SELECT email, display_name FROM seen_addresses
                    WHERE email LIKE ?1 OR display_name LIKE ?1
                    ORDER BY last_seen_at DESC
                    LIMIT ?2";
    let mut seen_stmt =
        conn.prepare(seen_sql).map_err(|e| e.to_string())?;
    let seen_rows = seen_stmt
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
    let groups_sql =
        "SELECT g.id, g.name,
                (SELECT COUNT(*) FROM contact_group_members m
                 WHERE m.group_id = g.id) AS member_count
         FROM contact_groups g
         WHERE g.name LIKE ?1
         ORDER BY g.name ASC
         LIMIT ?2";
    let mut stmt =
        conn.prepare(groups_sql).map_err(|e| e.to_string())?;
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

// ── Re-export core types for app-level use ────────────────────

/// A contact entry for the settings management UI.
/// Wraps the core `ContactSettingsEntry` for backward compatibility.
pub type ContactEntry = ContactSettingsEntry;

/// A contact group entry for the settings management UI.
/// Wraps the core `GroupSettingsEntry` for backward compatibility.
pub type GroupEntry = GroupSettingsEntry;

// ── Async autocomplete wrapper for Db ─────────────────────────

impl Db {
    /// Async wrapper for autocomplete search, suitable for
    /// `Task::perform`.
    pub async fn search_autocomplete(
        &self,
        query: String,
        limit: i64,
    ) -> Result<Vec<ContactMatch>, String> {
        self.with_conn(move |conn| {
            search_contacts_for_autocomplete(conn, &query, limit)
        })
        .await
    }
}

// ── Contact management CRUD (delegates to core) ──────────────

impl Db {
    /// Load contacts for the settings management list, optionally
    /// filtered.
    pub async fn get_contacts_for_settings(
        &self,
        filter: String,
    ) -> Result<Vec<ContactEntry>, String> {
        self.with_conn(move |conn| {
            load_contacts_for_settings_sync(conn, &filter)
        })
        .await
    }

    /// Load contact groups for the settings management list.
    pub async fn get_groups_for_settings(
        &self,
        filter: String,
    ) -> Result<Vec<GroupEntry>, String> {
        self.with_conn(move |conn| {
            load_groups_for_settings_sync(conn, &filter)
        })
        .await
    }

    /// Get member emails for a group.
    pub async fn get_group_member_emails(
        &self,
        group_id: String,
    ) -> Result<Vec<String>, String> {
        self.with_conn(move |conn| {
            load_group_member_emails_sync(conn, &group_id)
        })
        .await
    }

    /// Insert or update a contact.
    pub async fn save_contact(
        &self,
        entry: ContactEntry,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            save_contact_sync(conn, &entry)
        })
        .await
    }

    /// Delete a contact by ID.
    pub async fn delete_contact(
        &self,
        contact_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            delete_contact_sync(conn, &contact_id)
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
            save_group_sync(conn, &group, &member_emails)
        })
        .await
    }

    /// Delete a contact group by ID.
    pub async fn delete_group(
        &self,
        group_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            delete_group_sync(conn, &group_id)
        })
        .await
    }
}
