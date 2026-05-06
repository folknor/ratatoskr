//! `pinned_search.kick` notification handler (Phase 6a).
//!
//! `Drop`-class self-heal driven by the UI's 5-min `Message::SyncTick`
//! fan-out. Mirrors the shape of `gal.kick` and `calendar.kick`: the
//! UI fires the cadence, the Service runs the work and gates by a
//! staleness window. Missed kicks self-heal on the next `SyncTick`.
//!
//! The work is one global DELETE keyed on the 14-day staleness
//! threshold (matches today's UI-side `expire_stale_pinned_searches`
//! call). Pinned searches are not per-account; the table is global.
//! The DELETE is idempotent so concurrent kicks are harmless.
//!
//! There is no back-notification to the UI. The sidebar's in-memory
//! pinned-search list will pick up deletions on the next path that
//! re-lists (PinnedSearchesLoaded). Acceptable because the staleness
//! threshold is 14 days; the user is not watching the sidebar for a
//! 14-day expiry to land in real time.

use std::sync::Arc;

use crate::boot::BootSharedState;

/// Today's threshold from the UI-side caller: 14 days
/// (1_209_600 seconds). Held as a constant here so the relocation
/// preserves the historical behavior verbatim.
const STALENESS_SECS: i64 = 1_209_600;

pub(crate) async fn handle_kick(boot_state: &Arc<BootSharedState>) -> Result<(), String> {
    let write_db = match boot_state.write_db_state() {
        Ok(db) => db,
        Err(_) => {
            log::debug!("pinned_search.kick received before db_conn available; ignoring");
            return Ok(());
        }
    };
    let deleted = write_db
        .with_conn(move |conn| {
            db::db::pinned_searches::db_expire_stale_pinned_searches_sync(conn, STALENESS_SECS)
        })
        .await?;
    if deleted > 0 {
        log::info!("pinned_search.kick: expired {deleted} stale pinned searches");
    }
    Ok(())
}
