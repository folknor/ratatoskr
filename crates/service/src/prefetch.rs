//! Phase 4 (attachments roadmap): `PrefetchRuntime` - Service-side
//! attachment-bytes pre-fetch worker.
//!
//! Modeled on `ExtractRuntime`. Differs in:
//! - Two priority queues: sync-time (capacity 64, drained first) and
//!   backfill (capacity 256). A biased `tokio::select!` ensures live
//!   sync-time work never starves under historical backfill.
//! - Per-account `Semaphore` (4 permits) - caps concurrent in-flight
//!   provider calls per account so a chatty inbox doesn't monopolise
//!   the tokio runtime or the provider's rate limit.
//! - Per-account circuit breaker: K=5 consecutive timeouts inside W=60s
//!   opens the circuit; backoff doubles from 30s to a 5min cap. Open
//!   accounts skip with `SkipReason::CircuitOpen` instead of fetching.
//! - ENOSPC backstop: every fetch checks `statvfs` against
//!   `MIN_DISK_FREE_GB`; below it, the fetch is skipped with
//!   `SkipReason::DiskLow` and logged. No `attachments` row is
//!   mutated.
//!
//! The runtime is provider-agnostic. Phase 4 only wires JMAP enqueue
//! sites (post-sync sweep + backfill kick); Phase 7 adds the others
//! without touching this file. A non-JMAP `PrefetchWork` would still
//! process correctly - `create_provider` dispatches by account row -
//! but Phase 4 doesn't enqueue any.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use service_api::{Notification, PrefetchCompleted, PrefetchProgress};
use service_state::WriteDbState;
use tokio::sync::{Mutex, Semaphore, mpsc};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::boot_progress::NotificationSender;

/// Sync-time queue capacity (live work, drained first).
const SYNC_QUEUE_CAPACITY: usize = 64;
/// Backfill queue capacity (historical work, drained second).
const BACKFILL_QUEUE_CAPACITY: usize = 256;
/// Per-account concurrent provider-fetch cap.
const PER_ACCOUNT_PERMITS: usize = 4;
/// Hard cap on the in-flight dedupe set. When exceeded, oldest entries
/// are evicted FIFO - a re-enqueue of the same `(account, attachment)`
/// will pass dedupe and be processed twice. Acceptable: PackStore::put
/// is idempotent on content hash.
const IN_FLIGHT_CAP: usize = 10_000;
/// Per-fetch wallclock cap. A provider call exceeding this counts as a
/// timeout for circuit-breaker purposes.
const PER_FETCH_TIMEOUT_SECS: u64 = 5 * 60;
/// Circuit-breaker trip threshold: K consecutive timeouts within W
/// seconds.
const CIRCUIT_BREAKER_K: u32 = 5;
const CIRCUIT_BREAKER_W: Duration = Duration::from_secs(60);
/// Circuit-breaker backoff bounds. The breaker doubles from MIN to MAX
/// for each successive trip while the account stays unhealthy.
const CIRCUIT_BREAKER_BACKOFF_MIN: Duration = Duration::from_secs(30);
const CIRCUIT_BREAKER_BACKOFF_MAX: Duration = Duration::from_secs(5 * 60);
/// Disk-free threshold for the ENOSPC backstop. Below this, prefetch
/// skips writes rather than fill the disk.
const MIN_DISK_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;
/// Backfill page size when `kick_backfill_account` walks historical
/// rows.
const BACKFILL_PAGE_SIZE: i64 = 256;
/// Safety ceiling on `kick_backfill_account` page iterations. With a
/// 256-row page size this caps a single kick at ~32k attachments. Real
/// mailboxes inside a 1-year window stay well below; the cap exists
/// to bound pathological "every fetch fails, content_hash stays NULL"
/// loops that otherwise burn the write-conn mutex.
const MAX_BACKFILL_PAGES: u32 = 128;
/// Minimum wallclock spacing between two `PrefetchProgress`
/// notifications. With a 1000-row backfill the unthrottled path emits
/// 1000 mpsc sends before the downstream `Coalesce` collapses them;
/// throttling cuts this to ~10 events even though the underlying
/// counters still tick per-item. `PrefetchCompleted` drained-to-zero
/// fires unconditionally regardless of throttle.
const PROGRESS_THROTTLE_MS: u64 = 100;
/// Window cap on the post-sync sweep - the most recent N attachments
/// for the account whose hash is still NULL. Public so `sync.rs`
/// passes the same bound the runtime documents.
pub(crate) const SYNC_SWEEP_LIMIT: i64 = 64;

/// Priority lane a `PrefetchWork` belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefetchPriority {
    /// Live, fired by the post-sync hook for newly-arrived messages.
    Sync,
    /// Historical, fired by boot recovery, account-add, and
    /// window-extend.
    Backfill,
}

/// Outcome of a single `enqueue` call. The `kick_backfill_account`
/// pagination logic distinguishes "row was actually placed on the
/// channel" from "row was already in flight." Counting both as a hit
/// causes the kick loop to spin when workers haven't drained yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EnqueueOutcome {
    /// New (account, attachment) key; placed on the mpsc.
    Newly,
    /// Already in the in-flight set; not re-sent.
    Dedupe,
}

/// One attachment to fetch.
#[derive(Debug, Clone)]
pub struct PrefetchItem {
    pub account_id: String,
    pub message_id: String,
    /// Local `attachments.id` (PK).
    pub attachment_id: String,
    /// Provider-side blob/part identifier
    /// (`attachments.remote_attachment_id`). Resolved at enqueue time
    /// so the worker doesn't need to revisit the row before calling
    /// `fetch_attachment`. IMAP sync writes the part path into this
    /// same column, so a single source of truth covers every provider.
    pub remote_attachment_id: String,
    /// Provider type for the account (`"jmap"`, `"gmail"`, `"graph"`,
    /// `"imap"`). Resolved at enqueue time so the worker doesn't have
    /// to re-query the accounts row. Drives per-provider concurrency
    /// caps and the breaker keyspace introduced by Phase 7.
    pub provider: String,
}

/// Unit of work on the prefetch queues. Most providers produce one
/// item per attachment. IMAP groups items by folder so the worker can
/// reuse a single `SELECT` across the batch - Phase 7 of the
/// attachments roadmap.
#[derive(Debug, Clone)]
pub enum PrefetchWork {
    /// Per-attachment work. Used by every provider on the cache-miss
    /// `attachment.fetch` path and by JMAP/Gmail/Graph prefetch.
    Item(PrefetchItem),
    /// IMAP folder-batch: every item in the vec shares
    /// `(account_id, folder_id)`. Drained serially with one IMAP
    /// session held across all items.
    ImapBatch {
        account_id: String,
        folder_id: String,
        items: Vec<PrefetchItem>,
    },
}

impl PrefetchWork {
    fn account_id(&self) -> &str {
        match self {
            Self::Item(it) => &it.account_id,
            Self::ImapBatch { account_id, .. } => account_id,
        }
    }
    fn item_count(&self) -> usize {
        match self {
            Self::Item(_) => 1,
            Self::ImapBatch { items, .. } => items.len(),
        }
    }
    fn item_keys(&self) -> Vec<InFlightKey> {
        match self {
            Self::Item(it) => vec![(it.account_id.clone(), it.attachment_id.clone())],
            Self::ImapBatch { items, .. } => items
                .iter()
                .map(|it| (it.account_id.clone(), it.attachment_id.clone()))
                .collect(),
        }
    }
}

