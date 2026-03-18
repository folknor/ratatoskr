use super::super::DbState;
use super::super::types::{DbLocalDraft, DbScheduledEmail, DbSendAsAlias, DbSignature, DbTemplate};
use super::dynamic_update;
use crate::db::from_row::FromRow;
use crate::db::{query_as, query_one};
use rusqlite::params;

pub async fn db_get_templates_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbTemplate>, String> {
    db.with_conn(move |conn| {
        query_as::<DbTemplate>(
            conn,
            "SELECT id, account_id, name, subject, body_html, shortcut, sort_order, created_at
                 FROM templates WHERE account_id = ?1 OR account_id IS NULL
                 ORDER BY sort_order, created_at",
            &[&account_id],
        )
    })
    .await
}

pub async fn db_insert_template(
    db: &DbState,
    account_id: Option<String>,
    name: String,
    subject: Option<String>,
    body_html: String,
    shortcut: Option<String>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let ret_id = id.clone();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO templates (id, account_id, name, subject, body_html, shortcut)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, account_id, name, subject, body_html, shortcut],
        )
        .map_err(|e| e.to_string())?;
        Ok(ret_id)
    })
    .await
}

pub async fn db_update_template(
    db: &DbState,
    id: String,
    name: Option<String>,
    subject: Option<String>,
    subject_set: bool,
    body_html: Option<String>,
    shortcut: Option<String>,
    shortcut_set: bool,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
        if let Some(v) = name {
            sets.push(("name", Box::new(v)));
        }
        if subject_set {
            sets.push(("subject", Box::new(subject)));
        }
        if let Some(v) = body_html {
            sets.push(("body_html", Box::new(v)));
        }
        if shortcut_set {
            sets.push(("shortcut", Box::new(shortcut)));
        }
        dynamic_update(conn, "templates", "id", &id, sets)
    })
    .await
}

