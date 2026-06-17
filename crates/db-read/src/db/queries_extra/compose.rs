use super::super::ReadDbState;
use super::super::types::{DbLocalDraft, DbSignature};
use crate::db::{query_as, query_one};
use rusqlite::params;

const SIG_COLS: &str = "id, account_id, name, body_html, body_text, \
    is_default, is_reply_default, sort_order, source, server_id, \
    server_html_hash, last_synced_at, created_at";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

pub async fn db_get_all_signatures(db: &ReadDbState) -> Result<Vec<DbSignature>, String> {
    db.with_read(move |conn| {
        let sql = format!(
            "SELECT {SIG_COLS} FROM signatures
             ORDER BY account_id, sort_order, name"
        );
        query_as::<DbSignature>(conn, &sql, &[])
    })
    .await
}

pub async fn db_resolve_signature_for_compose(
    db: &ReadDbState,
    account_id: String,
    from_email: Option<String>,
    is_reply: bool,
) -> Result<Option<DbSignature>, String> {
    db.with_read(move |conn| {
        if let Some(ref email) = from_email {
            let alias_sig_id = match conn.query_row(
                "SELECT signature_id FROM send_as_aliases
                 WHERE account_id = ?1 AND email = ?2 AND signature_id IS NOT NULL",
                params![account_id, email],
                |row| row.get::<_, String>(0),
            ) {
                Ok(id) => Some(id),
                Err(crate::db::ReadError::Sql(rusqlite::Error::QueryReturnedNoRows)) => None,
                Err(e) => return Err(e.to_string()),
            };
            if let Some(sig_id) = alias_sig_id {
                let sql = format!("SELECT {SIG_COLS} FROM signatures WHERE id = ?1 LIMIT 1");
                if let Some(sig) = query_one::<DbSignature>(conn, &sql, &[&sig_id])? {
                    return Ok(Some(sig));
                }
            }
        }

        let sql = if is_reply {
            format!(
                "SELECT {SIG_COLS} FROM signatures
                 WHERE account_id = ?1 AND (is_reply_default = 1 OR is_default = 1)
                 ORDER BY is_reply_default DESC LIMIT 1"
            )
        } else {
            format!(
                "SELECT {SIG_COLS} FROM signatures
                 WHERE account_id = ?1 AND is_default = 1 LIMIT 1"
            )
        };
        query_one::<DbSignature>(conn, &sql, &[&account_id])
    })
    .await
}

pub async fn db_get_local_draft(
    db: &ReadDbState,
    id: String,
) -> Result<Option<DbLocalDraft>, String> {
    db.with_read(move |conn| {
        query_one::<DbLocalDraft>(conn, "SELECT * FROM local_drafts WHERE id = ?1", &[&id])
    })
    .await
}
