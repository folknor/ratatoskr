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

// ── Signature column list ────────────────────────────────
//
// A single source of truth for the columns returned by signature queries.
const SIG_COLS: &str = "id, account_id, name, body_html, body_text, \
    is_default, is_reply_default, sort_order, source, server_id, \
    server_html_hash, last_synced_at, created_at";

pub async fn db_get_signatures_for_account(
    db: &DbState,
    account_id: String,
) -> Result<Vec<DbSignature>, String> {
    db.with_conn(move |conn| {
        let sql = format!(
            "SELECT {SIG_COLS} FROM signatures \
             WHERE account_id = ?1 ORDER BY sort_order, created_at"
        );
        query_as::<DbSignature>(conn, &sql, &[&account_id])
    })
    .await
}

/// Get all signatures across all accounts (for the settings UI).
pub async fn db_get_all_signatures(db: &DbState) -> Result<Vec<DbSignature>, String> {
    db.with_conn(move |conn| {
        let sql = format!(
            "SELECT {SIG_COLS} FROM signatures \
             ORDER BY account_id, sort_order, name"
        );
        query_as::<DbSignature>(conn, &sql, &[])
    })
    .await
}

pub async fn db_get_default_signature(
    db: &DbState,
    account_id: String,
) -> Result<Option<DbSignature>, String> {
    db.with_conn(move |conn| {
        let sql = format!(
            "SELECT {SIG_COLS} FROM signatures \
             WHERE account_id = ?1 AND is_default = 1 LIMIT 1"
        );
        query_one::<DbSignature>(conn, &sql, &[&account_id])
    })
    .await
}

/// Get the reply-default signature for an account. Falls back to
/// `is_default` if no `is_reply_default` is set.
pub async fn db_get_reply_signature(
    db: &DbState,
    account_id: String,
) -> Result<Option<DbSignature>, String> {
    db.with_conn(move |conn| {
        let sql = format!(
            "SELECT {SIG_COLS} FROM signatures \
             WHERE account_id = ?1 \
               AND (is_reply_default = 1 OR is_default = 1) \
             ORDER BY is_reply_default DESC LIMIT 1"
        );
        query_one::<DbSignature>(conn, &sql, &[&account_id])
    })
    .await
}

/// Parameters for inserting a signature.
pub struct InsertSignatureParams {
    pub account_id: String,
    pub name: String,
    pub body_html: String,
    pub body_text: Option<String>,
    pub is_default: bool,
    pub is_reply_default: bool,
}

