use rusqlite::params;

use crate::db::{ReadConn, WriteTarget};

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
    conn: &ReadConn<'_>,
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
    conn: &impl WriteTarget,
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
pub fn delete_contact_sync(conn: &impl WriteTarget, id: &str) -> Result<(), String> {
    conn.execute("DELETE FROM contacts WHERE id = ?1", params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
