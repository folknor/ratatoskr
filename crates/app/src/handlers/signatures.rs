//! Signature CRUD handlers for the app crate.
//!
//! These functions replace the raw SQL that was previously inlined in
//! `main.rs`. They use the app's `Db` connection but implement proper
//! transactional default-clearing semantics.

use std::sync::Arc;

use iced::Task;
use rusqlite::params;

use crate::db::Db;
use crate::ui::settings::SignatureEntry;

/// Save a signature (insert or update) with transactional default management.
///
/// When `is_default` is true, clears `is_default` on all other signatures for
/// the same account. Same for `is_reply_default`. Auto-generates `body_text`
/// from `body_html`.
pub fn handle_save_signature(
    db: &Arc<Db>,
    req: crate::ui::settings::SignatureSaveRequest,
) -> Task<super::SignatureResult> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            let body_text = html_to_plain_text(&req.body_html);
            db.with_write_conn(move |conn| {
                let tx = conn.unchecked_transaction().map_err(|e| e.to_string())?;

                if let Some(ref id) = req.id {
                    // Update existing signature.
                    clear_defaults_if_needed(
                        &tx,
                        id,
                        req.is_default,
                        req.is_reply_default,
                    )?;
                    tx.execute(
                        "UPDATE signatures SET name = ?1, body_html = ?2, body_text = ?3, \
                         is_default = ?4, is_reply_default = ?5 WHERE id = ?6",
                        params![
                            req.name,
                            req.body_html,
                            body_text,
                            i64::from(req.is_default),
                            i64::from(req.is_reply_default),
                            id,
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                } else {
                    // Insert new signature.
                    let id = uuid::Uuid::new_v4().to_string();
                    clear_defaults_for_account(
                        &tx,
                        &req.account_id,
                        req.is_default,
                        req.is_reply_default,
                    )?;
                    tx.execute(
                        "INSERT INTO signatures \
                         (id, account_id, name, body_html, body_text, is_default, is_reply_default) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            id,
                            req.account_id,
                            req.name,
                            req.body_html,
                            body_text,
                            i64::from(req.is_default),
                            i64::from(req.is_reply_default),
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                }

                tx.commit().map_err(|e| e.to_string())?;
                Ok(())
            })
            .await
        },
        |result| {
            if let Err(ref e) = result {
                eprintln!("Failed to save signature: {e}");
            }
            super::SignatureResult::Saved(result)
        },
    )
}

/// Delete a signature by ID.
pub fn handle_delete_signature(
    db: &Arc<Db>,
    sig_id: String,
) -> Task<super::SignatureResult> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            db.with_write_conn(move |conn| {
                conn.execute(
                    "DELETE FROM signatures WHERE id = ?1",
                    params![sig_id],
                )
                .map_err(|e| e.to_string())?;
                Ok(())
            })
            .await
        },
        |result| {
            if let Err(ref e) = result {
                eprintln!("Failed to delete signature: {e}");
            }
            super::SignatureResult::Deleted(result)
        },
    )
}

/// Load all signatures from the DB asynchronously.
pub fn load_signatures_async(
    db: &Arc<Db>,
) -> Task<super::SignatureResult> {
    let db = Arc::clone(db);
    Task::perform(
        async move {
            db.with_conn(|conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, account_id, name, body_html, body_text, is_default, \
                         is_reply_default, sort_order \
                         FROM signatures ORDER BY account_id, sort_order, name",
                    )
                    .map_err(|e| e.to_string())?;
                let rows = stmt
                    .query_map([], |row| {
                        Ok(SignatureEntry {
                            id: row.get("id")?,
                            account_id: row.get("account_id")?,
                            name: row.get("name")?,
                            body_html: row.get::<_, Option<String>>("body_html")?
                                .unwrap_or_default(),
                            body_text: row.get("body_text")?,
                            is_default: row.get::<_, i64>("is_default").unwrap_or(0) != 0,
                            is_reply_default: row.get::<_, i64>("is_reply_default")
                                .unwrap_or(0)
                                != 0,
                        })
                    })
                    .map_err(|e| e.to_string())?;
                rows.collect::<Result<Vec<_>, _>>()
                    .map_err(|e| e.to_string())
            })
            .await
        },
        |result| super::SignatureResult::Loaded(result),
    )
}

// ── Internal helpers ────────────────────────────────────

/// When updating a signature: look up its account_id, then clear defaults
/// for that account if needed.
fn clear_defaults_if_needed(
    conn: &rusqlite::Connection,
    signature_id: &str,
    is_default: bool,
    is_reply_default: bool,
) -> Result<(), String> {
    let account_id: Option<String> = conn
        .query_row(
            "SELECT account_id FROM signatures WHERE id = ?1",
            params![signature_id],
            |row| row.get(0),
        )
        .ok();
    if let Some(ref aid) = account_id {
        clear_defaults_for_account(conn, aid, is_default, is_reply_default)?;
    }
    Ok(())
}

/// Clear `is_default` and/or `is_reply_default` for all signatures in the
/// given account, in preparation for setting a new default.
fn clear_defaults_for_account(
    conn: &rusqlite::Connection,
    account_id: &str,
    clear_default: bool,
    clear_reply_default: bool,
) -> Result<(), String> {
    if clear_default {
        conn.execute(
            "UPDATE signatures SET is_default = 0 WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
    }
    if clear_reply_default {
        conn.execute(
            "UPDATE signatures SET is_reply_default = 0 WHERE account_id = ?1",
            params![account_id],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// ── HTML-to-plain-text ──────────────────────────────────

/// Strip HTML tags to produce a plain-text fallback for the signature.
///
/// Block elements insert newlines; inline elements are dropped.
fn html_to_plain_text(html: &str) -> String {
    // Delegate to the core implementation.
    ratatoskr_core::db::queries_extra::html_to_plain_text(html)
}
