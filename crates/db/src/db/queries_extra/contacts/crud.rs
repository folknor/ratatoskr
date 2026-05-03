use crate::db::ReadDbState;
use crate::db::FromRow;
use crate::db::types::DbContact;
use rusqlite::{Connection, OptionalExtension, params};

#[derive(Debug, Clone)]
pub struct ExpandedGroupContact {
    pub email: String,
    pub display_name: Option<String>,
}

pub async fn db_get_all_contacts(
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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

/// Mutable fields written by `db_upsert_contact_full`.
#[derive(Debug, Clone, Copy)]
pub struct UpsertContactParams<'a> {
    pub id: &'a str,
    pub email: &'a str,
    pub display_name: Option<&'a str>,
    pub email2: Option<&'a str>,
    pub phone: Option<&'a str>,
    pub company: Option<&'a str>,
    pub notes: Option<&'a str>,
    pub account_id: Option<&'a str>,
    pub source: &'a str,
}

/// Upsert a contact with all mutable fields.
///
/// Used by the contact action service. The app-crate `save_contact_inner`
/// has equivalent SQL - this is the core-accessible version.
pub fn db_upsert_contact_full(
    conn: &Connection,
    input: UpsertContactParams<'_>,
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
            input.id,
            input.email,
            input.display_name,
            input.email2,
            input.phone,
            input.company,
            input.notes,
            input.account_id,
            input.source,
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn db_delete_contact(db: &ReadDbState, id: String) -> Result<(), String> {
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

pub async fn db_update_contact_avatar(
    db: &ReadDbState,
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