/// Why a work item finished without writing bytes. Bumps the runtime's
/// skip counter; never poisons the queue.
#[derive(Debug, Clone, Copy)]
pub enum SkipReason {
    /// The account-scoped circuit breaker is open. Work is dropped;
    /// the next backfill kick will re-emit.
    CircuitOpen,
    /// Attachments roadmap Phase 6: the per-account
    /// `cache_attachments_enabled` flag is `0`. The row stays
    /// `content_hash IS NULL` until the user re-enables the toggle,
    /// at which point the next sync's post-sync sweep / next boot's
    /// recovery kick picks it up.
    AccountDisabled,
    /// Free disk below `MIN_DISK_FREE_BYTES`.
    DiskLow,
    /// The row's `content_hash` is no longer NULL (another path won
    /// the race - typically a user-initiated `attachment.fetch`).
    AlreadyCached,
    /// The attachment row vanished between enqueue and dispatch (cascading
    /// delete from a message or account drop).
    RowGone,
    /// 5min provider timeout. Feeds the circuit breaker.
    ProviderTimeout,
    /// Provider failure expected to recover on its own (network blip,
    /// 5xx, rate limit). Logged, counted, not retried in-session; the
    /// next backfill kick will re-emit.
    ProviderTransient,
    /// Provider failure that will keep failing without external state
    /// changing (expired token, 404, 4xx). Same in-session handling as
    /// `ProviderTransient` for now (recorded as a failure, no breaker
    /// feed) but split out so logs / future skip-attempt logic can
    /// distinguish "retry me" from "stop trying."
    ProviderPermanent,
    /// PackStore::put failed. Logged; counted as failed.
    PackStoreError,
    /// DB update failed after a successful fetch. The bytes are in
    /// PackStore (dedupe-safe) but `attachments.content_hash` is still
    /// NULL; the next backfill kick will re-fetch and `PackStore::put`
    /// will dedupe to a no-op, then the UPDATE re-runs.
    DbUpdateError,
}

impl SkipReason {
    fn is_timeout(self) -> bool { matches!(self, Self::ProviderTimeout) }
    fn is_failure(self) -> bool {
        matches!(
            self,
            Self::PackStoreError
                | Self::DbUpdateError
                | Self::ProviderTransient
                | Self::ProviderPermanent
        )
    }
}

/// Per-account circuit-breaker state. Closed by default; trips open
/// after `CIRCUIT_BREAKER_K` timeouts within `CIRCUIT_BREAKER_W`.
#[derive(Debug, Default)]
struct BreakerState {
    /// Wallclock instants of recent timeouts. Older than W are pruned.
    timeouts:           VecDeque<Instant>,
    /// When the circuit was last tripped open. While `Some`, fetches
    /// for this account skip until `tripped_at + current_backoff`.
    tripped_at:         Option<Instant>,
    /// Current backoff window. Doubles each trip up to MAX.
    current_backoff:    Duration,
    /// Total trips this incarnation. Bumped each time we transition
    /// Closed -> Open. Drives backoff doubling.
    consecutive_trips:  u32,
}

impl BreakerState {
    /// Returns `true` if the breaker is currently open (skip the
    /// fetch). Cleans expired trip state.
    fn is_open(&mut self) -> bool {
        let Some(tripped) = self.tripped_at else {
            return false;
        };
        if tripped.elapsed() >= self.current_backoff {
            // Half-open: clear trip; the next failure re-trips with a
            // longer backoff.
            self.tripped_at = None;
            self.timeouts.clear();
            return false;
        }
        true
    }

    /// Record a timeout. May transition Closed -> Open.
    fn note_timeout(&mut self) {
        let now = Instant::now();
        // Prune timeouts older than W.
        while let Some(front) = self.timeouts.front() {
            if now.duration_since(*front) > CIRCUIT_BREAKER_W {
                self.timeouts.pop_front();
            } else {
                break;
            }
        }
        self.timeouts.push_back(now);
        if self.tripped_at.is_none() && self.timeouts.len() >= CIRCUIT_BREAKER_K as usize {
            self.tripped_at = Some(now);
            self.consecutive_trips = self.consecutive_trips.saturating_add(1);
            self.current_backoff = backoff_for(self.consecutive_trips);
        }
    }

    /// Record a success. Resets the consecutive-trip counter.
    fn note_success(&mut self) {
        self.timeouts.clear();
        self.tripped_at = None;
        self.consecutive_trips = 0;
        self.current_backoff = CIRCUIT_BREAKER_BACKOFF_MIN;
    }
}

fn backoff_for(trips: u32) -> Duration {
    // Trip #1 -> MIN, #2 -> 2*MIN, capped at MAX.
    let mult = 1u64 << (trips.saturating_sub(1).min(8) as u64);
    let secs = CIRCUIT_BREAKER_BACKOFF_MIN
        .as_secs()
        .saturating_mul(mult);
    let bounded = secs.min(CIRCUIT_BREAKER_BACKOFF_MAX.as_secs());
    Duration::from_secs(bounded)
}

/// `(account_id, attachment_id)` pair - the dedup key on the
/// in-flight set, also recorded in a FIFO queue for capped eviction.
type InFlightKey = (String, String);
/// Membership set + FIFO eviction queue. The two are kept locked
/// together so `IN_FLIGHT_CAP` can be enforced without a separate
/// stop-the-world scan.
type InFlightGuard = (HashSet<InFlightKey>, VecDeque<InFlightKey>);

pub(crate) struct PrefetchRuntimeInner {
    closed:               AtomicBool,
    in_flight:            Mutex<InFlightGuard>,
    sync_tx:              mpsc::Sender<PrefetchWork>,
    backfill_tx:          mpsc::Sender<PrefetchWork>,
    db:                   WriteDbState,
    boot_state:           Arc<crate::boot::BootSharedState>,
    notification_tx:      NotificationSender,
    service_generation:   u32,
    cancellation:         CancellationToken,
    worker_handle:        std::sync::Mutex<Option<JoinHandle<()>>>,
    queue_depth:          AtomicU64,
    fetched_count:        AtomicU64,
    skipped_count:        AtomicU64,
    failed_count:         AtomicU64,
    /// Last wallclock at which `finalize_item` emitted a
    /// `PrefetchProgress` notification. Throttles the per-item firehose
    /// down to one event per `PROGRESS_THROTTLE` interval; the
    /// `PrefetchCompleted` drained-to-zero event still fires unthrottled.
    last_progress_emit_ms: AtomicU64,
    /// Counter snapshot mutex used when building `PrefetchCompleted`.
    /// Without this, fetched/skipped/failed read at different instants
    /// can disagree by O(in-flight) and the ack totals look impossible.
    counters_snapshot:    Mutex<()>,
    /// Per-account semaphore cache. Populated lazily on first dispatch
    /// for a new account_id. Capped implicitly by total accounts.
    account_semaphores:   Mutex<HashMap<String, Arc<Semaphore>>>,
    /// Circuit breakers keyed by `(provider, account_id)`. Today the
    /// two-tuple is practically equivalent to a per-account map since
    /// every account has exactly one provider, but threading provider
    /// through the breaker call sites lets a later promotion to
    /// per-provider-only land without churning every caller.
    breakers:             Mutex<HashMap<(String, String), BreakerState>>,
    /// Account ids whose deletion is in flight. `cancel_account`
    /// inserts; `run_pipeline` checks before PackStore::put so a queued
    /// or in-flight prefetch can't land bytes after the
    /// `AttachmentCache` deletion-step snapshotted hashes.
    cancelling_accounts:  Mutex<HashSet<String>>,
}

