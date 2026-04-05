pub use db::db::queries_extra::thread_persistence::*;

use search::{SearchDocument, SearchState};
use store::body_store::{BodyStoreState, MessageBody};
use store::inline_image_store::{InlineImage, InlineImageStoreState};

pub async fn store_message_bodies<T, FId, FHtml, FText>(
    body_store: &BodyStoreState,
    messages: &[T],
    provider_name: &str,
    id_of: FId,
    html_of: FHtml,
    text_of: FText,
) where
    FId: Fn(&T) -> &str,
    FHtml: Fn(&T) -> Option<&String>,
    FText: Fn(&T) -> Option<&String>,
{
    let bodies: Vec<MessageBody> = messages
        .iter()
        .filter(|message| html_of(message).is_some() || text_of(message).is_some())
        .map(|message| MessageBody {
            message_id: id_of(message).to_string(),
            body_html: html_of(message).cloned(),
            body_text: text_of(message).cloned(),
        })
        .collect();

    if bodies.is_empty() {
        return;
    }

    log::debug!(
        "Storing {} message bodies for {}",
        bodies.len(),
        provider_name
    );
    if let Err(error) = body_store.put_batch(bodies).await {
        log::warn!("Failed to store {provider_name} bodies: {error}");
    }
}

pub async fn store_inline_images(
    inline_images: &InlineImageStoreState,
    images: Vec<InlineImage>,
    provider_name: &str,
) {
    if images.is_empty() {
        return;
    }

    log::debug!("Storing inline images for {provider_name}");
    if let Err(error) = inline_images.put_batch(images).await {
        log::warn!("Failed to store {provider_name} inline images: {error}");
    }
}

pub async fn index_search_documents(
    search: &SearchState,
    documents: Vec<SearchDocument>,
    provider_name: &str,
) {
    log::debug!(
        "Indexing {} search documents for {}",
        documents.len(),
        provider_name
    );
    if let Err(error) = search.index_messages_batch(&documents).await {
        log::warn!("Failed to index {provider_name} messages: {error}");
    }
}
