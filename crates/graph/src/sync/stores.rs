use ratatoskr_stores::body_store::BodyStoreState;
use ratatoskr_stores::inline_image_store::{InlineImage, InlineImageStoreState};
use ratatoskr_search::{SearchDocument, SearchState};

use super::super::parse::ParsedGraphMessage;
use super::SyncCtx;
use ratatoskr_sync::{persistence as sync_persistence, progress as sync_progress};

// ---------------------------------------------------------------------------
// Body store helper
// ---------------------------------------------------------------------------

pub(super) async fn store_bodies(body_store: &BodyStoreState, messages: &[ParsedGraphMessage]) {
    sync_persistence::store_message_bodies(
        body_store,
        messages,
        "Graph",
        |message| &message.base.id,
        |message| message.base.body_html.as_ref(),
        |message| message.base.body_text.as_ref(),
    )
    .await;
}

pub(super) async fn store_inline_images(
    inline_images: &InlineImageStoreState,
    messages: &[ParsedGraphMessage],
) {
    let images: Vec<InlineImage> = messages
        .iter()
        .flat_map(|m| &m.attachments)
        .filter_map(|att| {
            let data = att.inline_data.as_ref()?;
            let hash = att.content_hash.as_ref()?;
            let mime = att.mime_type.as_ref()?;
            Some(InlineImage {
                content_hash: hash.clone(),
                data: data.clone(),
                mime_type: mime.clone(),
            })
        })
        .collect();

    sync_persistence::store_inline_images(inline_images, images, "Graph").await;
}

// ---------------------------------------------------------------------------
// Search index helper
// ---------------------------------------------------------------------------

pub(super) async fn index_messages(
    search: &SearchState,
    account_id: &str,
    messages: &[ParsedGraphMessage],
) {
    let docs: Vec<SearchDocument> = messages
        .iter()
        .map(|m| SearchDocument {
            message_id: m.base.id.clone(),
            account_id: account_id.to_string(),
            thread_id: m.base.thread_id.clone(),
            subject: m.base.subject.clone(),
            from_name: m.base.from_name.clone(),
            from_address: m.base.from_address.clone(),
            to_addresses: m.base.to_addresses.clone(),
            body_text: m.base.body_text.clone(),
            snippet: Some(m.base.snippet.clone()),
            date: m.base.date / 1000, // tantivy expects seconds
            is_read: m.base.is_read,
            is_starred: m.base.is_starred,
            has_attachment: m.base.has_attachments,
        })
        .collect();

    sync_persistence::index_search_documents(search, docs, "Graph").await;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Folder sync priority for initial sync ordering.
/// Lower number = higher priority (synced first).
pub(super) fn folder_priority(label_id: &str) -> u8 {
    match label_id {
        "INBOX" | "SENT" | "DRAFT" => 0,
        "archive" | "TRASH" | "SPAM" => 1,
        _ => 2,
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_progress(
    sctx: &SyncCtx<'_>,
    phase: &str,
    folder_name: &str,
    current_folder: u64,
    total_folders: u64,
    messages_processed: u64,
) {
    sync_progress::emit_sync_progress(
        sctx.progress,
        "graph-sync-progress",
        sctx.account_id,
        phase,
        if phase == "messages" {
            messages_processed
        } else {
            current_folder
        },
        total_folders,
        (!folder_name.is_empty()).then_some(folder_name),
    );
}
