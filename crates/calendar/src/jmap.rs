//! JMAP arm for `calendar_sync_account_impl`.
//!
//! Wraps `jmap::calendar_sync::sync_calendars` so that the JMAP path
//! lives behind the same `CalendarRuntime` boundary as Google, Graph,
//! and CalDAV. Before Phase 5 the JMAP email-sync pipeline called
//! `sync_calendars` directly from inside email sync (bypassing the
//! runtime); that bypass is removed in the same commit that introduces
//! this arm.
//!
//! Cancellation is a coarse entry-point check today: the underlying
//! `jmap::calendar_sync::sync_calendars` does not accept a token.
//! Threading per-batch cancellation through the JMAP calendar pipeline
//! is tracked in the same Phase-3-retrospective bucket as the
//! Gmail/Graph entry-only check (see `docs/service/discrepancies.md`).

use jmap::client::JmapState;
use rtsk::db::ReadDbState;
use tokio_util::sync::CancellationToken;

pub(crate) async fn sync_jmap_calendar_account(
    account_id: &str,
    db: &ReadDbState,
    jmap: &JmapState,
    cancellation_token: &CancellationToken,
    mutated: &mut bool,
) -> Result<(), String> {
    if cancellation_token.is_cancelled() {
        return Err("calendar sync cancelled".to_string());
    }
    let client = jmap.get(account_id).await?;
    // sync_calendars writes calendars + events whenever it succeeds
    // partially or in full. The current API doesn't surface a
    // partial-mutation flag, so flip `mutated` unconditionally before
    // the call - any failure that lands here may still have committed
    // earlier batches, and the runtime must drive a UI reload either
    // way (same rule the per-provider arms use).
    *mutated = true;
    jmap::calendar_sync::sync_calendars(&client, account_id, db).await
}