#[derive(Clone)]
pub struct PrefetchRuntime {
    inner: Arc<PrefetchRuntimeInner>,
}

impl PrefetchRuntime {
    pub fn new(
        db: WriteDbState,
        boot_state: Arc<crate::boot::BootSharedState>,
        notification_tx: NotificationSender,
        service_generation: u32,
        cancellation: CancellationToken,
    ) -> Self {
        let (sync_tx, sync_rx) = mpsc::channel::<PrefetchWork>(SYNC_QUEUE_CAPACITY);
        let (backfill_tx, backfill_rx) =
            mpsc::channel::<PrefetchWork>(BACKFILL_QUEUE_CAPACITY);
        let inner = Arc::new(PrefetchRuntimeInner {
            closed:             AtomicBool::new(false),
            in_flight:          Mutex::new((HashSet::new(), VecDeque::new())),
            sync_tx,
            backfill_tx,
            db,
            boot_state,
            notification_tx,
            service_generation,
            cancellation,
            worker_handle:      std::sync::Mutex::new(None),
            queue_depth:        AtomicU64::new(0),
            fetched_count:      AtomicU64::new(0),
            skipped_count:      AtomicU64::new(0),
            failed_count:       AtomicU64::new(0),
            last_progress_emit_ms: AtomicU64::new(0),
            counters_snapshot:  Mutex::new(()),
            account_semaphores: Mutex::new(HashMap::new()),
            breakers:           Mutex::new(HashMap::new()),
            cancelling_accounts: Mutex::new(HashSet::new()),
        });
        let inner_for_worker = Arc::clone(&inner);
        let handle = tokio::spawn(async move {
            run_worker(inner_for_worker, sync_rx, backfill_rx).await;
        });
        *inner.worker_handle.lock().expect("worker_handle poisoned") = Some(handle);
        Self { inner }
    }

    /// Internal: enqueue a single work item on the chosen priority
    /// lane. Callers reach this via `enqueue_window_for_account` or
    /// `kick_backfill_account`; direct single-item enqueue is reserved
    /// for future per-attachment hooks (Phase 7+).
    pub(crate) async fn enqueue(
        &self,
        work: PrefetchWork,
        priority: PrefetchPriority,
    ) -> Result<EnqueueOutcome, String> {
        if self.inner.closed.load(Ordering::Relaxed) {
            return Err("PrefetchRuntime is shutting down".into());
        }
        if self
            .inner
            .cancelling_accounts
            .lock()
            .await
            .contains(work.account_id())
        {
            return Ok(EnqueueOutcome::Dedupe);
        }

        // Reduce the work unit to the items that aren't already in
        // flight. For an `Item`, this is at most one survivor; for an
        // `ImapBatch`, we drop dupes per-item and keep the rest.
        let work = {
            let mut guard = self.inner.in_flight.lock().await;
            let (set, fifo) = &mut *guard;
            let mut accepted: Vec<InFlightKey> = Vec::new();
            let surviving = match work {
                PrefetchWork::Item(it) => {
                    let key = (it.account_id.clone(), it.attachment_id.clone());
                    if !set.insert(key.clone()) {
                        return Ok(EnqueueOutcome::Dedupe);
                    }
                    accepted.push(key);
                    PrefetchWork::Item(it)
                }
                PrefetchWork::ImapBatch { account_id, folder_id, items } => {
                    let mut kept = Vec::with_capacity(items.len());
                    for it in items {
                        let key = (it.account_id.clone(), it.attachment_id.clone());
                        if set.insert(key.clone()) {
                            accepted.push(key);
                            kept.push(it);
                        }
                    }
                    if kept.is_empty() {
                        return Ok(EnqueueOutcome::Dedupe);
                    }
                    PrefetchWork::ImapBatch { account_id, folder_id, items: kept }
                }
            };
            for key in &accepted {
                fifo.push_back(key.clone());
            }
            // Evict-oldest if we've blown the cap. The evicted pair is
            // no longer dedupe-protected; a re-enqueue will pass. This
            // is fine - PackStore::put is content-hash idempotent.
            while set.len() > IN_FLIGHT_CAP {
                if let Some(old) = fifo.pop_front() {
                    set.remove(&old);
                } else {
                    break;
                }
            }
            surviving
        };

        let added = work.item_count() as u64;
        self.inner.queue_depth.fetch_add(added, Ordering::Relaxed);
        let tx = match priority {
            PrefetchPriority::Sync     => &self.inner.sync_tx,
            PrefetchPriority::Backfill => &self.inner.backfill_tx,
        };
        if let Err(e) = tx.send(work.clone()).await {
            self.inner.queue_depth.fetch_sub(added, Ordering::Relaxed);
            // Roll back the in-flight tracker so a future re-enqueue
            // isn't dedupe-suppressed and the FIFO doesn't carry
            // phantom entries past the next cap eviction.
            let mut guard = self.inner.in_flight.lock().await;
            let (set, fifo) = &mut *guard;
            for key in work.item_keys() {
                set.remove(&key);
                fifo.retain(|k| k != &key);
            }
            return Err(format!("PrefetchRuntime worker exited: {e}"));
        }
        Ok(EnqueueOutcome::Newly)
    }

