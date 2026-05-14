//! Tasks spawned at init time that park on `boot.ready` and then
//! construct + install per-subsystem runtimes (push, calendar, extract,
//! schema-rebuild). Each task holds an `out_tx` clone via the
//! `NotificationSender` it eventually hands to the runtime; the
//! shutdown drain is responsible for releasing those clones in order.
//!
//! Moved verbatim from the old monolithic `dispatch.rs` - no behaviour
//! change in Phase 1 of the bulletproofing refactor.

use crate::boot;
use std::sync::Arc;
use tokio::sync::mpsc;

/// Phase 4 task 5: post-ready push startup task.
///
/// Spawn a task that waits for `boot.ready`, then constructs a
/// `PushRuntime` and starts a bridge per JMAP account. Per-account
/// starts are themselves `tokio::spawn`'d inside `PushRuntime::start_account`,
/// so a slow initial connect (TLS+HTTPS+OAuth refresh) for one account
/// does not delay the others.
///
/// Push startup explicitly runs *after* `boot.ready` rather than as a
/// boot phase: readiness must not depend on push setup work, and a
/// missing JMAP server (network down at boot) must not block the
/// splash transition. Per-account failure is log-and-continue.
pub(crate) fn spawn_post_ready_push_startup(
    boot_state: Arc<boot::BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if boot_state.wait_for_ready().await.is_err() {
            log::debug!("post-ready push startup: boot failed, skipping push setup");
            return;
        }

        let Some(sync_runtime) = boot_state.sync_runtime() else {
            log::error!(
                "post-ready push startup: SyncRuntime missing after boot.ready - programming error",
            );
            return;
        };
        let Some(db_conn) = boot_state.db_conn() else {
            log::error!(
                "post-ready push startup: db_conn missing after boot.ready - programming error",
            );
            return;
        };
        let Some(key_bytes) = boot_state.encryption_key() else {
            log::error!(
                "post-ready push startup: encryption key missing after boot.ready - programming error",
            );
            return;
        };

        let db_state = service_state::WriteDbState::from_arc(db_conn);
        let encryption_key = crypto_key::SecretKey::from_bytes(key_bytes);
        let notification_tx = crate::boot_progress::NotificationSender::new(out_tx);

        let push_runtime = Arc::new(crate::push::PushRuntime::new(
            db_state.clone(),
            encryption_key,
            sync_runtime,
            notification_tx,
            0,
        ));
        boot_state.install_push_runtime(Arc::clone(&push_runtime));

        let jmap_account_ids: Result<Vec<String>, String> = db_state
            .with_conn(|conn| {
                let mut stmt = conn
                    .prepare("SELECT id FROM accounts WHERE provider = 'jmap'")
                    .map_err(|e| format!("prepare jmap accounts query: {e}"))?;
                let ids = stmt
                    .query_map([], |row| row.get::<_, String>(0))
                    .map_err(|e| format!("query jmap accounts: {e}"))?
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|e| format!("collect jmap account ids: {e}"))?;
                Ok(ids)
            })
            .await;

        let account_ids = match jmap_account_ids {
            Ok(ids) => ids,
            Err(e) => {
                log::warn!("post-ready push startup: failed to enumerate JMAP accounts: {e}");
                return;
            }
        };

        // Phase 8-3: discover dirty accounts via the Phase 3 sync-marker
        // signal. JMAP accounts in this set get a fresh-start push
        // (push_state cleared) so the server delivers `Initial` rather
        // than attempting to resume a cursor that may be ahead of the
        // local DB. Bounded one-time file-listing; no-op on clean boot.
        let app_data_dir = boot_state.app_data_dir().to_path_buf();
        let dirty: std::collections::HashSet<String> =
            crate::startup_invariants::discover_dirty_accounts(&app_data_dir)
                .await
                .into_iter()
                .map(|d| d.account_id)
                .collect();

        log::info!(
            "post-ready push startup: starting bridges for {} JMAP account(s) ({} dirty)",
            account_ids.len(),
            account_ids.iter().filter(|id| dirty.contains(*id)).count()
        );
        for account_id in account_ids {
            let push_runtime = Arc::clone(&push_runtime);
            let fresh_start = dirty.contains(&account_id);
            tokio::spawn(async move {
                if let Err(e) = push_runtime
                    .start_account(account_id.clone(), fresh_start)
                    .await
                {
                    log::warn!("[push] start_account({account_id}) failed: {e}");
                }
            });
        }
    })
}

