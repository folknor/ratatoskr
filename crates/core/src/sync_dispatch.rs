//! Sync dispatch — runs delta sync for a single account through the provider.
//!
//! This is the read path (server → local DB), moved to core so the app crate
//! doesn't need direct provider dependencies.

use crate::actions::provider::create_provider;
use crate::body_store::BodyStoreState;
use crate::db::DbState;
use crate::search::SearchState;
use ratatoskr_provider_utils::types::ProviderCtx;
use ratatoskr_stores::inline_image_store::InlineImageStoreState;

/// Run a delta sync for a single account.
///
/// Constructs the provider client internally. The caller provides
/// pre-initialized stores and a progress reporter.
pub async fn sync_delta_for_account(
    db: &DbState,
    account_id: &str,
    encryption_key: [u8; 32],
    body_store: &BodyStoreState,
    inline_images: &InlineImageStoreState,
    search: &SearchState,
    progress: &dyn ratatoskr_db::progress::ProgressReporter,
) -> Result<(), String> {
    let provider = create_provider(db, account_id, encryption_key).await?;
    let ctx = ProviderCtx {
        account_id,
        db,
        body_store,
        inline_images,
        search,
        progress,
    };
    provider
        .sync_delta(&ctx, None)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
