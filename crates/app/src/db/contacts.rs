use rusqlite::{Connection, params};

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
    /// Load contacts for the settings management list, optionally
    /// filtered.
    pub async fn get_contacts_for_settings(
        &self,
        filter: String,
    ) -> Result<Vec<ContactEntry>, String> {
        self.with_conn(move |conn| {
            load_contacts_filtered(conn, &filter)
        })
        .await
    }

    /// Load contact groups for the settings management list.
    pub async fn get_groups_for_settings(
        &self,
        filter: String,
    ) -> Result<Vec<GroupEntry>, String> {
        self.with_conn(move |conn| load_groups_filtered(conn, &filter))
            .await
    }

    /// Get member emails for a group.
    pub async fn get_group_member_emails(
        &self,
        group_id: String,
    ) -> Result<Vec<String>, String> {
        self.with_conn(move |conn| {
            load_group_member_emails(conn, &group_id)
        })
        .await
    }

    /// Insert or update a contact.
    pub async fn save_contact(
        &self,
        entry: ContactEntry,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            save_contact_inner(conn, &entry)
        })
        .await
    }

    /// Delete a contact by ID.
    pub async fn delete_contact(
        &self,
        contact_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "DELETE FROM contacts WHERE id = ?1",
                params![contact_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
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
            save_group_inner(conn, &group, &member_emails)
        })
        .await
    }

    /// Delete a contact group by ID.
    pub async fn delete_group(
        &self,
        group_id: String,
    ) -> Result<(), String> {
        self.with_write_conn(move |conn| {
            conn.execute(
                "DELETE FROM contact_groups WHERE id = ?1",
                params![group_id],
            )
            .map_err(|e| e.to_string())?;
            Ok(())
        })
        .await
    }
}

/// Load contacts with group memberships via a single JOIN query
/// (replaces the N+1 pattern of calling load_contact_groups per
/// contact).
fn load_contacts_filtered(
    conn: &Connection,
    filter: &str,
) -> Result<Vec<ContactEntry>, String> {
    let trimmed = filter.trim();
    let pattern = format!("%{trimmed}%");

    // Single query that JOINs contacts with their group memberships.
    let sql = if trimmed.is_empty() {
        "SELECT c.id, c.email, c.display_name, c.email2, c.phone,
                c.company, c.notes, c.account_id, c.source,
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
                c.company, c.notes, c.account_id, c.source,
                a.account_color,
                GROUP_CONCAT(g.name, '||') AS group_names
         FROM contacts c
         LEFT JOIN accounts a ON a.id = c.account_id
         LEFT JOIN contact_group_members m
           ON m.member_type = 'email' AND m.member_value = c.email
         LEFT JOIN contact_groups g ON g.id = m.group_id
         WHERE c.source != 'seen'
           AND (c.email LIKE ?1
                OR c.display_name LIKE ?1
                OR c.company LIKE ?1)
         GROUP BY c.id
         ORDER BY c.last_contacted_at DESC NULLS LAST,
                  c.display_name ASC
         LIMIT 200"
    };

    let db_params: &[&dyn rusqlite::types::ToSql] = if trimmed.is_empty() {
        &[]
    } else {
        &[&pattern]
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(db_params, |row| {
            let group_names: Option<String> = row.get("group_names")?;
            let groups = group_names
                .map(|s| {
                    s.split("||")
                        .map(String::from)
                        .collect::<Vec<_>>()
                })
                .unwrap_or_default();
            Ok(ContactEntry {
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
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn load_groups_filtered(
    conn: &Connection,
    filter: &str,
) -> Result<Vec<GroupEntry>, String> {
    let trimmed = filter.trim();
    let pattern = format!("%{trimmed}%");

    let sql = if trimmed.is_empty() {
        "SELECT g.id, g.name, g.created_at, g.updated_at,
                (SELECT COUNT(*) FROM contact_group_members m
                 WHERE m.group_id = g.id) AS member_count
         FROM contact_groups g
         ORDER BY g.updated_at DESC
         LIMIT 100"
    } else {
        "SELECT g.id, g.name, g.created_at, g.updated_at,
                (SELECT COUNT(*) FROM contact_group_members m
                 WHERE m.group_id = g.id) AS member_count
         FROM contact_groups g
         WHERE g.name LIKE ?1
         ORDER BY g.updated_at DESC
         LIMIT 100"
    };

    let db_params: &[&dyn rusqlite::types::ToSql] = if trimmed.is_empty() {
        &[]
    } else {
        &[&pattern]
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(db_params, |row| {
            Ok(GroupEntry {
                id: row.get("id")?,
                name: row.get("name")?,
                member_count: row.get("member_count")?,
                created_at: row.get("created_at")?,
                updated_at: row.get("updated_at")?,
            })
        })
        .map_err(|e| e.to_string())?;

    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn load_group_member_emails(
    conn: &Connection,
    group_id: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT member_value FROM contact_group_members
             WHERE group_id = ?1 AND member_type = 'email'
             ORDER BY member_value ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![group_id], |row| row.get::<_, String>(0))
        .map_err(|e| e.to_string())?;
    rows.collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
}

fn save_contact_inner(
    conn: &Connection,
    entry: &ContactEntry,
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

fn save_group_inner(
    conn: &Connection,
    group: &GroupEntry,
    member_emails: &[String],
) -> Result<(), String> {
    let now = chrono::Utc::now().timestamp();
    conn.execute(
        "INSERT INTO contact_groups (id, name, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?3)
         ON CONFLICT(id) DO UPDATE SET
             name = excluded.name,
             updated_at = excluded.updated_at",
        params![group.id, group.name, now],
    )
    .map_err(|e| e.to_string())?;

    // Replace all members
    conn.execute(
        "DELETE FROM contact_group_members WHERE group_id = ?1",
        params![group.id],
    )
    .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "INSERT INTO contact_group_members
             (group_id, member_type, member_value)
             VALUES (?1, 'email', ?2)",
        )
        .map_err(|e| e.to_string())?;

    for email in member_emails {
        stmt.execute(params![group.id, email])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