/// Phase 5 task 8: post-ready calendar startup.
///
/// Parks until `boot.ready`, constructs the `CalendarRuntime`, and
/// installs it on `BootSharedState` so calendar handlers can reach it.
/// Unlike push startup, this does NOT iterate accounts - calendar is
/// kick-driven (`calendar.kick` notification from the UI's `SyncTick`),
/// and the kick handler enumerates accounts itself.
pub(crate) fn spawn_post_ready_calendar_startup(
    boot_state: Arc<boot::BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if boot_state.wait_for_ready().await.is_err() {
            log::debug!("post-ready calendar startup: boot failed, skipping");
            return;
        }

        let Some(db_conn) = boot_state.db_conn() else {
            log::error!(
                "post-ready calendar startup: db_conn missing after boot.ready - programming error",
            );
            return;
        };
        let Some(key_bytes) = boot_state.encryption_key() else {
            log::error!(
                "post-ready calendar startup: encryption key missing after boot.ready - programming error",
            );
            return;
        };

        let db_state = service_state::WriteDbState::from_arc(db_conn);
        let encryption_key = crypto_key::SecretKey::from_bytes(key_bytes);
        let notification_tx = crate::boot_progress::NotificationSender::new(out_tx);

        let calendar_runtime = Arc::new(crate::calendar::CalendarRuntime::new(
            db_state,
            &encryption_key,
            notification_tx,
            0,
        ));
        boot_state.install_calendar_runtime(Arc::clone(&calendar_runtime));

        log::info!("post-ready calendar startup: CalendarRuntime installed");
    })
}

/// Phase 7-4d: post-ready extract startup. Mirrors
/// `spawn_post_ready_calendar_startup` - waits for boot.ready,
/// snapshots the search-writer + body-store + db handles, constructs
/// `ExtractRuntime`, installs it on `BootSharedState`. Extract is
/// kick-driven (`extract.backfill_kick` and per-`attachment.fetch`
/// enqueues), so the post-ready task does not iterate accounts.
pub(crate) fn spawn_post_ready_extract_startup(
    boot_state: Arc<boot::BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: std::path::PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if boot_state.wait_for_ready().await.is_err() {
            log::debug!("post-ready extract startup: boot failed, skipping");
            return;
        }

        let Some(db_conn) = boot_state.db_conn() else {
            log::error!(
                "post-ready extract startup: db_conn missing after boot.ready - programming error",
            );
            return;
        };
        let Some(search_write) = boot_state.take_search_write() else {
            // H1 fix: take_search_write (consume), not search_write
            // (clone). The slot is single-use as the plan promised:
            // either the post-ready spawn consumes it on success, or
            // run_shutdown_drain's defensive take_search_write drains
            // it before awaiting the writer task. Cloning left a
            // SearchWriteHandle in the slot that drain would correctly
            // take, but ALSO held a separate clone in this spawn's
            // local that drain couldn't see - if drain raced ahead of
            // install_extract_runtime, the writer-task await blocked
            // forever on the orphan clone here.
            log::debug!("post-ready extract startup: search_write slot empty (shutdown raced)");
            return;
        };
        let body_read = match store::body_store::BodyStoreReadState::init(&app_data_dir) {
            Ok(b) => b,
            Err(e) => {
                log::error!("post-ready extract startup: body_store init failed: {e}");
                return;
            }
        };

        let db_state = service_state::WriteDbState::from_arc(db_conn);
        let notification_tx = crate::boot_progress::NotificationSender::new(out_tx);

        // ExtractRuntime shares the parent shutdown token via a child:
        // when `Subsystems::drain_runtimes` fires the parent at step 0,
        // the child fires too and the extract worker exits cleanly. A
        // child token (not the parent itself) so a future caller of
        // `extract_runtime.shutdown()` outside the consolidated drain
        // doesn't accidentally cascade to other shutdown_token
        // subscribers.
        let cancellation = boot_state.shutdown_token().child_token();
        let extract_runtime = crate::extract::ExtractRuntime::new(
            db_state,
            app_data_dir,
            Arc::clone(&boot_state),
            search_write,
            body_read,
            notification_tx,
            0,
            cancellation,
        );
        boot_state.install_extract_runtime(extract_runtime);

        log::info!("post-ready extract startup: ExtractRuntime installed");

        // L6 fix: fire the initial backfill kick from inside the
        // post-ready spawn so a UI-side `extract.backfill_kick` that
        // landed before runtime install (race against ServiceBootReady)
        // doesn't get silently no-op'd.
        if let Err(e) = crate::handlers::extract::handle_backfill_kick(&boot_state).await {
            log::warn!(
                "post-ready extract startup: initial backfill kick failed: {e} \
                 (next hourly tick will retry)",
            );
        }
    })
}

