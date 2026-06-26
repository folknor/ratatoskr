use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::time::Duration;

use bifrost_types::{
    AccountControl, AccountId, CursorScope, FolderId, HintPayload, InvalidationHint, ObjectType,
    PauseReason, PushSource, WatchEvent,
};
use common::types::{FolderKind, SystemFolderId};
use db::db::ReadDbState;
use service_api::{
    AccountPausedNotification, Notification, OperationResult, PushEvent, SyncPauseReason,
};
use service_state::WriteDbState;
use tokio::sync::{Mutex, broadcast, watch};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use super::engine_sync::{prepare_gmail_labels, prepare_graph_folders, prepare_jmap_mailboxes};
use super::error_map;
use super::{
    BifrostConsumerStores, BifrostProviderKind, BifrostSyncEngine, ChangeStreamConsumer,
    PushIngress, RoutingKey, build_account_factory,
};
use crate::boot_progress::NotificationSender;

const RESIDENT_REDRIVE_BACKOFF: Duration = Duration::from_millis(250);
/// Ceiling for the consumer re-drive backoff. Chosen on the worst-case
/// re-establish latency a transiently-lagged HEALTHY account should tolerate
/// (B3c review finding F11): the re-drive backoff bounds RETRY RATE under a
/// stalled/failing stream, a different regime from the accumulator
/// `RESIDENT_FLUSH_INTERVAL` memory bound, so it is NOT aligned to 30s. Five
/// seconds keeps a healthy account's re-establish well under the original
/// fixed-250ms regression budget while still defanging the hot-loop.
const RESIDENT_REDRIVE_BACKOFF_CAP: Duration = Duration::from_secs(5);
/// First auxiliary pass runs shortly after attach so the engine's own
/// attach + initial drive gets a head start; thereafter the passes run on a
/// wall-clock cadence chosen to match the legacy per-kick cadence, which was
/// driven by the app's 5-minute `SyncTick` (`app::subscription`).
const RESIDENT_AUX_INITIAL_DELAY: Duration = Duration::from_secs(5);
const RESIDENT_AUX_CADENCE: Duration = Duration::from_secs(300);

#[derive(Clone)]
pub struct ResidentEngine {
    inner: Arc<ResidentEngineInner>,
}

struct ResidentEngineInner {
    engine: BifrostSyncEngine,
    stores: BifrostConsumerStores,
    read_db: ReadDbState,
    write_db: WriteDbState,
    encryption_key: [u8; 32],
    ingress: Arc<PushIngress>,
    notification_tx: NotificationSender,
    service_generation: u32,
    slots: Mutex<HashMap<String, Arc<ResidentSlot>>>,
    attach_guards: Mutex<HashMap<String, Arc<Mutex<()>>>>,
    shutting_down: AtomicBool,
}

struct ResidentSlot {
    cancel: CancellationToken,
    consumer_task: Mutex<Option<JoinHandle<()>>>,
    control_task: Mutex<Option<JoinHandle<()>>>,
    aux_task: Mutex<Option<JoinHandle<()>>>,
    run_seq: AtomicU64,
    caught_up: watch::Sender<u64>,
    terminal: watch::Sender<Option<TerminalOutcome>>,
    /// Cumulative count of bounded-backoff re-drives this slot has performed
    /// (lag / closed-stream / drive-error re-subscribes). Monotonic for the
    /// life of the slot; never reset. A genuine recovery climbs this by a
    /// small bounded amount, so a gate can assert the re-drive does NOT
    /// hot-loop (§ 4.3 / § 6 "bounded re-drive") by reading it off the probe.
    redrive_total: AtomicU64,
    /// The current consecutive re-drive attempt index that drives the
    /// exponential backoff (`redrive_backoff_for_attempt`). Bumped on every
    /// backoff re-drive and reset to 0 on a clean caught-up edge (the
    /// `on_caught_up` callback) or a cleared terminal latch, so a healthy
    /// account always re-establishes at the base delay. Readable off the
    /// probe to prove the clean edge reset the backoff.
    redrive_attempt: AtomicU32,
    /// Latches true once the resident consumer has observed (or set) the
    /// account's `initial_sync_completed` marker, so the per-caught-up
    /// callback stops re-reading the marker from the DB on every idle
    /// quiescence edge (the resident loop reaches a caught-up boundary every
    /// `COMPLETION_IDLE_INTERVAL` while idle).
    initial_marked: AtomicBool,
    provider: BifrostProviderKind,
}

#[derive(Debug, Clone)]
struct TerminalOutcome {
    result: Option<OperationResult>,
    message: String,
    pause: Option<SyncPauseReason>,
}

