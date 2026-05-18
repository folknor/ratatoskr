//! Attachments roadmap Phase 8a: logical retention-window eviction.
//!
//! Sweeps `attachment_blobs` for hashes whose every referencing
//! message is older than `window_start_unix`, and tombstones each via
//! `PackStore::tombstone`. Cross-account dedup is preserved by the
//! `NOT EXISTS` correlated subquery: a blob shared by two accounts
//! evicts only when every account's referencing message is out of
//! window, and orphans (no `attachments` row) tombstone unconditionally.
//!
//! Three trigger sites: startup (after PrefetchRuntime install),
//! post-sync (after the existing prefetch sweep), and window-shrink
//! (mirror of the extend hook in `handlers::settings::handle_set`).
//!
//! `PackStore::tombstone` itself is non-transactional (SELECT pack_id
//! -> UPDATE attachment_blobs -> append tombstone log). Concurrent
//! sweeps targeting the same hash can write duplicate log entries,
//! which is harmless because the UPDATE is idempotent on
//! `WHERE tombstoned_at IS NULL` and index-rebuild uses
//! `INSERT OR IGNORE`. The runtime-authority-column / log-wins-on-
//! rebuild contract is preserved.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use db::blob_hash::BlobHash;
use service_api::{EvictionCompleted, Notification};
use service_state::WriteDbState;
use store::PackStore;

use crate::boot_progress::NotificationSender;

/// One paginated query batch. Matches `BACKFILL_PAGE_SIZE` in
/// `prefetch.rs` so the two sweeps live in the same operational
/// neighbourhood.
const PAGE_SIZE: i64 = 256;

/// Which trigger drove this sweep. Reflected on the wire in
/// `EvictionCompleted.trigger`.
#[derive(Debug, Clone, Copy)]
pub enum EvictionTrigger {
    Startup,
    PostSync,
    WindowShrink,
}

impl EvictionTrigger {
    fn as_str(self) -> &'static str {
        match self {
            Self::Startup => "startup",
            Self::PostSync => "post_sync",
            Self::WindowShrink => "window_shrink",
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct EvictionStats {
    pub blobs_tombstoned: u64,
    pub pages_walked:     u64,
    pub superseded:       bool,
}

/// Walk `attachment_blobs` in `content_hash` order, tombstoning any
/// blob whose every referencing message is older than
/// `window_start_unix` (or which has no referencing rows at all).
///
/// `max_pages` caps how many `PAGE_SIZE`-row batches we'll walk per
/// invocation. Use a small value (~4) for post-sync calls to keep
/// latency bounded; the startup pass and the window-shrink trigger
/// can pass a larger value to drain bulk work.
///
/// `epoch_at_start` is the value of `epoch` captured by the caller
/// before spawning. If `epoch` advances mid-sweep, we set
/// `stats.superseded = true` and exit early - a later trigger now
/// owns the up-to-date window.
#[allow(clippy::too_many_arguments)]
pub async fn run_eviction_sweep(
    db: WriteDbState,
    pack_store: Arc<PackStore>,
    notification_tx: NotificationSender,
    service_generation: u32,
    trigger: EvictionTrigger,
    window_start_unix: i64,
    max_pages: usize,
    epoch: Arc<AtomicU64>,
    epoch_at_start: u64,
) -> EvictionStats {
    let mut stats = EvictionStats::default();
    let mut cursor: Option<BlobHash> = None;

    for _ in 0..max_pages {
        if epoch.load(Ordering::Relaxed) != epoch_at_start {
            stats.superseded = true;
            break;
        }

        let cursor_for_query = cursor;
        let rows: Result<Vec<BlobHash>, String> = db
            .with_write(move |conn| {
                // `messages.date` is stored as a Unix millisecond
                // timestamp (JMAP wire format passes straight through;
                // see `provider_sync::jmap::sync::storage`). The caller
                // supplies `window_start_unix` in SECONDS to match the
                // rest of the codebase's window conventions, so we
                // multiply by 1000 inside the query rather than make
                // every caller think about units.
                let mut stmt = conn
                    .prepare(
                        "SELECT ab.content_hash \
                         FROM attachment_blobs ab \
                         WHERE ab.tombstoned_at IS NULL \
                           AND (?1 IS NULL OR ab.content_hash > ?1) \
                           AND NOT EXISTS ( \
                             SELECT 1 FROM attachments a \
                             JOIN messages m \
                               ON m.account_id = a.account_id \
                              AND m.id = a.message_id \
                             WHERE a.content_hash = ab.content_hash \
                               AND m.date >= ?2 * 1000 \
                           ) \
                         ORDER BY ab.content_hash \
                         LIMIT ?3",
                    )
                    .map_err(|e| format!("prepare eviction sweep: {e}"))?;
                let it = stmt
                    .query_map(
                        rusqlite::params![cursor_for_query, window_start_unix, PAGE_SIZE],
                        |row| row.get::<_, BlobHash>(0),
                    )
                    .map_err(|e| format!("query eviction sweep: {e}"))?;
                let mut out = Vec::new();
                for r in it {
                    out.push(r.map_err(|e| format!("row eviction sweep: {e}"))?);
                }
                Ok(out)
            })
            .await;

        let rows = match rows {
            Ok(r) => r,
            Err(e) => {
                log::warn!("eviction sweep ({}): query failed: {e}", trigger.as_str());
                break;
            }
        };

        if rows.is_empty() {
            break;
        }

        stats.pages_walked = stats.pages_walked.saturating_add(1);
        cursor = rows.last().copied();

        for hash in rows {
            if let Err(e) = pack_store.tombstone(&hash).await {
                log::warn!(
                    "eviction sweep ({}): tombstone {} failed: {e}",
                    trigger.as_str(),
                    hash.to_hex()
                );
                continue;
            }
            stats.blobs_tombstoned = stats.blobs_tombstoned.saturating_add(1);
        }
    }

    let payload = EvictionCompleted {
        service_generation,
        trigger: trigger.as_str().to_string(),
        blobs_tombstoned: stats.blobs_tombstoned,
        pages_walked: stats.pages_walked,
        superseded: stats.superseded,
    };
    if let Err(e) = notification_tx
        .send(Notification::EvictionCompleted(payload))
        .await
    {
        log::warn!("eviction sweep ({}): notification send failed: {e}", trigger.as_str());
    }

    log::debug!(
        "eviction sweep ({}): tombstoned={} pages={} superseded={}",
        trigger.as_str(),
        stats.blobs_tombstoned,
        stats.pages_walked,
        stats.superseded
    );

    stats
}

/// Compute the inclusive lower-bound `messages.date` value for an
/// `n`-day retention window relative to wallclock now.
pub fn window_start_unix(days: i64) -> i64 {
    chrono::Utc::now()
        .timestamp()
        .saturating_sub(days.saturating_mul(86_400))
}