/// Attachments roadmap Phase 4: post-ready prefetch startup. Mirrors
/// `spawn_post_ready_extract_startup`. Waits for boot.ready, builds a
/// `PrefetchRuntime`, installs it on `BootSharedState`, then fires a
/// boot-recovery backfill kick for every JMAP account inside the
/// configured `sync_period_days` window.
pub(crate) fn spawn_post_ready_prefetch_startup(
    boot_state: Arc<boot::BootSharedState>,
    out_tx: mpsc::Sender<Vec<u8>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if boot_state.wait_for_ready().await.is_err() {
            log::debug!("post-ready prefetch startup: boot failed, skipping");
            return;
        }
        let Some(db_conn) = boot_state.db_conn() else {
            log::error!(
                "post-ready prefetch startup: db_conn missing after boot.ready - programming error",
            );
            return;
        };
        let db_state = service_state::WriteDbState::from_arc(db_conn);
        let notification_tx = crate::boot_progress::NotificationSender::new(out_tx);
        let cancellation = boot_state.shutdown_token().child_token();
        let runtime = crate::prefetch::PrefetchRuntime::new(
            db_state.clone(),
            Arc::clone(&boot_state),
            notification_tx.clone(),
            0,
            cancellation,
        );
        boot_state.install_prefetch_runtime(runtime);
        let Some(runtime) = boot_state.prefetch_runtime() else {
            // Lost the race with drain. Nothing to do.
            log::debug!(
                "post-ready prefetch startup: runtime was drained before backfill kick",
            );
            return;
        };

        log::info!("post-ready prefetch startup: PrefetchRuntime installed");

        // Boot recovery kick: for every JMAP account, walk every
        // attachment row with NULL content_hash inside the retention
        // window and enqueue. Idempotent w.r.t. the post-sync sweep
        // (in-flight dedupe blocks duplicates).
        let window_days = match db_state
            .with_conn(|conn| Ok(sync::config::get_sync_period_days(conn)))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                log::warn!("post-ready prefetch startup: sync_period_days read failed: {e}");
                return;
            }
        };
        let window_start_unix = chrono::Utc::now().timestamp()
            - window_days.saturating_mul(86_400);

        // Attachments roadmap Phase 8a: eviction sweep BEFORE the
        // backfill kick, so we don't pre-fetch bytes only to tombstone
        // them moments later. max_pages=128 mirrors MAX_BACKFILL_PAGES
        // for the bulk-work startup pass.
        if let Some(pack_store) = boot_state.pack_store() {
            let epoch_arc = boot_state.eviction_epoch();
            let epoch_at_start = epoch_arc.load(std::sync::atomic::Ordering::Relaxed);
            let _ = crate::eviction::run_eviction_sweep(
                db_state.clone(),
                pack_store,
                notification_tx.clone(),
                0,
                crate::eviction::EvictionTrigger::Startup,
                window_start_unix,
                128,
                epoch_arc,
                epoch_at_start,
            ).await;
        }

        // Phase 7: enumerate every provider, not just JMAP. The
        // per-provider concurrency caps and the IMAP folder-batch
        // worker handle the differences inside `PrefetchRuntime`.
        let accounts: Vec<(String, String)> = match db_state
            .with_conn(move |conn| {
                let mut stmt = conn
                    .prepare(
                        "SELECT id, COALESCE(provider, '') FROM accounts \
                         WHERE COALESCE(is_active, 1) = 1 \
                           AND COALESCE(is_deleting, 0) = 0 \
                           AND COALESCE(cache_attachments_enabled, 1) = 1",
                    )
                    .map_err(|e| format!("prepare prefetch boot-kick: {e}"))?;
                let it = stmt
                    .query_map([], |row| {
                        Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                    })
                    .map_err(|e| format!("query prefetch boot-kick: {e}"))?;
                let mut out = Vec::new();
                for r in it {
                    out.push(r.map_err(|e| format!("row prefetch boot-kick: {e}"))?);
                }
                Ok(out)
            })
            .await
        {
            Ok(v) => v,
            Err(e) => {
                log::warn!("post-ready prefetch startup: account enum failed: {e}");
                return;
            }
        };
        for (account_id, provider) in accounts {
            if provider.is_empty() {
                continue;
            }
            if let Err(e) = runtime
                .kick_backfill_account(&account_id, &provider, window_start_unix)
                .await
            {
                log::debug!(
                    "post-ready prefetch startup: backfill kick {account_id} failed: {e}",
                );
            }
        }
    })
}