    /// Walk `attachments` for `account_id` joining against `messages`
    /// for date filtering, restrict to rows with `content_hash IS NULL`
    /// and `messages.date >= window_start`. Each result is enqueued on
    /// the chosen priority lane. The sweep is bounded by `limit` (None
    /// = unbounded; backfill paginates via repeated calls).
    ///
    /// Returns `(rows_returned_by_query, newly_enqueued)`. The two
    /// differ when in-flight dedupe suppresses a re-query. Callers
    /// paginating off this function must use `rows_returned_by_query`
    /// as the termination signal - using `newly_enqueued` causes a
    /// spin when workers haven't drained the previous page yet.
    pub async fn enqueue_window_for_account(
        &self,
        account_id: &str,
        provider: &str,
        window_start_unix: i64,
        priority: PrefetchPriority,
        limit: Option<i64>,
    ) -> Result<(u64, u64), String> {
        let aid = account_id.to_string();
        let lim = limit.unwrap_or(i64::MAX);
        // For IMAP we also pull the message folder so the worker can
        // group items into per-folder batches and reuse a single
        // session across them.
        let rows: Vec<(String, String, Option<String>, Option<String>)> = self
            .inner
            .db
            .with_conn(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT a.id, a.message_id, a.remote_attachment_id, m.imap_folder \
                         FROM attachments a \
                         JOIN messages m \
                           ON m.account_id = a.account_id AND m.id = a.message_id \
                         WHERE a.account_id = ?1 \
                           AND a.content_hash IS NULL \
                           AND COALESCE(a.is_inline, 0) = 0 \
                           AND m.date >= ?2 \
                         ORDER BY m.date DESC \
                         LIMIT ?3",
                    )
                    .map_err(|e| format!("prepare prefetch sweep: {e}"))?;
                let it = stmt
                    .query_map(
                        rusqlite::params![aid, window_start_unix, lim],
                        |row| {
                            Ok((
                                row.get::<_, String>(0)?,
                                row.get::<_, String>(1)?,
                                row.get::<_, Option<String>>(2)?,
                                row.get::<_, Option<String>>(3)?,
                            ))
                        },
                    )
                    .map_err(|e| format!("query prefetch sweep: {e}"))?;
                let mut out = Vec::new();
                for r in it {
                    out.push(r.map_err(|e| format!("row prefetch sweep: {e}"))?);
                }
                Ok(out)
            })
            .await?;

        let row_count = rows.len() as u64;
        let mut newly = 0u64;

        if provider == "imap" {
            // Group IMAP rows by folder so the worker can SELECT once
            // per batch and reuse the session for every UID inside it.
            // Rows missing imap_folder fall back to per-item dispatch
            // (this should never happen for IMAP-synced messages but
            // we keep the safety net rather than dropping rows).
            let mut by_folder: HashMap<String, Vec<PrefetchItem>> = HashMap::new();
            let mut orphans: Vec<PrefetchItem> = Vec::new();
            for (attachment_id, message_id, remote, folder) in rows {
                let remote_attachment_id = match remote {
                    Some(s) if !s.is_empty() => s,
                    _ => continue,
                };
                let item = PrefetchItem {
                    account_id: account_id.to_string(),
                    message_id,
                    attachment_id,
                    remote_attachment_id,
                    provider: provider.to_string(),
                };
                match folder {
                    Some(f) if !f.is_empty() => by_folder.entry(f).or_default().push(item),
                    _ => orphans.push(item),
                }
            }
            for (folder_id, items) in by_folder {
                let work = PrefetchWork::ImapBatch {
                    account_id: account_id.to_string(),
                    folder_id,
                    items,
                };
                if let Ok(EnqueueOutcome::Newly) = self.enqueue(work, priority).await {
                    newly += 1;
                }
            }
            for it in orphans {
                if let Ok(EnqueueOutcome::Newly) = self
                    .enqueue(PrefetchWork::Item(it), priority)
                    .await
                {
                    newly += 1;
                }
            }
        } else {
            for (attachment_id, message_id, remote, _folder) in rows {
                let remote_attachment_id = match remote {
                    Some(s) if !s.is_empty() => s,
                    _ => continue,
                };
                let work = PrefetchWork::Item(PrefetchItem {
                    account_id: account_id.to_string(),
                    message_id,
                    attachment_id,
                    remote_attachment_id,
                    provider: provider.to_string(),
                });
                if let Ok(EnqueueOutcome::Newly) = self.enqueue(work, priority).await {
                    newly += 1;
                }
            }
        }
        Ok((row_count, newly))
    }

    /// Walk every page of `account_id`'s NULL-hash attachments inside
    /// the given window. Paginates via repeated bounded sweeps. Used
    /// by boot recovery, account-add, and window-extend triggers.
    /// Fire-and-forget: the worker drains asynchronously.
    ///
    /// Termination: the loop exits when the SQL query returns fewer
    /// rows than the page limit (genuine drain) OR when an entire page
    /// is dedupe-suppressed (workers haven't completed the prior page;
    /// next kick will pick up the rest). A safety ceiling of
    /// `MAX_BACKFILL_PAGES` caps pathological loops where rows keep
    /// failing without `content_hash` being populated.
    pub async fn kick_backfill_account(
        &self,
        account_id: &str,
        provider: &str,
        window_start_unix: i64,
    ) -> Result<u64, String> {
        let mut total = 0u64;
        let mut pages = 0u32;
        loop {
            let (rows, newly) = self
                .enqueue_window_for_account(
                    account_id,
                    provider,
                    window_start_unix,
                    PrefetchPriority::Backfill,
                    Some(BACKFILL_PAGE_SIZE),
                )
                .await?;
            total += newly;
            pages = pages.saturating_add(1);
            if rows < BACKFILL_PAGE_SIZE as u64 {
                break;
            }
            // Every row was dedupe-suppressed - workers haven't drained
            // the previous page. Yield; the next kick (or post-sync
            // sweep) catches the rest.
            if newly == 0 {
                log::debug!(
                    "kick_backfill_account({account_id}): full page dedupe-suppressed; \
                     yielding after {pages} page(s)",
                );
                break;
            }
            if pages >= MAX_BACKFILL_PAGES {
                log::warn!(
                    "kick_backfill_account({account_id}): hit MAX_BACKFILL_PAGES ({pages}); \
                     yielding to next kick",
                );
                break;
            }
        }
        Ok(total)
    }

    /// Emit a `PrefetchCompleted` notification if the runtime is
    /// currently idle (queue empty AND in-flight set empty). Used by
    /// the window-extend kick (and any other batch caller) so a kick
    /// that enqueues zero rows still produces an observable
    /// "kick done" event. When the runtime is busy, this is a no-op:
    /// the natural drained-to-zero path inside `finalize_item` will
    /// fire `PrefetchCompleted` once the work completes.
    pub async fn emit_completed_if_idle(&self) {
        let queue_empty = self.inner.queue_depth.load(Ordering::Relaxed) == 0;
        let in_flight_empty = self.inner.in_flight.lock().await.0.is_empty();
        if !(queue_empty && in_flight_empty) {
            return;
        }
        let _snap = self.inner.counters_snapshot.lock().await;
        let completed = Notification::PrefetchCompleted(PrefetchCompleted {
            service_generation: self.inner.service_generation,
            fetched: self.inner.fetched_count.load(Ordering::Relaxed),
            skipped: self.inner.skipped_count.load(Ordering::Relaxed),
            failed:  self.inner.failed_count.load(Ordering::Relaxed),
        });
        if let Err(e) = self.inner.notification_tx.send(completed).await {
            log::debug!("PrefetchRuntime emit_completed_if_idle send failed: {e}");
        }
    }

    /// Mark `account_id` as cancelling and drop every queued work
    /// item for it. Subsequent `enqueue` calls short-circuit until
    /// `release_account` clears the marker. Provider fetches already
    /// in flight cannot be aborted, but `run_pipeline` checks the
    /// cancelling-set before `PackStore::put` and before the
    /// `attachments.content_hash` UPDATE so they can no longer leak
    /// blobs past the `AttachmentCache` deletion-step snapshot.
    ///
    /// Used by `account.delete` (cancellation persists for the
    /// lifetime of the deletion). For Phase 4's "cancel the queue
    /// but the account isn't really going away" callers there is no
    /// release path; until those exist, every caller pairs with
    /// `release_account` or accepts permanent skipping.
    pub async fn cancel_account(&self, account_id: &str) {
        self.inner
            .cancelling_accounts
            .lock()
            .await
            .insert(account_id.to_string());
        // The in-flight set still owns the (account, attachment) keys;
        // we evict them so a subsequent backfill against a freshly-
        // recreated account-id wouldn't be silently dedupe-suppressed.
        let mut guard = self.inner.in_flight.lock().await;
        let (set, fifo) = &mut *guard;
        set.retain(|(a, _)| a != account_id);
        fifo.retain(|(a, _)| a != account_id);
    }

    /// Reverse `cancel_account`. Currently unused; reserved for a
    /// future "abort the queued backfill but keep the account" caller
    /// that account-delete does not need.
    #[allow(dead_code)]
    pub async fn release_account(&self, account_id: &str) {
        self.inner
            .cancelling_accounts
            .lock()
            .await
            .remove(account_id);
    }



    /// Begin shutdown. Idempotent. Mirrors `ExtractRuntime::shutdown`.
    pub async fn shutdown(&self) {
        self.inner.closed.store(true, Ordering::Relaxed);
        self.inner.cancellation.cancel();
        let handle = self
            .inner
            .worker_handle
            .lock()
            .expect("worker_handle poisoned during shutdown")
            .take();
        if let Some(h) = handle
            && let Err(e) = h.await
        {
            log::warn!("PrefetchRuntime worker join error during shutdown: {e}");
        }
    }
}