impl ResidentEngine {
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn new(
        engine: BifrostSyncEngine,
        stores: BifrostConsumerStores,
        read_db: ReadDbState,
        write_db: WriteDbState,
        encryption_key: [u8; 32],
        ingress: Arc<PushIngress>,
        notification_tx: NotificationSender,
        service_generation: u32,
    ) -> Self {
        Self {
            inner: Arc::new(ResidentEngineInner {
                engine,
                stores,
                read_db,
                write_db,
                encryption_key,
                ingress,
                notification_tx,
                service_generation,
                slots: Mutex::new(HashMap::new()),
                attach_guards: Mutex::new(HashMap::new()),
                shutting_down: AtomicBool::new(false),
            }),
        }
    }

    pub async fn start_ingress(&self) {
        self.inner.ingress.spawn().await;
    }

    pub async fn kick_account(
        &self,
        account_id: &str,
        cancellation_token: &CancellationToken,
    ) -> Result<(), String> {
        self.attach_account(account_id).await?;
        let slot = {
            let slots = self.inner.slots.lock().await;
            slots
                .get(account_id)
                .cloned()
                .ok_or_else(|| format!("resident slot missing after attach for {account_id}"))?
        };
        let target = slot
            .run_seq
            .fetch_add(1, Ordering::SeqCst)
            .saturating_add(1);
        self.invalidate_account(account_id, PushSource::Coalesced);
        let mut caught_up = slot.caught_up.subscribe();
        let mut terminal = slot.terminal.subscribe();
        loop {
            if let Some(outcome) = terminal.borrow().clone() {
                return Err(outcome.message);
            }
            if *caught_up.borrow() >= target {
                return Ok(());
            }
            tokio::select! {
                changed = caught_up.changed() => {
                    if changed.is_err() {
                        return Err("resident caught-up channel closed".to_string());
                    }
                }
                changed = terminal.changed() => {
                    if changed.is_err() {
                        return Err("resident terminal channel closed".to_string());
                    }
                }
                () = cancellation_token.cancelled() => return Err("sync cancelled".to_string()),
            }
        }
    }

    pub async fn attach_account(&self, account_id: &str) -> Result<(), String> {
        if self.inner.shutting_down.load(Ordering::SeqCst) {
            return Err("resident engine is shutting down".to_string());
        }
        let attach_guard = {
            let mut guards = self.inner.attach_guards.lock().await;
            Arc::clone(
                guards
                    .entry(account_id.to_string())
                    .or_insert_with(|| Arc::new(Mutex::new(()))),
            )
        };
        let _attach_guard = attach_guard.lock().await;
        if self.inner.slots.lock().await.contains_key(account_id) {
            return Ok(());
        }

        let provider = self.provider_for_account(account_id).await?;
        let folder_map = self.prepare_folder_map(account_id, provider).await?;
        let factory = build_account_factory(
            &self.inner.read_db,
            self.inner.write_db.writer_pool(),
            account_id,
            self.inner.encryption_key,
        )
        .await
        .map_err(|error| error.to_string())?;
        let account = AccountId(account_id.to_string());
        self.inner
            .engine
            .engine()
            .attach(account.clone(), factory)
            .await
            .map_err(|error| format!("{error:?}"))?;
        self.subscribe_push(account_id, &account, provider, &folder_map)
            .await;
        self.register_routing_keys(account_id, provider, &folder_map)
            .await;

        let (caught_tx, _) = watch::channel(0);
        let (terminal_tx, _) = watch::channel(None);
        let slot = Arc::new(ResidentSlot {
            cancel: CancellationToken::new(),
            consumer_task: Mutex::new(None),
            control_task: Mutex::new(None),
            aux_task: Mutex::new(None),
            run_seq: AtomicU64::new(0),
            caught_up: caught_tx,
            terminal: terminal_tx,
            redrive_total: AtomicU64::new(0),
            redrive_attempt: AtomicU32::new(0),
            initial_marked: AtomicBool::new(false),
            provider,
        });
        let task_slot = Arc::clone(&slot);
        let inner = Arc::clone(&self.inner);
        let account_for_task = account_id.to_string();
        let task = tokio::spawn(async move {
            resident_consumer_loop(inner, task_slot, account_for_task, folder_map).await;
        });
        *slot.consumer_task.lock().await = Some(task);
        let control_inner = Arc::clone(&self.inner);
        let control_slot = Arc::clone(&slot);
        let control_account = account_id.to_string();
        let control = tokio::spawn(async move {
            resident_control_loop(control_inner, control_slot, control_account).await;
        });
        *slot.control_task.lock().await = Some(control);
        // Auxiliary passes (contacts, signatures, master categories, Exchange
        // groups, reactions, IMAP PERMANENTFLAGS probe, JMAP shared-account /
        // identity / ShareNotification) ran per one-shot kick under B3a. The
        // keep-attached lifecycle has no per-kick boundary, so they move to a
        // per-slot cadence task. Sound deviation from spec 3.5's "build the
        // aux client ONCE and hold it": the cadence task rebuilds its client
        // per tick instead. The amortization spec 3.5 names (collapse the two
        // per-kick connections to one) is already delivered by keeping the
        // ENGINE connection resident; the aux pass fires every 5 minutes, so a
        // fresh client there costs one reconnect per cadence, not per kick, and
        // - load-bearing for IMAP - avoids holding an IDLE session open across
        // the whole slot life where the server would time it out. It does not
        // move the steady-state-delta gate, which measures a delta kick, not
        // the aux cadence.
        let aux_inner = Arc::clone(&self.inner);
        let aux_cancel = slot.cancel.clone();
        let aux_account = account_id.to_string();
        let aux = tokio::spawn(async move {
            resident_aux_loop(aux_inner, aux_cancel, aux_account, provider).await;
        });
        *slot.aux_task.lock().await = Some(aux);
        self.inner
            .slots
            .lock()
            .await
            .insert(account_id.to_string(), slot);
        Ok(())
    }