/// Phase 7-9c: post-ready schema-version rebuild dispatcher.
///
/// If `check_schema_version_and_dispatch` marked a pending rebuild
/// during boot (the persisted `.version` differs from
/// `INDEX_SCHEMA_VERSION`), this task dispatches a PreserveExisting
/// rebuild via the in-process IPC handler. The rebuild writes the new
/// `.version` into its staging slot before publishing the active-index
/// pointer.
///
/// On no-flag: the task immediately exits (steady-state boot).
pub(crate) fn spawn_post_ready_schema_rebuild(
    boot_state: Arc<boot::BootSharedState>,
    app_data_dir: std::path::PathBuf,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        if boot_state.wait_for_ready().await.is_err() {
            return;
        }
        if !boot_state.take_pending_schema_rebuild() {
            return;
        }
        log::info!(
            "post-ready schema rebuild: dispatching PreserveExisting rebuild for INDEX_SCHEMA_VERSION change",
        );

        // M7 fix: if a rebuild is already in flight (the user fired
        // the palette command between boot.ready and our reach here),
        // adopt that rebuild_id rather than trying to dispatch a new
        // one (which would Err with "already in flight" and leave us
        // unable to bump .version - next boot would redundantly
        // re-fire the schema rebuild).
        let rebuild_id = if let Some(in_flight) = boot_state.rebuild_in_flight_id() {
            log::info!(
                "post-ready schema rebuild: adopting in-flight rebuild {in_flight} \
                 (palette racing post-ready) instead of dispatching a new one",
            );
            in_flight
        } else {
            let params = service_api::IndexRebuildParams {
                policy: service_api::RebuildPolicy::PreserveExisting,
                force:  false,
            };
            match crate::handlers::extract::handle_rebuild(&boot_state, params).await {
                Ok(value) => match value
                    .get("rebuild_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
                {
                    Some(id) => id,
                    None => {
                        log::warn!(
                            "post-ready schema rebuild: handle_rebuild ack missing rebuild_id; \
                             skipping .version bookkeeping",
                        );
                        return;
                    }
                },
                Err(e) => {
                    log::warn!("post-ready schema rebuild: dispatch failed: {e:?}");
                    return;
                }
            }
        };

        // Poll the slot for completion. The rebuild task itself calls
        // `take_rebuild_task` on graceful exit; on shutdown drain the
        // slot is also taken. Slot becoming None signals the rebuild
        // ended - cross-check `last_completed_rebuild_id` before
        // writing `.version`.
        let poll_interval = std::time::Duration::from_millis(500);
        loop {
            if boot_state.rebuild_in_flight_id().is_none() {
                break;
            }
            tokio::time::sleep(poll_interval).await;
        }

        // C4 fix: gate the `.version` write to "this specific rebuild
        // ran to clean completion." Cancellation, drain abort, and
        // run_wipe_rebuild_inner errors all leave
        // last_completed_rebuild_id unchanged.
        let completed = boot_state.last_completed_rebuild_id();
        if completed.as_deref() != Some(rebuild_id.as_str()) {
            log::warn!(
                "post-ready schema rebuild {rebuild_id}: did not complete cleanly \
                 (last completed rebuild_id: {completed:?}); leaving .version unchanged \
                 so next boot re-fires",
            );
            return;
        }
        if let Err(e) = boot::write_current_search_index_version(&app_data_dir) {
            log::warn!("post-ready schema rebuild: .version write failed: {e}");
            return;
        }
        log::info!(
            "post-ready schema rebuild {rebuild_id}: .version updated to {}",
            search::INDEX_SCHEMA_VERSION,
        );
    })
}
