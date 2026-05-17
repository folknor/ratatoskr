//! Bridge between core's `get_thread_detail()` and the app's display types.
//!
//! Replaces the raw SQL shim that loaded messages and attachments separately.
//! Core provides `ThreadDetail` with body text from BodyStore, ownership
//! detection, collapsed summaries, resolved label colors, and persisted
//! attachment collapse state.

use std::collections::HashMap;

use iced::widget::image;
use rtsk::body_store::BodyStoreReadState;
use rtsk::db::queries_extra::thread_detail::{
    self, ThreadDetail, assemble_thread_detail, fetch_thread_bodies, query_inline_cid_hashes,
    query_thread_from_db,
};
use store::inline_image_store::InlineImageStoreReadState;

use super::connection::Db;
use super::types::{ThreadAttachment, ThreadMessage};
use crate::ui::label_paint::LabelPaint;

/// A label-group pill resolved for the reading-pane display. Every entry
/// is a `label_groups` row by construction - raw provider labels never
/// reach this surface. `label_id` holds the stringified group id.
#[derive(Debug, Clone)]
pub struct ResolvedLabel {
    pub label_id: String,
    pub name: String,
    pub paint: LabelPaint,
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
    pub inline_images: HashMap<String, image::Handle>,
}

/// Convert core's ThreadDetail into app display types.
fn convert_thread_detail(detail: ThreadDetail) -> AppThreadDetail {
    let messages = detail.messages.into_iter().map(convert_message).collect();

    let labels = detail
        .labels
        .into_iter()
        .map(|l| ResolvedLabel {
            label_id: l.label_id,
            name: l.name,
            paint: LabelPaint::from_hex_pair(&l.color_bg, &l.color_fg),
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
        message_id_header: msg.message_id_header,
        snippet: msg.collapsed_summary,
        body_html: msg.body_html,
        body_text: msg.body_text,
        is_read: msg.is_read,
        is_starred: msg.is_starred,
        is_replied: msg.is_replied,
        is_forwarded: msg.is_forwarded,
        is_own_message: msg.is_own_message,
    }
}

fn convert_attachment(att: thread_detail::ThreadAttachment) -> ThreadAttachment {
    ThreadAttachment {
        id: att.id,
        message_id: att.message_id,
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
/// - Body text from the BodyStore (decompressed)
/// - Message ownership detection (is_own_message)
/// - Quote/signature-stripped collapsed summaries
/// - Resolved label colors
/// - Persisted attachment collapse state
#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub async fn load_thread_detail(
    db: &Db,
    body_store: &BodyStoreReadState,
    inline_image_store: Option<&InlineImageStoreReadState>,
    account_id: String,
    thread_id: String,
) -> Result<AppThreadDetail, String> {
    let bs_conn = body_store.conn();
    let iis_conn = inline_image_store.map(InlineImageStoreReadState::conn);
    let db_account_id = account_id.clone();
    let db_thread_id = thread_id.clone();
    let (db_data, cid_hashes) = db
        .with_read(move |conn| {
            let data = query_thread_from_db(conn, &db_account_id, &db_thread_id)?;
            let cids = query_inline_cid_hashes(conn, &db_account_id, &db_thread_id)?;
            Ok((data, cids))
        })
        .await?;

    tokio::task::spawn_blocking(move || {
        let body_map = {
            let bs = bs_conn
                .lock()
                .map_err(|e| format!("body store lock: {e}"))?;
            fetch_thread_bodies(&bs, &db_data.messages)?
        };

        let inline_images: HashMap<String, image::Handle> = if let Some(ref iis_conn) = iis_conn {
            if !cid_hashes.is_empty() {
                let iis = iis_conn
                    .lock()
                    .map_err(|e| format!("inline image store lock: {e}"))?;
                InlineImageStoreReadState::get_batch_sync(&iis, &cid_hashes)?
                    .into_iter()
                    .map(|(cid, bytes)| (cid, image::Handle::from_bytes(bytes)))
                    .collect()
            } else {
                HashMap::new()
            }
        } else {
            HashMap::new()
        };

        let detail = assemble_thread_detail(db_data, body_map, &account_id, &thread_id);
        let mut app_detail = convert_thread_detail(detail);
        app_detail.inline_images = inline_images;
        Ok(app_detail)
    })
    .await
    .map_err(|e| format!("spawn_blocking: {e}"))?
}


// ── Per-message queries - delegated to core ─────────────
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
        self.with_read(move |conn| message_queries::get_message_body(conn, &account_id, &message_id))
            .await
    }

    /// Load attachments for a single message (used by pop-out windows).
    pub async fn load_message_attachments(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<Vec<super::types::MessageViewAttachment>, String> {
        self.with_read(move |conn| {
            let core_atts = message_queries::get_message_attachments(conn, &account_id, &message_id)?;
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
    }

    /// Load raw email source for a message (used by pop-out Source view).
    pub async fn load_raw_source(
        &self,
        account_id: String,
        message_id: String,
    ) -> Result<String, String> {
        self.with_read(move |conn| message_queries::get_message_raw_source(conn, &account_id, &message_id))
            .await
    }
}

/// Initialize the body store for loading message bodies.
pub fn init_body_store() -> Result<BodyStoreReadState, String> {
    let data_dir = crate::APP_DATA_DIR
        .get()
        .ok_or_else(|| "APP_DATA_DIR not set".to_string())?;
    BodyStoreReadState::init(data_dir)
}
