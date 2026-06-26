pub mod hydrate;
pub mod imap_threading;
pub mod post_persist;
pub mod write;

#[cfg(test)]
mod golden_test;

#[cfg(test)]
mod move_purge_test;

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use bifrost_sync::CheckpointStore;
use bifrost_sync::backfill::BackfillState;
use bifrost_sync::{Error, SyncEngine};
use bifrost_types::{
    AccountId, BackfillCheckpoint, BackfillProgress, Checkpoint, CursorScope, ObjectType,
    Partition, SyncEvent,
};
use common::types::FolderKind;
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};
use tokio::sync::{Mutex, broadcast};

use self::hydrate::HydrateBatch;
use self::imap_threading::ImapThreadAccumulator;
use super::SqliteCheckpointStore;
use super::checkpoint_store::BACKFILL_COMPLETION_PARTITION;

const COMPLETION_IDLE_INTERVAL: Duration = Duration::from_secs(2);
const RESIDENT_FLUSH_INTERVAL: Duration = Duration::from_secs(30);
const RESIDENT_DEFERRED_ACK_CAP: usize = 128;
const RESIDENT_PENDING_DELETION_CAP: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BifrostProviderKind {
    Gmail,
    Graph,
    Imap,
    Jmap,
}

impl BifrostProviderKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gmail => "gmail",
            Self::Graph => "graph",
            Self::Imap => "imap",
            Self::Jmap => "jmap",
        }
    }
}

#[derive(Clone)]
pub struct BifrostConsumerStores {
    pub db: WriteDbState,
    pub body_store: BodyStoreWriteState,
    pub inline_images: InlineImageStoreWriteState,
    pub search: SearchWriteHandle,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConsumerHook {
    StallConsumer {
        after_ms: u64,
    },
    CrashBeforeAck,
    CrashAfterAckNoSentinel,
    CrashBeforeDriveEndThreading,
    /// Force the drive loop to report `lagged` (and detach) on the next
    /// event, simulating sustained structural broadcast lag without needing
    /// a real overflow. The resident loop then re-subscribes and re-pushes a
    /// full reconcile, matching the B3b stopgap for real `RecvError::Lagged`.
    ForceLag,
}

#[derive(Default)]
pub struct ConsumerHookRegistry {
    hooks: Mutex<std::collections::HashMap<String, ConsumerHook>>,
}

impl ConsumerHookRegistry {
    pub async fn arm(&self, account_id: impl Into<String>, hook: ConsumerHook) {
        self.hooks.lock().await.insert(account_id.into(), hook);
    }

    async fn take(&self, account_id: &str) -> Option<ConsumerHook> {
        self.hooks.lock().await.remove(account_id)
    }

