//! Attachments roadmap Phase 8b: physical GC orchestration.
//!
//! `PackStore::gc(density_threshold)` is the primitive - this module
//! decides when to call it. Two triggers today:
//!
//!   1. Startup, immediately after the Phase 8a eviction sweep
//!      finishes. Reclaims bytes from packs that were tombstoned
//!      during a previous session but never compacted.
//!   2. Post-eviction (window-shrink chain): when the eviction sweep
//!      fired by `kick_window_shrink` reports
//!      `blobs_tombstoned > 0`, run GC inside the same detached
//!      tokio task so the freshly-tombstoned bytes are actually
//!      reclaimed instead of waiting until next startup.
//!
//! Post-sync eviction does NOT chain to GC: it evicts at most 4 pages
//! × 256 = 1024 blobs and is a steady-state drip. Running GC on every
//! sync would add latency for very little reclaim; the startup pass
//! and window-shrink chain catch the bulk work.

use std::sync::Arc;

use service_api::{GcCompleted, Notification};
use store::{GcStats, PackStore};

use crate::boot_progress::NotificationSender;

/// Per-pack dead-frame ratio at or above which `PackStore::gc` will
/// repack the pack. 0.25 matches the roadmap's "25% of any single
/// pack" trigger.
pub const DEFAULT_DENSITY_THRESHOLD: f32 = 0.25;

#[derive(Debug, Clone, Copy)]
pub enum GcTrigger {
    Startup,
    PostEviction,
    /// Phase 8c "Clear cache now" chained-GC pass: the user just
    /// bulk-tombstoned every blob, this GC pass physically reclaims
    /// them. Distinct from `PostEviction` so UI consumers and logs
    /// can tell apart a user-initiated wipe from a routine
    /// window-shrink eviction.
    ClearCache,
}

impl GcTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::PostEviction => "post_eviction",
            Self::ClearCache => "clear_cache",
        }
    }
}

pub async fn run_gc_pass(
    pack_store: Arc<PackStore>,
    notification_tx: NotificationSender,
    service_generation: u32,
    trigger: GcTrigger,
    density_threshold: f32,
) -> GcStats {
    let stats = match pack_store.gc(density_threshold).await {
        Ok(s) => s,
        Err(e) => {
            log::warn!("gc pass ({}): {e}", trigger.as_str());
            GcStats::default()
        }
    };
    let payload = GcCompleted {
        service_generation,
        trigger: trigger.as_str().to_string(),
        packs_compacted: stats.packs_compacted,
        blobs_dropped: stats.blobs_dropped,
        bytes_reclaimed: stats.bytes_reclaimed,
    };
    if let Err(e) = notification_tx
        .send(Notification::GcCompleted(payload))
        .await
    {
        log::warn!("gc pass ({}): notification send failed: {e}", trigger.as_str());
    }
    log::debug!(
        "gc pass ({}): packs_compacted={} blobs_dropped={} bytes_reclaimed={}",
        trigger.as_str(),
        stats.packs_compacted,
        stats.blobs_dropped,
        stats.bytes_reclaimed,
    );
    stats
}
