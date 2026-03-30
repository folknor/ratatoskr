//! Bridge between core's `get_thread_detail()` and the app's display types.
//!
//! Replaces the raw SQL shim that loaded messages and attachments separately.
//! Core provides `ThreadDetail` with body text from BodyStore, ownership
//! detection, collapsed summaries, resolved label colors, and persisted
//! attachment collapse state.

use std::collections::HashMap;

use rtsk::body_store::BodyStoreState;
use rtsk::db::queries_extra::thread_detail::{
    self, ThreadDetail, assemble_thread_detail, fetch_thread_bodies, query_inline_cid_hashes,
    query_thread_from_db,
};
use rtsk::db::queries_extra::set_attachments_collapsed;
use store::inline_image_store::InlineImageStoreState;

use super::connection::Db;
use super::types::{ThreadAttachment, ThreadMessage};

/// Label color info resolved from core's ThreadLabel.
#[derive(Debug, Clone)]
pub struct ResolvedLabel {
    pub label_id: String,
    pub name: String,
    pub color_bg: String,
    pub color_fg: String,
    /// "container" (folder/mailbox) or "tag" (category/keyword).
    pub label_kind: String,
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
    /// Pre-loaded CID-to-image-bytes map for inline `<img src="cid:...">` resolution.
    pub inline_images: HashMap<String, Vec<u8>>,
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
            label_kind: l.label_kind,
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
        inline_images: HashMap::new(),
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
    inline_image_store: Option<&InlineImageStoreState>,
    account_id: String,
    thread_id: String,
) -> Result<AppThreadDetail, String> {
    let bs_conn = body_store.conn();
    let db_conn = db.conn_arc();
    let iis_conn = inline_image_store.map(InlineImageStoreState::conn);

    tokio::task::spawn_blocking(move || {
        // Phase 1: hold main DB lock only for DB queries, then release.
        let (db_data, cid_hashes) = {
            let conn = db_conn
                .lock()
                .map_err(|e| format!("db lock: {e}"))?;
            let data = query_thread_from_db(&conn, &account_id, &thread_id)?;
            let cids = query_inline_cid_hashes(&conn, &account_id, &thread_id)?;
            (data, cids)
        };

        // Phase 2: hold body store lock only for body fetches, then release.
        let body_map = {
            let bs = bs_conn
                .lock()
                .map_err(|e| format!("body store lock: {e}"))?;
            fetch_thread_bodies(&bs, &db_data.messages)?
        };

        // Phase 2b: fetch inline images from the inline image store.
        let inline_images = if let Some(ref iis_conn) = iis_conn {
            if !cid_hashes.is_empty() {
                let iis = iis_conn
                    .lock()
                    .map_err(|e| format!("inline image store lock: {e}"))?;
                InlineImageStoreState::get_batch_sync(&iis, &cid_hashes)?
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        // Phase 3: pure computation, no locks needed.
        let detail = assemble_thread_detail(db_data, body_map, &account_id, &thread_id);
        let mut app_detail = convert_thread_detail(detail);
        app_detail.inline_images = inline_images;
        Ok(app_detail)
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


// ── Per-message queries — delegated to core ─────────────
//
// These thin wrappers call core's `message_queries` module and convert
// the results into app display types. Raw SQL formerly lived here but
// has been moved to `crates/core/src/db/queries_extra/message_queries.rs`.

use rtsk::db::queries_extra::message_queries;

impl Db {
    /// Load body text and HTML for a single message (used by pop-out windows).
    pub async fn load_message_body(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<(Option<String>, Option<String>), String> {
        let conn = self.conn_arc();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
            message_queries::get_message_body(&conn, &account_id, &message_id)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Load attachments for a single message (used by pop-out windows).
    pub async fn load_message_attachments(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<Vec<super::types::MessageViewAttachment>, String> {
        let conn = self.conn_arc();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
            let core_atts = message_queries::get_message_attachments(
                &conn, &account_id, &message_id,
            )?;
            Ok(core_atts
                .into_iter()
                .map(|a| super::types::MessageViewAttachment {
                    id: a.id,
                    filename: a.filename,
                    mime_type: a.mime_type,
                    size: a.size,
                })
                .collect())
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }

    /// Load raw email source for a message (used by pop-out Source view).
    pub async fn load_raw_source(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<String, String> {
        let conn = self.conn_arc();
        tokio::task::spawn_blocking(move || {
            let conn = conn.lock().map_err(|e| format!("db lock: {e}"))?;
            message_queries::get_message_raw_source(&conn, &account_id, &message_id)
        })
        .await
        .map_err(|e| format!("spawn_blocking: {e}"))?
    }
}

/// Initialize the body store for loading message bodies.
pub fn init_body_store() -> Result<BodyStoreState, String> {
    let data_dir = crate::APP_DATA_DIR
        .get()
        .ok_or_else(|| "APP_DATA_DIR not set".to_string())?;
    BodyStoreState::init(data_dir)
}