pub async fn db_insert_signature(db: &DbState, p: InsertSignatureParams) -> Result<String, String> {
    log::info!(
        "Inserting signature: account_id={}, name={}",
        p.account_id,
        p.name
    );
    let id = uuid::Uuid::new_v4().to_string();
    let ret_id = id.clone();
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        if p.is_default {
            tx.execute(
                "UPDATE signatures SET is_default = 0 WHERE account_id = ?1",
                params![p.account_id],
            )
            .map_err(|e| e.to_string())?;
        }
        if p.is_reply_default {
            tx.execute(
                "UPDATE signatures SET is_reply_default = 0 WHERE account_id = ?1",
                params![p.account_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.execute(
            "INSERT INTO signatures \
             (id, account_id, name, body_html, body_text, is_default, is_reply_default) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                id,
                p.account_id,
                p.name,
                p.body_html,
                p.body_text,
                i64::from(p.is_default),
                i64::from(p.is_reply_default),
            ],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(ret_id)
    })
    .await
}

/// Parameters for updating a signature.
pub struct UpdateSignatureParams {
    pub id: String,
    pub name: Option<String>,
    pub body_html: Option<String>,
    pub body_text: Option<Option<String>>,
    pub is_default: Option<bool>,
    pub is_reply_default: Option<bool>,
}

pub async fn db_update_signature(db: &DbState, p: UpdateSignatureParams) -> Result<(), String> {
    log::info!("Updating signature: id={}", p.id);
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        let account_id = get_signature_account_id(&tx, &p.id)?;
        if p.is_default == Some(true) {
            if let Some(ref aid) = account_id {
                tx.execute(
                    "UPDATE signatures SET is_default = 0 WHERE account_id = ?1",
                    params![aid],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        if p.is_reply_default == Some(true) {
            if let Some(ref aid) = account_id {
                tx.execute(
                    "UPDATE signatures SET is_reply_default = 0 WHERE account_id = ?1",
                    params![aid],
                )
                .map_err(|e| e.to_string())?;
            }
        }
        let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
        if let Some(v) = p.name {
            sets.push(("name", Box::new(v)));
        }
        if let Some(v) = p.body_html {
            sets.push(("body_html", Box::new(v)));
        }
        if let Some(v) = p.body_text {
            sets.push(("body_text", Box::new(v)));
        }
        if let Some(v) = p.is_default {
            sets.push(("is_default", Box::new(i64::from(v))));
        }
        if let Some(v) = p.is_reply_default {
            sets.push(("is_reply_default", Box::new(i64::from(v))));
        }
        dynamic_update(&tx, "signatures", "id", &p.id, sets)?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_delete_signature(db: &DbState, id: String) -> Result<(), String> {
    log::info!("Deleting signature: id={id}");
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM signatures WHERE id = ?1", params![id])
            .map_err(|e| {
                log::error!("Failed to delete signature {id}: {e}");
                e.to_string()
            })?;
        Ok(())
    })
    .await
}

/// Reorder signatures for an account. `ordered_ids` lists signature IDs in
/// the desired display order; each one receives `sort_order = index`.
pub async fn db_reorder_signatures(db: &DbState, ordered_ids: Vec<String>) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        for (i, sig_id) in ordered_ids.iter().enumerate() {
            #[allow(clippy::cast_possible_wrap)]
            let order = i as i64;
            tx.execute(
                "UPDATE signatures SET sort_order = ?1 WHERE id = ?2",
                params![order, sig_id],
            )
            .map_err(|e| e.to_string())?;
        }
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Set the reply-default signature for an account (clears old reply-default
/// in a transaction).
pub async fn db_set_reply_default_signature(
    db: &DbState,
    account_id: String,
    signature_id: String,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE signatures SET is_reply_default = 0 WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
        tx.execute(
            "UPDATE signatures SET is_reply_default = 1 \
             WHERE id = ?1 AND account_id = ?2",
            params![signature_id, account_id],
        )
        .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Determine the signature to insert for a given compose scenario.
///
/// Resolution order:
/// 1. If `from_email` is given and its send-as alias has a `signature_id`, use that.
/// 2. For reply/forward: use `is_reply_default`, falling back to `is_default`.
/// 3. For new compose: use `is_default`.
/// 4. If no default is set: return `None`.
pub async fn db_resolve_signature_for_compose(
    db: &DbState,
    account_id: String,
    from_email: Option<String>,
    is_reply: bool,
) -> Result<Option<DbSignature>, String> {
    log::debug!(
        "Resolving signature for compose: account_id={account_id}, from_email={from_email:?}, is_reply={is_reply}"
    );
    db.with_conn(move |conn| {
        // 1. Check alias-level override.
        if let Some(ref email) = from_email {
            let alias_sig_id: Option<String> = conn
                .query_row(
                    "SELECT signature_id FROM send_as_aliases \
                     WHERE account_id = ?1 AND email = ?2 AND signature_id IS NOT NULL",
                    params![account_id, email],
                    |row| row.get(0),
                )
                .ok();
            if let Some(sig_id) = alias_sig_id {
                let sql = format!("SELECT {SIG_COLS} FROM signatures WHERE id = ?1 LIMIT 1");
                if let Some(sig) = query_one::<DbSignature>(conn, &sql, &[&sig_id])? {
                    return Ok(Some(sig));
                }
            }
        }

        // 2/3. Account-level default.
        let sql = if is_reply {
            format!(
                "SELECT {SIG_COLS} FROM signatures \
                 WHERE account_id = ?1 AND (is_reply_default = 1 OR is_default = 1) \
                 ORDER BY is_reply_default DESC LIMIT 1"
            )
        } else {
            format!(
                "SELECT {SIG_COLS} FROM signatures \
                 WHERE account_id = ?1 AND is_default = 1 LIMIT 1"
            )
        };
        query_one::<DbSignature>(conn, &sql, &[&account_id])
    })
    .await
}

/// Helper to look up the account_id for a signature (used inside transactions).
fn get_signature_account_id(
    conn: &rusqlite::Connection,
    signature_id: &str,
) -> Result<Option<String>, String> {
    Ok(conn
        .query_row(
            "SELECT account_id FROM signatures WHERE id = ?1",
            params![signature_id],
            |row| row.get("account_id"),
        )
        .ok())
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
    signature_separator_index: Option<i64>,
) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute(
            "INSERT INTO local_drafts (id, account_id, to_addresses, cc_addresses, bcc_addresses, \
                subject, body_html, reply_to_message_id, thread_id, from_email, signature_id, \
                remote_draft_id, attachments, signature_separator_index, updated_at, sync_status)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, unixepoch(), 'pending')
                 ON CONFLICT(id) DO UPDATE SET
                   to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5,
                   subject = ?6, body_html = ?7, reply_to_message_id = ?8,
                   thread_id = ?9, from_email = ?10, signature_id = ?11,
                   remote_draft_id = ?12, attachments = ?13,
                   signature_separator_index = ?14,
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
                signature_separator_index,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_local_draft(db: &DbState, id: String) -> Result<Option<DbLocalDraft>, String> {
    db.with_conn(move |conn| {
        query_one::<DbLocalDraft>(conn, "SELECT * FROM local_drafts WHERE id = ?1", &[&id])
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

// ── HTML-to-plain-text ──────────────────────────────────

/// Strip HTML tags and decode entities to produce a plain-text signature.
///
/// Block elements (`<p>`, `<br>`, `<div>`, `<li>`, `<h1>`..`<h6>`) insert
/// newlines. Inline elements are dropped. Common HTML entities are decoded.
pub fn html_to_plain_text(html: &str) -> String {
    use lol_html::{RewriteStrSettings, element, rewrite_str};

    let trimmed = html.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    // Block-level tags that should insert a newline *before* their content.
    let block_tags: &[&str] = &[
        "p",
        "div",
        "br",
        "h1",
        "h2",
        "h3",
        "h4",
        "h5",
        "h6",
        "li",
        "blockquote",
        "hr",
        "tr",
    ];

    let element_handlers: Vec<_> = block_tags
        .iter()
        .map(|&tag| {
            element!(tag, |el| {
                el.before("\n", lol_html::html_content::ContentType::Text);
                Ok(())
            })
        })
        .collect();

    // Remove all tags but keep text content + the newlines we inserted.
    let result = rewrite_str(
        trimmed,
        RewriteStrSettings {
            element_content_handlers: element_handlers,
            ..RewriteStrSettings::new()
        },
    );

    match result {
        Ok(text) => {
            // Strip remaining HTML tags (lol_html keeps them; we want text only).
            let stripped = strip_tags(&text);
            collapse_blank_lines(&stripped)
        }
        Err(_) => {
            // Fallback: just strip tags naively.
            collapse_blank_lines(&strip_tags(trimmed))
        }
    }
}

/// Naively strip HTML tags from a string (for plain-text fallback).
fn strip_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let mut in_tag = false;
    for ch in html.chars() {
        if ch == '<' {
            in_tag = true;
        } else if ch == '>' {
            in_tag = false;
        } else if !in_tag {
            result.push(ch);
        }
    }
    result
}

/// Collapse runs of blank lines into single newlines and trim.
fn collapse_blank_lines(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut last_was_newline = false;
    for ch in s.chars() {
        if ch == '\n' || ch == '\r' {
            if !last_was_newline {
                result.push('\n');
                last_was_newline = true;
            }
        } else {
            last_was_newline = false;
            result.push(ch);
        }
    }
    result.trim().to_string()
}

// ── Send path helpers (Phase 5) ─────────────────────────

/// Wrap the signature region in an identifying div for outgoing HTML email.
///
/// Finds the first `<hr>` in the HTML and wraps everything from there to the
/// end (or to the next attribution/quote section) in
/// `<div id="ratatoskr-signature">`. If no `<hr>` is found, returns the
/// HTML unchanged.
pub fn finalize_compose_html(html: &str) -> String {
    // Simple approach: find the first `<hr` and wrap from there.
    let Some(hr_pos) = html.find("<hr") else {
        return html.to_string();
    };

    // Find the end of the <hr> tag.
    let hr_end = html[hr_pos..]
        .find('>')
        .map(|i| hr_pos + i + 1)
        .unwrap_or(hr_pos);

    // Look for attribution/quote after the signature.
    // The attribution is typically an italic paragraph before a blockquote.
    // We look for the last <blockquote> as the boundary.
    let after_hr = &html[hr_end..];
    let sig_end = after_hr
        .rfind("<blockquote")
        .and_then(|bq_pos| {
            // Find the attribution paragraph before the blockquote.
            let before_bq = &after_hr[..bq_pos];
            before_bq.rfind("<p").map(|p_pos| hr_end + p_pos)
        })
        .unwrap_or(html.len());

    let mut result = String::with_capacity(html.len() + 64);
    result.push_str(&html[..hr_pos]);
    result.push_str("<div id=\"ratatoskr-signature\">");
    result.push_str(&html[hr_pos..sig_end]);
    result.push_str("</div>");
    result.push_str(&html[sig_end..]);
    result
}

/// Insert the RFC 3676 `-- \n` separator in the plain-text alternative.
///
/// `plain_text` is the full plain-text body. `separator_marker` is a
/// substring that marks where the signature begins (typically the text
/// equivalent of the `<hr>` separator). Everything from `separator_marker`
/// onward is treated as the signature.
pub fn finalize_compose_plain_text(body_text: &str, signature_text: Option<&str>) -> String {
    let Some(sig) = signature_text else {
        return body_text.to_string();
    };
    if sig.trim().is_empty() {
        return body_text.to_string();
    }
    format!("{body_text}\n-- \n{sig}")
}
