use super::super::DbState;
use super::super::queries::row_to_contact;
use super::super::types::{
    ContactAttachmentRow, ContactStats, DbContact, RecentThread, SameDomainContact,
};
use rusqlite::params;

pub async fn db_get_all_contacts(
    db: &DbState,
    limit: Option<i64>,
    offset: Option<i64>,
) -> Result<Vec<DbContact>, String> {
    db.with_conn(move |conn| {
        let lim = limit.unwrap_or(500);
        let off = offset.unwrap_or(0);
        let mut stmt = conn
            .prepare(
                "SELECT * FROM contacts ORDER BY frequency DESC, display_name ASC LIMIT ?1 OFFSET ?2",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![lim, off], row_to_contact)
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
               display_name_overridden = CASE WHEN source = 'graph' THEN 1 ELSE display_name_overridden END, \
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

pub async fn db_delete_contact(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM contacts WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_contact_stats(db: &DbState, email: String) -> Result<ContactStats, String> {
    db.with_conn(move |conn| {
        let normalized = email.to_lowercase();
        conn.query_row(
            "SELECT COUNT(*) as cnt, MIN(date) as first_date, MAX(date) as last_date
                 FROM messages WHERE from_address = ?1",
            params![normalized],
            |row| {
                Ok(ContactStats {
                    email_count: row.get(0)?,
                    first_email: row.get(1)?,
                    last_email: row.get(2)?,
                })
            },
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
        stmt.query_map(params![domain, normalized, lim], |row| {
            Ok(SameDomainContact {
                email: row.get(0)?,
                display_name: row.get(1)?,
                avatar_url: row.get(2)?,
            })
        })
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
                |row| row.get::<_, String>(0),
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
        stmt.query_map(params![normalized, lim], |row| {
            Ok(RecentThread {
                thread_id: row.get(0)?,
                subject: row.get(1)?,
                last_message_at: row.get(2)?,
            })
        })
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
        stmt.query_map(params![normalized, lim], |row| {
            Ok(ContactAttachmentRow {
                filename: row.get(0)?,
                mime_type: row.get(1)?,
                size: row.get(2)?,
                date: row.get(3)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())
    })
    .await
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