async fn run_worker(
    inner: Arc<PrefetchRuntimeInner>,
    mut sync_rx: mpsc::Receiver<PrefetchWork>,
    mut backfill_rx: mpsc::Receiver<PrefetchWork>,
) {
    let cancellation = inner.cancellation.clone();
    // JoinSet so the worker can abort + await per-item tasks on
    // cancellation, mirroring ExtractRuntime's H1 fix.
    let mut tasks: tokio::task::JoinSet<()> = tokio::task::JoinSet::new();

    loop {
        tokio::select! {
            biased;
            () = cancellation.cancelled() => {
                log::debug!("PrefetchRuntime cancelled; draining {} in-flight tasks", tasks.len());
                break;
            }
            // Drain finished per-item tasks so the JoinSet doesn't grow
            // unboundedly.
            Some(result) = tasks.join_next(), if !tasks.is_empty() => {
                if let Err(e) = result
                    && e.is_panic()
                {
                    log::error!("PrefetchRuntime per-item task panicked: {e}");
                }
            }
            Some(work) = sync_rx.recv() => {
                spawn_one(&inner, &mut tasks, work).await;
            }
            Some(work) = backfill_rx.recv(), if sync_rx.is_empty() => {
                spawn_one(&inner, &mut tasks, work).await;
            }
            else => break,
        }
    }
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
}

async fn spawn_one(
    inner: &Arc<PrefetchRuntimeInner>,
    tasks: &mut tokio::task::JoinSet<()>,
    work: PrefetchWork,
) {
    let inner_for_task = Arc::clone(inner);
    tasks.spawn(async move {
        process_one(inner_for_task, work).await;
    });
}

async fn process_one(inner: Arc<PrefetchRuntimeInner>, work: PrefetchWork) {
    match work {
        PrefetchWork::Item(item) => process_item(inner, item).await,
        PrefetchWork::ImapBatch { account_id, folder_id, items } => {
            process_imap_batch(inner, account_id, folder_id, items).await;
        }
    }
}

async fn process_item(inner: Arc<PrefetchRuntimeInner>, item: PrefetchItem) {
    // Per-account permit. Acquired first so the circuit-breaker check
    // and disk check are serialized through the same gate that bounds
    // outbound provider load.
    let semaphore = account_semaphore(&inner, &item.account_id, &item.provider).await;
    let _permit = match semaphore.acquire_owned().await {
        Ok(p) => p,
        Err(_) => {
            record_item_outcome(&inner, &item, Err(SkipReason::ProviderTransient)).await;
            return;
        }
    };
    let outcome = run_item_pipeline(&inner, &item, ItemFetch::ViaProvider).await;
    record_item_outcome(&inner, &item, outcome).await;
}

/// IMAP folder-batch: hold the per-account semaphore once, open one
/// session, SELECT the folder once, drain every item over the same
/// connection. Each item still runs through the full pre/post-fetch
/// invariant set (RowGone, AlreadyCached, account-deletion race), so a
/// batch can land partial results without compromising correctness.
async fn process_imap_batch(
    inner: Arc<PrefetchRuntimeInner>,
    account_id: String,
    folder_id: String,
    items: Vec<PrefetchItem>,
) {
    let semaphore = account_semaphore(&inner, &account_id, "imap").await;
    let _permit = match semaphore.acquire_owned().await {
        Ok(p) => p,
        Err(_) => {
            for item in items {
                record_item_outcome(&inner, &item, Err(SkipReason::ProviderTransient)).await;
            }
            return;
        }
    };

    // Gate the whole batch on shared invariants before paying for a
    // session: the breaker, disk headroom, and per-account toggle
    // apply to every item identically.
    if breaker_is_open(&inner, "imap", &account_id).await {
        for item in items {
            record_item_outcome(&inner, &item, Err(SkipReason::CircuitOpen)).await;
        }
        return;
    }
    if !disk_has_headroom(&inner) {
        for item in items {
            record_item_outcome(&inner, &item, Err(SkipReason::DiskLow)).await;
        }
        return;
    }
    if !account_caching_enabled(&inner, &account_id).await {
        for item in items {
            record_item_outcome(&inner, &item, Err(SkipReason::AccountDisabled)).await;
        }
        return;
    }

    // Load IMAP config + open session. A connect failure aborts the
    // batch; every item is recorded as Transient so the next backfill
    // kick re-emits without tripping the breaker (LOGIN failures land
    // as `Network` once classified through ProviderError::kind, but
    // the raw `String` here doesn't carry the kind - treat it as
    // transient because IMAP servers do drop and recover).
    let read_db = match inner.boot_state.write_db_state() {
        Ok(w) => w.to_read_state(),
        Err(_) => {
            for item in items {
                record_item_outcome(&inner, &item, Err(SkipReason::ProviderTransient)).await;
            }
            return;
        }
    };
    let key = match inner.boot_state.encryption_key() {
        Some(k) => k,
        None => {
            for item in items {
                record_item_outcome(&inner, &item, Err(SkipReason::ProviderTransient)).await;
            }
            return;
        }
    };
    let config = match imap::account_config::load_imap_config(&read_db, &account_id, &key).await {
        Ok(c) => c,
        Err(e) => {
            log::debug!("prefetch imap load_config {account_id}: {e}");
            for item in items {
                record_item_outcome(&inner, &item, Err(SkipReason::ProviderTransient)).await;
            }
            return;
        }
    };
    let mut session = match imap::connection::connect(&config).await {
        Ok(s) => s,
        Err(e) => {
            log::debug!("prefetch imap connect {account_id}: {e}");
            for item in items {
                record_item_outcome(&inner, &item, Err(SkipReason::ProviderTransient)).await;
            }
            return;
        }
    };
    if let Err(e) = tokio::time::timeout(
        Duration::from_secs(30),
        session.select(&folder_id),
    )
    .await
    .map_err(|_| format!("SELECT {folder_id} timed out"))
    .and_then(|r| r.map_err(|e| format!("SELECT {folder_id}: {e}")))
    {
        log::debug!("prefetch imap SELECT {account_id}/{folder_id}: {e}");
        let _ = session.logout().await;
        for item in items {
            record_item_outcome(&inner, &item, Err(SkipReason::ProviderTransient)).await;
        }
        return;
    }

    for item in items {
        if inner.cancellation.is_cancelled() {
            record_item_outcome(&inner, &item, Err(SkipReason::ProviderTransient)).await;
            continue;
        }
        let outcome = run_item_pipeline(
            &inner,
            &item,
            ItemFetch::ImapSession { session: &mut session },
        )
        .await;
        record_item_outcome(&inner, &item, outcome).await;
    }

    let _ = session.logout().await;
}

