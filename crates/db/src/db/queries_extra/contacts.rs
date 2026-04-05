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
        let lim = limit.unwrap_or(500);
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
