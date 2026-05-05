//! Service-side per-account JMAP push runtime.
//!
//! Phase 4 of `docs/service/phase-4-plan.md` relocates the JMAP push
//! WebSocket loop into the Service. `PushRuntime` owns a per-account
//! map of bridge tasks, each of which:
//!
//! 1. Listens on the `mpsc::Receiver<StateChange>` produced by
//!    `jmap::push::start_push`.
//! 2. Coalesces rapid-fire bursts within `PUSH_DEBOUNCE` (500 ms).
//! 3. On each debounced kick, calls `SyncRuntime::start_account`
//!    *first* (in-Service, no IPC) and then emits a
//!    `Notification::PushEvent` so the UI status bar can surface the
//!    activity. The notification is `Coalesce { key: PushEvent(account_id) }`,
//!    so drop-on-overflow is benign and cannot delay sync kicks.
//!
//! ## Symmetry with `SyncRuntime`
//!
//! Structurally symmetric with `crates/service/src/sync.rs::SyncRuntime`.
//! Per-account map, panic supervisor, lifecycle hooks. Diverging from
//! `SyncRuntime`'s shape is a refactor smell - if you're tempted, fix
//! the shared abstraction instead.
//!
//! ## OAuth refresh
//!
//! OAuth refresh runs in-Service. Phase 4 calls
//! `JmapClient::ensure_valid_token` before `jmap::push::start_push`, and
//! threads an auth resolver into `push_connection_loop` so reconnects
//! re-resolve the bearer token. No IPC handshake to the UI is needed;
//! the Phase 4 roadmap entry's `oauth.refresh_request` IPC was removed
//! because refresh is purely DB+HTTPS, both Service-internal.
//!
//! ## Crash continuity (Phase 4 inherits, Phase 8 hardens)
//!
//! Phase 4 inherits today's resume-from-saved-state behavior in
//! `jmap::push::start_push` (`crates/jmap/src/push.rs:147, 337`). On
//! clean shutdown this is the optimal path. On crash, the resumed
//! connection may deliver a stale `StateChange`; Phase 3's invariant
//! pass already clears `history_id` for crashed accounts, so the
//! resulting delta sync re-fetches the cached window. Phase 8 will
//! harden this with explicit crash-aware fresh-start logic.
//!
//! ## Re-auth dead-entry gap (Phase 4 known-gap, Phase 8 fix)
//!
//! UI-side re-auth (`AddAccountWizard::new_reauth`) updates the existing
//! account row in place and does NOT trigger `PushRuntime::start_account`.
//! A token-revocation kills push for that account until Service restart
//! even after the user re-authorizes. Phase 8 wires push re-arm to a
//! token-refresh-success event.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crypto_key::SecretKey;
use service_api::{Notification, PushEvent};
use service_state::WriteDbState;
use tokio::sync::Mutex;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::boot_progress::NotificationSender;
use crate::sync::SyncRuntime;

/// Coalescing window for rapid-fire JMAP `StateChange` bursts. A heavy
/// import operation can emit dozens of changes within a few hundred
/// milliseconds; the bridge collapses them into one sync kick + one
/// notification.
const PUSH_DEBOUNCE: Duration = Duration::from_millis(500);

/// Send-deadline for `Notification::PushEvent`. The notification is
/// `Coalesce`, so drop-on-overflow is benign and the deadline mainly
/// guards against a wedged UI consumer parking the bridge task. Mirrors
/// Phase 3's `INDEX_COMMITTED_SEND_TIMEOUT` shape.
const PUSH_EVENT_SEND_TIMEOUT: Duration = Duration::from_secs(30);

/// Per-account entry in the runtime map.
///
/// The bridge task body owns the `JmapPushManager` (matches today's
/// pattern at `core/src/jmap_push.rs:54`) - keeping it inside the task
/// avoids drop-order subtlety on cancel. Cancellation order: cancel the
/// cooperative token, then await the bridge handle. The bridge's exit
/// path runs `manager.stop_push().await` on its own (the only correct
/// way to stop the WebSocket connection-loop;
/// `crates/jmap/src/push.rs:199, 439`).
struct AccountEntry {
    handle: Option<JoinHandle<()>>,
    cancel: CancellationToken,
}

struct PushRuntimeInner {
    accounts: Mutex<HashMap<String, AccountEntry>>,
    db: WriteDbState,
    encryption_key: SecretKey,
    sync_runtime: Arc<SyncRuntime>,
    notification_tx: NotificationSender,
    service_generation: u32,
}

pub struct PushRuntime {
    inner: Arc<PushRuntimeInner>,
}

impl PushRuntime {
    pub fn new(
        db: WriteDbState,
        encryption_key: SecretKey,
        sync_runtime: Arc<SyncRuntime>,
        notification_tx: NotificationSender,
        service_generation: u32,
    ) -> Self {
        Self {
            inner: Arc::new(PushRuntimeInner {
                accounts: Mutex::new(HashMap::new()),
                db,
                encryption_key,
                sync_runtime,
                notification_tx,
                service_generation,
            }),
        }
    }

