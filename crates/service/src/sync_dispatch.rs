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

use crate::actions::provider::create_provider_with_writer;

/// Run sync for a single account.
///
/// `db` is the writer-half connection; the function passes it to the
/// provider's `sync_initial` or `sync_delta` impl via `SyncProviderCtx`.
/// The provider internally derives a read-state view onto the same
/// connection for the helpers that have not yet been retyped onto the
/// write half (transitional bridge from Phase 3 task 4).
///
/// `cancellation_token` is observed at JMAP sync's per-mailbox /
/// per-batch / network-call checkpoints (Phase 3 task 6); the runner
/// in `service::sync` flips the token when the UI dispatches
/// `sync.cancel_account`.
#[allow(clippy::too_many_arguments)]
pub async fn sync_for_account(
    write_db: &WriteDbState,
    read_db: &ReadDbState,
    account_id: &str,
    encryption_key: [u8; 32],
    body_store: &BodyStoreWriteState,
    inline_images: &InlineImageStoreWriteState,
    search: &SearchWriteHandle,
    progress: &dyn db::progress::ProgressReporter,
    cancellation_token: &CancellationToken,
) -> Result<(), String> {
    let aid = account_id.to_string();
    let initial_sync_completed = write_db
        .with_write(move |conn| {
            conn.query_row(
                "SELECT initial_sync_completed FROM accounts WHERE id = ?1",
                rusqlite::params![aid],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .map_err(|e| format!("read initial_sync_completed: {e}"))
        })
        .await?;
    let provider = create_provider_with_writer(read_db, write_db, account_id, encryption_key).await?;
    let ctx = SyncProviderCtx {
        account_id,
        db: write_db,
        read_db,
        body_store,
        inline_images,
        search,
        progress,
        cancellation_token,
    };
    // Attachments roadmap Phase 4: initial sync walks back
    // `sync_period_days` so the metadata exists for the prefetch
    // backfill driver to find. Fallback to 365 if the setting is
    // missing or out of range; saturating cast keeps an absurd
    // pref value from underflowing.
    let initial_window_days: i64 = read_db
        .with_read_sync(|conn| Ok(sync::config::get_sync_period_days(conn).max(1)))
        .unwrap_or(365);
    let result = if initial_sync_completed {
        provider.sync_delta(&ctx, None).await
    } else {
        provider.sync_initial(&ctx, initial_window_days).await
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}