/// How the fetch step should source the bytes. `ViaProvider` builds a
/// fresh `ProviderOps` per call (the original Phase 4 path);
/// `ImapSession` reuses an already-`SELECT`ed IMAP session held by the
/// caller (Phase 7 batch path).
enum ItemFetch<'a> {
    ViaProvider,
    ImapSession {
        session: &'a mut imap::connection::ImapSession,
    },
}

/// Item-level pipeline: pre-fetch invariants → fetch → post-fetch
/// invariants → PackStore → DB update → ExtractRuntime enqueue.
/// Returns Ok(()) on success or the relevant SkipReason. Counter
/// updates and finalize live in `record_item_outcome`.
async fn run_item_pipeline(
    inner: &Arc<PrefetchRuntimeInner>,
    item: &PrefetchItem,
    fetch: ItemFetch<'_>,
) -> Result<(), SkipReason> {
    // For the per-item (ViaProvider) path these checks are caller
    // serialized via the semaphore; for the batch path we've already
    // gated on them in `process_imap_batch`. Re-checking per item in
    // the batch path is cheap and lets a per-account toggle flip
    // mid-batch take effect at the next item boundary.
    if let ItemFetch::ViaProvider = &fetch {
        if breaker_is_open(inner, &item.provider, &item.account_id).await {
            return Err(SkipReason::CircuitOpen);
        }
        if !disk_has_headroom(inner) {
            return Err(SkipReason::DiskLow);
        }
        if !account_caching_enabled(inner, &item.account_id).await {
            return Err(SkipReason::AccountDisabled);
        }
    }

    // Confirm the row still wants bytes (cache-hit by another path
    // would have populated `content_hash` between enqueue and now).
    let read_db = inner.boot_state.write_db_state()
        .map_err(|_| SkipReason::RowGone)?
        .to_read_state();
    let lookup_account = item.account_id.clone();
    let lookup_message = item.message_id.clone();
    let lookup_attachment = item.attachment_id.clone();
    let info = read_db
        .with_conn(move |conn| {
            db::db::queries_extra::find_attachment_cache_info(
                conn,
                &lookup_account,
                &lookup_message,
                &lookup_attachment,
            )
        })
        .await
        .map_err(|_| SkipReason::RowGone)?;
    let Some(info) = info else {
        return Err(SkipReason::RowGone);
    };
    if info.content_hash.is_some() {
        return Err(SkipReason::AlreadyCached);
    }

    let bytes = match fetch {
        ItemFetch::ViaProvider => {
            let key = inner
                .boot_state
                .encryption_key()
                .ok_or(SkipReason::ProviderTransient)?;
            let provider =
                crate::actions::provider::create_provider(&read_db, &item.account_id, key)
                    .await
                    .map_err(|e| {
                        log::debug!("prefetch create_provider {}: {e}", item.account_id);
                        SkipReason::ProviderTransient
                    })?;
            let provider_ctx = common::types::ProviderCtx {
                account_id: &item.account_id,
                db:         &read_db,
                progress:   &db::progress::NoopProgressReporter,
            };
            let fetch_fut =
                provider.fetch_attachment(&provider_ctx, &item.message_id, &item.remote_attachment_id);
            match tokio::time::timeout(
                Duration::from_secs(PER_FETCH_TIMEOUT_SECS),
                fetch_fut,
            )
            .await
            {
                Ok(Ok(a)) => a.bytes,
                Ok(Err(e)) => {
                    let kind = e.kind();
                    log::debug!(
                        "prefetch fetch_attachment {}/{} ({:?}): {e}",
                        item.account_id, item.attachment_id, kind,
                    );
                    return Err(match kind {
                        common::error::ProviderErrorKind::Transient => SkipReason::ProviderTransient,
                        common::error::ProviderErrorKind::Permanent => SkipReason::ProviderPermanent,
                    });
                }
                Err(_) => return Err(SkipReason::ProviderTimeout),
            }
        }
        ItemFetch::ImapSession { session } => {
            let (_msg_folder, uid) = imap::ops::parse_imap_message_id(&item.message_id, &item.account_id)
                .map_err(|e| {
                    log::debug!(
                        "prefetch imap parse_message_id {}/{}: {e}",
                        item.account_id, item.attachment_id,
                    );
                    SkipReason::ProviderPermanent
                })?;
            let fetch_fut = imap::client::fetch_attachment_on_selected(
                session,
                uid,
                &item.remote_attachment_id,
            );
            match tokio::time::timeout(Duration::from_secs(PER_FETCH_TIMEOUT_SECS), fetch_fut).await {
                Ok(Ok(bytes)) => bytes,
                Ok(Err(e)) => {
                    log::debug!(
                        "prefetch imap fetch {}/{}: {e}",
                        item.account_id, item.attachment_id,
                    );
                    return Err(SkipReason::ProviderTransient);
                }
                Err(_) => return Err(SkipReason::ProviderTimeout),
            }
        }
    };

    let pack_store = inner
        .boot_state
        .pack_store()
        .ok_or(SkipReason::PackStoreError)?;

    // Account-delete race close: the fetch above can take seconds; in
    // that window account.delete may have flipped `is_deleting` and
    // called `cancel_account`. If we PackStore::put and UPDATE now,
    // the AttachmentCache step's snapshot of cached hashes won't
    // include the new blob and the row's CASCADE will strand it. Drop
    // the bytes instead.
    if account_is_cancelling_or_deleting(inner, &item.account_id).await {
        return Err(SkipReason::RowGone);
    }

    let content_hash = pack_store
        .put(bytes)
        .await
        .map_err(|e| {
            log::warn!("prefetch PackStore::put {}: {e}", item.attachment_id);
            SkipReason::PackStoreError
        })?;

    // Re-check after the write. PackStore::put is content-hash
    // idempotent so the worst case here is a tombstone of a blob the
    // deletion step already accounted for - cheap and safe.
    if account_is_cancelling_or_deleting(inner, &item.account_id).await {
        if let Err(e) = pack_store.tombstone(&content_hash).await {
            log::debug!(
                "prefetch tombstone-after-cancel {}: {e}", item.attachment_id,
            );
        }
        return Err(SkipReason::RowGone);
    }

    let id_for_update = info.id.clone();
    inner
        .db
        .with_conn(move |conn| {
            db::db::queries_extra::update_attachment_cache_fields(conn, &id_for_update, &content_hash)
        })
        .await
        .map_err(|e| {
            log::warn!("prefetch update_attachment_cache_fields {}: {e}", item.attachment_id);
            SkipReason::DbUpdateError
        })?;

    enqueue_extraction_after_prefetch(inner, item, content_hash);
    Ok(())
}

async fn record_item_outcome(
    inner: &Arc<PrefetchRuntimeInner>,
    item: &PrefetchItem,
    outcome: Result<(), SkipReason>,
) {
    match outcome {
        Ok(()) => {
            inner.fetched_count.fetch_add(1, Ordering::Relaxed);
            note_breaker_success(inner, &item.provider, &item.account_id).await;
        }
        Err(reason) => {
            if reason.is_failure() {
                inner.failed_count.fetch_add(1, Ordering::Relaxed);
            } else {
                inner.skipped_count.fetch_add(1, Ordering::Relaxed);
            }
            if reason.is_timeout() {
                note_breaker_timeout(inner, &item.provider, &item.account_id).await;
            }
            log::debug!(
                "PrefetchRuntime {acct}/{att}: {reason:?}",
                acct = item.account_id,
                att = item.attachment_id,
            );
        }
    }
    finalize_item(inner, item).await;
}