    pub async fn resume_account(&self, account_id: &str) -> Result<bool, String> {
        let slot = {
            let slots = self.inner.slots.lock().await;
            slots.get(account_id).cloned()
        };
        let Some(slot) = slot else {
            return Ok(false);
        };
        let account = AccountId(account_id.to_string());
        self.inner
            .engine
            .engine()
            .resume_account(&account)
            .map_err(|error| format!("{error:?}"))?;
        let _ = slot.terminal.send(None);
        Ok(true)
    }

    /// Read the slot's re-drive telemetry as `(cumulative_total, current_attempt)`,
    /// or `None` when no slot is attached for the account. Used by the test probe
    /// to gate § 4.3's bounded re-drive: `total` proves the backoff path fired
    /// without hot-looping, `attempt` proves a clean caught-up edge reset it.
    pub async fn redrive_telemetry(&self, account_id: &str) -> Option<(u64, u32)> {
        let slots = self.inner.slots.lock().await;
        slots.get(account_id).map(|slot| {
            (
                slot.redrive_total.load(Ordering::SeqCst),
                slot.redrive_attempt.load(Ordering::SeqCst),
            )
        })
    }

    pub async fn detach_account(&self, account_id: &str) -> Result<(), String> {
        let attach_guard = {
            let guards = self.inner.attach_guards.lock().await;
            guards.get(account_id).cloned()
        };
        let _attach_guard = match attach_guard.as_ref() {
            Some(attach_guard) => Some(attach_guard.lock().await),
            None => None,
        };
        let slot = self.inner.slots.lock().await.remove(account_id);
        // Intentionally do NOT remove the per-account guard from the map. A
        // concurrent attach_account may already hold a clone of this guard
        // (it cloned under the map lock and is now blocked on the guard lock);
        // removing the map entry here would let a later attach mint a *fresh*
        // guard via `or_insert_with`, so that attach and the still-running one
        // would serialize on two different mutexes and could race. Keeping one
        // canonical guard per account for the engine's lifetime makes
        // attach/detach mutually exclusive for the whole detach body, including
        // the engine teardown below. The map is bounded by the number of
        // distinct accounts ever attached; each entry is a zero-sized mutex.
        let Some(slot) = slot else {
            return Ok(());
        };
        slot.cancel.cancel();
        // Abort (do not await) the aux cadence task: it shares the cancel
        // token but only observes it between ticks, so it may be mid-network
        // inside a provider call. Awaiting it would stall the account-delete /
        // shutdown drain (spec 6.4); its writes are best-effort and
        // transactional, so dropping it mid-flight is safe.
        if let Some(aux) = slot.aux_task.lock().await.take() {
            aux.abort();
        }
        if let Some(control) = slot.control_task.lock().await.take() {
            control.abort();
        }
        self.inner.ingress.unregister_account(account_id).await;
        let account = AccountId(account_id.to_string());
        if let Err(error) = self.inner.engine.engine().unsubscribe_push(&account).await {
            log::debug!("unsubscribe_push for {account_id} during detach: {error:?}");
        }
        match self.inner.engine.engine().detach(&account).await {
            Ok(()) | Err(bifrost_sync::Error::AccountNotAttached(_)) => {}
            Err(error) => return Err(format!("{error:?}")),
        }
        if let Some(task) = slot.consumer_task.lock().await.take()
            && let Err(error) = task.await
        {
            log::warn!("resident consumer task join for {account_id}: {error}");
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        self.inner.shutting_down.store(true, Ordering::SeqCst);
        let account_ids = self
            .inner
            .slots
            .lock()
            .await
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        for account_id in account_ids {
            if let Err(error) = self.detach_account(&account_id).await {
                log::warn!("resident detach during shutdown for {account_id}: {error}");
            }
        }
        self.inner.ingress.shutdown().await;
    }

    fn invalidate_account(&self, account_id: &str, source: PushSource) {
        self.inner.engine.engine().invalidation_sink().push(
            AccountId(account_id.to_string()),
            WatchEvent::Invalidated {
                hint: InvalidationHint {
                    source,
                    payload: HintPayload::Unknown,
                },
            },
        );
    }

    async fn subscribe_push(
        &self,
        account_id: &str,
        account: &AccountId,
        provider: BifrostProviderKind,
        folder_map: &HashMap<String, FolderKind>,
    ) {
        let scopes = push_subscribe_scopes(provider, folder_map);
        match self
            .inner
            .engine
            .engine()
            .subscribe_push(account, &scopes)
            .await
        {
            Ok(_) => log::debug!("resident push subscribed for {account_id}"),
            Err(error) => {
                log::debug!(
                    "resident push unavailable for {account_id}; falling back to poll: {error:?}"
                );
            }
        }
    }

    async fn provider_for_account(&self, account_id: &str) -> Result<BifrostProviderKind, String> {
        let aid = account_id.to_string();
        let provider = self
            .inner
            .read_db
            .with_read(move |conn| {
                conn.query_row(
                    "SELECT provider FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |row| row.get::<_, String>(0),
                )
                .map_err(|error| format!("read account provider: {error}"))
            })
            .await?;
        match provider.as_str() {
            "jmap" => Ok(BifrostProviderKind::Jmap),
            "graph" => Ok(BifrostProviderKind::Graph),
            "gmail_api" => Ok(BifrostProviderKind::Gmail),
            "imap" => Ok(BifrostProviderKind::Imap),
            _ => Err(format!("unsupported sync provider: {provider}")),
        }
    }

    async fn prepare_folder_map(
        &self,
        account_id: &str,
        provider: BifrostProviderKind,
    ) -> Result<HashMap<String, common::types::FolderKind>, String> {
        match provider {
            BifrostProviderKind::Jmap => {
                let client = jmap::client::JmapClient::from_account(
                    &self.inner.read_db,
                    self.inner.write_db.writer_pool(),
                    account_id,
                    &self.inner.encryption_key,
                )
                .await
                .map_err(|error| error.clone())?;
                client
                    .ensure_valid_token()
                    .await
                    .map_err(|error| error.clone())?;
                prepare_jmap_mailboxes(
                    &client,
                    account_id,
                    &self.inner.read_db,
                    &self.inner.write_db,
                )
                .await
            }
            BifrostProviderKind::Graph => {
                let client = graph::client::GraphClient::from_account(
                    &self.inner.read_db,
                    self.inner.write_db.writer_pool(),
                    account_id,
                    self.inner.encryption_key,
                )
                .await?;
                prepare_graph_folders(
                    &client,
                    account_id,
                    &self.inner.read_db,
                    &self.inner.write_db,
                )
                .await
            }
            BifrostProviderKind::Gmail => {
                let client = gmail::client::GmailClient::from_account(
                    &self.inner.read_db,
                    self.inner.write_db.writer_pool(),
                    account_id,
                    self.inner.encryption_key,
                )
                .await?;
                client.get_access_token(&self.inner.read_db).await?;
                prepare_gmail_labels(
                    &client,
                    account_id,
                    &self.inner.read_db,
                    &self.inner.write_db,
                )
                .await
            }
            BifrostProviderKind::Imap => {
                let imap_ops = imap::ops::ImapOps::new(
                    self.inner.encryption_key,
                    self.inner.write_db.writer_pool(),
                );
                let imap_config = imap_ops
                    .load_config(&self.inner.read_db, account_id)
                    .await?;
                let mut aux_session = imap::connection::connect(&imap_config).await?;
                provider_sync::consumer_support::sync_imap_folder_map(
                    &mut aux_session,
                    account_id,
                    &self.inner.write_db,
                )
                .await
            }
        }
    }

    async fn register_routing_keys(
        &self,
        account_id: &str,
        provider: BifrostProviderKind,
        folder_map: &HashMap<String, FolderKind>,
    ) {
        match provider {
            BifrostProviderKind::Gmail => {
                if let Some(email) = self.account_email(account_id).await {
                    self.inner
                        .ingress
                        .register(RoutingKey::GmailEmail(email), account_id.to_string())
                        .await;
                }
            }
            BifrostProviderKind::Graph => {
                for folder_id in folder_map.keys() {
                    self.inner
                        .ingress
                        .register(
                            RoutingKey::GraphResource(format!(
                                "me/mailFolders/{folder_id}/messages"
                            )),
                            account_id.to_string(),
                        )
                        .await;
                }
            }
            BifrostProviderKind::Jmap => {}
            BifrostProviderKind::Imap => {}
        }
    }

    async fn account_email(&self, account_id: &str) -> Option<String> {
        let aid = account_id.to_string();
        self.inner
            .read_db
            .with_read(move |conn| {
                Ok(conn
                    .query_row(
                        "SELECT email FROM accounts WHERE id = ?1",
                        rusqlite::params![aid],
                        |row| row.get::<_, String>(0),
                    )
                    .ok())
            })
            .await
            .ok()
            .flatten()
    }
}

fn push_subscribe_scopes(
    provider: BifrostProviderKind,
    folder_map: &HashMap<String, FolderKind>,
) -> Vec<CursorScope> {
    match provider {
        BifrostProviderKind::Graph => folder_map
            .keys()
            .map(|folder_id| CursorScope::FolderType {
                folder: FolderId(folder_id.clone()),
                ty: ObjectType::Email,
            })
            .collect(),
        BifrostProviderKind::Imap => {
            let mut scopes = folder_map
                .iter()
                .filter(|(_, kind)| {
                    matches!(
                        kind,
                        FolderKind::System(SystemFolderId::Inbox) | FolderKind::ImapUser(_)
                    )
                })
                .map(|(folder_id, _)| CursorScope::Folder(FolderId(folder_id.clone())))
                .collect::<Vec<_>>();
            if scopes.is_empty() {
                scopes.push(CursorScope::Account);
            }
            scopes
        }
        BifrostProviderKind::Gmail | BifrostProviderKind::Jmap => vec![CursorScope::Account],
    }
}

async fn resident_control_loop(
    inner: Arc<ResidentEngineInner>,
    slot: Arc<ResidentSlot>,
    account_id: String,
) {
    let account = AccountId(account_id.clone());
    let mut rx = match inner.engine.engine().account_control_stream(&account) {
        Ok(rx) => rx,
        Err(error) => {
            log::warn!("account control stream unavailable for {account_id}: {error:?}");
            return;
        }
    };
    loop {
        tokio::select! {
            () = slot.cancel.cancelled() => return,
            ev = rx.recv() => match ev {
                Ok(AccountControl::Pause(reason)) => {
                    handle_account_pause(&inner, &slot, &account_id, reason).await;
                }
                Ok(AccountControl::Resume) => {
                    let _ = slot.terminal.send(None);
                }
                Err(broadcast::error::RecvError::Lagged(missed)) => {
                    log::warn!(
                        "account control stream lagged for {account_id} after missing {missed} events"
                    );
                    latch_pause_outcome(&slot, SyncPauseReason::NeedsAttention);
                    emit_account_paused(
                        &inner,
                        &account_id,
                        SyncPauseReason::NeedsAttention,
                    ).await;
                }
                Err(broadcast::error::RecvError::Closed) => return,
                Ok(other) => {
                    log::debug!("unhandled AccountControl for {account_id}: {other:?}");
                }
            }
        }
    }
}

async fn handle_account_pause(
    inner: &ResidentEngineInner,
    slot: &ResidentSlot,
    account_id: &str,
    reason: PauseReason,
) {
    match reason {
        PauseReason::RetryBudgetExhausted | PauseReason::OperatorOverrideRequired => {
            let pause = pause_reason_for_latched_outcome(slot);
            latch_pause_outcome(slot, pause.clone());
            emit_account_paused(inner, account_id, pause).await;
        }
        PauseReason::TenantThrottle => {
            log::debug!("account {account_id} paused for tenant throttle; engine will resume");
        }
        PauseReason::ConsumerRequested => {
            log::debug!("account {account_id} observed consumer-requested pause");
        }
        _ => {
            log::debug!("account {account_id} observed unknown pause reason: {reason:?}");
        }
    }
}

fn pause_reason_for_latched_outcome(slot: &ResidentSlot) -> SyncPauseReason {
    if let Some(outcome) = slot.terminal.borrow().clone()
        && let Some(result) = outcome.result
    {
        return error_map::pause_reason_to_wire(&result);
    }
    SyncPauseReason::NeedsAttention
}

/// Merge a pause latch into the slot's terminal cell. Preserves any
/// result-bearing terminal outcome the consumer already latched (F3 merge
/// discipline: a result-bearing outcome is never downgraded to pause-only);
/// only the `pause` field is (re)set here.
fn latch_pause_outcome(slot: &ResidentSlot, pause: SyncPauseReason) {
    let mut merged = slot.terminal.borrow().clone().unwrap_or(TerminalOutcome {
        result: None,
        message: "sync.account-paused".to_string(),
        pause: None,
    });
    merged.pause = Some(pause);
    let _ = slot.terminal.send(Some(merged));
}

/// Merge a terminal failure into the slot's terminal cell. Preserves any
/// `pause` already latched by the control loop (F3 merge discipline); only
/// the `result`/`message` fields are set here.
fn latch_terminal_failure(slot: &ResidentSlot, result: OperationResult, message: String) {
    let mut merged = slot.terminal.borrow().clone().unwrap_or(TerminalOutcome {
        result: None,
        message: String::new(),
        pause: None,
    });
    merged.result = Some(result);
    merged.message = message;
    let _ = slot.terminal.send(Some(merged));
}

async fn emit_account_paused(
    inner: &ResidentEngineInner,
    account_id: &str,
    reason: SyncPauseReason,
) {
    let notif = Notification::AccountPaused(AccountPausedNotification {
        account_id: account_id.to_string(),
        reason,
        service_generation: inner.service_generation,
    });
    if let Err(error) = inner.notification_tx.send(notif).await {
        log::debug!("emit account paused for {account_id}: {error}");
    }
}

async fn resident_consumer_loop(
    inner: Arc<ResidentEngineInner>,
    slot: Arc<ResidentSlot>,
    account_id: String,
    folder_map: HashMap<String, common::types::FolderKind>,
) {
    let account = AccountId(account_id.clone());
    loop {
        if slot.cancel.is_cancelled() {
            return;
        }
        let mut consumer = ChangeStreamConsumer::new(
            inner.engine.engine(),
            account.clone(),
            slot.provider,
            inner.stores.clone(),
        )
        .with_folder_map(folder_map.clone())
        .with_checkpoint_store(inner.engine.checkpoints())
        .with_hooks(crate::handlers::test_helpers::bifrost_hooks());
        let result = tokio::select! {
            result = consumer.drive_resident({
                let inner = Arc::clone(&inner);
                let slot = Arc::clone(&slot);
                let account_id = account_id.clone();
                move |report| {
                    let inner = Arc::clone(&inner);
                    let slot = Arc::clone(&slot);
                    let account_id = account_id.clone();
                    async move {
                        let initial_was_completed = if slot.initial_marked.load(Ordering::SeqCst) {
                            true
                        } else {
                            match mark_initial_sync_completed_once(&inner.write_db, &account_id)
                                .await
                            {
                                Ok(was_completed) => {
                                    slot.initial_marked.store(true, Ordering::SeqCst);
                                    was_completed
                                }
                                Err(error) => {
                                    log::warn!(
                                        "mark initial sync completed for {account_id}: {error}"
                                    );
                                    false
                                }
                            }
                        };
                        // A genuine caught-up edge is the clean signal that the
                        // re-drive recovered: reset the exponential backoff so the
                        // next lag/closed re-drive starts at the base delay again
                        // (§ 4.3 "reset on a clean caught-up edge").
                        slot.redrive_attempt.store(0, Ordering::SeqCst);
                        let previous_seq = *slot.caught_up.borrow();
                        let seq = slot.run_seq.load(Ordering::SeqCst);
                        let _ = slot.caught_up.send(seq);
                        if report.batches_acked > 0 && seq == previous_seq && initial_was_completed {
                            emit_push_event(&inner, &account_id).await;
                        }
                    }
                }
            }) => result,
            () = slot.cancel.cancelled() => return,
        };
        match result {
            Ok(report) => {
                if let Some(terminal) = report.terminal {
                    let pause = error_map::pause_reason_to_wire(&terminal.result);
                    latch_terminal_failure(&slot, terminal.result, terminal.message);
                    // The standing banner is normally emitted by the control
                    // loop when it observes the accompanying `Pause`. Emit here
                    // only to UPGRADE a banner that the control loop already
                    // raised as the generic `NeedsAttention` (the Pause-arrives-
                    // before-Terminated race, F3) to the auth-specific
                    // `NeedsReauth` now that the originating error is latched.
                    // When no pause is latched yet (the common Terminated-first
                    // ordering), the control loop will derive `NeedsReauth` from
                    // the latched result itself, so we stay silent to avoid a
                    // duplicate.
                    if matches!(pause, SyncPauseReason::NeedsReauth)
                        && slot
                            .terminal
                            .borrow()
                            .as_ref()
                            .and_then(|outcome| outcome.pause.as_ref())
                            .is_some()
                    {
                        emit_account_paused(&inner, &account_id, pause).await;
                    }
                    wait_for_terminal_clear(&slot).await;
                    slot.redrive_attempt.store(0, Ordering::SeqCst);
                    continue;
                }
                // `drive_resident` only returns on a terminal stream condition
                // (lag, stream-closed, or the ForceLag test hook); a genuine
                // caught-up edge stays in the loop. So a return here always
                // means the subscription must be re-established. Re-push a full
                // reconcile only on lag, where the bounded broadcast dropped
                // events; stream closure re-subscribes from the durable cursor.
                if slot.cancel.is_cancelled() {
                    return;
                }
                if report.completed {
                    slot.redrive_attempt.store(0, Ordering::SeqCst);
                }
                if report.lagged {
                    repush_full_reconcile(&inner, &account_id);
                }
                perform_redrive_backoff(&slot, &account_id).await;
            }
            Err(error) => {
                if slot.cancel.is_cancelled() {
                    return;
                }
                log::warn!("resident consumer for {account_id} failed: {error:?}");
                perform_redrive_backoff(&slot, &account_id).await;
            }
        }
    }
}

async fn wait_for_terminal_clear(slot: &ResidentSlot) {
    let mut terminal = slot.terminal.subscribe();
    loop {
        if slot.cancel.is_cancelled() || terminal.borrow().is_none() {
            return;
        }
        tokio::select! {
            () = slot.cancel.cancelled() => return,
            changed = terminal.changed() => {
                if changed.is_err() {
                    return;
                }
            }
        }
    }
}

/// Exponential backoff for the Nth consecutive re-drive: `BASE * 2^attempt`,
/// capped at `RESIDENT_REDRIVE_BACKOFF_CAP`. `attempt == 0` (the first
/// re-drive, or the first after a clean caught-up edge reset it) is the base
/// delay, so a transiently-lagged healthy account re-establishes promptly.
fn redrive_backoff_for_attempt(attempt: u32) -> Duration {
    let factor = 1u32.checked_shl(attempt).unwrap_or(u32::MAX);
    RESIDENT_REDRIVE_BACKOFF
        .saturating_mul(factor)
        .min(RESIDENT_REDRIVE_BACKOFF_CAP)
}

/// Sleep the bounded, jittered backoff for the slot's current re-drive attempt,
/// then bump the per-slot telemetry: `redrive_attempt` (drives the next
/// backoff) and the monotonic `redrive_total` (the "did not hot-loop" gate
/// signal). A clean caught-up edge resets `redrive_attempt` to 0 (§ 4.3).
async fn perform_redrive_backoff(slot: &ResidentSlot, account_id: &str) {
    let attempt = slot.redrive_attempt.load(Ordering::SeqCst);
    let backoff = redrive_backoff_for_attempt(attempt);
    sleep_redrive_backoff(account_id, backoff, attempt).await;
    slot.redrive_attempt
        .store(attempt.saturating_add(1), Ordering::SeqCst);
    slot.redrive_total.fetch_add(1, Ordering::SeqCst);
}

async fn sleep_redrive_backoff(account_id: &str, base: Duration, attempt: u32) {
    tokio::time::sleep(jittered_backoff(account_id, base, attempt)).await;
}

fn jittered_backoff(account_id: &str, base: Duration, attempt: u32) -> Duration {
    let millis = u64::try_from(base.as_millis()).unwrap_or(u64::MAX);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    account_id.hash(&mut hasher);
    attempt.hash(&mut hasher);
    let bucket = hasher.finish() % 41;
    let percent = 80 + bucket;
    Duration::from_millis(millis.saturating_mul(percent) / 100)
}

async fn mark_initial_sync_completed_once(
    write_db: &WriteDbState,
    account_id: &str,
) -> Result<bool, String> {
    let aid = account_id.to_string();
    write_db
        .with_write(move |conn| {
            let completed = conn
                .query_row(
                    "SELECT initial_sync_completed FROM accounts WHERE id = ?1",
                    rusqlite::params![aid],
                    |row| row.get::<_, i64>(0),
                )
                .map(|value| value != 0)
                .map_err(|error| format!("read initial_sync_completed: {error}"))?;
            if !completed {
                sync::pipeline::mark_initial_sync_completed(conn, &aid)?;
            }
            Ok(completed)
        })
        .await
}

fn repush_full_reconcile(inner: &ResidentEngineInner, account_id: &str) {
    inner.engine.engine().invalidation_sink().push(
        AccountId(account_id.to_string()),
        WatchEvent::Invalidated {
            hint: InvalidationHint {
                source: PushSource::Coalesced,
                payload: HintPayload::Unknown,
            },
        },
    );
}

async fn emit_push_event(inner: &ResidentEngineInner, account_id: &str) {
    let notif = Notification::PushEvent(PushEvent {
        account_id: account_id.to_string(),
        service_generation: inner.service_generation,
    });
    if let Err(error) = inner.notification_tx.send(notif).await {
        log::debug!("emit push event for {account_id}: {error}");
    }
}

async fn resident_aux_loop(
    inner: Arc<ResidentEngineInner>,
    cancel: CancellationToken,
    account_id: String,
    provider: BifrostProviderKind,
) {
    let mut delay = RESIDENT_AUX_INITIAL_DELAY;
    loop {
        tokio::select! {
            () = cancel.cancelled() => return,
            () = tokio::time::sleep(delay) => {}
        }
        delay = RESIDENT_AUX_CADENCE;
        if cancel.is_cancelled() {
            return;
        }
        run_aux_pass(&inner, &account_id, provider).await;
    }
}

async fn run_aux_pass(
    inner: &ResidentEngineInner,
    account_id: &str,
    provider: BifrostProviderKind,
) {
    // Read the marker fresh each pass, mirroring the legacy runner which read
    // it before each kick: a fresh account whose initial drive has not yet
    // completed gets the initial pass (full contacts pull etc.), later passes
    // get the delta path once the consumer marks the account complete.
    let initial_sync_completed = read_initial_sync_completed(&inner.read_db, account_id)
        .await
        .unwrap_or(false);
    let result: Result<(), String> = match provider {
        BifrostProviderKind::Jmap => {
            async {
                let client = jmap::client::JmapClient::from_account(
                    &inner.read_db,
                    inner.write_db.writer_pool(),
                    account_id,
                    &inner.encryption_key,
                )
                .await
                .map_err(|error| error.clone())?;
                client.ensure_valid_token().await.map_err(|e| e.clone())?;
                provider_sync::consumer_support::run_jmap_auxiliary_sync(
                    &client,
                    account_id,
                    &inner.read_db,
                    &inner.write_db,
                    initial_sync_completed,
                )
                .await;
                Ok(())
            }
            .await
        }
        BifrostProviderKind::Graph => {
            async {
                let client = graph::client::GraphClient::from_account(
                    &inner.read_db,
                    inner.write_db.writer_pool(),
                    account_id,
                    inner.encryption_key,
                )
                .await?;
                provider_sync::consumer_support::run_graph_auxiliary_sync(
                    &client,
                    account_id,
                    &inner.read_db,
                    &inner.write_db,
                    initial_sync_completed,
                )
                .await;
                Ok(())
            }
            .await
        }
        BifrostProviderKind::Gmail => {
            async {
                let client = gmail::client::GmailClient::from_account(
                    &inner.read_db,
                    inner.write_db.writer_pool(),
                    account_id,
                    inner.encryption_key,
                )
                .await?;
                client.get_access_token(&inner.read_db).await?;
                provider_sync::consumer_support::run_gmail_auxiliary_sync(
                    &client,
                    account_id,
                    &inner.read_db,
                    &inner.write_db,
                    initial_sync_completed,
                )
                .await;
                Ok(())
            }
            .await
        }
        BifrostProviderKind::Imap => {
            async {
                let imap_ops =
                    imap::ops::ImapOps::new(inner.encryption_key, inner.write_db.writer_pool());
                let imap_config = imap_ops.load_config(&inner.read_db, account_id).await?;
                let mut session = imap::connection::connect(&imap_config).await?;
                let folder_map = provider_sync::consumer_support::sync_imap_folder_map(
                    &mut session,
                    account_id,
                    &inner.write_db,
                )
                .await?;
                let folder_paths = folder_map.keys().cloned().collect::<Vec<_>>();
                provider_sync::consumer_support::run_imap_auxiliary_sync(
                    &mut session,
                    account_id,
                    &inner.write_db,
                    &folder_paths,
                )
                .await;
                Ok(())
            }
            .await
        }
    };
    if let Err(error) = result {
        log::debug!("resident auxiliary pass for {account_id} skipped: {error}");
    }
}

async fn read_initial_sync_completed(
    read_db: &ReadDbState,
    account_id: &str,
) -> Result<bool, String> {
    let aid = account_id.to_string();
    read_db
        .with_read(move |conn| {
            conn.query_row(
                "SELECT initial_sync_completed FROM accounts WHERE id = ?1",
                rusqlite::params![aid],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value != 0)
            .map_err(|error| format!("read initial_sync_completed: {error}"))
        })
        .await
}

#[cfg(test)]
mod tests {
    fn index_after(source: &str, needle: &str, after: usize) -> usize {
        source[after..]
            .find(needle)
            .map(|offset| offset + after)
            .unwrap_or_else(|| panic!("missing resident teardown step {needle:?}"))
    }

