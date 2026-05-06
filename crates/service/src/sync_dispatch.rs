//! Sync dispatch - runs delta sync for a single account through the provider.
//!
//! Phase 3 task 7/8: lives in the `service` crate (alongside the
//! `SyncRuntime` runner that spawns it) so the Service-side caller
//! does not have to depend back into `core` and trip the
//! `core -> service` re-export cycle. The function itself is
//! provider-agnostic; the only `core` call is `create_provider`,
//! which has a sibling implementation in `service::actions::provider`.
//!
//! Pre-Phase-3 this lived in `core::sync_dispatch` and consumed the
//! unified store states + read-half DB; Phase 3 reshapes it for
//! writer halves + cancellation token.

use db::db::ReadDbState;
use provider_sync::SyncProviderCtx;
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};
use tokio_util::sync::CancellationToken;

use crate::actions::provider::create_provider;

/// Run a delta sync for a single account.
///
/// `db` is the writer-half connection; the function passes it to the
/// provider's `sync_delta` impl via `SyncProviderCtx`. The provider
/// internally derives a read-state view onto the same connection for
/// the helpers that have not yet been retyped onto the write half
/// (transitional bridge from Phase 3 task 4).
///
/// `cancellation_token` is observed at JMAP sync's per-mailbox /
/// per-batch / network-call checkpoints (Phase 3 task 6); the runner
/// in `service::sync` flips the token when the UI dispatches
/// `sync.cancel_account`.
#[allow(clippy::too_many_arguments)]
pub async fn sync_delta_for_account(
    write_db: &WriteDbState,
    account_id: &str,
    encryption_key: [u8; 32],
    body_store: &BodyStoreWriteState,
    inline_images: &InlineImageStoreWriteState,
    search: &SearchWriteHandle,
    progress: &dyn db::progress::ProgressReporter,
    cancellation_token: &CancellationToken,
) -> Result<(), String> {
    let read_db: ReadDbState = write_db.to_read_state();
    let provider = create_provider(&read_db, account_id, encryption_key).await?;
    let ctx = SyncProviderCtx {
        account_id,
        db: write_db,
        body_store,
        inline_images,
        search,
        progress,
        cancellation_token,
    };
    provider
        .sync_delta(&ctx, None)
        .await
        .map(|_| ())
        .map_err(|e| e.to_string())
}