fn enqueue_extraction_after_prefetch(
    inner: &Arc<PrefetchRuntimeInner>,
    work: &PrefetchItem,
    content_hash: db::blob_hash::BlobHash,
) {
    let Some(runtime) = inner.boot_state.extract_runtime() else {
        log::debug!(
            "prefetch: ExtractRuntime not installed; skipping enqueue \
             for hash {content_hash} (boot in flight or shutting down)",
        );
        return;
    };
    let extract_work = crate::extract::ExtractWork {
        content_hash,
        account_id:    work.account_id.clone(),
        message_id:    work.message_id.clone(),
        attachment_id: work.attachment_id.clone(),
    };
    if let Err(e) = runtime.try_enqueue(extract_work) {
        log::debug!("prefetch extract enqueue {content_hash}: {e}");
    }
}

async fn finalize_item(inner: &Arc<PrefetchRuntimeInner>, work: &PrefetchItem) {
    let key = (work.account_id.clone(), work.attachment_id.clone());
    let in_flight_empty = {
        let mut guard = inner.in_flight.lock().await;
        let (set, fifo) = &mut *guard;
        set.remove(&key);
        fifo.retain(|k| k != &key);
        set.is_empty()
    };
    let new_depth = inner.queue_depth
        .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
            if v > 0 { Some(v - 1) } else { None }
        })
        .map(|p| p.saturating_sub(1))
        .unwrap_or(0);

    // Throttled progress emission. Always emit when the queue drains
    // to zero (so the "Caching ..." UI clears promptly) or when the
    // wallclock has advanced past the throttle window since the last
    // emission. Skip otherwise; the downstream `Coalesce` would drop
    // these anyway, and skipping here saves the mpsc send.
    let now_ms = wallclock_ms();
    let should_emit = new_depth == 0
        || now_ms.saturating_sub(inner.last_progress_emit_ms.load(Ordering::Relaxed))
            >= PROGRESS_THROTTLE_MS;
    if should_emit {
        inner.last_progress_emit_ms.store(now_ms, Ordering::Relaxed);
        let progress = Notification::PrefetchProgress(PrefetchProgress {
            service_generation: inner.service_generation,
            remaining:          new_depth,
            fetched_in_session: inner.fetched_count.load(Ordering::Relaxed),
        });
        if let Err(e) = inner.notification_tx.send(progress).await {
            log::debug!("PrefetchRuntime progress send failed: {e}");
        }
    }
    if new_depth == 0 && in_flight_empty {
        // Hold the snapshot mutex across the three loads so a
        // concurrent `process_one` can't bump one counter between
        // reads. The mutex is contended only at drained-to-zero
        // moments, so it's not on the per-item hot path.
        let _snap = inner.counters_snapshot.lock().await;
        let completed = Notification::PrefetchCompleted(PrefetchCompleted {
            service_generation: inner.service_generation,
            fetched: inner.fetched_count.load(Ordering::Relaxed),
            skipped: inner.skipped_count.load(Ordering::Relaxed),
            failed:  inner.failed_count.load(Ordering::Relaxed),
        });
        if let Err(e) = inner.notification_tx.send(completed).await {
            log::debug!("PrefetchRuntime completed send failed: {e}");
        }
    }
}

fn wallclock_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}

/// Per-provider concurrency cap. IMAP serializes attachment fetches
/// inside the per-account semaphore (one folder-batch at a time);
/// JMAP/Gmail/Graph stay at the Phase 4 default. Phase 7 of the
/// attachments roadmap.
fn provider_semaphore_cap(provider: &str) -> usize {
    match provider {
        "imap" => 1,
        _ => PER_ACCOUNT_PERMITS,
    }
}

async fn account_semaphore(
    inner: &Arc<PrefetchRuntimeInner>,
    account_id: &str,
    provider: &str,
) -> Arc<Semaphore> {
    let cap = provider_semaphore_cap(provider);
    let mut map = inner.account_semaphores.lock().await;
    let sem = map
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(cap)));
    Arc::clone(sem)
}

/// Phase 7: circuit breaker keyspace is `(provider, account_id)`.
/// Today each account has one provider so the dimension is implicit,
/// but encoding it explicitly lets a future "promote to per-provider"
/// change land as a key-derivation tweak rather than a refactor.
fn breaker_key(provider: &str, account_id: &str) -> (String, String) {
    (provider.to_string(), account_id.to_string())
}

async fn breaker_is_open(
    inner: &Arc<PrefetchRuntimeInner>,
    provider: &str,
    account_id: &str,
) -> bool {
    let mut map = inner.breakers.lock().await;
    map.entry(breaker_key(provider, account_id))
        .or_default()
        .is_open()
}

async fn note_breaker_timeout(
    inner: &Arc<PrefetchRuntimeInner>,
    provider: &str,
    account_id: &str,
) {
    let mut map = inner.breakers.lock().await;
    map.entry(breaker_key(provider, account_id))
        .or_default()
        .note_timeout();
}

async fn note_breaker_success(
    inner: &Arc<PrefetchRuntimeInner>,
    provider: &str,
    account_id: &str,
) {
    let mut map = inner.breakers.lock().await;
    map.entry(breaker_key(provider, account_id))
        .or_default()
        .note_success();
}

/// Attachments roadmap Phase 6: read `accounts.cache_attachments_enabled`
/// for the given account. Returns `false` if the row is missing - a
/// missing row means the account was deleted (or never existed) and
/// we must not attempt a fetch against it.
async fn account_caching_enabled(
    inner: &Arc<PrefetchRuntimeInner>,
    account_id: &str,
) -> bool {
    let aid = account_id.to_string();
    inner
        .db
        .with_conn(move |conn| {
            let v: Option<i64> = conn
                .query_row(
                    "SELECT COALESCE(cache_attachments_enabled, 1) \
                     FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |r| r.get(0),
                )
                .ok();
            Ok(v.map(|n| n != 0).unwrap_or(false))
        })
        .await
        .unwrap_or(false)
}

/// True if the account is in the cancelling-set OR carries
/// `accounts.is_deleting = 1`. Either condition means the account is
/// disappearing and a freshly-written blob would be orphaned by the
/// `AttachmentCache` deletion-step snapshot.
async fn account_is_cancelling_or_deleting(
    inner: &Arc<PrefetchRuntimeInner>,
    account_id: &str,
) -> bool {
    if inner
        .cancelling_accounts
        .lock()
        .await
        .contains(account_id)
    {
        return true;
    }
    let aid = account_id.to_string();
    inner
        .db
        .with_conn(move |conn| {
            let v: i64 = conn
                .query_row(
                    "SELECT COALESCE(is_deleting, 0) FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |r| r.get(0),
                )
                .unwrap_or(0);
            Ok(v != 0)
        })
        .await
        .unwrap_or(false)
}

fn disk_has_headroom(inner: &Arc<PrefetchRuntimeInner>) -> bool {
    let dir = inner.boot_state.app_data_dir();
    match statvfs_free_bytes(dir) {
        Some(free) => free >= MIN_DISK_FREE_BYTES,
        None => true, // Best-effort: don't block on a stat failure.
    }
}