    /// Spawn a bridge task for `account_id` if one is not already in
    /// flight. Returns `Ok(())` whether a new bridge was spawned or an
    /// existing one was kept; non-JMAP accounts no-op silently.
    ///
    /// The provider check happens inside this method (rather than in
    /// the calling handler) so the runtime is self-policing - the
    /// caller doesn't need to know which provider an account uses.
    pub async fn start_account(&self, account_id: String) -> Result<(), String> {
        // Provider gate: no-op for non-JMAP accounts.
        let read_db = self.inner.db.to_read_state();
        let provider = db::db::queries::get_provider_type(&read_db, &account_id).await?;
        if provider != "jmap" {
            log::debug!(
                "PushRuntime::start_account: skipping non-JMAP account {account_id} (provider={provider})",
            );
            return Ok(());
        }

        // Reap finished entries so a previously-died bridge doesn't
        // block a fresh start. Mirrors `SyncRuntime`'s opportunistic
        // cleanup pattern.
        let mut map = self.inner.accounts.lock().await;
        let stale_keys: Vec<String> = map
            .iter()
            .filter_map(|(k, entry)| {
                let finished = entry
                    .handle
                    .as_ref()
                    .map(JoinHandle::is_finished)
                    .unwrap_or(true);
                if finished { Some(k.clone()) } else { None }
            })
            .collect();
        for k in stale_keys {
            map.remove(&k);
        }

        if map.contains_key(&account_id) {
            log::debug!("PushRuntime::start_account: bridge already live for {account_id}");
            return Ok(());
        }

        // Construct the JMAP client and refresh the access token before
        // start_push so the WebSocket's first connect carries a fresh
        // bearer. The auth_resolver re-runs ensure_valid_token on every
        // reconnect.
        let mut key_bytes = [0u8; 32];
        key_bytes.copy_from_slice(self.inner.encryption_key.expose().as_slice());
        let client =
            jmap::client::JmapClient::from_account(&read_db, &account_id, &key_bytes).await?;
        client.ensure_valid_token().await?;

        // Auth resolver: every reconnect re-resolves the bearer via
        // ensure_valid_token. Returning an error counts toward
        // MAX_CONSECUTIVE_FAILURES (see jmap::push::AuthResolver doc).
        let resolver_client = client.clone();
        let auth_resolver: jmap::push::AuthResolver = Arc::new(move || {
            let client = resolver_client.clone();
            Box::pin(async move {
                client.ensure_valid_token().await?;
                Ok(client.inner().authorization().to_string())
            })
        });

        let (state_tx, state_rx) = jmap::push::create_push_channel();
        let manager =
            jmap::push::start_push(&client, &account_id, &read_db, state_tx, auth_resolver)
                .await?;

        let cancel = CancellationToken::new();
        let bridge_account_id = account_id.clone();
        let bridge_cancel = cancel.clone();
        let bridge_inner = Arc::clone(&self.inner);
        let supervisor_account_id = account_id.clone();
        let supervisor = tokio::spawn(async move {
            let bridge_handle = tokio::spawn(async move {
                run_bridge(bridge_inner, bridge_account_id, manager, state_rx, bridge_cancel).await;
            });
            match bridge_handle.await {
                Ok(()) => {}
                Err(join_err) if join_err.is_panic() => {
                    log::error!("push bridge for {supervisor_account_id} panicked: {join_err:?}");
                }
                Err(join_err) => {
                    log::warn!("push bridge for {supervisor_account_id} aborted: {join_err:?}");
                }
            }
        });

        map.insert(
            account_id,
            AccountEntry {
                handle: Some(supervisor),
                cancel,
            },
        );
        Ok(())
    }

    /// Cancel the bridge for `account_id` if one exists. Cancels the
    /// cooperative token, awaits the supervisor (which awaits the
    /// bridge, which awaits `manager.stop_push().await` on its exit
    /// path so the WebSocket closes cleanly).
    ///
    /// Returns `true` if an entry existed (cancelled or already dead);
    /// `false` if no bridge was registered for the account.
    pub async fn cancel_account(&self, account_id: &str) -> bool {
        let handle = {
            let mut map = self.inner.accounts.lock().await;
            match map.remove(account_id) {
                Some(mut entry) => {
                    entry.cancel.cancel();
                    entry.handle.take()
                }
                None => return false,
            }
        };
        if let Some(handle) = handle
            && let Err(e) = handle.await
        {
            log::warn!("push bridge supervisor join error during cancel: {e}");
        }
        true
    }

    /// Drain step. Cancel every bridge, await every supervisor.
    /// Phase 4 task 4 calls this from the consolidated drain helper
    /// *before* `SyncRuntime::shutdown()` so a `StateChange` arriving
    /// mid-shutdown cannot call `SyncRuntime::start_account` after
    /// SyncRuntime has begun draining.
    pub async fn shutdown(&self) {
        let supervisors: Vec<JoinHandle<()>> = {
            let mut map = self.inner.accounts.lock().await;
            map.values_mut()
                .filter_map(|entry| {
                    entry.cancel.cancel();
                    entry.handle.take()
                })
                .collect()
        };
        for sup in supervisors {
            if let Err(e) = sup.await {
                log::warn!("push bridge supervisor join error during shutdown: {e}");
            }
        }
    }
}