    #[test]
    fn resident_slot_teardown_unsubscribes_and_detaches() {
        let source = include_str!("resident.rs");
        let detach = source
            .find("pub async fn detach_account")
            .expect("detach_account exists");
        let cancel = index_after(source, "slot.cancel.cancel()", detach);
        let abort_aux = index_after(source, "slot.aux_task.lock().await.take()", cancel);
        let abort_control = index_after(source, "slot.control_task.lock().await.take()", abort_aux);
        let unregister = index_after(source, "ingress.unregister_account", abort_control);
        let unsubscribe = index_after(source, "unsubscribe_push", unregister);
        let engine_detach = index_after(source, ".detach(&account)", unsubscribe);
        let await_consumer = index_after(source, "task.await", engine_detach);

        assert!(
            cancel < abort_aux
                && abort_aux < abort_control
                && abort_control < unregister
                && unregister < unsubscribe
                && unsubscribe < engine_detach
                && engine_detach < await_consumer,
            "resident detach must cancel, abort aux/control, unregister ingress, unsubscribe push, detach engine, then await the consumer task",
        );
        assert_eq!(
            source[detach..await_consumer]
                .matches(".unsubscribe_push(")
                .count(),
            1,
            "resident detach should call unsubscribe_push exactly once",
        );
    }

    #[test]
    fn push_state_tables_have_no_writer() {
        let checked_sources = [
            include_str!("resident.rs"),
            include_str!("push_ingress/mod.rs"),
            include_str!("push_ingress/pubsub.rs"),
            include_str!("push_ingress/webhook.rs"),
            include_str!("engine_sync.rs"),
            include_str!("../sync.rs"),
            include_str!("../dispatch/post_ready.rs"),
            include_str!("../handlers/account.rs"),
            include_str!("../handlers/oauth.rs"),
        ];
        let writer_verbs = ["INSERT", "UPDATE", "REPLACE", "DELETE"];
        let retired_tables = ["jmap_push_state", "graph_subscriptions"];

        for source in checked_sources {
            for table in retired_tables {
                for verb in writer_verbs {
                    let writer = format!("{verb} {table}");
                    assert!(
                        !source.contains(&writer),
                        "retired push-state table writer still present: {writer}",
                    );
                }
            }
        }
    }
}