#[cfg(unix)]
fn statvfs_free_bytes(path: &std::path::Path) -> Option<u64> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let cpath = CString::new(path.as_os_str().as_bytes()).ok()?;
    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::statvfs(cpath.as_ptr(), &mut stat) };
    if rc != 0 {
        return None;
    }
    // f_bavail = blocks free to unprivileged users; f_frsize = block size.
    let bavail = stat.f_bavail as u64;
    let frsize = stat.f_frsize as u64;
    Some(bavail.saturating_mul(frsize))
}

#[cfg(windows)]
fn statvfs_free_bytes(path: &std::path::Path) -> Option<u64> {
    // Phase 7 of the attachments roadmap: bring the disk-headroom
    // backstop to Windows so the prefetch worker stops writing before
    // it corrupts the SQLite WAL on a near-full volume, matching the
    // Unix `statvfs` path.
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    // GetDiskFreeSpaceExW accepts a directory path. The pack-store dir
    // is the canonical caller; if it doesn't exist yet we fall back to
    // the parent, then to the volume root, returning None if every
    // step fails (matches the Unix fallback contract).
    let mut candidate = Some(path.to_path_buf());
    while let Some(p) = candidate {
        let wide: Vec<u16> = p.as_os_str().encode_wide().chain(std::iter::once(0)).collect();
        let mut free_to_caller: u64 = 0;
        let rc = unsafe {
            GetDiskFreeSpaceExW(
                wide.as_ptr(),
                &mut free_to_caller as *mut u64,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        if rc != 0 {
            return Some(free_to_caller);
        }
        candidate = p.parent().map(std::path::Path::to_path_buf);
    }
    None
}

#[cfg(not(any(unix, windows)))]
fn statvfs_free_bytes(_path: &std::path::Path) -> Option<u64> {
    // No backstop on truly exotic targets. Cache fills until the OS
    // surfaces ENOSPC as `SkipReason::PackStoreError`.
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_doubles_until_max() {
        assert_eq!(backoff_for(1), CIRCUIT_BREAKER_BACKOFF_MIN);
        assert_eq!(backoff_for(2), CIRCUIT_BREAKER_BACKOFF_MIN * 2);
        assert_eq!(backoff_for(3), CIRCUIT_BREAKER_BACKOFF_MIN * 4);
        // Trip 8 -> 30s * 128 = 3840s clamped to 300s (MAX).
        assert_eq!(backoff_for(8), CIRCUIT_BREAKER_BACKOFF_MAX);
        assert_eq!(backoff_for(99), CIRCUIT_BREAKER_BACKOFF_MAX);
    }

    #[test]
    fn breaker_trips_after_k_timeouts_within_w() {
        let mut b = BreakerState::default();
        assert!(!b.is_open());
        for _ in 0..(CIRCUIT_BREAKER_K - 1) {
            b.note_timeout();
            assert!(!b.is_open());
        }
        b.note_timeout();
        assert!(b.is_open());
    }

    #[test]
    fn breaker_resets_on_success() {
        let mut b = BreakerState::default();
        for _ in 0..CIRCUIT_BREAKER_K {
            b.note_timeout();
        }
        assert!(b.is_open());
        // Force half-open by clearing tripped_at as `is_open` would
        // after backoff elapses; here we just call note_success which
        // also clears.
        b.note_success();
        assert!(!b.is_open());
        assert_eq!(b.consecutive_trips, 0);
    }

    // Standalone in-flight tracker tests. The PrefetchRuntime wires
    // this pair into `enqueue` / `finalize_item` / `cancel_account`;
    // these tests verify the data-structure invariants in isolation
    // so a future refactor can't regress on dedupe or FIFO drift.

    fn insert(
        set: &mut HashSet<InFlightKey>,
        fifo: &mut VecDeque<InFlightKey>,
        k: InFlightKey,
    ) -> bool {
        if set.insert(k.clone()) {
            fifo.push_back(k);
            true
        } else {
            false
        }
    }

    fn evict_over_cap(
        set: &mut HashSet<InFlightKey>,
        fifo: &mut VecDeque<InFlightKey>,
        cap: usize,
    ) {
        while set.len() > cap {
            if let Some(old) = fifo.pop_front() {
                set.remove(&old);
            } else {
                break;
            }
        }
    }

    fn remove_both(
        set: &mut HashSet<InFlightKey>,
        fifo: &mut VecDeque<InFlightKey>,
        k: &InFlightKey,
    ) {
        set.remove(k);
        fifo.retain(|x| x != k);
    }

    #[test]
    fn in_flight_dedupe_suppresses_second_insert() {
        let mut set = HashSet::new();
        let mut fifo = VecDeque::new();
        let k = ("a".into(), "x".into());
        assert!(insert(&mut set, &mut fifo, k.clone()));
        assert!(!insert(&mut set, &mut fifo, k.clone()));
        assert_eq!(set.len(), 1);
        assert_eq!(fifo.len(), 1);
    }

    #[test]
    fn in_flight_cap_evicts_oldest() {
        let mut set = HashSet::new();
        let mut fifo = VecDeque::new();
        for i in 0..5 {
            insert(&mut set, &mut fifo, ("a".into(), i.to_string()));
        }
        evict_over_cap(&mut set, &mut fifo, 3);
        assert_eq!(set.len(), 3);
        assert_eq!(fifo.len(), 3);
        // Oldest two evicted; entries "2", "3", "4" remain.
        assert!(set.contains(&("a".into(), "2".into())));
        assert!(set.contains(&("a".into(), "4".into())));
        assert!(!set.contains(&("a".into(), "0".into())));
    }

    #[test]
    fn in_flight_remove_clears_both_set_and_fifo() {
        // Regression: phase-4 enqueue's tx.send-error cleanup used to
        // remove only from the HashSet, leaving phantom FIFO entries.
        let mut set = HashSet::new();
        let mut fifo = VecDeque::new();
        let k = ("a".into(), "x".into());
        insert(&mut set, &mut fifo, k.clone());
        remove_both(&mut set, &mut fifo, &k);
        assert!(set.is_empty());
        assert!(fifo.is_empty(), "FIFO must be drained when key removed");
    }

    #[test]
    fn in_flight_remove_then_reinsert_succeeds() {
        let mut set = HashSet::new();
        let mut fifo = VecDeque::new();
        let k = ("a".into(), "x".into());
        insert(&mut set, &mut fifo, k.clone());
        remove_both(&mut set, &mut fifo, &k);
        // Without the fix above, the FIFO would still hold the key,
        // and a later cap eviction would pop it (no-op on the set),
        // leaking an entry off the deque. With both halves cleared,
        // a reinsert behaves like the first.
        assert!(insert(&mut set, &mut fifo, k.clone()));
        assert_eq!(set.len(), 1);
        assert_eq!(fifo.len(), 1);
    }

    #[test]
    fn in_flight_cancel_account_drops_all_keys_for_account() {
        let mut set = HashSet::new();
        let mut fifo = VecDeque::new();
        insert(&mut set, &mut fifo, ("a".into(), "x".into()));
        insert(&mut set, &mut fifo, ("a".into(), "y".into()));
        insert(&mut set, &mut fifo, ("b".into(), "z".into()));
        set.retain(|(acct, _)| acct != "a");
        fifo.retain(|(acct, _)| acct != "a");
        assert_eq!(set.len(), 1);
        assert_eq!(fifo.len(), 1);
        assert!(set.contains(&("b".into(), "z".into())));
    }
}