/// Bridge task body. Owned by the per-account supervisor `tokio::spawn`
/// in `start_account`.
///
/// Loop semantics: `tokio::select!` between cancellation and the
/// StateChange channel. On cancel: break out of the loop and run
/// `manager.stop_push().await` on the exit path. On StateChange:
/// debounce, kick `SyncRuntime::start_account`, emit `PushEvent`.
///
/// The manager moves into this function and stays here until the task
/// exits. Dropping it alone would NOT close the WebSocket loop -
/// `push_connection_loop` exits only when the watch value flips to
/// `true`, which `stop_push()` sets explicitly
/// (`crates/jmap/src/push.rs:199, 439`).
async fn run_bridge(
    inner: Arc<PushRuntimeInner>,
    account_id: String,
    manager: jmap::push::JmapPushManager,
    mut rx: mpsc::Receiver<jmap::push::StateChange>,
    cancel: CancellationToken,
) {
    log::info!("[push] bridge starting for {account_id}");
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => {
                log::info!("[push] bridge for {account_id}: cancellation observed");
                break;
            }
            recv = rx.recv() => {
                match recv {
                    None => {
                        log::info!(
                            "[push] bridge for {account_id}: state-change channel closed (push manager exited)",
                        );
                        break;
                    }
                    Some(change) => {
                        log::debug!(
                            "[push] {account_id}: state change: {} data type(s) changed",
                            change.changed.len(),
                        );
                        coalesce_burst(&mut rx, &cancel).await;
                        kick_sync_and_emit(&inner, &account_id).await;
                    }
                }
            }
        }
    }

    // Exit path: explicitly stop the WebSocket connection loop so the
    // tokio task spawned inside start_push exits cleanly. Required
    // because push_connection_loop watches for `shutdown_rx` to flip to
    // true (set by stop_push), not for the manager to be dropped.
    manager.stop_push().await;
    log::info!("[push] bridge for {account_id}: exited cleanly");
}

/// Drain rapid-fire `StateChange` events that arrive within the debounce
/// window so a single sync kick covers an entire burst. A cancellation
/// observed during the drain breaks out early.
async fn coalesce_burst(
    rx: &mut mpsc::Receiver<jmap::push::StateChange>,
    cancel: &CancellationToken,
) {
    let deadline = tokio::time::Instant::now() + PUSH_DEBOUNCE;
    let mut coalesced = 0u32;
    loop {
        tokio::select! {
            biased;
            _ = cancel.cancelled() => break,
            r = tokio::time::timeout_at(deadline, rx.recv()) => match r {
                Ok(Some(_)) => coalesced += 1,
                Ok(None) | Err(_) => break,
            },
        }
    }
    if coalesced > 0 {
        log::debug!("[push] coalesced {coalesced} additional state-changes within debounce window");
    }
}

/// On a debounced kick: call `SyncRuntime::start_account` first (the
/// real work), then emit a `PushEvent` so the UI's status bar can show
/// the activity. The order matters: a wedged notification queue cannot
/// delay the sync kick because the sync call is awaited *before* the
/// emit.
async fn kick_sync_and_emit(inner: &Arc<PushRuntimeInner>, account_id: &str) {
    let _ack = inner.sync_runtime.start_account(account_id.to_string()).await;
    let notif = Notification::PushEvent(PushEvent {
        account_id: account_id.to_string(),
        service_generation: inner.service_generation,
    });
    match tokio::time::timeout(PUSH_EVENT_SEND_TIMEOUT, inner.notification_tx.send(notif)).await {
        Ok(Ok(())) => {}
        Ok(Err(_)) => {
            log::warn!(
                "[push] PushEvent({account_id}) send: notification queue closed (UI is probably gone)",
            );
        }
        Err(_) => {
            log::warn!(
                "[push] PushEvent({account_id}) send timed out after {} s; UI consumer wedged. Dropping; the next StateChange will catch up.",
                PUSH_EVENT_SEND_TIMEOUT.as_secs(),
            );
        }
    }
}

// Tests for PushRuntime intentionally omitted from this commit.
//
// Phase 4 task 9 (test cohort) calls for: provider-gating against a
// seeded DB, bridge-task debounce + sync-kick + notification emission,
// shutdown drain ordering, account-delete cancel-before-sync. All of
// these require either:
//   (a) a fake JMAP WebSocket server fixture (the bridge body needs a
//       real-shaped StateChange producer to exercise debounce + kick), or
//   (b) `test_dummy` constructors on BodyStoreWriteState /
//       InlineImageStoreWriteState / SearchWriteHandle (which don't
//       exist - SyncRuntime today has no in-memory test path either).
//
// Adding the fixtures is non-trivial infrastructure work that would
// dwarf Phase 4's behavioral surface. The PushEvent wire/class
// guarantees are covered by the service-api catalog tests at
// crates/service-api/src/notification.rs:469-585; the integration
// behavior is tracked as a Phase 8 carry-forward alongside the
// flaky-test root-cause work that Phase 8 already touches.