    /// Peek (without consuming) whether the armed hook for `account_id`
    /// will deliberately WITHHOLD the checkpoint ack for the next batch -
    /// i.e. `CrashBeforeAck`, which exits the drive task between the search
    /// flush and the ack. The test inject handler uses this so it does not
    /// block waiting for a cursor advance that, by construction, will never
    /// land.
    pub async fn peek_withholds_ack(&self, account_id: &str) -> bool {
        matches!(
            self.hooks.lock().await.get(account_id),
            Some(ConsumerHook::CrashBeforeAck | ConsumerHook::CrashBeforeDriveEndThreading)
        )
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ConsumerDriveReport {
    pub completed: bool,
    pub scopes_completed: u32,
    pub batches_acked: u32,
    pub lagged: bool,
}

pub struct ChangeStreamConsumer {
    engine: Arc<SyncEngine>,
    account_id: AccountId,
    provider: BifrostProviderKind,
    stores: BifrostConsumerStores,
    checkpoint_store: Option<Arc<SqliteCheckpointStore>>,
    hook_registry: Option<Arc<ConsumerHookRegistry>>,
    folder_map: HashMap<String, FolderKind>,
    /// The set of scopes the consumer has OBSERVED a `MultiplexerEvent` for
    /// on the broadcast. This is the completion-synthesis scope enumeration
    /// (spec 4.1.2), used in place of `engine.cursors.all_scopes()`.
    ///
    /// Sound deviation from the spec's original `all_scopes()` phrasing,
    /// not a smoke-test simplification: at the frozen bifrost commit
    /// `aa9172d`, `SyncEngine` exposes no public accessor for its internal
    /// `CursorRegistry`, so `all_scopes()` is unreachable without a bifrost
    /// change, which the migration's frozen-commit discipline forbids for
    /// the duration of this item. The observed-scope set is the better
    /// surface anyway: the idle-cadence completion half can only reason
    /// about scopes that actually emitted an event this kick (a warm
    /// delta-only scope in `all_scopes()` would never produce a `Batch` to
    /// distinguish "quiet" from "mid-burst"), and the empty-observed case is
    /// vacuously caught-up - the empty-stream "completes immediately" edge.
    observed_scopes: HashSet<CursorScope>,
    imap_threading: ImapThreadAccumulator,
    deferred_imap_acks: Vec<(CursorScope, Checkpoint)>,
    imap_seen_by_scope: HashMap<CursorScope, u64>,
    crash_before_drive_end_threading: bool,
}

#[derive(Default)]
pub struct ResidentFlushTelemetry {
    forced_flushes: AtomicU32,
    max_deferred_acks: AtomicU32,
    max_pending_deletions: AtomicU32,
    batches_acked: AtomicU32,
}

impl ResidentFlushTelemetry {
    pub fn snapshot(&self) -> ResidentFlushTelemetrySnapshot {
        ResidentFlushTelemetrySnapshot {
            forced_flushes: self.forced_flushes.load(Ordering::Relaxed),
            max_deferred_acks: self.max_deferred_acks.load(Ordering::Relaxed),
            max_pending_deletions: self.max_pending_deletions.load(Ordering::Relaxed),
            batches_acked: self.batches_acked.load(Ordering::Relaxed),
        }
    }

    fn observe(&self, deferred_acks: usize, pending_deletions: usize) {
        update_max(&self.max_deferred_acks, deferred_acks);
        update_max(&self.max_pending_deletions, pending_deletions);
    }

    fn observe_report(&self, report: &ConsumerDriveReport) {
        self.batches_acked
            .store(report.batches_acked, Ordering::Relaxed);
    }

    fn record_forced_flush(&self) {
        self.forced_flushes.fetch_add(1, Ordering::Relaxed);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResidentFlushTelemetrySnapshot {
    pub forced_flushes: u32,
    pub max_deferred_acks: u32,
    pub max_pending_deletions: u32,
    pub batches_acked: u32,
}

fn update_max(value: &AtomicU32, candidate: usize) {
    let candidate = u32::try_from(candidate).unwrap_or(u32::MAX);
    let mut current = value.load(Ordering::Relaxed);
    while candidate > current {
        match value.compare_exchange_weak(current, candidate, Ordering::Relaxed, Ordering::Relaxed)
        {
            Ok(_) => break,
            Err(next) => current = next,
        }
    }
}

impl ChangeStreamConsumer {
    pub fn new(
        engine: Arc<SyncEngine>,
        account_id: AccountId,
        provider: BifrostProviderKind,
        stores: BifrostConsumerStores,
    ) -> Self {
        Self {
            engine,
            account_id,
            provider,
            stores,
            checkpoint_store: None,
            hook_registry: None,
            folder_map: HashMap::new(),
            observed_scopes: HashSet::new(),
            imap_threading: ImapThreadAccumulator::default(),
            deferred_imap_acks: Vec::new(),
            imap_seen_by_scope: HashMap::new(),
            crash_before_drive_end_threading: false,
        }
    }

    #[must_use]
    pub fn with_checkpoint_store(mut self, checkpoint_store: Arc<SqliteCheckpointStore>) -> Self {
        self.checkpoint_store = Some(checkpoint_store);
        self
    }

    #[must_use]
    pub fn with_hooks(mut self, hook_registry: Arc<ConsumerHookRegistry>) -> Self {
        self.hook_registry = Some(hook_registry);
        self
    }

    #[must_use]
    pub fn with_folder_map(mut self, folder_map: HashMap<String, FolderKind>) -> Self {
        self.folder_map = folder_map;
        self
    }

    pub async fn drive_to_caught_up(&mut self) -> Result<ConsumerDriveReport, Error> {
        let mut report = ConsumerDriveReport::default();
        let mut rx = self.engine.account_changes_stream(&self.account_id)?;
        self.drive_receiver(&mut rx, &mut report).await?;
        Ok(report)
    }

    pub async fn drive_resident<F, Fut>(
        &mut self,
        mut on_caught_up: F,
    ) -> Result<ConsumerDriveReport, Error>
    where
        F: FnMut(ConsumerDriveReport) -> Fut,
        Fut: Future<Output = ()>,
    {
        self.drive_resident_loop(
            self.engine.account_changes_stream(&self.account_id)?,
            &mut on_caught_up,
            None,
        )
        .await
    }

    pub async fn drive_resident_injected_stream<F, Fut>(
        &mut self,
        rx: broadcast::Receiver<bifrost_sync::multiplexer::MultiplexerEvent>,
        mut on_caught_up: F,
        telemetry: Option<Arc<ResidentFlushTelemetry>>,
    ) -> Result<ConsumerDriveReport, Error>
    where
        F: FnMut(ConsumerDriveReport) -> Fut,
        Fut: Future<Output = ()>,
    {
        self.drive_resident_loop(rx, &mut on_caught_up, telemetry)
            .await
    }

    async fn drive_resident_loop<F, Fut>(
        &mut self,
        mut rx: broadcast::Receiver<bifrost_sync::multiplexer::MultiplexerEvent>,
        on_caught_up: &mut F,
        telemetry: Option<Arc<ResidentFlushTelemetry>>,
    ) -> Result<ConsumerDriveReport, Error>
    where
        F: FnMut(ConsumerDriveReport) -> Fut,
        Fut: Future<Output = ()>,
    {
        let mut report = ConsumerDriveReport::default();
        let mut pending = PendingDeletions::default();
        let mut last_forced_flush = Instant::now();
        loop {
            match tokio::time::timeout(COMPLETION_IDLE_INTERVAL, rx.recv()).await {
                Ok(Ok(event)) => {
                    self.observed_scopes.insert(event.scope.clone());
                    let hook = if let Some(registry) = &self.hook_registry {
                        registry.take(&self.account_id.0).await
                    } else {
                        None
                    };
                    if matches!(hook, Some(ConsumerHook::ForceLag)) {
                        log::warn!(
                            "bifrost resident consumer FORCED lag for account {} (test hook)",
                            self.account_id.0
                        );
                        report.lagged = true;
                        // Mirror the real `RecvError::Lagged` arm: flush pending
                        // deletions (a per-batch-acked purge must still apply) but
                        // skip the IMAP drive-end flush on a lagged drive, exactly
                        // as the one-shot `drive_receiver` path does.
                        self.flush_drive_end(&pending, &mut report, false).await?;
                        return Ok(report);
                    }
                    let hook = if let Some(hook @ ConsumerHook::StallConsumer { .. }) = hook {
                        apply_hook_before_batch(hook).await?;
                        None
                    } else {
                        hook
                    };
                    if self
                        .handle_event(event, hook, &mut report, &mut pending)
                        .await?
                    {
                        self.flush_drive_end(&pending, &mut report, false).await?;
                        return Ok(report);
                    }
                    if let Some(telemetry) = &telemetry {
                        telemetry.observe(self.deferred_imap_acks.len(), pending.len());
                        telemetry.observe_report(&report);
                    }
                    if self.resident_flush_due(&pending, last_forced_flush) {
                        if let Some(telemetry) = &telemetry {
                            telemetry.record_forced_flush();
                        }
                        self.flush_drive_end(&pending, &mut report, true).await?;
                        pending.clear();
                        last_forced_flush = Instant::now();
                        if let Some(telemetry) = &telemetry {
                            telemetry.observe(self.deferred_imap_acks.len(), pending.len());
                            telemetry.observe_report(&report);
                        }
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(missed))) => {
                    log::warn!(
                        "bifrost resident consumer lagged for account {} after missing {} events",
                        self.account_id.0,
                        missed
                    );
                    report.lagged = true;
                    self.flush_drive_end(&pending, &mut report, false).await?;
                    return Ok(report);
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    self.flush_drive_end(&pending, &mut report, false).await?;
                    return Ok(report);
                }
                Err(_) => {
                    if self.scopes_backfill_completed() {
                        report.completed = true;
                        report.scopes_completed = self.completed_scope_count();
                        self.flush_drive_end(&pending, &mut report, true).await?;
                        on_caught_up(report.clone()).await;
                        if let Some(telemetry) = &telemetry {
                            telemetry.observe_report(&report);
                        }
                        report = ConsumerDriveReport::default();
                        pending.clear();
                        self.observed_scopes.clear();
                        last_forced_flush = Instant::now();
                    }
                }
            }
        }
    }

    pub async fn drive_injected_stream(
        &mut self,
        mut rx: broadcast::Receiver<bifrost_sync::multiplexer::MultiplexerEvent>,
    ) -> Result<ConsumerDriveReport, Error> {
        let mut report = ConsumerDriveReport::default();
        self.drive_receiver(&mut rx, &mut report).await?;
        Ok(report)
    }

    async fn drive_receiver(
        &mut self,
        rx: &mut broadcast::Receiver<bifrost_sync::multiplexer::MultiplexerEvent>,
        report: &mut ConsumerDriveReport,
    ) -> Result<(), Error> {
        let mut pending = PendingDeletions::default();
        let outcome = self.recv_loop(rx, report, &mut pending).await;
        // Drive-end Graph deletion reconcile (finding A / spec 4.4). A Graph
        // `ScopeChange{Removed}` is only a CANDIDATE: a folder move surfaces as
        // `Removed` in the source folder's per-folder scope batch and as
        // `Updated`/`Added` in the DESTINATION folder's SEPARATE batch, so the
        // move is only distinguishable from a true purge once every batch in
        // the drive has been observed. We therefore accumulate the candidate
        // (`removed`) and live (`live`) id sets across the whole drive and
        // delete only the unreconciled remainder here, at drive end.
        //
        // Flush on EVERY clean exit (caught-up, stream-closed, AND lagged), not
        // just the caught-up edge: a purge's `Removed` batch is acked
        // per-batch, so its cursor has already advanced past the deletion. If a
        // lagged drive skipped the flush, that purge would never be re-emitted
        // and the local row would leak forever. The narrow cost is that a move
        // whose destination `Updated` had not yet arrived when the drive lagged
        // is transiently deleted, then re-created on the re-kick (the
        // destination folder's cursor never advanced, so its `Updated`
        // re-emits) - eventual consistency, never permanent loss. On an error
        // exit (crash hooks / real engine error) we skip the flush: a partial,
        // un-acked drive must not apply deletions.
        if outcome.is_ok() {
            self.flush_drive_end(&pending, report, !report.lagged)
                .await?;
        }
        outcome
    }

    async fn recv_loop(
        &mut self,
        rx: &mut broadcast::Receiver<bifrost_sync::multiplexer::MultiplexerEvent>,
        report: &mut ConsumerDriveReport,
        pending: &mut PendingDeletions,
    ) -> Result<(), Error> {
        loop {
            match tokio::time::timeout(COMPLETION_IDLE_INTERVAL, rx.recv()).await {
                Ok(Ok(event)) => {
                    self.observed_scopes.insert(event.scope.clone());
                    let hook = if let Some(registry) = &self.hook_registry {
                        registry.take(&self.account_id.0).await
                    } else {
                        None
                    };
                    // ForceLag short-circuits to the lag arm WITHOUT persisting
                    // this event, so the cursor never advances past the gap -
                    // exactly the no-message-loss invariant a real
                    // `RecvError::Lagged` preserves (the dropped events are
                    // refetched from the last durable cursor on re-attach).
                    if matches!(hook, Some(ConsumerHook::ForceLag)) {
                        log::warn!(
                            "bifrost consumer FORCED lag for account {} (test hook)",
                            self.account_id.0
                        );
                        report.lagged = true;
                        self.engine.detach(&self.account_id).await?;
                        return Ok(());
                    }
                    let hook = if let Some(hook @ ConsumerHook::StallConsumer { .. }) = hook {
                        apply_hook_before_batch(hook).await?;
                        None
                    } else {
                        hook
                    };
                    if self.handle_event(event, hook, report, pending).await? {
                        return Ok(());
                    }
                }
                Ok(Err(broadcast::error::RecvError::Lagged(missed))) => {
                    log::warn!(
                        "bifrost consumer lagged for account {} after missing {} events",
                        self.account_id.0,
                        missed
                    );
                    report.lagged = true;
                    self.engine.detach(&self.account_id).await?;
                    return Ok(());
                }
                Ok(Err(broadcast::error::RecvError::Closed)) => {
                    // The engine tore the stream down (detach / shutdown /
                    // terminated). That is NOT the caught-up edge, so do not
                    // synthesize completion; report the state reached so far.
                    return Ok(());
                }
                Err(_) => {
                    // A full COMPLETION_IDLE_INTERVAL elapsed with no Batch.
                    // Per 4.1.2 that is the change-stream idle observation;
                    // it only counts as caught-up once every observed scope's
                    // backfill is Completed (checked FIRST, authoritative).
                    if self.scopes_backfill_completed() {
                        report.completed = true;
                        report.scopes_completed = self.completed_scope_count();
                        return Ok(());
                    }
                }
            }
        }
    }

    /// Process one event. Returns `true` when the drive loop must stop
    /// (the stream terminated).
    async fn handle_event(
        &mut self,
        event: bifrost_sync::multiplexer::MultiplexerEvent,
        hook: Option<ConsumerHook>,
        report: &mut ConsumerDriveReport,
        pending: &mut PendingDeletions,
    ) -> Result<bool, Error> {
        match &*event.event {
            SyncEvent::Batch(batch) => {
                let hydrated = if is_email_scope(&event.scope) {
                    HydrateBatch::from_changes(
                        &self.engine,
                        &self.account_id,
                        self.provider,
                        &self.folder_map,
                        &batch.items,
                    )
                    .await?
                } else {
                    HydrateBatch::default()
                };
                let batch_checkpoint = batch.checkpoint.clone();
                if self.provider == BifrostProviderKind::Imap {
                    let seen = u64::try_from(batch.items.len()).unwrap_or(u64::MAX);
                    self.imap_seen_by_scope
                        .entry(event.scope.clone())
                        .and_modify(|total| *total = total.saturating_add(seen))
                        .or_insert(seen);
                }
                // Accumulate this batch's live ids and Graph removed-candidates
                // into the drive-level reconcile state (finding A). The actual
                // deletion of unreconciled candidates is deferred to drive end.
                pending.live.extend(hydrated.live_ids.iter().cloned());
                pending.removed.extend(hydrated.removed_ids.iter().cloned());
                let affected = write::persist(
                    &self.stores,
                    &self.account_id.0,
                    self.provider,
                    &hydrated.rows,
                    &hydrated.deleted_ids,
                )
                .await
                .map_err(|error| Error::Other(format!("bifrost persist: {error}")))?;
                if self.provider == BifrostProviderKind::Imap {
                    // Key the threadable accumulator on the id each row was
                    // PERSISTED under: the IMAP write arm adopts the existing
                    // local id of any `(account_id, imap_folder, imap_uid)`
                    // row before insert, and the drive-end threading pass
                    // reassigns by message id, so it must see the adopted id
                    // (`affected.message_ids`, 1:1 with `hydrated.rows`), not
                    // the provisional hydrate-time id.
                    self.imap_threading
                        .push_rows_with_ids(&hydrated.rows, &affected.message_ids);
                }
                post_persist::run(
                    &self.stores.db,
                    &self.account_id.0,
                    self.provider,
                    &event.scope,
                    batch_checkpoint.as_ref(),
                    &hydrated.rows,
                    &affected,
                )
                .await
                .map_err(|error| Error::Other(format!("bifrost post-persist: {error}")))?;
                self.stores.search.flush_now().await.map_err(Error::Other)?;
                if matches!(hook, Some(ConsumerHook::CrashBeforeAck)) {
                    return Err(Error::Other(
                        "bifrost consumer crash_before_ack hook fired".to_string(),
                    ));
                }
                if matches!(hook, Some(ConsumerHook::CrashBeforeDriveEndThreading)) {
                    self.crash_before_drive_end_threading = true;
                }
                if let (Some(checkpoint), false) = (batch_checkpoint, hydrated.blocked) {
                    if self.provider == BifrostProviderKind::Imap {
                        self.deferred_imap_acks
                            .push((event.scope.clone(), checkpoint));
                    } else {
                        self.ack_checkpoint(event.scope.clone(), checkpoint).await?;
                        post_persist::prune_marker_window(
                            &self.stores.db,
                            &self.account_id.0,
                            &event.scope,
                        )
                        .await
                        .map_err(Error::Other)?;
                        report.batches_acked = report.batches_acked.saturating_add(1);
                        if matches!(hook, Some(ConsumerHook::CrashAfterAckNoSentinel)) {
                            return Err(Error::Other(
                                "bifrost consumer crash_after_ack_no_sentinel hook fired"
                                    .to_string(),
                            ));
                        }
                    }
                }
            }
            SyncEvent::Terminated(error) => {
                // B3a-infra: log and break. Mapping a terminal AccountError
                // to an OperationResult and surfacing it is B3c.
                log::warn!(
                    "bifrost consumer stream terminated for account {}: {}",
                    self.account_id.0,
                    error
                );
                return Ok(true);
            }
            SyncEvent::Done(checkpoint) => {
                if let Some(checkpoint) = checkpoint.clone() {
                    if self.provider == BifrostProviderKind::Imap {
                        // IMAP-only: synthesize a durable Backfill COMPLETION
                        // sentinel from the Done's Change cursor. The engine's
                        // backfill orchestrator reads this sentinel on the next
                        // attach (`get_backfill` -> `backfill_complete_recorded`)
                        // to SKIP re-walking a scope's full inventory; without
                        // it, every kick would re-fetch the whole mailbox.
                        //
                        // The HTTP siblings get this sentinel for free: their
                        // backfill completion arrives as a real
                        // `Checkpoint::Backfill` on the completion partition
                        // (the engine's `emit_backfill_complete`, acked through
                        // the normal batch path). The bifrost IMAP data path
                        // never emits one - both its `inventory_stream` and
                        // `changes_stream` checkpoint exclusively with
                        // `Checkpoint::Change` (the QRESYNC/CONDSTORE folder
                        // cursor) and signal per-folder completion via
                        // `Done(Change)`. So for IMAP the consumer must derive
                        // the completion sentinel itself, mirroring the
                        // `items_done = seen + 1` / `items_estimated = seen`
                        // shape `emit_backfill_complete` records (the
                        // per-scope total accumulated in `imap_seen_by_scope`).
                        if matches!(checkpoint, Checkpoint::Change(_)) {
                            let seen = self
                                .imap_seen_by_scope
                                .get(&event.scope)
                                .copied()
                                .unwrap_or(0);
                            self.deferred_imap_acks.push((
                                event.scope.clone(),
                                Checkpoint::Backfill(completion_backfill_checkpoint(
                                    event.scope.clone(),
                                    seen,
                                )),
                            ));
                        }
                        self.deferred_imap_acks
                            .push((event.scope.clone(), checkpoint));
                    } else {
                        self.ack_checkpoint(event.scope.clone(), checkpoint).await?;
                        post_persist::prune_marker_window(
                            &self.stores.db,
                            &self.account_id.0,
                            &event.scope,
                        )
                        .await
                        .map_err(Error::Other)?;
                        report.batches_acked = report.batches_acked.saturating_add(1);
                    }
                }
            }
            SyncEvent::Progress(_) | SyncEvent::Warning(_) => {}
            _ => {}
        }
        Ok(false)
    }

    async fn ack_checkpoint(
        &self,
        scope: CursorScope,
        checkpoint: Checkpoint,
    ) -> Result<(), Error> {
        if let Some(store) = &self.checkpoint_store {
            match checkpoint {
                Checkpoint::Change(cursor) => {
                    store.put_change_cursor(&self.account_id, cursor).await
                }
                Checkpoint::Backfill(backfill) => {
                    store.put_backfill(&self.account_id, backfill).await
                }
                _ => Err(Error::Other(format!(
                    "unsupported checkpoint for synthetic bifrost consumer scope {scope:?}"
                ))),
            }
        } else {
            self.engine
                .ack_checkpoint(&self.account_id, scope, checkpoint)
                .await
        }
    }

    fn scopes_backfill_completed(&self) -> bool {
        self.observed_scopes.iter().all(|scope| {
            matches!(
                self.engine.backfill_registry().snapshot(scope),
                None | Some(BackfillState::Completed)
            )
        })
    }

    fn completed_scope_count(&self) -> u32 {
        self.observed_scopes
            .iter()
            .filter(|scope| {
                matches!(
                    self.engine.backfill_registry().snapshot(scope),
                    Some(BackfillState::Completed)
                )
            })
            .count()
            .try_into()
            .unwrap_or(u32::MAX)
    }

    /// Apply the drive-level Graph deletion reconcile (finding A): delete every
    /// `ScopeChange{Removed}` candidate that NO live signal (`Updated`/`Added`)
    /// revived anywhere in this drive. A surviving move keeps its row (its
    /// destination folder re-asserted membership); only a true purge - removed
    /// and never seen live - is deleted. No-op for non-Graph providers (they
    /// never populate `removed`) and when nothing needs deleting.
    async fn flush_pending_deletions(&self, pending: &PendingDeletions) -> Result<(), Error> {
        let to_delete: Vec<String> = pending
            .removed
            .iter()
            .filter(|id| !pending.live.contains(*id))
            .cloned()
            .collect();
        if to_delete.is_empty() {
            return Ok(());
        }
        write::persist(
            &self.stores,
            &self.account_id.0,
            self.provider,
            &[],
            &to_delete,
        )
        .await
        .map_err(|error| Error::Other(format!("bifrost drive-end delete: {error}")))?;
        Ok(())
    }

    async fn flush_imap_drive_end(
        &mut self,
        report: &mut ConsumerDriveReport,
    ) -> Result<(), Error> {
        if self.provider != BifrostProviderKind::Imap {
            return Ok(());
        }
        imap_threading::run_drive_end_threading(
            &self.stores,
            &self.account_id.0,
            &self.imap_threading,
        )
        .await
        .map_err(|error| Error::Other(format!("bifrost IMAP threading: {error}")))?;
        self.stores.search.flush_now().await.map_err(Error::Other)?;
        let deferred = std::mem::take(&mut self.deferred_imap_acks);
        for (scope, checkpoint) in deferred {
            self.ack_checkpoint(scope.clone(), checkpoint).await?;
            post_persist::prune_marker_window(&self.stores.db, &self.account_id.0, &scope)
                .await
                .map_err(Error::Other)?;
            report.batches_acked = report.batches_acked.saturating_add(1);
        }
        self.imap_threading.clear();
        self.imap_seen_by_scope.clear();
        Ok(())
    }

    async fn flush_drive_end(
        &mut self,
        pending: &PendingDeletions,
        report: &mut ConsumerDriveReport,
        flush_imap: bool,
    ) -> Result<(), Error> {
        self.flush_pending_deletions(pending).await?;
        if self.crash_before_drive_end_threading {
            return Err(Error::Other(
                "bifrost consumer crash_before_drive_end_threading hook fired".to_string(),
            ));
        }
        if flush_imap {
            self.flush_imap_drive_end(report).await?;
        }
        Ok(())
    }

    fn resident_flush_due(&self, pending: &PendingDeletions, last_flush: Instant) -> bool {
        self.deferred_imap_acks.len() >= RESIDENT_DEFERRED_ACK_CAP
            || pending.len() >= RESIDENT_PENDING_DELETION_CAP
            || (!self.deferred_imap_acks.is_empty()
                && last_flush.elapsed() >= RESIDENT_FLUSH_INTERVAL)
            || (!pending.is_empty() && last_flush.elapsed() >= RESIDENT_FLUSH_INTERVAL)
    }
}

fn completion_backfill_checkpoint(scope: CursorScope, seen: u64) -> BackfillCheckpoint {
    BackfillCheckpoint {
        scope,
        partition: Partition(BACKFILL_COMPLETION_PARTITION.to_vec()),
        progress_marker: None,
        progress: BackfillProgress {
            items_done: seen.saturating_add(1),
            items_estimated: Some(seen),
        },
        envelope_version: bifrost_sync::ENGINE_VERSION,
    }
}

/// Drive-level accumulator for the Graph move-vs-purge reconcile (finding A).
/// `removed` collects `ScopeChange{Removed}` candidates and `live` collects the
/// `Updated`/`Added` ids seen across every per-folder scope batch in the drive;
/// at drive end `removed - live` is the set actually deleted.
#[derive(Default)]
struct PendingDeletions {
    removed: HashSet<String>,
    live: HashSet<String>,
}

impl PendingDeletions {
    fn clear(&mut self) {
        self.removed.clear();
        self.live.clear();
    }

    fn is_empty(&self) -> bool {
        self.removed.is_empty() && self.live.is_empty()
    }

    fn len(&self) -> usize {
        self.removed.len().saturating_add(self.live.len())
    }
}

fn is_email_scope(scope: &CursorScope) -> bool {
    // `CursorScope::Folder(_)` is accepted unconditionally rather than gated to
    // the IMAP provider. It is the scope IMAP drives on (the bifrost IMAP
    // `Account` emits per-folder `Folder` scopes); the HTTP providers drive
    // `Account` / `Type(Email)` / `FolderType` scopes and never emit a bare
    // `Folder` scope, so this arm is exercised only by IMAP. Leaving it
    // provider-agnostic keeps this predicate from having to thread the
    // `BifrostProviderKind` through, and is harmless because no non-IMAP
    // provider produces a `Folder`-scoped batch to mis-route here.
    matches!(
        scope,
        CursorScope::Account
            | CursorScope::Type(ObjectType::Email)
            | CursorScope::Folder(_)
            | CursorScope::FolderType {
                ty: ObjectType::Email,
                ..
            }
    )
}

async fn apply_hook_before_batch(hook: ConsumerHook) -> Result<(), Error> {
    match hook {
        ConsumerHook::StallConsumer { after_ms } => {
            tokio::time::sleep(Duration::from_millis(after_ms)).await;
            Ok(())
        }
        ConsumerHook::CrashBeforeAck | ConsumerHook::CrashAfterAckNoSentinel => Err(Error::Other(
            "bifrost consumer crash hook fired in in-process handler".to_string(),
        )),
        ConsumerHook::CrashBeforeDriveEndThreading => Ok(()),
        // ForceLag is handled inline in `drive_receiver` (it short-circuits to
        // the lag arm before any batch work), so it never reaches here.
        ConsumerHook::ForceLag => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::{BifrostProviderKind, is_email_scope};
    use bifrost_types::{CursorScope, FolderId};

    #[test]
    fn provider_kind_names_match_account_provider_values() {
        assert_eq!(BifrostProviderKind::Gmail.as_str(), "gmail");
        assert_eq!(BifrostProviderKind::Graph.as_str(), "graph");
        assert_eq!(BifrostProviderKind::Imap.as_str(), "imap");
        assert_eq!(BifrostProviderKind::Jmap.as_str(), "jmap");
    }

    #[test]
    fn imap_folder_cursor_scope_is_email_scope() {
        assert!(is_email_scope(&CursorScope::Folder(FolderId(
            "INBOX".to_string(),
        ))));
    }
}
