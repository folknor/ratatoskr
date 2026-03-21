//! Bridge between core's `get_thread_detail()` and the app's display types.
//!
//! Replaces the raw SQL shim that loaded messages and attachments separately.
//! Core provides `ThreadDetail` with body text from BodyStore, ownership
//! detection, collapsed summaries, resolved label colors, and persisted
//! attachment collapse state.

use std::sync::Arc;

use ratatoskr_core::body_store::BodyStoreState;
use ratatoskr_core::db::queries_extra::thread_detail::{
    self, ThreadDetail, get_thread_detail,
};
use ratatoskr_core::db::queries_extra::set_attachments_collapsed;

use super::connection::Db;
use super::types::{ThreadAttachment, ThreadMessage};

/// Label color info resolved from core's ThreadLabel.
#[derive(Debug, Clone)]
pub struct ResolvedLabel {
    pub label_id: String,
    pub name: String,
    pub color_bg: String,
    pub color_fg: String,
}

/// Full thread detail data for the reading pane.
#[derive(Debug, Clone)]
pub struct AppThreadDetail {
    pub thread_id: String,
    pub account_id: String,
    pub subject: Option<String>,
    pub is_starred: bool,
    pub messages: Vec<ThreadMessage>,
    pub labels: Vec<ResolvedLabel>,
    pub attachments: Vec<ThreadAttachment>,
    pub attachments_collapsed: bool,
}

/// Convert core's ThreadDetail into app display types.
fn convert_thread_detail(detail: ThreadDetail) -> AppThreadDetail {
    let messages = detail
        .messages
        .into_iter()
        .map(convert_message)
        .collect();

    let labels = detail
        .labels
        .into_iter()
        .map(|l| ResolvedLabel {
            label_id: l.label_id,
            name: l.name,
            color_bg: l.color_bg,
            color_fg: l.color_fg,
        })
        .collect();

    let attachments = detail
        .attachments
        .into_iter()
        .map(convert_attachment)
        .collect();

    AppThreadDetail {
        thread_id: detail.thread_id,
        account_id: detail.account_id,
        subject: detail.subject,
        is_starred: detail.is_starred,
        messages,
        labels,
        attachments,
        attachments_collapsed: detail.attachments_collapsed,
    }
}

fn convert_message(msg: thread_detail::ThreadDetailMessage) -> ThreadMessage {
    ThreadMessage {
        id: msg.id,
        thread_id: msg.thread_id,
        account_id: msg.account_id,
        from_name: msg.from_name,
        from_address: msg.from_address,
        to_addresses: msg.to_addresses,
        cc_addresses: msg.cc_addresses,
        date: Some(msg.date),
        subject: msg.subject,
        snippet: msg.collapsed_summary,
        body_html: msg.body_html,
        body_text: msg.body_text,
        is_read: msg.is_read,
        is_starred: msg.is_starred,
        is_own_message: msg.is_own_message,
    }
}

fn convert_attachment(att: thread_detail::ThreadAttachment) -> ThreadAttachment {
    ThreadAttachment {
        id: att.id,
        filename: att.filename,
        mime_type: att.mime_type,
        size: att.size,
        from_name: att.from_name,
        date: Some(att.date),
    }
}

/// Load full thread detail via core's `get_thread_detail()`.
///
/// This replaces the two separate `get_thread_messages` + `get_thread_attachments`
/// calls with a single core function that also provides:
/// - Body text from the BodyStore (decompressed from zstd)
/// - Message ownership detection (is_own_message)
/// - Quote/signature-stripped collapsed summaries
/// - Resolved label colors
/// - Persisted attachment collapse state
pub async fn load_thread_detail(
    db: &Db,
    body_store: &BodyStoreState,
    account_id: String,
    thread_id: String,
) -> Result<AppThreadDetail, String> {
    let bs_conn = body_store.conn();
    let db_conn = db.conn_arc();

    tokio::task::spawn_blocking(move || {
        let conn = db_conn
            .lock()
            .map_err(|e| format!("db lock: {e}"))?;
        let bs = bs_conn
            .lock()
            .map_err(|e| format!("body store lock: {e}"))?;
        let detail = get_thread_detail(&conn, &bs, &account_id, &thread_id)?;
        Ok(convert_thread_detail(detail))
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))?
}

/// Persist attachment collapse state to core's thread_ui_state table.
pub async fn persist_attachments_collapsed(
    db: &Db,
    account_id: String,
    thread_id: String,
    collapsed: bool,
) -> Result<(), String> {
    let conn = db.write_conn_arc();
    tokio::task::spawn_blocking(move || {
        let conn = conn
            .lock()
            .map_err(|e| format!("db write lock: {e}"))?;
        set_attachments_collapsed(&conn, &account_id, &thread_id, collapsed)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))?
}

