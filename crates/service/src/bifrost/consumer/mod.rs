pub mod hydrate;
pub mod post_persist;
pub mod write;

#[cfg(test)]
mod golden_test;

#[cfg(test)]
mod move_purge_test;

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

use bifrost_sync::CheckpointStore;
use bifrost_sync::backfill::BackfillState;
use bifrost_sync::{Error, SyncEngine};
use bifrost_types::{AccountId, Checkpoint, CursorScope, ObjectType, SyncEvent};
use common::types::FolderKind;
use service_state::{
    BodyStoreWriteState, InlineImageStoreWriteState, SearchWriteHandle, WriteDbState,
};
use tokio::sync::{Mutex, broadcast};

use self::hydrate::HydrateBatch;
use super::SqliteCheckpointStore;

const COMPLETION_IDLE_INTERVAL: Duration = Duration::from_secs(2);

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
    /// Force the drive loop to report `lagged` (and detach) on the next
    /// event, simulating sustained structural broadcast lag WITHOUT needing
    /// a real overflow. The production bounded lag-backoff loop in
    /// `engine_sync::sync_jmap_account` then exercises its re-attach budget,
    /// backoff delays, and failed-with-lag terminal exactly as it would
    /// under a real `RecvError::Lagged`. Gate target for the
    /// production-lag-backoff harness (B3a-cut-jmap 6.4).
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
            Some(ConsumerHook::CrashBeforeAck)
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
            self.flush_pending_deletions(&pending).await?;
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
        &self,
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
                post_persist::run(
                    &self.stores.db,
                    &self.account_id.0,
                    self.provider,
                    &event.scope,
                    event.checkpoint.as_ref(),
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
                if let (Some(checkpoint), false) = (event.checkpoint, hydrated.blocked) {
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
                            "bifrost consumer crash_after_ack_no_sentinel hook fired".to_string(),
                        ));
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
            SyncEvent::Done(_) | SyncEvent::Progress(_) | SyncEvent::Warning(_) => {}
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

fn is_email_scope(scope: &CursorScope) -> bool {
    matches!(
        scope,
        CursorScope::Account
            | CursorScope::Type(ObjectType::Email)
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
        // ForceLag is handled inline in `drive_receiver` (it short-circuits to
        // the lag arm before any batch work), so it never reaches here.
        ConsumerHook::ForceLag => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::BifrostProviderKind;

    #[test]
    fn provider_kind_names_match_account_provider_values() {
        assert_eq!(BifrostProviderKind::Gmail.as_str(), "gmail");
        assert_eq!(BifrostProviderKind::Graph.as_str(), "graph");
        assert_eq!(BifrostProviderKind::Imap.as_str(), "imap");
        assert_eq!(BifrostProviderKind::Jmap.as_str(), "jmap");
    }
}
