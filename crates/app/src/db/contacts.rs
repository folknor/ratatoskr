use rusqlite::{Connection, params};

use super::connection::Db;

// ── Contact search types ─────────────────────────────────────

/// A contact result from the autocomplete search.
#[derive(Debug, Clone)]
pub struct ContactMatch {
    pub email: String,
    pub display_name: Option<String>,
}

/// Search contacts and seen addresses for autocomplete.
///
/// Searches the `contacts` table and `seen_addresses` table using LIKE
/// matching. Deduplicates by email (contacts take priority over seen
/// addresses). Returns up to `limit` results ordered by relevance.
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
    let mut seen_emails: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Search contacts table first (higher priority).
    // Order by frequency DESC so frequently-contacted people rank higher,
    // matching the product spec's "recency dominates" ranking model.
    let contacts_sql = "SELECT email, display_name FROM contacts
                        WHERE email LIKE ?1 OR display_name LIKE ?1
                        ORDER BY frequency DESC, display_name ASC
                        LIMIT ?2";
    let mut stmt = conn.prepare(contacts_sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![&pattern, limit], |row| {
            Ok(ContactMatch {
                email: row.get("email")?,
                display_name: row.get("display_name")?,
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

    // Search seen_addresses table (lower priority, fills remaining slots).
    // Order by last_seen_at DESC for recency.
    let remaining = limit - results.len() as i64;
    if remaining > 0 {
        let seen_sql = "SELECT email, display_name FROM seen_addresses
                        WHERE email LIKE ?1 OR display_name LIKE ?1
                        ORDER BY last_seen_at DESC
                        LIMIT ?2";
        let mut seen_stmt = conn.prepare(seen_sql).map_err(|e| e.to_string())?;
        let seen_rows = seen_stmt
            .query_map(params![&pattern, remaining], |row| {
                Ok(ContactMatch {
                    email: row.get("email")?,
                    display_name: row.get("display_name")?,
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
    }

    Ok(results)
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
    /// Load contacts for the settings management list, optionally filtered.
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
        self.with_conn(move |conn| {
            load_groups_filtered(conn, &filter)
        })
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

fn load_contacts_filtered(
    conn: &Connection,
    filter: &str,
) -> Result<Vec<ContactEntry>, String> {
    let trimmed = filter.trim();
    let pattern = format!("%{trimmed}%");

    // Always pass the pattern param; when no filter is active the WHERE clause
    // is trivially true (empty pattern = '%') so the param is harmless.
    let sql = if trimmed.is_empty() {
        "SELECT c.id, c.email, c.display_name, c.email2, c.phone,
                c.company, c.notes, c.account_id,
                a.account_color
         FROM contacts c
         LEFT JOIN accounts a ON a.id = c.account_id
         WHERE c.source != 'seen'
         ORDER BY c.frequency DESC, c.display_name ASC
         LIMIT 200"
    } else {
        "SELECT c.id, c.email, c.display_name, c.email2, c.phone,
                c.company, c.notes, c.account_id,
                a.account_color
         FROM contacts c
         LEFT JOIN accounts a ON a.id = c.account_id
         WHERE c.source != 'seen'
           AND (c.email LIKE ?1
                OR c.display_name LIKE ?1
                OR c.company LIKE ?1)
         ORDER BY c.frequency DESC, c.display_name ASC
         LIMIT 200"
    };

    let params: &[&dyn rusqlite::types::ToSql] = if trimmed.is_empty() {
        &[]
    } else {
        &[&pattern]
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params, |row| {
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
                groups: Vec::new(),
            })
        })
        .map_err(|e| e.to_string())?;

    let mut contacts: Vec<ContactEntry> = Vec::new();
    for row in rows {
        contacts.push(row.map_err(|e| e.to_string())?);
    }

    // Load group memberships for each contact.
    for contact in &mut contacts {
        contact.groups = load_contact_groups(conn, &contact.email)?;
    }
    Ok(contacts)
}

fn load_contact_groups(
    conn: &Connection,
    email: &str,
) -> Result<Vec<String>, String> {
    let mut stmt = conn
        .prepare(
            "SELECT g.name FROM contact_groups g
             INNER JOIN contact_group_members m
               ON m.group_id = g.id
             WHERE m.member_type = 'email' AND m.member_value = ?1
             ORDER BY g.name ASC",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params![email], |row| row.get::<_, String>(0))
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

    let params: &[&dyn rusqlite::types::ToSql] = if trimmed.is_empty() {
        &[]
    } else {
        &[&pattern]
    };

    let mut stmt = conn.prepare(sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(params, |row| {
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
    conn.execute(
        "INSERT INTO contacts (id, email, display_name, email2, phone,
                               company, notes, account_id, source,
                               created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 'user', ?9, ?9)
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
            "INSERT INTO contact_group_members (group_id, member_type, member_value)
             VALUES (?1, 'email', ?2)",
        )
        .map_err(|e| e.to_string())?;

    for email in member_emails {
        stmt.execute(params![group.id, email])
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}
