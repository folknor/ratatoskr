use super::super::ReadDbState;
use super::super::types::{DbLocalDraft, DbScheduledEmail, DbSendAsAlias, DbSignature, DbTemplate};
use super::dynamic_update;
use crate::db::from_row::FromRow;
use crate::db::{query_as, query_one};
use rusqlite::params;

pub async fn db_get_templates_for_account(
    db: &ReadDbState,
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
    db: &ReadDbState,
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

// TODO(refactor): wrap fields in an UpdateTemplateParams struct.
#[allow(clippy::too_many_arguments)]
pub async fn db_update_template(
    db: &ReadDbState,
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

pub async fn db_delete_template(db: &ReadDbState, id: String) -> Result<(), String> {
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
    db: &ReadDbState,
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
pub async fn db_get_all_signatures(db: &ReadDbState) -> Result<Vec<DbSignature>, String> {
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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

/// Insert one row into `signatures` and return the new id. Inside a
/// single transaction the helper also clears `is_default` /
/// `is_reply_default` on every other signature for the same account
/// when the new row claims either flag, so the per-account "exactly
/// one default" invariant is preserved without UI-side care.
///
/// Phase 6a: callable from the Service-side `signature.create`
/// handler via `WriteDbState::with_conn` - the synchronous shape lets
/// the handler hold the connection across the transaction without an
/// async wrapper.
pub fn db_insert_signature_sync(
    conn: &rusqlite::Connection,
    p: &InsertSignatureParams,
) -> Result<String, String> {
    log::info!(
        "Inserting signature: account_id={}, name={}",
        p.account_id,
        p.name
    );
    let id = uuid::Uuid::new_v4().to_string();
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
    Ok(id)
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

/// Partial-update a signature row by id. Each `Option` field on
/// `UpdateSignatureParams` is "no change" if `None`, else "set to
/// value." Setting `is_default` / `is_reply_default` to `Some(true)`
/// also clears the same flag on every other signature for the same
/// account inside the transaction.
///
/// Phase 6a: paired sync version of the prior async function so the
/// `signature.update` handler can run inside `WriteDbState::with_conn`.
pub fn db_update_signature_sync(
    conn: &rusqlite::Connection,
    p: UpdateSignatureParams,
) -> Result<(), String> {
    log::info!("Updating signature: id={}", p.id);
    let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;
    let account_id = get_signature_account_id(&tx, &p.id)?;
    if p.is_default == Some(true)
        && let Some(ref aid) = account_id
    {
        tx.execute(
            "UPDATE signatures SET is_default = 0 WHERE account_id = ?1",
            params![aid],
        )
        .map_err(|e| e.to_string())?;
    }
    if p.is_reply_default == Some(true)
        && let Some(ref aid) = account_id
    {
        tx.execute(
            "UPDATE signatures SET is_reply_default = 0 WHERE account_id = ?1",
            params![aid],
        )
        .map_err(|e| e.to_string())?;
    }
    let UpdateSignatureParams {
        id,
        name,
        body_html,
        body_text,
        is_default,
        is_reply_default,
    } = p;
    let mut sets: Vec<(&str, Box<dyn rusqlite::types::ToSql>)> = Vec::new();
    if let Some(v) = name {
        sets.push(("name", Box::new(v)));
    }
    if let Some(v) = body_html {
        sets.push(("body_html", Box::new(v)));
    }
    if let Some(v) = body_text {
        sets.push(("body_text", Box::new(v)));
    }
    if let Some(v) = is_default {
        sets.push(("is_default", Box::new(i64::from(v))));
    }
    if let Some(v) = is_reply_default {
        sets.push(("is_reply_default", Box::new(i64::from(v))));
    }
    dynamic_update(&tx, "signatures", "id", &id, sets)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(())
}

/// Delete a signature row by id. Idempotent: delete-of-missing
/// returns `Ok` (rusqlite's `execute` reports rows-affected but does
/// not error on zero matches), so callers do not need to pre-check.
pub fn db_delete_signature_sync(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<(), String> {
    log::info!("Deleting signature: id={id}");
    conn.execute("DELETE FROM signatures WHERE id = ?1", params![id])
        .map_err(|e| {
            log::error!("Failed to delete signature {id}: {e}");
            e.to_string()
        })?;
    Ok(())
}

/// Reorder signatures by assigning `sort_order = index_in_list` for
/// each id in `ordered_ids`. Ids absent from the list keep their
/// existing `sort_order`, so callers reordering a per-account subset
/// only need to pass that account's ids.
pub fn db_reorder_signatures_sync(
    conn: &rusqlite::Connection,
    ordered_ids: &[String],
) -> Result<(), String> {
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
}

/// Set the reply-default signature for an account (clears old reply-default
/// in a transaction).
pub async fn db_set_reply_default_signature(
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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

// TODO(refactor): wrap fields in an UpsertAliasParams struct.
#[allow(clippy::too_many_arguments)]
pub async fn db_upsert_alias(
    db: &ReadDbState,
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
    db: &ReadDbState,
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
    db: &ReadDbState,
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

pub async fn db_delete_alias(db: &ReadDbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM send_as_aliases WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

/// Mutable fields written by the local-draft upsert.
///
/// Used by both the async `db_save_local_draft` and the sync
/// `db_save_local_draft_sync`. Owned strings so the value can move into the
/// `with_conn` closure on the async path.
#[derive(Debug, Clone)]
pub struct SaveLocalDraftParams {
    pub id: String,
    pub account_id: String,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub bcc_addresses: Option<String>,
    pub subject: Option<String>,
    pub body_html: Option<String>,
    pub reply_to_message_id: Option<String>,
    pub thread_id: Option<String>,
    pub from_email: Option<String>,
    pub signature_id: Option<String>,
    pub remote_draft_id: Option<String>,
    pub attachments: Option<String>,
    pub signature_separator_index: Option<i64>,
}

const SAVE_LOCAL_DRAFT_SQL: &str = "INSERT INTO local_drafts (id, account_id, to_addresses, cc_addresses, bcc_addresses, \
    subject, body_html, reply_to_message_id, thread_id, from_email, signature_id, \
    remote_draft_id, attachments, signature_separator_index, updated_at, sync_status)
     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, unixepoch(), 'pending')
     ON CONFLICT(id) DO UPDATE SET
       to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5,
       subject = ?6, body_html = ?7, reply_to_message_id = ?8,
       thread_id = ?9, from_email = ?10, signature_id = ?11,
       remote_draft_id = ?12, attachments = ?13,
       signature_separator_index = ?14,
       updated_at = unixepoch(), sync_status = 'pending'";

fn exec_save_local_draft(
    conn: &crate::db::Connection,
    p: &SaveLocalDraftParams,
) -> Result<(), String> {
    conn.execute(
        SAVE_LOCAL_DRAFT_SQL,
        params![
            p.id,
            p.account_id,
            p.to_addresses,
            p.cc_addresses,
            p.bcc_addresses,
            p.subject,
            p.body_html,
            p.reply_to_message_id,
            p.thread_id,
            p.from_email,
            p.signature_id,
            p.remote_draft_id,
            p.attachments,
            p.signature_separator_index,
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}

pub async fn db_save_local_draft(
    db: &ReadDbState,
    params: SaveLocalDraftParams,
) -> Result<(), String> {
    db.with_conn(move |conn| exec_save_local_draft(conn, &params))
        .await
}

pub fn db_save_local_draft_sync(
    conn: &crate::db::Connection,
    params: &SaveLocalDraftParams,
) -> Result<(), String> {
    exec_save_local_draft(conn, params)
}

pub fn db_mark_queued_drafts_failed_sync(conn: &crate::db::Connection) -> Result<usize, String> {
    conn.execute(
        "UPDATE local_drafts SET sync_status = 'failed' WHERE sync_status = 'queued'",
        [],
    )
    .map_err(|e| e.to_string())
}

pub async fn db_get_local_draft(db: &ReadDbState, id: String) -> Result<Option<DbLocalDraft>, String> {
    db.with_conn(move |conn| {
        query_one::<DbLocalDraft>(conn, "SELECT * FROM local_drafts WHERE id = ?1", &[&id])
    })
    .await
}

pub async fn db_get_unsynced_drafts(
    db: &ReadDbState,
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
    db: &ReadDbState,
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

pub async fn db_delete_local_draft(db: &ReadDbState, id: String) -> Result<(), String> {
    db.with_conn(move |conn| {
        conn.execute("DELETE FROM local_drafts WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })
    .await
}

pub async fn db_get_pending_scheduled_emails(
    db: &ReadDbState,
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
    db: &ReadDbState,
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

// TODO(refactor): 14 args - migrate to a ScheduledEmailParams struct.
#[allow(clippy::too_many_arguments)]
pub async fn db_insert_scheduled_email(
    db: &ReadDbState,
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
    db: &ReadDbState,
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

pub async fn db_delete_scheduled_email(db: &ReadDbState, id: String) -> Result<(), String> {
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

#[cfg(test)]
mod sync_signature_tests {
    use super::*;
    use crate::db::migrations;
    use rusqlite::Connection;

    fn setup_db() -> Connection {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .expect("pragmas");
        migrations::run_all(&conn).expect("migrations");
        // Two accounts so the cross-account "default" guard is
        // exercised.
        conn.execute(
            "INSERT INTO accounts (id, email) VALUES \
             ('acc-1', 'acc-1@example.com'), \
             ('acc-2', 'acc-2@example.com')",
            [],
        )
        .expect("seed accounts");
        conn
    }

    fn count_default_for(conn: &Connection, account_id: &str, col: &str) -> i64 {
        let sql = format!(
            "SELECT COUNT(*) FROM signatures WHERE account_id = ?1 AND {col} = 1"
        );
        conn.query_row(&sql, params![account_id], |row| row.get(0))
            .expect("count default")
    }

    fn get_sort_order(conn: &Connection, sig_id: &str) -> i64 {
        conn.query_row(
            "SELECT sort_order FROM signatures WHERE id = ?1",
            params![sig_id],
            |row| row.get(0),
        )
        .expect("sort_order")
    }

    fn get_text(conn: &Connection, sig_id: &str, col: &str) -> Option<String> {
        let sql = format!("SELECT {col} FROM signatures WHERE id = ?1");
        conn.query_row(&sql, params![sig_id], |row| row.get(0))
            .expect("get text col")
    }

    #[test]
    fn insert_returns_id_and_persists_row() {
        let conn = setup_db();
        let p = InsertSignatureParams {
            account_id: "acc-1".into(),
            name: "Work".into(),
            body_html: "<p>Best</p>".into(),
            body_text: Some("Best".into()),
            is_default: false,
            is_reply_default: false,
        };
        let id = db_insert_signature_sync(&conn, &p).expect("insert");
        assert!(!id.is_empty(), "must return a uuid");
        let name: String = conn
            .query_row(
                "SELECT name FROM signatures WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("row exists");
        assert_eq!(name, "Work");
    }

    #[test]
    fn insert_with_is_default_clears_other_defaults_in_same_account() {
        let conn = setup_db();
        // First sig: default for acc-1.
        let first = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-1".into(),
                name: "First".into(),
                body_html: "<p>1</p>".into(),
                body_text: None,
                is_default: true,
                is_reply_default: false,
            },
        )
        .expect("insert first");
        // Second sig: also default. Should flip first row.
        let _second = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-1".into(),
                name: "Second".into(),
                body_html: "<p>2</p>".into(),
                body_text: None,
                is_default: true,
                is_reply_default: false,
            },
        )
        .expect("insert second");
        assert_eq!(
            count_default_for(&conn, "acc-1", "is_default"),
            1,
            "exactly one default per account"
        );
        let first_default: i64 = conn
            .query_row(
                "SELECT is_default FROM signatures WHERE id = ?1",
                params![first],
                |row| row.get(0),
            )
            .expect("first row");
        assert_eq!(first_default, 0, "first row was demoted to non-default");
    }

    #[test]
    fn insert_does_not_clear_default_in_different_account() {
        let conn = setup_db();
        let acc1 = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-1".into(),
                name: "A1".into(),
                body_html: "<p>1</p>".into(),
                body_text: None,
                is_default: true,
                is_reply_default: false,
            },
        )
        .expect("insert acc-1 default");
        let _acc2 = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-2".into(),
                name: "A2".into(),
                body_html: "<p>2</p>".into(),
                body_text: None,
                is_default: true,
                is_reply_default: false,
            },
        )
        .expect("insert acc-2 default");
        let acc1_default: i64 = conn
            .query_row(
                "SELECT is_default FROM signatures WHERE id = ?1",
                params![acc1],
                |row| row.get(0),
            )
            .expect("acc-1 row");
        assert_eq!(
            acc1_default, 1,
            "acc-1's default must survive a different account's default insert"
        );
    }

    #[test]
    fn update_partial_only_changes_named_fields() {
        let conn = setup_db();
        let id = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-1".into(),
                name: "Old".into(),
                body_html: "<p>old</p>".into(),
                body_text: Some("old".into()),
                is_default: true,
                is_reply_default: false,
            },
        )
        .expect("insert");

        // Update only the name.
        db_update_signature_sync(
            &conn,
            UpdateSignatureParams {
                id: id.clone(),
                name: Some("New".into()),
                body_html: None,
                body_text: None,
                is_default: None,
                is_reply_default: None,
            },
        )
        .expect("update");

        assert_eq!(get_text(&conn, &id, "name"), Some("New".into()));
        assert_eq!(get_text(&conn, &id, "body_html"), Some("<p>old</p>".into()));
        assert_eq!(get_text(&conn, &id, "body_text"), Some("old".into()));
        assert_eq!(
            count_default_for(&conn, "acc-1", "is_default"),
            1,
            "is_default must survive a name-only update"
        );
    }

    #[test]
    fn update_promotes_to_default_clears_others() {
        let conn = setup_db();
        let _first = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-1".into(),
                name: "First".into(),
                body_html: "<p>1</p>".into(),
                body_text: None,
                is_default: true,
                is_reply_default: false,
            },
        )
        .expect("first");
        let second = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-1".into(),
                name: "Second".into(),
                body_html: "<p>2</p>".into(),
                body_text: None,
                is_default: false,
                is_reply_default: false,
            },
        )
        .expect("second");

        db_update_signature_sync(
            &conn,
            UpdateSignatureParams {
                id: second.clone(),
                name: None,
                body_html: None,
                body_text: None,
                is_default: Some(true),
                is_reply_default: None,
            },
        )
        .expect("promote second");

        assert_eq!(
            count_default_for(&conn, "acc-1", "is_default"),
            1,
            "still exactly one default after promotion"
        );
        let second_default: i64 = conn
            .query_row(
                "SELECT is_default FROM signatures WHERE id = ?1",
                params![second],
                |row| row.get(0),
            )
            .expect("second row");
        assert_eq!(second_default, 1, "second is now the default");
    }

    #[test]
    fn delete_removes_row_and_is_idempotent() {
        let conn = setup_db();
        let id = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-1".into(),
                name: "X".into(),
                body_html: "<p>x</p>".into(),
                body_text: None,
                is_default: false,
                is_reply_default: false,
            },
        )
        .expect("insert");

        db_delete_signature_sync(&conn, &id).expect("delete");
        let n: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM signatures WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .expect("count");
        assert_eq!(n, 0);

        // Idempotent: delete-of-missing is Ok.
        db_delete_signature_sync(&conn, &id).expect("idempotent delete");
    }

    #[test]
    fn reorder_assigns_indices_in_order() {
        let conn = setup_db();
        let mut ids = Vec::new();
        for name in ["A", "B", "C"] {
            let id = db_insert_signature_sync(
                &conn,
                &InsertSignatureParams {
                    account_id: "acc-1".into(),
                    name: name.into(),
                    body_html: "<p>x</p>".into(),
                    body_text: None,
                    is_default: false,
                    is_reply_default: false,
                },
            )
            .expect("insert");
            ids.push(id);
        }
        // Reorder to C, A, B.
        let new_order = vec![ids[2].clone(), ids[0].clone(), ids[1].clone()];
        db_reorder_signatures_sync(&conn, &new_order).expect("reorder");

        assert_eq!(get_sort_order(&conn, &ids[2]), 0);
        assert_eq!(get_sort_order(&conn, &ids[0]), 1);
        assert_eq!(get_sort_order(&conn, &ids[1]), 2);
    }

    #[test]
    fn reorder_leaves_absent_ids_untouched() {
        let conn = setup_db();
        let acc1 = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-1".into(),
                name: "A1".into(),
                body_html: "<p>x</p>".into(),
                body_text: None,
                is_default: false,
                is_reply_default: false,
            },
        )
        .expect("insert acc-1");
        let acc2 = db_insert_signature_sync(
            &conn,
            &InsertSignatureParams {
                account_id: "acc-2".into(),
                name: "A2".into(),
                body_html: "<p>y</p>".into(),
                body_text: None,
                is_default: false,
                is_reply_default: false,
            },
        )
        .expect("insert acc-2");

        // Set acc-2's sort_order to a known sentinel; reorder only
        // acc-1's id and assert acc-2 is untouched.
        conn.execute(
            "UPDATE signatures SET sort_order = 99 WHERE id = ?1",
            params![acc2],
        )
        .expect("seed sort_order");

        db_reorder_signatures_sync(&conn, std::slice::from_ref(&acc1)).expect("reorder");

        assert_eq!(get_sort_order(&conn, &acc1), 0);
        assert_eq!(
            get_sort_order(&conn, &acc2),
            99,
            "absent ids keep their prior sort_order"
        );
    }
}
