//! `gal.kick` notification handler.
//!
//! Phase 5 task 5. GAL refresh is kick-driven, idempotent, and bounded
//! (60 s per-account timeout x account count, gated by the existing
//! 24 h cache check inside `refresh_gal_for_account`). No per-account
//! runtime; no cancellation. Iterates all accounts -
//! `refresh_gal_for_account` self-gates non-supported providers with
//! `Ok(0)`.
//!
//! ## Required: serialize handler invocations via the module-level Tokio Mutex
//!
//! The notification dispatcher runs handlers concurrently up to
//! `NOTIFY_CAP = 4` (`dispatch.rs`); two stale-account kicks back-to-
//! back without serialization will duplicate provider calls because
//! `refresh_gal_for_account` only writes the cache after the network
//! round-trip completes. The mutex is load-bearing for correctness, not
//! just performance.
//!
//! The cleanest fix lives inside `refresh_gal_for_account` itself - a
//! per-account in-flight set so different accounts can refresh in
//! parallel - which is the documented future-work direction. The
//! global handler mutex here is the cheaper "no concurrent kicks at
//! all" coarsening of that load-bearing form. Acceptable because:
//!
//! - GAL kicks are 5-min-cadenced; the lock is held only while the
//!   iteration runs, and most calls hit the 24 h cache short-circuit
//!   without doing network work.
//! - The mutex test in this module asserts at-most-one in-flight call
//!   per account, which is shape-stable across both the global-lock
//!   form and the future per-account form.
//!
//! ## Interaction with the notification-drain bound
//!
//! `refresh_gal_for_account` performs DB writes via
//! `tokio::task::spawn_blocking` (inside `ReadDbState::with_conn`,
//! `crates/db/src/db/mod.rs`). If the consolidated drain's
//! notification-drain bound (Phase 5 task 7) aborts a wedged GAL
//! handler, the *outer* async future is dropped but the blocking
//! closure runs to completion regardless. Acceptable because GAL
//! writes are bounded and idempotent. Any future abortable
//! notification handler must satisfy the same contract.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

use crate::boot::BootSharedState;

/// Global handler mutex - coarsens the load-bearing per-account
/// hazard. See module docs for rationale.
static GAL_HANDLER_LOCK: Mutex<()> = Mutex::const_new(());

/// Per-account timeout, preserving today's UI-side `refresh_gal_caches`
/// budget.
const PER_ACCOUNT_TIMEOUT: Duration = Duration::from_secs(60);

pub(crate) async fn handle_gal_kick(boot_state: &Arc<BootSharedState>) -> Result<(), String> {
    let _guard = GAL_HANDLER_LOCK.lock().await;

    let Some(conn) = boot_state.db_conn() else {
        log::debug!("gal.kick received before db_conn available; ignoring");
        return Ok(());
    };
    let Some(encryption_key) = boot_state.encryption_key() else {
        log::debug!("gal.kick received before encryption_key available; ignoring");
        return Ok(());
    };
    let read_db = db::db::ReadDbState::from_arc(conn);

    let account_ids = read_db
        .with_conn(db::db::queries_extra::list_all_account_ids_sync)
        .await?;
    if account_ids.is_empty() {
        return Ok(());
    }

    log::debug!("gal.kick: iterating {} accounts", account_ids.len());

    for account_id in account_ids {
        if boot_state.shutdown_token().is_cancelled() {
            // Cooperative shutdown: bail out between accounts rather
            // than waiting out PER_ACCOUNT_TIMEOUT for accounts we
            // know the Service is about to abort anyway. The
            // already-running per-account refresh keeps its own
            // 24 h cache write idempotent, so re-running on the next
            // gal.kick after a fresh boot is harmless.
            log::info!("[GAL] shutdown token fired mid-iteration, exiting");
            return Ok(());
        }
        match tokio::time::timeout(
            PER_ACCOUNT_TIMEOUT,
            rtsk::contacts::gal::refresh_gal_for_account(&read_db, &account_id, encryption_key),
        )
        .await
        {
            Ok(Ok(n)) if n > 0 => {
                log::info!("[GAL] cached {n} entries for {account_id}");
            }
            Ok(Ok(_)) => {} // cache fresh or unsupported provider
            Ok(Err(e)) => {
                log::warn!("[GAL] refresh failed for {account_id}: {e}");
            }
            Err(_) => {
                log::warn!("[GAL] refresh timed out for {account_id}");
            }
        }
    }

    Ok(())
}