pub async fn db_delete_template(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM templates WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_signatures_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbSignature>, String> {
    db.with_conn(move |conn| {
        query_as::<DbSignature>(
            conn,
            "SELECT id, account_id, name, body_html, is_default, sort_order
                 FROM signatures WHERE account_id = ?1
                 ORDER BY sort_order, created_at",
            &[&account_id],
        )
    })
    .await
}

pub async fn db_get_default_signature(
    db: &DbState,
    account_id: String,
) -> Result<Option<DbSignature>, String> {
    db.with_conn(move |conn| {
        query_one::<DbSignature>(
            conn,
            "SELECT id, account_id, name, body_html, is_default, sort_order
                 FROM signatures WHERE account_id = ?1 AND is_default = 1 LIMIT 1",
            &[&account_id],
        )
    })
    .await
}

pub async fn db_insert_signature(
    db: &DbState,
    account_id: String,
    name: String,
    body_html: String,
    is_default: bool,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let ret_id = id.clone();
    let is_default_int = i64::from(is_default);
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        if is_default {
            tx.execute(
                "UPDATE signatures SET is_default = 0 WHERE account_id = ?1",
                params![account_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "INSERT INTO signatures (id, account_id, name, body_html, is_default)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, account_id, name, body_html, is_default_int],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(ret_id)
    })
    .await
}

pub async fn db_update_signature(
    db: &DbState,
    id: String,
    name: Option<String>,
    body_html: Option<String>,
    is_default: Option<bool>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        if is_default == Some(true) {
            let account_id: Option<String> = tx
                .query_row(
                    "SELECT account_id FROM signatures WHERE id = ?1",
                    params![id],
                    |row| row.get("account_id"),
                )
                .ok();
            if let Some(aid) = account_id {
                tx.execute(
                    "UPDATE signatures SET is_default = 0 WHERE account_id = ?1",
                    params![aid],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
        if let Some(v) = name {
            sets.push(("name", Box::new(v)));
        }
        if let Some(v) = body_html {
            sets.push(("body_html", Box::new(v)));
        }
        if let Some(v) = is_default {
            sets.push(("is_default", Box::new(i64::from(v))));
        }
        dynamic_update(&tx, "signatures", "id", &id, sets)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_signature(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM signatures WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_aliases_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbSendAsAlias>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare("SELECT * FROM send_as_aliases WHERE account_id = ?1 ORDER BY is_primary DESC, email")
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], DbSendAsAlias::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_upsert_alias(
    db: &DbState,
    account_id: String,
    email: String,
    display_name: Option<String>,
    reply_to_address: Option<String>,
    signature_id: Option<String>,
    is_primary: bool,
    is_default: bool,
    treat_as_alias: bool,
    verification_status: String,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let id_clone = id.clone();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO send_as_aliases (id, account_id, email, display_name, reply_to_address, signature_id, is_primary, is_default, treat_as_alias, verification_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(account_id, email) DO UPDATE SET
                   display_name = excluded.display_name,
                   reply_to_address = excluded.reply_to_address,
                   signature_id = excluded.signature_id,
                   is_primary = excluded.is_primary,
                   treat_as_alias = excluded.treat_as_alias,
                   verification_status = excluded.verification_status",
            params![
                id_clone,
                account_id,
                email,
                display_name,
                reply_to_address,
                signature_id,
                i64::from(is_primary),
                i64::from(is_default),
                i64::from(treat_as_alias),
                verification_status,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await?;
    Ok(id)
}

pub async fn db_get_default_alias(
    db: &DbState,
    account_id: String,
) -> Result<Option<DbSendAsAlias>, String> {
    db.with_conn(move |conn| {
        let result = conn
            .query_row(
                "SELECT * FROM send_as_aliases WHERE account_id = ?1 AND is_default = 1 LIMIT 1",
                params![account_id],
                DbSendAsAlias::from_row,
            )
            .ok();
        if result.is_some() {
            return Ok(result);
        }
        Ok(conn
            .query_row(
                "SELECT * FROM send_as_aliases WHERE account_id = ?1 AND is_primary = 1 LIMIT 1",
                params![account_id],
                DbSendAsAlias::from_row,
            )
            .ok())
    })
    .await
}

pub async fn db_set_default_alias(
    db: &DbState,
    account_id: String,
    alias_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE send_as_aliases SET is_default = 0 WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE send_as_aliases SET is_default = 1 WHERE id = ?1 AND account_id = ?2",
            params![alias_id, account_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_alias(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM send_as_aliases WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_save_local_draft(
    db: &DbState,
    id: String,
    account_id: String,
    to_addresses: Option<String>,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    subject: Option<String>,
    body_html: Option<String>,
    reply_to_message_id: Option<String>,
    thread_id: Option<String>,
    from_email: Option<String>,
    signature_id: Option<String>,
    remote_draft_id: Option<String>,
    attachments: Option<String>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO local_drafts (id, account_id, to_addresses, cc_addresses, bcc_addresses, subject, body_html, reply_to_message_id, thread_id, from_email, signature_id, remote_draft_id, attachments, updated_at, sync_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, unixepoch(), 'pending')
                 ON CONFLICT(id) DO UPDATE SET
                   to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5,
                   subject = ?6, body_html = ?7, reply_to_message_id = ?8,
                   thread_id = ?9, from_email = ?10, signature_id = ?11,
                   remote_draft_id = ?12, attachments = ?13,
                   updated_at = unixepoch(), sync_status = 'pending'",
            params![
                id,
                account_id,
                to_addresses,
                cc_addresses,
                bcc_addresses,
                subject,
                body_html,
                reply_to_message_id,
                thread_id,
                from_email,
                signature_id,
                remote_draft_id,
                attachments,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_local_draft(db: &DbState, id: String) -> Result<Option<DbLocalDraft>, String> {
    db.with_conn(move |conn| {
        query_one::<DbLocalDraft>(
            conn,
            "SELECT * FROM local_drafts WHERE id = ?1",
            &[&id],
        )
    })
    .await
}

pub async fn db_get_unsynced_drafts(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbLocalDraft>, String> {
    db.with_conn(move |conn| {
        query_as::<DbLocalDraft>(
            conn,
            "SELECT * FROM local_drafts WHERE account_id = ?1 AND sync_status = 'pending' ORDER BY updated_at ASC",
            &[&account_id],
        )
    })
    .await
}

pub async fn db_mark_draft_synced(
    db: &DbState,
    id: String,
    remote_draft_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE local_drafts SET sync_status = 'synced', remote_draft_id = ?1 WHERE id = ?2",
            params![remote_draft_id, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_local_draft(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM local_drafts WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_pending_scheduled_emails(
    db: &DbState,
    now: i64,
) -> Result<Vec<DbScheduledEmail>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM scheduled_emails WHERE status = 'pending' AND scheduled_at <= ?1 ORDER BY scheduled_at ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![now], DbScheduledEmail::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_get_scheduled_emails_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbScheduledEmail>, String> {
    db.with_conn(move |conn| {
        let mut stmt = conn
            .prepare(
                "SELECT * FROM scheduled_emails WHERE account_id = ?1 AND status = 'pending' ORDER BY scheduled_at ASC",
            )
            .map_err(|e| e.to_string())?;
        stmt.query_map(params![account_id], DbScheduledEmail::from_row)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
    })
    .await
}

pub async fn db_insert_scheduled_email(
    db: &DbState,
    account_id: String,
    to_addresses: String,
    cc_addresses: Option<String>,
    bcc_addresses: Option<String>,
    subject: Option<String>,
    body_html: String,
    reply_to_message_id: Option<String>,
    thread_id: Option<String>,
    scheduled_at: i64,
    signature_id: Option<String>,
    delegation: String,
    from_email: Option<String>,
    timezone: Option<String>,
) -> Result<String, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let id_clone = id.clone();
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO scheduled_emails (id, account_id, to_addresses, cc_addresses, bcc_addresses, subject, body_html, reply_to_message_id, thread_id, scheduled_at, signature_id, delegation, from_email, timezone)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            params![
                id_clone,
                account_id,
                to_addresses,
                cc_addresses,
                bcc_addresses,
                subject,
                body_html,
                reply_to_message_id,
                thread_id,
                scheduled_at,
                signature_id,
                delegation,
                from_email,
                timezone,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await?;
    Ok(id)
}

pub async fn db_update_scheduled_email_status(
    db: &DbState,
    id: String,
    status: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "UPDATE scheduled_emails SET status = ?1 WHERE id = ?2",
            params![status, id],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_scheduled_email(db: &DbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM scheduled_emails WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}