// ── Legacy per-message query methods (used by pop-out and main thread loading) ──

impl Db {
    /// Load messages for a thread (used by main window thread selection).
    pub async fn get_thread_messages(
        &self,
        account_id: String,
        thread_id: String,
    ) -> Result<Vec<ThreadMessage>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT m.id, m.thread_id, m.account_id,
                            m.from_name, m.from_address,
                            m.to_addresses, m.cc_addresses,
                            m.date, m.subject, m.snippet,
                            m.is_read, m.is_starred
                     FROM messages m
                     WHERE m.account_id = ?1 AND m.thread_id = ?2
                     ORDER BY m.date ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(rusqlite::params![account_id, thread_id], |row| {
                Ok(ThreadMessage {
                    id: row.get("id")?,
                    thread_id: row.get("thread_id")?,
                    account_id: row.get("account_id")?,
                    from_name: row.get("from_name")?,
                    from_address: row.get("from_address")?,
                    to_addresses: row.get("to_addresses")?,
                    cc_addresses: row.get("cc_addresses")?,
                    date: row.get("date")?,
                    subject: row.get("subject")?,
                    snippet: row.get("snippet")?,
                    body_html: None,
                    body_text: None,
                    is_read: row.get::<_, i64>("is_read")? != 0,
                    is_starred: row.get::<_, i64>("is_starred")? != 0,
                    is_own_message: false,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Load attachments for a thread (used by main window thread selection).
    pub async fn get_thread_attachments(
        &self,
        account_id: String,
        thread_id: String,
    ) -> Result<Vec<super::types::ThreadAttachment>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT a.id, a.filename, a.mime_type, a.size,
                            m.from_name, m.date
                     FROM attachments a
                     JOIN messages m ON m.id = a.message_id AND m.account_id = a.account_id
                     WHERE a.account_id = ?1 AND m.thread_id = ?2
                     ORDER BY m.date ASC, a.id ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(rusqlite::params![account_id, thread_id], |row| {
                Ok(super::types::ThreadAttachment {
                    id: row.get("id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
                    from_name: row.get("from_name")?,
                    date: row.get("date")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Load body text and HTML for a single message (used by pop-out windows).
    pub async fn load_message_body(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<(Option<String>, Option<String>), String> {
        self.with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT body_text, body_html FROM messages
                 WHERE account_id = ?1 AND id = ?2",
                rusqlite::params![account_id, message_id],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>("body_text")?,
                        row.get::<_, Option<String>>("body_html")?,
                    ))
                },
            );
            match result {
                Ok(pair) => Ok(pair),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok((None, None)),
                Err(e) => Err(e.to_string()),
            }
        })
        .await
    }

    /// Load attachments for a single message (used by pop-out windows).
    pub async fn load_message_attachments(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<Vec<super::types::MessageViewAttachment>, String> {
        self.with_conn(move |conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, filename, mime_type, size
                     FROM attachments
                     WHERE account_id = ?1 AND message_id = ?2
                     ORDER BY id ASC",
                )
                .map_err(|e| e.to_string())?;

            stmt.query_map(rusqlite::params![account_id, message_id], |row| {
                Ok(super::types::MessageViewAttachment {
                    id: row.get("id")?,
                    filename: row.get("filename")?,
                    mime_type: row.get("mime_type")?,
                    size: row.get("size")?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())
        })
        .await
    }

    /// Load raw email source for a message (used by pop-out Source view).
    pub async fn load_raw_source(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<String, String> {
        self.with_conn(move |conn| {
            let result = conn.query_row(
                "SELECT raw_source FROM messages
                 WHERE account_id = ?1 AND id = ?2",
                rusqlite::params![account_id, message_id],
                |row| row.get::<_, Option<String>>(0),
            );
            match result {
                Ok(Some(source)) => Ok(source),
                Ok(None) => Ok("(no source available)".to_string()),
                Err(rusqlite::Error::QueryReturnedNoRows) => {
                    Err("Message not found".to_string())
                }
                Err(e) => Err(e.to_string()),
            }
        })
        .await
    }
}

/// Initialize the body store for loading message bodies.
pub fn init_body_store() -> Result<BodyStoreState, String> {
    let data_dir = crate::APP_DATA_DIR
        .get()
        .ok_or_else(|| "APP_DATA_DIR not set".to_string())?;
    BodyStoreState::init(data_dir)
}
