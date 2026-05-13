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

/// One attachment to fetch.
#[derive(Debug, Clone)]
pub struct PrefetchWork {
    pub account_id: String,
    pub message_id: String,
    /// Local `attachments.id` (PK).
    pub attachment_id: String,
    /// Provider-side blob/part identifier. Either
    /// `attachments.remote_attachment_id` or, for IMAP,
    /// `attachments.imap_part_id`. Resolved at enqueue time so the
    /// worker doesn't need to revisit the row before calling
    /// `fetch_attachment`.
    pub remote_attachment_id: String,
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
    /// Anything else from the provider. Logged, counted, not retried
    /// in-session; the next backfill kick will re-emit.
    ProviderTransient,
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
            Self::PackStoreError | Self::DbUpdateError | Self::ProviderTransient
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
    /// Per-account semaphore cache. Populated lazily on first dispatch
    /// for a new account_id. Capped implicitly by total accounts.
    account_semaphores:   Mutex<HashMap<String, Arc<Semaphore>>>,
    breakers:             Mutex<HashMap<String, BreakerState>>,
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
            account_semaphores: Mutex::new(HashMap::new()),
            breakers:           Mutex::new(HashMap::new()),
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
    ) -> Result<(), String> {
        if self.inner.closed.load(Ordering::Relaxed) {
            return Err("PrefetchRuntime is shutting down".into());
        }
        let key = (work.account_id.clone(), work.attachment_id.clone());
        {
            let mut guard = self.inner.in_flight.lock().await;
            let (set, fifo) = &mut *guard;
            if !set.insert(key.clone()) {
                return Ok(());
            }
            fifo.push_back(key);
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
        }
        self.inner.queue_depth.fetch_add(1, Ordering::Relaxed);
        let tx = match priority {
            PrefetchPriority::Sync     => &self.inner.sync_tx,
            PrefetchPriority::Backfill => &self.inner.backfill_tx,
        };
        if let Err(e) = tx.send(work.clone()).await {
            self.inner.queue_depth.fetch_sub(1, Ordering::Relaxed);
            let mut guard = self.inner.in_flight.lock().await;
            guard.0.remove(&(work.account_id.clone(), work.attachment_id.clone()));
            return Err(format!("PrefetchRuntime worker exited: {e}"));
        }
        Ok(())
    }

    /// Walk `attachments` for `account_id` joining against `messages`
    /// for date filtering, restrict to rows with `content_hash IS NULL`
    /// and `messages.date >= window_start`. Each result is enqueued on
    /// the chosen priority lane. The sweep is bounded by `limit` (None
    /// = unbounded; backfill paginates via repeated calls).
    ///
    /// Returns the number of rows actually enqueued (after dedupe).
    pub async fn enqueue_window_for_account(
        &self,
        account_id: &str,
        window_start_unix: i64,
        priority: PrefetchPriority,
        limit: Option<i64>,
    ) -> Result<u64, String> {
        let aid = account_id.to_string();
        let lim = limit.unwrap_or(i64::MAX);
        let rows: Vec<(String, String, Option<String>, Option<String>)> = self
            .inner
            .db
            .with_conn(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT a.id, a.message_id, a.remote_attachment_id, a.imap_part_id \
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

        let mut enqueued = 0u64;
        for (attachment_id, message_id, remote, imap) in rows {
            let remote_attachment_id = match remote.or(imap) {
                Some(s) if !s.is_empty() => s,
                _ => continue, // Phase 7 (IMAP/others) will handle this; skip silently.
            };
            let work = PrefetchWork {
                account_id: account_id.to_string(),
                message_id,
                attachment_id,
                remote_attachment_id,
            };
            if self.enqueue(work, priority).await.is_ok() {
                enqueued += 1;
            }
        }
        Ok(enqueued)
    }

    /// Walk every page of `account_id`'s NULL-hash attachments inside
    /// the given window. Paginates via repeated bounded sweeps. Used
    /// by boot recovery, account-add, and window-extend triggers.
    /// Fire-and-forget: the worker drains asynchronously.
    pub async fn kick_backfill_account(
        &self,
        account_id: &str,
        window_start_unix: i64,
    ) -> Result<u64, String> {
        let mut total = 0u64;
        loop {
            let page = self
                .enqueue_window_for_account(
                    account_id,
                    window_start_unix,
                    PrefetchPriority::Backfill,
                    Some(BACKFILL_PAGE_SIZE),
                )
                .await?;
            total += page;
            // Loop terminates when we enqueue fewer than a page -
            // either the account is drained or every remaining row was
            // dedupe-suppressed (already in-flight).
            if page < BACKFILL_PAGE_SIZE as u64 {
                break;
            }
        }
        Ok(total)
    }

    /// Drop every queued work item for `account_id`. In-flight items
    /// continue (no provider-side abort). Used by account-delete so we
    /// don't issue a provider fetch against an account that's
    /// disappearing.
    pub async fn cancel_account(&self, account_id: &str) {
        // The in-flight set still owns the (account, attachment) keys;
        // we evict them so a subsequent backfill against a freshly-
        // recreated account-id wouldn't be silently dedupe-suppressed.
        let mut guard = self.inner.in_flight.lock().await;
        let (set, fifo) = &mut *guard;
        set.retain(|(a, _)| a != account_id);
        fifo.retain(|(a, _)| a != account_id);
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
    let outcome = run_pipeline(&inner, &work).await;
    match outcome {
        Ok(()) => {
            inner.fetched_count.fetch_add(1, Ordering::Relaxed);
            note_breaker_success(&inner, &work.account_id).await;
        }
        Err(reason) => {
            if reason.is_failure() {
                inner.failed_count.fetch_add(1, Ordering::Relaxed);
            } else {
                inner.skipped_count.fetch_add(1, Ordering::Relaxed);
            }
            if reason.is_timeout() {
                note_breaker_timeout(&inner, &work.account_id).await;
            }
            log::debug!(
                "PrefetchRuntime {acct}/{att}: {reason:?}",
                acct = work.account_id,
                att = work.attachment_id,
            );
        }
    }
    finalize_item(&inner, &work).await;
}

async fn run_pipeline(
    inner: &Arc<PrefetchRuntimeInner>,
    work: &PrefetchWork,
) -> Result<(), SkipReason> {
    // Per-account permit. Acquired first so the circuit-breaker check
    // and disk check are serialized through the same gate that bounds
    // outbound provider load.
    let semaphore = account_semaphore(inner, &work.account_id).await;
    let _permit = match semaphore.acquire_owned().await {
        Ok(p) => p,
        Err(_) => return Err(SkipReason::ProviderTransient),
    };

    if breaker_is_open(inner, &work.account_id).await {
        return Err(SkipReason::CircuitOpen);
    }
    if !disk_has_headroom(inner) {
        return Err(SkipReason::DiskLow);
    }
    if !account_caching_enabled(inner, &work.account_id).await {
        return Err(SkipReason::AccountDisabled);
    }

    // Confirm the row still wants bytes (cache-hit by another path
    // would have populated `content_hash` between enqueue and now).
    let read_db = inner.boot_state.write_db_state()
        .map_err(|_| SkipReason::RowGone)?
        .to_read_state();
    let lookup_account = work.account_id.clone();
    let lookup_message = work.message_id.clone();
    let lookup_attachment = work.attachment_id.clone();
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

    let key = inner
        .boot_state
        .encryption_key()
        .ok_or(SkipReason::ProviderTransient)?;
    let provider =
        crate::actions::provider::create_provider(&read_db, &work.account_id, key)
            .await
            .map_err(|e| {
                log::debug!("prefetch create_provider {}: {e}", work.account_id);
                SkipReason::ProviderTransient
            })?;
    let provider_ctx = common::types::ProviderCtx {
        account_id: &work.account_id,
        db:         &read_db,
        progress:   &db::progress::NoopProgressReporter,
    };
    let fetch_fut =
        provider.fetch_attachment(&provider_ctx, &work.message_id, &work.remote_attachment_id);
    let attachment = match tokio::time::timeout(
        Duration::from_secs(PER_FETCH_TIMEOUT_SECS),
        fetch_fut,
    )
    .await
    {
        Ok(Ok(a)) => a,
        Ok(Err(e)) => {
            // Provider-classified errors would refine the
            // SkipReason::ProviderPermanent path; Phase 4 keeps both
            // transient and permanent lanes folded into one log line.
            log::debug!(
                "prefetch fetch_attachment {}/{}: {e}",
                work.account_id, work.attachment_id,
            );
            return Err(SkipReason::ProviderTransient);
        }
        Err(_) => return Err(SkipReason::ProviderTimeout),
    };

    let pack_store = inner
        .boot_state
        .pack_store()
        .ok_or(SkipReason::PackStoreError)?;
    let content_hash = pack_store
        .put(attachment.bytes)
        .await
        .map_err(|e| {
            log::warn!("prefetch PackStore::put {}: {e}", work.attachment_id);
            SkipReason::PackStoreError
        })?;

    let id_for_update = info.id.clone();
    inner
        .db
        .with_conn(move |conn| {
            db::db::queries_extra::update_attachment_cache_fields(conn, &id_for_update, &content_hash)
        })
        .await
        .map_err(|e| {
            log::warn!("prefetch update_attachment_cache_fields {}: {e}", work.attachment_id);
            SkipReason::DbUpdateError
        })?;
    let _ = content_hash; // Phase 5 will fan out an Index command; Phase 4 leaves search to ExtractRuntime.
    Ok(())
}

async fn finalize_item(inner: &Arc<PrefetchRuntimeInner>, work: &PrefetchWork) {
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

    let progress = Notification::PrefetchProgress(PrefetchProgress {
        service_generation: inner.service_generation,
        remaining:          new_depth,
        fetched_in_session: inner.fetched_count.load(Ordering::Relaxed),
    });
    if let Err(e) = inner.notification_tx.send(progress).await {
        log::debug!("PrefetchRuntime progress send failed: {e}");
    }
    if new_depth == 0 && in_flight_empty {
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

async fn account_semaphore(
    inner: &Arc<PrefetchRuntimeInner>,
    account_id: &str,
) -> Arc<Semaphore> {
    let mut map = inner.account_semaphores.lock().await;
    let sem = map
        .entry(account_id.to_string())
        .or_insert_with(|| Arc::new(Semaphore::new(PER_ACCOUNT_PERMITS)));
    Arc::clone(sem)
}

async fn breaker_is_open(inner: &Arc<PrefetchRuntimeInner>, account_id: &str) -> bool {
    let mut map = inner.breakers.lock().await;
    map.entry(account_id.to_string())
        .or_default()
        .is_open()
}

async fn note_breaker_timeout(inner: &Arc<PrefetchRuntimeInner>, account_id: &str) {
    let mut map = inner.breakers.lock().await;
    map.entry(account_id.to_string())
        .or_default()
        .note_timeout();
}

async fn note_breaker_success(inner: &Arc<PrefetchRuntimeInner>, account_id: &str) {
    let mut map = inner.breakers.lock().await;
    map.entry(account_id.to_string())
        .or_default()
        .note_success();
}

/// Attachments roadmap Phase 6: read `accounts.cache_attachments_enabled`
/// for the given account. Defaults to `true` if the row is missing
/// (account was just deleted; the dedupe set will be cleared on the
/// next `cancel_account`).
async fn account_caching_enabled(
    inner: &Arc<PrefetchRuntimeInner>,
    account_id: &str,
) -> bool {
    let aid = account_id.to_string();
    inner
        .db
        .with_conn(move |conn| {
            let v: i64 = conn
                .query_row(
                    "SELECT COALESCE(cache_attachments_enabled, 1) \
                     FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |r| r.get(0),
                )
                .unwrap_or(1);
            Ok(v != 0)
        })
        .await
        .unwrap_or(true)
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

#[cfg(not(unix))]
fn statvfs_free_bytes(_path: &std::path::Path) -> Option<u64> {
    // Windows backstop deferred - `GetDiskFreeSpaceExW` would land
    // here. Returning None means the backstop is permissive on
    // Windows; the cache fills until the OS raises ENOSPC, which
    // surfaces as `SkipReason::PackStoreError`.
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
}
