//! Service-side boot sequence orchestrator.
//!
//! Runs concurrently with the dispatch loop so `health.ping` continues to
//! round-trip while migrations run. Implements the full Phase 1.5 sequence:
//! key load -> DB open + velo->ratatoskr rename + schema migrations ->
//! pending-ops recovery -> queued-drafts sweep -> thread-participants
//! backfill. Each step emits a corresponding `BootPhase` notification so
//! the splash can render progress.
//!
//! On fatal boot failure (missing key, migration failure, etc.) the
//! sequence does NOT call `std::process::exit` directly: it returns a
//! `BootFailure` to the caller. This is what makes the in-process test
//! harness (`run_service_with_io` over `tokio::io::duplex`) safe to use -
//! a process exit there would kill the test runner. The outer
//! `run_service_blocking` in `lib.rs` is the only caller that converts
//! the boot exit code into an actual `std::process::exit`.

use crate::boot_progress;
use crate::dispatch::DispatchConfig;
use crypto_key::SecretKey;
use tokio_util::sync::CancellationToken;
use db::db::action_journal::recover_stale_leases;
use db::db::pending_ops::db_pending_ops_recover_on_boot_sync;
use db::db::queries_extra::{
    backfill_thread_participants_for_account_sync, db_mark_queued_drafts_failed_sync,
    get_all_accounts_sync, get_all_send_identity_emails,
};
use db::db::{Connection, apply_standard_pragmas, migrations, reconcile_velo_rename};
use db::progress::ProgressReporter;
use service_api::{BootExitCode, BootPhase, BootReadyResponse};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::{Notify, mpsc};

/// Service-side boot artifacts loaded once at boot. The action worker,
/// sync runtime, push runtime, calendar runtime, and rebuild path all
/// consume `encryption_key` and `db_conn` from here via the
/// `BootSharedState::encryption_key()` / `db_conn()` accessors. The
/// `#[allow(dead_code)]` markers below cover the lookup pattern (fields
/// are read through the accessors, not through direct struct access in
/// every consumer); they are not a TODO.
pub(crate) struct BootContext {
    /// AES-256-GCM key loaded from `<app_data>/ratatoskr.key`. Held in a
    /// `SecretKey` wrapper so the bytes zeroize on drop - the Service's
    /// long-lived process otherwise risks the key lingering in freed
    /// heap pages or core dumps for the lifetime of the runtime.
    /// Phase 2's `ActionContext` consumes via `key.expose()` and copies
    /// into the cipher's internal slot.
    #[allow(dead_code)]
    pub(crate) encryption_key: SecretKey,
    /// DB connection opened during boot. Held past the boot sequence so
    /// Phase 2's relocated action service can construct its `ActionContext`
    /// from it without re-opening the file. `Arc<Mutex<Connection>>` matches
    /// the shape `ReadDbState::from_arc` expects.
    #[allow(dead_code)]
    pub(crate) db_conn: Arc<Mutex<Connection>>,
    /// Highest applied schema version after migrations completed. Echoed
    /// to the UI in `BootReadyResponse`.
    pub(crate) schema_version: u32,
    /// Number of migrations actually applied this boot. 0 on a healthy
    /// repeat boot; non-zero only on first-run or after a schema bump.
    pub(crate) migrations_applied: u32,
    /// Short human-readable labels for each recovery step that failed
    /// non-fatally. Empty on a healthy boot. Echoed to the UI in
    /// `BootReadyResponse.recovery_warnings` so the splash transition can
    /// surface "boot ok but recovery had issues" without the UI having to
    /// grep the rolling log file.
    pub(crate) recovery_warnings: Vec<String>,
}

/// Per-Service-instance boot state. The boot task populates `result` (and
/// `context` on success) and fires `notify`; the `boot.ready` handler reads
/// from `result`.
///
/// Held per `run_service_with_io_and_lifecycle` invocation rather than as a
/// process-wide singleton so in-process tests can spawn multiple harnesses
/// without colliding on `OnceLock::set`. Phase 2 will introduce its own
/// per-instance state for the relocated `ActionContext`.
pub(crate) struct BootSharedState {
    notify: Notify,
    result: Mutex<Option<Result<BootReadyResponse, BootFailure>>>,
    #[allow(dead_code)]
    context: Mutex<Option<BootContext>>,
    /// Wakeup channel from the action handler to the action worker.
    /// The handler `notify_one`s this after journaling a plan; the
    /// worker (Phase 2 task 9c) parks on `notified()` and drains the
    /// journal. Using a `Notify` rather than an mpsc keeps the
    /// handler's notify-then-return path lock-free and means a missed
    /// wakeup costs at most one drain delay (the worker re-checks
    /// the journal on every wakeup, not just once per signal).
    action_worker_wakeup: Notify,
    /// Phase 1.5 carry-forward 19h / Phase 2 task 22: bounds parallel
    /// `boot.ready` handlers. Stdio is private so a malicious flood
    /// is not realistic, but a UI bug that re-issues boot.ready
    /// would balloon `JoinSet` linearly (each handler parks on
    /// `Notify` until boot completes). The first caller flips this
    /// flag to true; subsequent callers fail fast with
    /// `BootReadyInFlight` rather than queueing. If the result is
    /// already populated (boot completed but the previous ack got
    /// lost), even fail-fast callers can read the cached result -
    /// `wait_for_ready` checks `result` first so it returns
    /// immediately in that case.
    boot_ready_inflight: std::sync::atomic::AtomicBool,
    /// `<app_data>/` for this Service incarnation. Captured at
    /// `BootSharedState::new` time so handlers (specifically
    /// `action.send` for staging-vault path resolution; future
    /// surfaces likely to follow) can derive paths under
    /// `<app_data>/` without threading the directory through every
    /// dispatch signature. Immutable for the lifetime of the
    /// `BootSharedState`.
    app_data_dir: PathBuf,
    /// Per-account sync coordinator. Installed by the boot task once
    /// the writer halves (DB, body store, inline image store, search
    /// writer task) are ready, before `signal_ready` fires; from that
    /// point on every `sync.start_account` / `sync.cancel_account`
    /// handler reads it via `sync_runtime()`. Held in a `Mutex<Option<_>>`
    /// rather than `OnceLock` so the type stays consistent with the
    /// other boot-installed state on this struct (`context`, `result`).
    sync_runtime: Mutex<Option<Arc<crate::sync::SyncRuntime>>>,
    /// Per-account JMAP push coordinator. Installed by the post-ready
    /// runtime task in `dispatch.rs` (Phase 4 task 5) - readiness must
    /// not depend on push setup work (TLS+HTTPS+OAuth-refresh), so push
    /// startup runs *after* `boot.ready` is signaled. The drain consults
    /// this slot to shut down the push bridges *before* `SyncRuntime`
    /// in the consolidated drain (Phase 4 task 4).
    push_runtime: Mutex<Option<Arc<crate::push::PushRuntime>>>,
    /// Per-account calendar sync coordinator. Phase 5: installed by the
    /// post-ready runtime task in `dispatch.rs` so calendar handlers
    /// (start/cancel/kick) and the consolidated drain (Phase 5 task 7)
    /// can both reach a shared `Arc<CalendarRuntime>`. Same install-once
    /// pattern as `push_runtime` and `sync_runtime`.
    calendar_runtime: Mutex<Option<Arc<crate::calendar::CalendarRuntime>>>,
    /// Phase 7-4d: ExtractRuntime slot. The post-ready startup that
    /// installs into this slot is deferred (initial wiring surfaced
    /// a test-harness hang that needs triage); ExtractRuntime itself
    /// exists in `extract.rs` and the slot + accessors are scaffolded
    /// here so the next slice can wire production producers without
    /// shape churn. Same install-once pattern as `calendar_runtime`.
    #[allow(dead_code)] // 7-4 follow-up wires producers.
    extract_runtime: Mutex<Option<crate::extract::ExtractRuntime>>,
    /// Attachments roadmap Phase 4: `PrefetchRuntime` slot. Installed
    /// by `spawn_post_ready_prefetch_startup`. Drain order in
    /// `Subsystems::drain_runtimes` takes it between
    /// `drain_sync` (sync feeds prefetch) and `drain_extract`
    /// (whose backfill consumes the same rows once bytes land).
    prefetch_runtime: Mutex<Option<crate::prefetch::PrefetchRuntime>>,
    /// `JoinHandle` of the search-writer task. Phase 3 spawned it and
    /// discarded the handle; Phase 4 review-pass fix captures it so the
    /// consolidated drain can await termination after every
    /// `SearchWriteHandle` clone has been dropped (the task exits when
    /// its mpsc rx returns None). Without this, the consolidated drain
    /// has no signal that the writer has actually flushed; the prior
    /// "step 5: await search-writer JoinHandle" doc-comment was
    /// aspirational.
    search_writer_handle: Mutex<Option<tokio::task::JoinHandle<()>>>,
    /// Phase 7-4d: clone of the boot-installed `SearchWriteHandle`,
    /// kept so the post-ready extract startup can grab a clone after
    /// `boot.ready` resolves. The handle itself is `Clone` (cheap,
    /// `Arc<mpsc::Sender>` underneath); this slot installs a single
    /// canonical copy and hands out clones via `search_write()`.
    search_write: Mutex<Option<service_state::SearchWriteHandle>>,
    /// Attachments roadmap Phase 3: `PackStore` handle. Constructed
    /// during the `OpeningBodyAndInlineStores` boot phase (the variant
    /// name is a historical artefact - it now also covers the pack
    /// store). Consumed by `attachment.fetch` (for `materialize_blob`)
    /// and the ExtractRuntime worker. Drain flushes the open pack
    /// before the clean-shutdown sentinel write.
    pack_store: Mutex<Option<Arc<store::PackStore>>>,
    /// Attachments roadmap Phase 3: shared `InlineImageStoreReadState`
    /// for the cache-hit fallback in `attachment.fetch`. The handler
    /// previously called `InlineImageStoreReadState::init` per fetch,
    /// opening a fresh SQLite connection + PRAGMAs each time. A single
    /// shared handle is enough: `InlineImageStoreReadState` is `Clone`
    /// (cheap `Arc<Mutex<Connection>>`) so callers can take a clone
    /// without contending the boot-state mutex.
    inline_image_read:
        Mutex<Option<store::inline_image_store::InlineImageStoreReadState>>,
    /// Phase 7-9: in-flight `index.rebuild` task. Holds the
    /// rebuild_id, the `JoinHandle` for the spawned rebuild, and a
    /// `CancellationToken` the dispatch-side drain can cancel.
    /// `None` between rebuilds. The handler rejects a second rebuild
    /// while this is `Some` and `force == false`.
    rebuild_task: Mutex<Option<RebuildTaskState>>,
    /// Phase 7-9: clone of the dispatch-loop `out_tx`, used by
    /// handlers that need to spawn long-running tasks emitting
    /// notifications (the rebuild task in particular). `None` until
    /// dispatch installs it via `install_out_tx`. Cleared on shutdown
    /// to release the dispatch's stored clone.
    out_tx: Mutex<Option<tokio::sync::mpsc::Sender<Vec<u8>>>>,
    /// Phase 7-9c: pending schema-version rebuild flag.
    /// `check_schema_version_and_dispatch` sets this during boot when
    /// the persisted `.version` differs from
    /// `search::INDEX_SCHEMA_VERSION`. `spawn_post_ready_schema_rebuild`
    /// reads + clears the flag once after `boot.ready` and dispatches
    /// a PreserveExisting rebuild. Stored as `AtomicBool` so the boot
    /// path (synchronous) and the post-ready task can interact without
    /// a lock.
    pending_schema_rebuild: std::sync::atomic::AtomicBool,
    /// Phase 7 (C4 fix): rebuild_id of the most recently *successfully*
    /// completed rebuild. Set by `run_wipe_rebuild_inner` only on the
    /// `Ok(())` exit path - cancellation, drain abort, and error all
    /// leave the previous value unchanged. Consumed by
    /// `spawn_post_ready_schema_rebuild` to gate the `.version` write
    /// to "rebuild completed cleanly," so a drain mid-rebuild leaves
    /// the OLD `.version` on disk and the next boot re-fires.
    last_completed_rebuild_id: Mutex<Option<String>>,
    /// Phase 7 (H1 fix): set by `run_shutdown_drain` before it takes
    /// the extract runtime slot. `install_extract_runtime` checks this
    /// inside its mutex - if set, the runtime is dropped instead of
    /// installed. Closes the post-ready-spawn vs drain race: without
    /// the gate, the spawn could finish constructing an ExtractRuntime
    /// (with a SearchWriteHandle clone in its Inner) and install it
    /// after drain had already taken extract_runtime, leaving the
    /// writer-task await blocked on the orphan clone.
    shutting_down: std::sync::atomic::AtomicBool,
    /// Test-only Service knobs parsed once from argv at Service launch.
    /// Read by the boot sequence (`fake_schema_version`, `boot_delay_ms`)
    /// and by `health.ping` (`fake_protocol_version`). Replaces the
    /// previous module-level `pub static TEST_BOOT_DELAY_MS` /
    /// `crate::test_fake_*()` globals so handlers don't reach across
    /// the crate to read process state.
    config: DispatchConfig,
    /// Cooperative cancellation signal. Fired by
    /// `Subsystems::drain_runtimes` at the start of the consolidated
    /// shutdown drain. Long-running handlers and runtime workers
    /// (`ExtractRuntime` worker, action worker lease loop, GAL
    /// handler) should `select!` on this token alongside their normal
    /// work so they exit promptly instead of waiting out 60 s
    /// per-account timeouts.
    ///
    /// Sub-second `spawn_blocking` DB writes do NOT need to check
    /// this - aborting the outer async future on shutdown is
    /// sufficient for their durations. Handlers with chains of slow
    /// `spawn_blocking` calls or multi-account loops should retrofit
    /// when added.
    shutdown_token: CancellationToken,
    /// Attachments roadmap Phase 8a: monotonic counter bumped on every
    /// window-shrink trigger. Each eviction sweep snapshots the value
    /// at start and re-checks between pages; a divergence means a
    /// later shrink superseded this sweep, and the loop bails so the
    /// fresh trigger can drive the up-to-date window. Startup and
    /// post-sync triggers do not bump but still snapshot, so a
    /// window-shrink fired mid-pass interrupts them too.
    eviction_epoch: Arc<std::sync::atomic::AtomicU64>,
    /// Phase 9 compression-pref cache. `compress_attachments` defaults
    /// to true and `allow_lossy_compression` defaults to false; both
    /// are repopulated from the `settings` table on every successful
    /// `settings.set` that touches either key. Cached atomically here
    /// so `maybe_compress` doesn't hit the DB writer-mutex for two
    /// reads on every attachment fetch (and every prefetch enqueue),
    /// which serialized against unrelated writes during backfill
    /// bursts.
    compress_attachments: std::sync::atomic::AtomicBool,
    allow_lossy_compression: std::sync::atomic::AtomicBool,
}

/// Phase 7-9: tracking state for an in-flight `index.rebuild` task.
/// The handler stores this on `BootSharedState` and the consolidated
/// drain consumes it via `take_rebuild_task` to cancel + await.
pub(crate) struct RebuildTaskState {
    pub rebuild_id: String,
    pub cancel:     tokio_util::sync::CancellationToken,
    pub handle:     tokio::task::JoinHandle<()>,
}

impl BootSharedState {
    pub(crate) fn new(app_data_dir: PathBuf, config: DispatchConfig) -> Arc<Self> {
        Arc::new(Self {
            notify: Notify::new(),
            result: Mutex::new(None),
            context: Mutex::new(None),
            action_worker_wakeup: Notify::new(),
            boot_ready_inflight: std::sync::atomic::AtomicBool::new(false),
            app_data_dir,
            sync_runtime: Mutex::new(None),
            push_runtime: Mutex::new(None),
            calendar_runtime: Mutex::new(None),
            extract_runtime: Mutex::new(None),
            prefetch_runtime: Mutex::new(None),
            search_writer_handle: Mutex::new(None),
            search_write: Mutex::new(None),
            pack_store: Mutex::new(None),
            inline_image_read: Mutex::new(None),
            rebuild_task: Mutex::new(None),
            out_tx: Mutex::new(None),
            pending_schema_rebuild: std::sync::atomic::AtomicBool::new(false),
            last_completed_rebuild_id: Mutex::new(None),
            shutting_down: std::sync::atomic::AtomicBool::new(false),
            config,
            shutdown_token: CancellationToken::new(),
            eviction_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            // Match the documented Phase 6 defaults: compress on,
            // lossy off. The boot path repopulates from the
            // `settings` table once the DB is open; this constructor
            // runs before then.
            compress_attachments: std::sync::atomic::AtomicBool::new(true),
            allow_lossy_compression: std::sync::atomic::AtomicBool::new(false),
        })
    }

    /// Attachments roadmap Phase 8a: bumped on each window-shrink
    /// trigger; eviction sweep snapshots and re-checks between pages
    /// so a fresh shrink supersedes an in-flight startup/post-sync.
    pub(crate) fn eviction_epoch(&self) -> Arc<std::sync::atomic::AtomicU64> {
        Arc::clone(&self.eviction_epoch)
    }

    /// Phase 9: cached compress_attachments setting (default true).
    pub(crate) fn compress_attachments_enabled(&self) -> bool {
        self.compress_attachments
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Phase 9: cached allow_lossy_compression setting (default false).
    pub(crate) fn allow_lossy_compression_enabled(&self) -> bool {
        self.allow_lossy_compression
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Phase 9: refresh both compression-pref atomics from the
    /// `settings` table. Called once during boot (after the DB opens)
    /// and again on every successful `settings.set` that touches
    /// either key.
    pub(crate) fn refresh_compression_prefs(&self, conn: &rusqlite::Connection) {
        let compress = match rtsk::db::queries::get_setting(conn, "compress_attachments") {
            Ok(Some(s)) => s == "true",
            _ => true,
        };
        let lossy = match rtsk::db::queries::get_setting(conn, "allow_lossy_compression") {
            Ok(Some(s)) => s == "true",
            _ => false,
        };
        self.compress_attachments
            .store(compress, std::sync::atomic::Ordering::Relaxed);
        self.allow_lossy_compression
            .store(lossy, std::sync::atomic::Ordering::Relaxed);
    }

    /// Test-only Service knobs parsed once at launch. Read by the
    /// boot sequence and `health.ping`; production builds see the
    /// default (all `false` / `None`).
    pub(crate) fn config(&self) -> &DispatchConfig {
        &self.config
    }

    /// Cooperative cancellation token, fired at the start of the
    /// shutdown drain. Long-running handlers should `select!` on
    /// `shutdown_token().cancelled()` alongside their normal work.
    pub(crate) fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown_token
    }

    /// Phase 7 (H1 fix): mark the boot state as shutting down. Future
    /// post-ready runtime installs drop their argument instead of
    /// installing. Called by `run_shutdown_drain` before it takes
    /// runtime slots.
    pub(crate) fn mark_shutting_down(&self) {
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_shutting_down(&self) -> bool {
        self.shutting_down
            .load(std::sync::atomic::Ordering::SeqCst)
    }

    /// Phase 7 (C4 fix): record that a rebuild ran to clean
    /// completion. `run_wipe_rebuild_inner` calls this only on its
    /// `Ok(())` exit path. The schema-version dispatcher gates its
    /// `.version` write on observing this matches the rebuild_id it
    /// dispatched, so cancellation / drain leaves the old `.version`
    /// on disk and the next boot re-fires.
    pub(crate) fn mark_rebuild_completed(&self, rebuild_id: String) {
        match self.last_completed_rebuild_id.lock() {
            Ok(mut slot) => {
                *slot = Some(rebuild_id);
            }
            Err(e) => {
                log::warn!("last_completed_rebuild_id mutex poisoned: {e}");
            }
        }
    }

    /// Phase 7 (C4 fix): read the last successfully-completed rebuild
    /// id without consuming it. Used by the schema-version dispatcher.
    pub(crate) fn last_completed_rebuild_id(&self) -> Option<String> {
        self.last_completed_rebuild_id
            .lock()
            .ok()
            .and_then(|s| s.clone())
    }

    /// Phase 7-9c: mark a schema-version rebuild as pending. Called
    /// from `check_schema_version_and_dispatch` when the persisted
    /// `.version` differs from `search::INDEX_SCHEMA_VERSION`. The
    /// post-ready spawn observes this and dispatches the rebuild.
    pub(crate) fn mark_pending_schema_rebuild(&self) {
        self.pending_schema_rebuild
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Phase 7-9c: read-and-clear the pending-schema-rebuild flag.
    /// Returns `true` if the flag was set; subsequent calls return
    /// `false` (the post-ready spawn fires the rebuild exactly once
    /// per boot).
    pub(crate) fn take_pending_schema_rebuild(&self) -> bool {
        self.pending_schema_rebuild
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// Phase 7-9: install the dispatch-loop's `out_tx` so non-spawn
    /// handlers (the `index.rebuild` request handler) can mint a
    /// `NotificationSender` for their tracked tasks. Cleared on
    /// shutdown via `take_out_tx` so the dispatch's writer-handle
    /// await is not pinned by the slot's stored clone.
    pub(crate) fn install_out_tx(&self, tx: tokio::sync::mpsc::Sender<Vec<u8>>) {
        let mut slot = self.out_tx.lock().expect("out_tx mutex poisoned");
        if slot.is_some() {
            log::warn!("install_out_tx called twice; second install ignored");
            return;
        }
        *slot = Some(tx);
    }

    /// Phase 7-9: clone the installed `out_tx` and wrap it in a
    /// `NotificationSender`. Returns `None` if dispatch hasn't
    /// installed the sender yet (boot still in progress) or has
    /// cleared it (drain in progress).
    pub(crate) fn notification_sender(
        &self,
    ) -> Option<crate::boot_progress::NotificationSender> {
        self.out_tx
            .lock()
            .expect("out_tx mutex poisoned")
            .clone()
            .map(crate::boot_progress::NotificationSender::new)
    }

    /// Phase 7-9: clear the `out_tx` slot. The dispatch-side drain
    /// calls this before its `drop(out_tx)` so the writer-handle
    /// await observes EOF.
    pub(crate) fn take_out_tx(&self) -> Option<tokio::sync::mpsc::Sender<Vec<u8>>> {
        self.out_tx
            .lock()
            .expect("out_tx mutex poisoned")
            .take()
    }

    /// Phase 7-4d: install the boot-constructed `SearchWriteHandle`.
    /// Boot calls this once after spawning the writer task. Subsequent
    /// async consumers (post-ready extract startup, 7-9 rebuild task,
    /// etc.) read clones via `search_write()`.
    /// `run_shutdown_drain` defensively clears the slot via
    /// `take_search_write` before awaiting the search-writer
    /// JoinHandle so no leftover clone pins the writer task past EOF.
    pub(crate) fn install_search_write(&self, handle: service_state::SearchWriteHandle) {
        let mut slot = self
            .search_write
            .lock()
            .expect("search_write mutex poisoned");
        if slot.is_some() {
            log::warn!("install_search_write called twice; second install ignored");
            return;
        }
        *slot = Some(handle);
    }

    /// Phase 7-4d: clone the installed `SearchWriteHandle` for an
    /// async-spawned consumer. Returns `None` until boot has installed
    /// the handle. Multiple callers may clone independently; the
    /// writer task only exits once *every* clone has been dropped.
    pub(crate) fn search_write(&self) -> Option<service_state::SearchWriteHandle> {
        self.search_write
            .lock()
            .expect("search_write mutex poisoned")
            .clone()
    }

    /// Phase 7-4d: clear the installed `SearchWriteHandle`. Drains
    /// the slot's stored clone so the dispatch-side
    /// `run_shutdown_drain` does not pin the writer task past EOF.
    /// Idempotent on repeat.
    pub(crate) fn take_search_write(&self) -> Option<service_state::SearchWriteHandle> {
        self.search_write
            .lock()
            .expect("search_write mutex poisoned")
            .take()
    }

    /// Install the boot-constructed `PackStore`. Called once during
    /// `OpeningBodyAndInlineStores`. Subsequent installs are logged
    /// and ignored to mirror `install_search_write`.
    pub(crate) fn install_pack_store(&self, store: store::PackStore) {
        let mut slot = self
            .pack_store
            .lock()
            .expect("pack_store mutex poisoned");
        if slot.is_some() {
            log::warn!("install_pack_store called twice; second install ignored");
            return;
        }
        *slot = Some(Arc::new(store));
    }

    /// Clone the installed `PackStore` handle. Returns `None` if boot
    /// has not opened the store yet (handlers should treat that as a
    /// not-ready error).
    pub(crate) fn pack_store(&self) -> Option<Arc<store::PackStore>> {
        self.pack_store
            .lock()
            .expect("pack_store mutex poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    /// Clear the installed `PackStore` handle. The drain takes this
    /// just before flushing + sentinel write so no leftover handle
    /// pins the store past shutdown.
    pub(crate) fn take_pack_store(&self) -> Option<Arc<store::PackStore>> {
        self.pack_store
            .lock()
            .expect("pack_store mutex poisoned")
            .take()
    }

    /// Install the boot-constructed `InlineImageStoreReadState`. Called
    /// once during the inline-image-store init step. Subsequent
    /// installs are logged and ignored.
    pub(crate) fn install_inline_image_read(
        &self,
        read: store::inline_image_store::InlineImageStoreReadState,
    ) {
        let mut slot = self
            .inline_image_read
            .lock()
            .expect("inline_image_read mutex poisoned");
        if slot.is_some() {
            log::warn!("install_inline_image_read called twice; second install ignored");
            return;
        }
        *slot = Some(read);
    }

    /// Clone the installed `InlineImageStoreReadState`. Returns `None`
    /// if boot has not opened the store yet.
    pub(crate) fn inline_image_read(
        &self,
    ) -> Option<store::inline_image_store::InlineImageStoreReadState> {
        self.inline_image_read
            .lock()
            .expect("inline_image_read mutex poisoned")
            .clone()
    }

    /// Phase 7-4d: install the `ExtractRuntime` once the post-ready
    /// startup task has constructed it. Same install-once pattern as
    /// `install_calendar_runtime`.
    ///
    /// Phase 7 (H1 fix): if drain has already marked shutting_down,
    /// drop the runtime instead of installing. The post-ready spawn
    /// holds a `SearchWriteHandle` clone in the runtime's Inner; if
    /// we installed during shutdown, drain's `take_extract_runtime`
    /// would have already returned None (it ran before this call) and
    /// the writer-task await would block forever on the orphan clone.
    /// Dropping the runtime here releases the Inner Arc which drops
    /// the clone, unblocking the writer-task EOF.
    #[allow(dead_code)] // 7-4 follow-up wires producers.
    pub(crate) fn install_extract_runtime(
        &self,
        runtime: crate::extract::ExtractRuntime,
    ) {
        let mut guard = self
            .extract_runtime
            .lock()
            .expect("extract_runtime mutex poisoned");
        if self.is_shutting_down() {
            log::debug!(
                "BootSharedState::install_extract_runtime called during shutdown; \
                 dropping runtime so its SearchWriteHandle clone releases",
            );
            drop(guard);
            drop(runtime);
            return;
        }
        if guard.is_some() {
            log::warn!(
                "BootSharedState::install_extract_runtime called twice; second install ignored",
            );
            return;
        }
        *guard = Some(runtime);
    }

    /// Phase 7-4d: snapshot the active `ExtractRuntime` if boot has
    /// installed one. `None` until the post-ready extract startup
    /// task runs.
    #[allow(dead_code)] // 7-4 follow-up wires producers.
    pub(crate) fn extract_runtime(&self) -> Option<crate::extract::ExtractRuntime> {
        self.extract_runtime
            .lock()
            .expect("extract_runtime mutex poisoned")
            .clone()
    }

    /// Phase 7-4d: move the `ExtractRuntime` out of the slot. Used by
    /// the consolidated drain helper: the runtime drains *between* the
    /// sync drain and the search-writer await, so its
    /// `SearchWriteHandle` + `NotificationSender` clones are released
    /// before the writer task is asked to observe EOF.
    pub(crate) fn take_extract_runtime(&self) -> Option<crate::extract::ExtractRuntime> {
        self.extract_runtime
            .lock()
            .expect("extract_runtime mutex poisoned")
            .take()
    }

    /// Attachments roadmap Phase 4: install the `PrefetchRuntime`.
    /// Shape mirrors `install_extract_runtime` - guarded by
    /// `is_shutting_down` so a startup spawn that loses the race
    /// against drain drops the runtime cleanly instead of orphaning
    /// the worker's `NotificationSender` clone.
    pub(crate) fn install_prefetch_runtime(
        &self,
        runtime: crate::prefetch::PrefetchRuntime,
    ) {
        let mut guard = self
            .prefetch_runtime
            .lock()
            .expect("prefetch_runtime mutex poisoned");
        if self.is_shutting_down() {
            log::debug!(
                "BootSharedState::install_prefetch_runtime called during shutdown; dropping",
            );
            drop(guard);
            drop(runtime);
            return;
        }
        if guard.is_some() {
            log::warn!(
                "BootSharedState::install_prefetch_runtime called twice; second install ignored",
            );
            return;
        }
        *guard = Some(runtime);
    }

    /// Attachments roadmap Phase 4: snapshot the `PrefetchRuntime`
    /// for enqueue sites (post-sync sweep, account-add kick).
    pub(crate) fn prefetch_runtime(&self) -> Option<crate::prefetch::PrefetchRuntime> {
        self.prefetch_runtime
            .lock()
            .expect("prefetch_runtime mutex poisoned")
            .clone()
    }

    /// Attachments roadmap Phase 4: drain helper takes the runtime out
    /// so its `shutdown()` releases the `NotificationSender` clone
    /// before the action-worker await.
    pub(crate) fn take_prefetch_runtime(&self) -> Option<crate::prefetch::PrefetchRuntime> {
        self.prefetch_runtime
            .lock()
            .expect("prefetch_runtime mutex poisoned")
            .take()
    }

    /// Phase 7-9: install the in-flight `index.rebuild` task. The
    /// handler stores the spawned task here so a concurrent
    /// `index.rebuild` request can detect "rebuild already running"
    /// and the consolidated drain can cancel + await on shutdown.
    /// A second install while a rebuild is in flight is rejected by
    /// the handler (with `force == false`) or pre-empted (with
    /// `force == true`).
    pub(crate) fn install_rebuild_task(&self, state: RebuildTaskState) {
        let mut slot = self.rebuild_task.lock().expect("rebuild_task mutex poisoned");
        if let Some(prev) = slot.take() {
            // The handler is supposed to drain the previous slot
            // before installing; warn if not.
            log::warn!(
                "install_rebuild_task: previous rebuild {} not drained; cancelling",
                prev.rebuild_id,
            );
            prev.cancel.cancel();
            prev.handle.abort();
        }
        *slot = Some(state);
    }

    /// Phase 7-9: take the in-flight rebuild slot for cancellation +
    /// await. The consolidated drain calls this between the search-
    /// writer await and the sentinel write so a mid-rebuild Service
    /// shutdown does not leave the index in an inconsistent state.
    pub(crate) fn take_rebuild_task(&self) -> Option<RebuildTaskState> {
        self.rebuild_task
            .lock()
            .expect("rebuild_task mutex poisoned")
            .take()
    }

    /// Phase 7-9: peek the rebuild slot without taking it. The
    /// `index.rebuild` handler uses this to reject concurrent
    /// rebuild requests when `force == false`. Returns the
    /// rebuild_id of the in-flight rebuild for the error message.
    pub(crate) fn rebuild_in_flight_id(&self) -> Option<String> {
        self.rebuild_task
            .lock()
            .expect("rebuild_task mutex poisoned")
            .as_ref()
            .map(|s| s.rebuild_id.clone())
    }

    /// Install the `SyncRuntime` once the boot task has constructed it
    /// (Phase 3 task 12 wires this from `run_boot_sequence_inner`).
    /// Must be called exactly once; a second call is a programming
    /// error and is logged at warn (the first install wins).
    #[allow(dead_code)] // wired up in Phase 3 task 12
    pub(crate) fn install_sync_runtime(&self, runtime: Arc<crate::sync::SyncRuntime>) {
        let mut guard = self
            .sync_runtime
            .lock()
            .expect("sync_runtime mutex poisoned");
        if guard.is_some() {
            log::warn!(
                "BootSharedState::install_sync_runtime called twice; second install ignored",
            );
            return;
        }
        *guard = Some(runtime);
    }

    /// Snapshot the active `SyncRuntime` if boot has installed one.
    /// Returns `None` if boot has not reached the sync-runtime
    /// construction step yet (or if a non-sync code path reaches here
    /// before boot completes).
    pub(crate) fn sync_runtime(&self) -> Option<Arc<crate::sync::SyncRuntime>> {
        self.sync_runtime
            .lock()
            .expect("sync_runtime mutex poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    /// Move the `SyncRuntime` Arc out of the slot. Used by the
    /// dispatch loop's drain step (Phase 3 task 13): once the
    /// supervisors are awaited and this last Arc is dropped, the
    /// inner `SearchWriteHandle` releases its mpsc sender, the search
    /// writer task observes EOF, commits any straggler docs, exits,
    /// and drops its `notification_tx` (the last out_tx clone). At
    /// that point the dispatch loop's writer task can finish.
    pub(crate) fn take_sync_runtime(&self) -> Option<Arc<crate::sync::SyncRuntime>> {
        self.sync_runtime
            .lock()
            .expect("sync_runtime mutex poisoned")
            .take()
    }

    /// Install the `PushRuntime` once the post-ready runtime task has
    /// constructed it (Phase 4 task 5 wires this from `dispatch.rs`
    /// after the `boot.ready` handshake completes). Must be called
    /// exactly once; a second call is a programming error and is
    /// logged at warn (the first install wins).
    #[allow(dead_code)] // wired up in Phase 4 task 5
    pub(crate) fn install_push_runtime(&self, runtime: Arc<crate::push::PushRuntime>) {
        let mut guard = self
            .push_runtime
            .lock()
            .expect("push_runtime mutex poisoned");
        if self.is_shutting_down() {
            log::debug!(
                "BootSharedState::install_push_runtime called during shutdown; dropping runtime",
            );
            return;
        }
        if guard.is_some() {
            log::warn!(
                "BootSharedState::install_push_runtime called twice; second install ignored",
            );
            return;
        }
        *guard = Some(runtime);
    }

    /// Snapshot the active `PushRuntime` if the post-ready task has
    /// installed one. Returns `None` if the post-ready task has not yet
    /// run, or if push startup failed for every account in the iteration.
    #[allow(dead_code)] // consumed by sync.start_account piggyback in Phase 4 task 6
    pub(crate) fn push_runtime(&self) -> Option<Arc<crate::push::PushRuntime>> {
        self.push_runtime
            .lock()
            .expect("push_runtime mutex poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    /// Move the `PushRuntime` Arc out of the slot. Used by the
    /// consolidated drain helper (Phase 4 task 4): push drains *before*
    /// sync so a `StateChange` arriving mid-shutdown cannot call
    /// `SyncRuntime::start_account` after sync has begun draining.
    pub(crate) fn take_push_runtime(&self) -> Option<Arc<crate::push::PushRuntime>> {
        self.push_runtime
            .lock()
            .expect("push_runtime mutex poisoned")
            .take()
    }

    /// Install the `CalendarRuntime` once the post-ready runtime task has
    /// constructed it (Phase 5 task 8). Same install-once / first-wins
    /// pattern as `install_push_runtime`.
    #[allow(dead_code)] // wired up in Phase 5 task 8
    pub(crate) fn install_calendar_runtime(
        &self,
        runtime: Arc<crate::calendar::CalendarRuntime>,
    ) {
        let mut guard = self
            .calendar_runtime
            .lock()
            .expect("calendar_runtime mutex poisoned");
        if self.is_shutting_down() {
            log::debug!(
                "BootSharedState::install_calendar_runtime called during shutdown; dropping runtime",
            );
            return;
        }
        if guard.is_some() {
            log::warn!(
                "BootSharedState::install_calendar_runtime called twice; second install ignored",
            );
            return;
        }
        *guard = Some(runtime);
    }

    /// Snapshot the active `CalendarRuntime` if the post-ready task has
    /// installed one. Returns `None` if calendar startup hasn't run yet
    /// (or if a non-calendar code path reaches here pre-install).
    #[allow(dead_code)] // consumed by calendar handlers in Phase 5 task 4
    pub(crate) fn calendar_runtime(&self) -> Option<Arc<crate::calendar::CalendarRuntime>> {
        self.calendar_runtime
            .lock()
            .expect("calendar_runtime mutex poisoned")
            .as_ref()
            .map(Arc::clone)
    }

    /// Move the `CalendarRuntime` Arc out of the slot. Used by the
    /// consolidated drain helper (Phase 5 task 7): calendar drains
    /// *before* sync so a calendar runner's writes complete (or cancel
    /// cleanly) before sync teardown begins.
    #[allow(dead_code)] // consumed by drain wire-up in Phase 5 task 7
    pub(crate) fn take_calendar_runtime(
        &self,
    ) -> Option<Arc<crate::calendar::CalendarRuntime>> {
        self.calendar_runtime
            .lock()
            .expect("calendar_runtime mutex poisoned")
            .take()
    }

    /// Install the search-writer task's `JoinHandle` once the boot
    /// task has spawned the writer (Phase 4 review-pass fix). The
    /// consolidated drain awaits this handle after dropping the last
    /// `SearchWriteHandle` clone so termination of the writer is
    /// observed rather than relied-on as an undocumented invariant.
    pub(crate) fn install_search_writer_handle(&self, handle: tokio::task::JoinHandle<()>) {
        let mut guard = self
            .search_writer_handle
            .lock()
            .expect("search_writer_handle mutex poisoned");
        if guard.is_some() {
            log::warn!(
                "BootSharedState::install_search_writer_handle called twice; second install ignored",
            );
            return;
        }
        *guard = Some(handle);
    }

    /// Move the search-writer `JoinHandle` out of the slot. Called from
    /// the consolidated drain after `SyncRuntime::shutdown` has dropped
    /// the last `SearchWriteHandle` clone (the writer task exits when
    /// its mpsc rx returns None).
    pub(crate) fn take_search_writer_handle(&self) -> Option<tokio::task::JoinHandle<()>> {
        self.search_writer_handle
            .lock()
            .expect("search_writer_handle mutex poisoned")
            .take()
    }

    /// Path to `<app_data>/` for this Service incarnation. Used by
    /// handlers that need to derive paths under app-data without
    /// piping the directory through dispatch signatures (e.g.
    /// `action.send` resolves staging / vault dirs from this).
    pub(crate) fn app_data_dir(&self) -> &std::path::Path {
        &self.app_data_dir
    }

    /// Try to claim the boot.ready in-flight slot. Returns a guard on
    /// success that releases the slot on drop; returns `None` if a
    /// previous caller is still parked. The handler in
    /// `crates/service/src/handlers/boot.rs` consults this before
    /// awaiting `wait_for_ready` so a UI bug (or future surface) that
    /// fires multiple boot.ready calls cannot balloon the dispatch
    /// `JoinSet` with parked tasks.
    pub(crate) fn try_claim_boot_ready_slot(self: &Arc<Self>) -> Option<BootReadyGuard> {
        if self
            .boot_ready_inflight
            .swap(true, std::sync::atomic::Ordering::AcqRel)
        {
            None
        } else {
            Some(BootReadyGuard {
                state: Arc::clone(self),
            })
        }
    }

    /// Inspect the cached `result`, used by the boot.ready handler when
    /// it loses the in-flight slot race: if the result has already
    /// landed, a "second caller" can satisfy the request from the
    /// cache instead of failing.
    pub(crate) fn cached_result(&self) -> Option<Result<BootReadyResponse, BootFailure>> {
        self.result.lock().expect("boot result poisoned").clone()
    }

    /// True only after the boot sequence has completed successfully.
    /// Used by the shutdown drain to avoid writing a clean-shutdown
    /// sentinel for a Shutdown request that arrived while boot was still
    /// incomplete.
    pub(crate) fn boot_succeeded(&self) -> bool {
        matches!(self.cached_result(), Some(Ok(_)))
    }

    /// Wake up the action worker so it drains the journal. Called by
    /// the `action.execute_plan` handler after journaling a plan
    /// (Phase 2 task 9b). The worker (task 9c) parks on
    /// `await_action_worker_wakeup()` and re-scans the journal on
    /// every wake. Idempotent under the hood (Notify's
    /// `notify_one`-without-waiter semantics still wake the next
    /// `notified().await`).
    pub(crate) fn notify_action_worker(&self) {
        self.action_worker_wakeup.notify_one();
    }

    #[allow(dead_code)] // consumed by the worker in task 9c
    pub(crate) fn await_action_worker_wakeup(&self) -> tokio::sync::futures::Notified<'_> {
        self.action_worker_wakeup.notified()
    }

    /// Park until `signal_ready` fires. The boot.ready handler calls this.
    ///
    /// The outer `loop` is defense-in-depth: today `signal_ready` populates
    /// `result` exactly once and `notify.notify_waiters()` wakes every
    /// parked task, so the second iteration always finds `Some(result)`.
    /// The loop survives a future refactor that might use `notify_one` or
    /// add a spurious-wakeup path; keeping it costs one extra mutex check
    /// in the unreachable case.
    /// Get a clone of the DB connection arc once boot has populated
    /// `context`. Returns `None` if boot is still in flight or has not
    /// run (which is a contract violation for any caller other than the
    /// boot task itself - by the time `boot.ready` returns, `context`
    /// is populated and stays so for the lifetime of the Service).
    ///
    /// Used by handlers (`action.job_status` today; the action service
    /// handler+worker in task 9) that need to query the journal after
    /// boot. Cloning the `Arc` lets the handler drive its own
    /// `spawn_blocking` against the connection without holding the
    /// `BootSharedState` mutex across the blocking work.
    pub(crate) fn db_conn(&self) -> Option<Arc<Mutex<Connection>>> {
        let guard = self.context.lock().expect("boot context poisoned");
        guard.as_ref().map(|ctx| Arc::clone(&ctx.db_conn))
    }

    /// Build a fresh `WriteDbState` wrapper for a Phase 6a IPC handler.
    /// Returns `Err(ServiceError::Internal)` with a uniform message if
    /// `db_conn` is not yet populated (boot still in flight, or
    /// post-respawn pre-`boot.ready`).
    ///
    /// Phase 6a centralises the boilerplate every write-surface handler
    /// would otherwise repeat (`db_conn()? -> WriteDbState::from_arc`).
    /// Each `WriteDbState` is a cheap `Arc::clone` wrapper, so handlers
    /// constructing one per request stay fine even though boot already
    /// holds a canonical instance for `SyncRuntime`.
    pub(crate) fn write_db_state(
        &self,
    ) -> Result<service_state::WriteDbState, service_api::ServiceError> {
        let conn = self.db_conn().ok_or_else(|| {
            service_api::ServiceError::Internal(
                "request received before db_conn available; UI must wait for boot.ready".into(),
            )
        })?;
        Ok(service_state::WriteDbState::from_arc(conn))
    }

    /// Snapshot the encryption key out of `BootContext` once boot has
    /// populated it. Returns the raw 32 bytes by copy because the
    /// action service consumers (`ActionContext::encryption_key`,
    /// SMTP credential decrypt) take `[u8; 32]`. Returns `None` if
    /// boot is still in flight or has not run.
    pub(crate) fn encryption_key(&self) -> Option<[u8; 32]> {
        let guard = self.context.lock().expect("boot context poisoned");
        guard.as_ref().map(|ctx| *ctx.encryption_key.expose())
    }

    pub(crate) async fn wait_for_ready(&self) -> Result<BootReadyResponse, BootFailure> {
        loop {
            if let Some(result) = self.result.lock().expect("boot result poisoned").clone() {
                return result;
            }
            // Build the waiter BEFORE re-checking `result` to avoid losing a
            // `notify_waiters()` that fires between the check and the await.
            let waiter = self.notify.notified();
            tokio::pin!(waiter);
            if let Some(result) = self.result.lock().expect("boot result poisoned").clone() {
                return result;
            }
            waiter.as_mut().await;
        }
    }

    fn signal_ready(
        &self,
        result: Result<BootReadyResponse, BootFailure>,
        context: Option<BootContext>,
    ) {
        {
            let mut guard = self.result.lock().expect("boot result poisoned");
            if guard.is_some() {
                // Double-signal is a "should never happen" - the boot task
                // is the sole producer. A Phase 2 retry path that calls
                // `signal_ready` twice with different results would
                // silently lose the second one; warn so the situation is
                // visible in the rolling log without forcing a debug
                // build.
                log::warn!(
                    "BootSharedState::signal_ready called twice; second call ignored - \
                     this is a programming error, the boot task should signal exactly once",
                );
            } else {
                *guard = Some(result);
            }
        }
        if let Some(ctx) = context {
            let mut guard = self.context.lock().expect("boot context poisoned");
            if guard.is_none() {
                *guard = Some(ctx);
            }
        }
        self.notify.notify_waiters();
    }
}

/// RAII guard returned by `BootSharedState::try_claim_boot_ready_slot`.
/// Releases the in-flight flag on drop so a future legitimate retry
/// (e.g. after a respawn) can re-claim. Tied to the `BootSharedState`
/// the slot belongs to via an `Arc` clone.
pub(crate) struct BootReadyGuard {
    state: Arc<BootSharedState>,
}

impl Drop for BootReadyGuard {
    fn drop(&mut self) {
        self.state
            .boot_ready_inflight
            .store(false, std::sync::atomic::Ordering::Release);
    }
}

/// Discriminant of why the boot sequence failed. The caller maps this to a
/// `BootExitCode` via `as_exit_code()`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BootFailure {
    KeyLoadFailure,
    MigrationFailure,
}

impl BootFailure {
    pub(crate) fn as_exit_code(self) -> BootExitCode {
        match self {
            Self::KeyLoadFailure => BootExitCode::KeyLoadFailure,
            Self::MigrationFailure => BootExitCode::MigrationFailure,
        }
    }
}

/// Run the Service boot sequence.
///
/// Emits `BootPhase::*` notifications via `out_tx` so the UI splash can
/// render progress. Synchronous DB / filesystem work runs in
/// `tokio::task::spawn_blocking` so the dispatch task and the writer task
/// (which pumps notifications) never starve waiting on `rusqlite` /
/// `std::fs::read_to_string`.
///
/// On success, populates `state.context` and signals
/// `state.result(Ok(BootReadyResponse))`. On failure, signals
/// `state.result(Err(BootFailure))` and returns the failure to the caller;
/// the dispatch loop drives the actual process exit.
pub(crate) async fn run_boot_sequence(
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: PathBuf,
    state: Arc<BootSharedState>,
    had_clean_shutdown: bool,
) -> Result<(), BootFailure> {
    let inner = run_boot_sequence_inner(
        out_tx,
        app_data_dir,
        Arc::clone(&state),
        had_clean_shutdown,
    )
    .await;
    let (result, context) = match inner {
        Ok(ctx) => {
            let schema_version = state
                .config()
                .fake_schema_version
                .unwrap_or(ctx.schema_version);
            (
                Ok(BootReadyResponse {
                    ready: true,
                    schema_version,
                    migrations_applied: ctx.migrations_applied,
                    recovery_warnings: ctx.recovery_warnings.clone(),
                }),
                Some(ctx),
            )
        }
        Err(failure) => (Err(failure), None),
    };
    let outcome = match &result {
        Ok(_) => Ok(()),
        Err(failure) => Err(*failure),
    };
    state.signal_ready(result, context);
    outcome
}

async fn run_boot_sequence_inner(
    out_tx: mpsc::Sender<Vec<u8>>,
    app_data_dir: PathBuf,
    state: Arc<BootSharedState>,
    had_clean_shutdown: bool,
) -> Result<BootContext, BootFailure> {
    // Test-only: `--test-boot-delay-ms=N` inserts an artificial sleep
    // before the LoadingKey phase emits so the in-process integration
    // tests can verify that `boot.ready` actually parks on
    // `BootSharedState` rather than racing past via a fast-DB no-delay
    // path. Returns 0 by default in production builds.
    let delay = state.config().boot_delay_ms.unwrap_or(0);
    if delay > 0 {
        log::debug!("test-helpers: artificial boot delay {delay}ms before LoadingKey");
        tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
    }

    boot_progress::emit(&out_tx, BootPhase::LoadingKey, None);

    let key = match tokio::task::spawn_blocking({
        let dir = app_data_dir.clone();
        move || crypto_key::load_encryption_key(&dir)
    })
    .await
    {
        Ok(Ok(key)) => key,
        Ok(Err(error)) => {
            log::error!(
                "encryption key load failed for {}: {error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::KeyLoadFailure);
        }
        Err(join_error) => {
            log::error!(
                "encryption key load task panicked for {}: {join_error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::KeyLoadFailure);
        }
    };

    boot_progress::emit(&out_tx, BootPhase::OpeningDatabase, None);

    let migrate_outcome = tokio::task::spawn_blocking({
        let dir = app_data_dir.clone();
        let progress_tx = out_tx.clone();
        move || open_db_and_migrate(&dir, &progress_tx)
    })
    .await;

    let MigrateOutcome {
        conn,
        schema_version,
        migrations_applied,
    } = match migrate_outcome {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => {
            log::error!(
                "DB open / migration failed for {}: {error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::MigrationFailure);
        }
        Err(join_error) => {
            log::error!(
                "DB open / migration task panicked for {}: {join_error}",
                app_data_dir.display(),
            );
            return Err(BootFailure::MigrationFailure);
        }
    };

    let conn = Arc::new(Mutex::new(conn));
    let mut recovery_warnings: Vec<String> = Vec::new();

    // RecoveringPendingOps: state-repair on the pending_operations table
    // (resets stranded `status='executing'` rows to 'pending') AND on
    // local_drafts (resurfaces stranded `sync_status='sending'` rows as
    // 'failed'). The two repairs share a phase because both target rows
    // that were mid-mutation when the previous Service died, and both run
    // off the same connection. Distinct from SweepingQueuedDrafts below,
    // which targets a different sync_status value ('queued') and a
    // different failure mode (drafts that never made it into 'sending').
    boot_progress::emit(&out_tx, BootPhase::RecoveringPendingOps, None);
    if let Err(error) = run_boot_recovery(&conn, "pending-ops recovery", |c| {
        db_pending_ops_recover_on_boot_sync(c)
    })
    .await
    {
        log::warn!("pending-ops boot recovery failed: {error}");
        recovery_warnings.push("pending-ops recovery".to_string());
    }

    // Action-journal stale-lease reset. Any `action_jobs` row in `leased` /
    // `executing` and any `action_job_ops` row in `leased` / `executing`
    // belongs to a previous Service incarnation that's already gone; the
    // worker UUID in lease_owner cannot be the current one. Reset to
    // `queued` / `pending` so the worker re-leases on its first scheduling
    // pass. Runs before the worker spawns; without this, a SIGKILL during
    // batch_execute strands the in-flight op forever (lease_next_ready_op
    // filters strictly on status='pending' and ignores lease_expires_at,
    // and replay_unemitted requires outcome IS NOT NULL which the crashed
    // op never wrote).
    if let Err(error) = run_boot_recovery(&conn, "action-journal stale leases", |c| {
        let (jobs_reset, ops_reset) = recover_stale_leases(c)?;
        if jobs_reset > 0 || ops_reset > 0 {
            log::info!(
                "[action-journal] Reset {jobs_reset} stale jobs + {ops_reset} stale ops back to queued/pending"
            );
        }
        Ok(())
    })
    .await
    {
        log::warn!("action-journal stale-lease recovery failed: {error}");
        recovery_warnings.push("action-journal stale leases".to_string());
    }

    // Send-vault orphan cleanup (Phase 2 task 5 of compose-send
    // relocation). Walks `<app_data>/send_vault/` and removes any
    // subdirectory whose name does not parse as a UUIDv7 OR whose
    // parsed PlanId is not in the set of "live" send jobs (kind =
    // 'send' and status NOT IN ('completed', 'failed')). Three crash
    // scenarios this catches:
    //
    // - Handler died mid-transfer: bytes in vault, no journal row -
    //   orphan, removed.
    // - Worker died after finalize but before cleanup_vault_dir:
    //   journal terminal, vault still on disk - orphan, removed.
    // - Worker died mid-execution: journal queued/leased/executing
    //   (the `live` set); vault preserved - the next worker pass
    //   replays the SMTP submit.
    //
    // Folded into the same recovery block as pending-ops because both
    // are state-repair passes that run before the boot handshake
    // signals readiness; no separate BootPhase notification (the
    // pass is fast - filesystem walk, no user-visible progress).
    if let Err(error) = reconcile_send_vault(&conn, &app_data_dir).await {
        log::warn!("send-vault orphan cleanup failed: {error}");
        recovery_warnings.push("send-vault cleanup".to_string());
    }

    // SweepingQueuedDrafts: marks `local_drafts.sync_status='queued'` rows
    // as 'failed' so the user sees them surfaced in the drafts view rather
    // than stuck in a queue that the previous Service never drained. Phase
    // 1.5 has no live action service, so any 'queued' row is by definition
    // orphaned. Distinct from the 'sending' resurfacing above (different
    // sync_status, different lifecycle stage).
    boot_progress::emit(&out_tx, BootPhase::SweepingQueuedDrafts, None);
    if let Err(error) = run_boot_recovery(&conn, "queued-drafts sweep", |c| {
        let count = db_mark_queued_drafts_failed_sync(c)?;
        if count > 0 {
            log::info!("[drafts] Resurfaced {count} orphaned 'queued' drafts as 'failed'");
        }
        Ok(())
    })
    .await
    {
        log::warn!("queued-drafts sweep failed: {error}");
        recovery_warnings.push("queued-drafts sweep".to_string());
    }

    boot_progress::emit(&out_tx, BootPhase::BackfillingThreadParticipants, None);
    if let Err(error) = run_boot_recovery(&conn, "thread-participants backfill", |c| {
        run_backfill_for_all_accounts(c)
    })
    .await
    {
        log::warn!("thread-participants backfill failed: {error}");
        recovery_warnings.push("thread-participants backfill".to_string());
    }

    // Phase 6a-part-2: drain `<data_dir>/drafts.wal`. The UI's
    // compose auto-save and window-close paths append synchronously
    // to that file because an async IPC cannot meet the
    // sub-millisecond shutdown bound. Replay each entry into
    // `local_drafts` here so the UI's editor restore reads the
    // fully-replayed state after `boot.ready`. SQLite's UPSERT
    // makes a duplicate replay a no-op; a partial drain is safe to
    // re-run on the next boot.
    boot_progress::emit(&out_tx, BootPhase::DrainingDraftWal, None);
    let app_data_dir_for_drain = app_data_dir.clone();
    if let Err(error) = run_boot_recovery(&conn, "drafts WAL drain", move |c| {
        let count = crate::draft_wal::drain(c, &app_data_dir_for_drain)?;
        if count > 0 {
            log::info!("[drafts] Drained {count} entries from drafts.wal");
        }
        Ok(())
    })
    .await
    {
        log::warn!("drafts WAL drain failed: {error}");
        recovery_warnings.push("drafts WAL drain".to_string());
    }

    // Phase 3 task 12: writer halves + search writer task + sync runtime.
    // These phases land Service-side state that the relocated sync paths
    // depend on, and they run before `signal_ready` so any sync handler
    // that arrives after `boot.ready` finds a fully-installed runtime.
    boot_progress::emit(&out_tx, BootPhase::OpeningBodyAndInlineStores, None);
    let body_write = service_state::BodyStoreWriteState::init(&app_data_dir).map_err(|e| {
        log::error!("body store write init failed: {e}");
        BootFailure::MigrationFailure
    })?;
    let inline_write =
        service_state::InlineImageStoreWriteState::init(&app_data_dir).map_err(|e| {
            log::error!("inline image store write init failed: {e}");
            BootFailure::MigrationFailure
        })?;
    let inline_read =
        store::inline_image_store::InlineImageStoreReadState::init(&app_data_dir)
            .map_err(|e| {
                log::error!("inline image store read init failed: {e}");
                BootFailure::MigrationFailure
            })?;
    state.install_inline_image_read(inline_read);

    // Attachments roadmap Phase 3: open the pack store against the
    // main DB connection. `PackStore::open` runs the in-place open-pack
    // recovery sweep (torn trailing frames truncated, missing index
    // entries re-registered). The pack store shares `conn` because its
    // index (`attachment_blobs`) lives in the main DB schema.
    let pack_store = store::PackStore::open(
        app_data_dir.join("attachment_packs"),
        Arc::clone(&conn),
        store::DEFAULT_PACK_TARGET_SIZE,
    )
    .await
    .map_err(|e| {
        log::error!("PackStore open failed: {e}");
        BootFailure::MigrationFailure
    })?;
    state.install_pack_store(pack_store);

    // Phase 9: prime the compression-pref cache so the very first
    // post-ready attachment fetch reads the user's actual setting
    // rather than the constructor defaults. Re-read on every
    // `settings.set` that touches either key.
    {
        let conn_guard = conn.lock().expect("conn mutex poisoned for compression-pref load");
        state.refresh_compression_prefs(&conn_guard);
    }

    // Attachments roadmap Phase 8b: if `--rebuild-attachment-index`
    // was passed on the command line, walk every sealed pack's frames
    // and replay every tombstone log to repopulate `attachment_blobs`
    // from the on-disk truth. One-shot recovery primitive; service
    // continues normal boot afterwards.
    if crate::rebuild_attachment_index_requested()
        && let Some(pack_store) = state.pack_store()
    {
        match pack_store.rebuild_index().await {
            Ok(stats) => log::info!(
                "rebuild-attachment-index: packs_walked={} frames_indexed={} tombstones_replayed={}",
                stats.packs_walked,
                stats.frames_indexed,
                stats.tombstones_replayed,
            ),
            Err(e) => log::warn!("rebuild-attachment-index failed: {e}"),
        }
    }

    // Attachments roadmap Phase 8b followup: unlink sealed packs with
    // zero `attachment_blobs` rows. Catches two crash modes:
    //   1. Crash mid-`compact_pack` (destination sealed before the
    //      index swap commit) leaves an orphan `.pack` that no
    //      subsequent GC can rediscover.
    //   2. Post-`tombstone_all_live` + GC leaves an empty destination
    //      pack whose rows were DELETEd inside the swap txn.
    // Runs AFTER `rebuild_index` above so a corruption-recovery boot
    // doesn't unlink legitimate packs whose rows the rebuild was
    // about to repopulate.
    if let Some(pack_store) = state.pack_store() {
        match pack_store.sweep_orphan_sealed_packs().await {
            Ok(n) if n > 0 => log::info!("PackStore orphan sweep: unlinked {n} sealed pack(s)"),
            Ok(_) => {}
            Err(e) => log::warn!("PackStore orphan sweep failed: {e}"),
        }
    }

    boot_progress::emit(&out_tx, BootPhase::OpeningSearchIndex, None);

    // Phase 7-1 / Phase 8: schema-version sentinel. Compare the
    // active index slot's `.version` to `search::INDEX_SCHEMA_VERSION`.
    // A mismatch is rebuilt post-ready with PreserveExisting: a staging
    // writer catches up while reads keep serving from the old active
    // slot, then the active-index pointer flips to the rebuilt slot.
    // Called before writer spawn so steady-state boots open the active
    // slot selected by the previous PreserveExisting cutover.
    if let Err(e) = check_schema_version_and_dispatch(&app_data_dir, &state) {
        log::error!("search schema-version check failed: {e}");
        return Err(BootFailure::MigrationFailure);
    }

    let notification_tx = boot_progress::NotificationSender::new(out_tx.clone());
    let writer_db_read = db::db::ReadDbState::from_arc(Arc::clone(&conn));
    let (search_write, search_writer_handle) =
        match crate::search_writer::spawn(
            &app_data_dir,
            writer_db_read,
            notification_tx.clone(),
            0,
        ) {
            Ok(pair) => pair,
            Err(e) => {
                log::error!("search writer spawn failed: {e}");
                return Err(BootFailure::MigrationFailure);
            }
        };
    state.install_search_writer_handle(search_writer_handle);
    // Phase 7-4d: stash a clone for the post-ready extract startup to
    // grab after boot.ready. Sync still owns its own clone (moved
    // into SyncRuntime below); ExtractRuntime gets a separate clone.
    state.install_search_write(search_write.clone());

    let db_write = service_state::WriteDbState::from_arc(Arc::clone(&conn));

    // Phase 3 task 11: invariant pass. Skipped on clean shutdown.
    boot_progress::emit(&out_tx, BootPhase::RunningInvariantPass, None);
    if !had_clean_shutdown {
        let dirty =
            crate::startup_invariants::discover_dirty_accounts(&app_data_dir).await;
        if dirty.is_empty() {
            log::info!(
                "invariant pass: no dirty markers under {}/sync_markers, skipping",
                app_data_dir.display()
            );
        } else {
            // Phase 8-2: open a transient SearchReadState for the
            // Tantivy orphan iteration. The search-writer task already
            // ensured the index directory exists; opening a second
            // reader is cheap and independent of the writer. If the
            // open fails the rest of the pass still runs; the
            // history_id-clear plus next initial-style sync covers
            // index correctness even without orphan iteration.
            let search_read = search::SearchReadState::init(&app_data_dir)
                .map_err(|e| {
                    log::warn!(
                        "invariant pass: SearchReadState::init failed: {e}; \
                         Tantivy orphan iteration skipped"
                    );
                })
                .ok();
            let _stats = crate::startup_invariants::run_invariant_pass(
                &db_write,
                &body_write,
                &inline_write,
                &search_write,
                search_read.as_ref(),
                &app_data_dir,
                &dirty,
            )
            .await;
        }
    }

    // Phase 3 of the attachments roadmap retired the flat-cache
    // reconciliation pass that used to run here. PackStore's
    // open-time recovery (Phase 2) walks the open pack for torn
    // frames and re-indexes missing entries. Pack-level orphan
    // detection (blobs not referenced by any `attachments` row) is
    // Phase 8's responsibility - it lands with the date-windowed
    // tombstoner so the orphan walk and the eviction walk share one
    // pass over the index.

    // Phase 6b: resume any in-flight account deletions. Each marker
    // means a deletion that started but did not finish; the drain
    // walks the canonical step list (bodies, inline images,
    // attachment cache, search index, accounts row CASCADE) and
    // completes the un-finished steps. Idempotent on each step;
    // resilient to repeat boots if a step still fails.
    crate::accounts::drain_pending_deletions(
        &db_write,
        &body_write,
        &inline_write,
        &search_write,
        state.pack_store(),
        &app_data_dir,
    )
    .await;

    // Construct + install the SyncRuntime so the sync handlers
    // (`crates/service/src/handlers/sync.rs`) can reach it via
    // `BootSharedState::sync_runtime()` once boot.ready returns.
    let progress_reporter: Arc<dyn ProgressReporter> = Arc::new(
        crate::progress::IpcProgressReporter::new(out_tx.clone(), String::new()),
    );
    let runtime = Arc::new(crate::sync::SyncRuntime::new(
        db_write,
        body_write,
        inline_write,
        search_write,
        SecretKey::from_bytes(*key.expose()),
        progress_reporter,
        notification_tx,
        app_data_dir.clone(),
        0,
        Arc::clone(&state),
    ));
    state.install_sync_runtime(runtime);

    Ok(BootContext {
        encryption_key: key,
        db_conn: conn,
        schema_version,
        migrations_applied,
        recovery_warnings,
    })
}

/// Run a synchronous recovery step inside `spawn_blocking` against a shared
/// `Arc<Mutex<Connection>>`. The dispatch task and the writer task continue
/// to run on the async side; only the rusqlite work blocks. Errors from
/// recovery steps are logged at warn (the boot sequence proceeds) - per
/// scope items 4, 5, and 5a of `phase-1.5-plan.md`, these steps are state
/// repair, not correctness gates: a failure leaves the DB in the same state
/// the previous boot left it in, which is no worse than skipping recovery.
async fn run_boot_recovery<F>(
    conn: &Arc<Mutex<Connection>>,
    label: &'static str,
    f: F,
) -> Result<(), String>
where
    F: FnOnce(&Connection) -> Result<(), String> + Send + 'static,
{
    let conn = Arc::clone(conn);
    tokio::task::spawn_blocking(move || {
        let conn = conn
            .lock()
            .map_err(|e| format!("db lock poisoned during {label}: {e}"))?;
        f(&conn)
    })
    .await
    .map_err(|e| format!("{label} task panicked: {e}"))?
}

/// Boot-time send-vault orphan cleanup (Phase 2 task 5).
///
/// Reads the set of live send job IDs from the journal, then walks
/// `<app_data>/send_vault/` and removes every subdirectory whose
/// PlanId is not in that set. Each step lives behind its own
/// spawn_blocking (the journal query holds the DB lock; the
/// filesystem walk holds neither lock and can be relatively slow on
/// a populated vault tree).
async fn reconcile_send_vault(
    conn: &Arc<Mutex<Connection>>,
    app_data_dir: &std::path::Path,
) -> Result<(), String> {
    let conn = Arc::clone(conn);
    let live_ids: std::collections::HashSet<service_api::PlanId> =
        tokio::task::spawn_blocking(move || -> Result<_, String> {
            let conn = conn
                .lock()
                .map_err(|e| format!("db lock poisoned: {e}"))?;
            let ids = db::db::action_journal::live_send_job_ids(&conn)?;
            Ok(ids
                .into_iter()
                .map(|bytes| service_api::PlanId(uuid::Uuid::from_bytes(bytes)))
                .collect())
        })
        .await
        .map_err(|e| format!("live_send_job_ids task: {e}"))??;

    let app_data = app_data_dir.to_path_buf();
    let removed = tokio::task::spawn_blocking(move || {
        crate::send_vault::cleanup_orphan_vaults(&app_data, &live_ids)
            .map_err(|e| format!("cleanup_orphan_vaults: {e}"))
    })
    .await
    .map_err(|e| format!("cleanup_orphan_vaults task: {e}"))??;

    if removed > 0 {
        log::info!("[send-vault] removed {removed} orphan vault dir(s) at boot");
    }
    Ok(())
}

/// Per-account thread-participants backfill. Reads accounts + cross-account
/// send-identity emails synchronously, then runs `backfill_thread_participants
/// _for_account_sync` for each account. The helper itself is idempotent (it
/// scans only threads that have no participants row), so this is safe to
/// call on every boot.
///
/// Returns Err if any per-account backfill fails. The outer caller turns
/// that into a `recovery_warnings` entry on `BootReadyResponse`; per-
/// account detail stays in the rolling log file. Without this propagation
/// the UI would see `BootReady` with no warning even though a subset of
/// accounts had partial state repair.
fn run_backfill_for_all_accounts(conn: &Connection) -> Result<(), String> {
    let accounts = get_all_accounts_sync(conn)?;
    if accounts.is_empty() {
        return Ok(());
    }
    let mut user_emails: Vec<String> = accounts
        .iter()
        .map(|a| a.email.to_lowercase())
        .collect();
    for email in get_all_send_identity_emails(conn)? {
        let lower = email.to_lowercase();
        if !user_emails.contains(&lower) {
            user_emails.push(lower);
        }
    }
    let total_accounts = accounts.len();
    let mut failed_accounts: Vec<String> = Vec::new();
    for account in accounts {
        match backfill_thread_participants_for_account_sync(conn, &account.id, &user_emails) {
            Ok(0) => {}
            Ok(count) => log::info!(
                "[chat] thread_participants backfill rebuilt {count} threads for {}",
                account.id
            ),
            Err(error) => {
                log::warn!(
                    "[chat] thread_participants backfill for {} failed: {error}",
                    account.id
                );
                failed_accounts.push(account.id.clone());
            }
        }
    }
    if failed_accounts.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {total_accounts} account(s) failed: {}",
            failed_accounts.len(),
            failed_accounts.join(", "),
        ))
    }
}

/// Outcome of the synchronous DB open + migration step. A struct rather
/// than a magic 3-tuple so future additions (e.g. a "DB was reopened from
/// the post-rename path" flag) can land as named fields.
struct MigrateOutcome {
    conn: Connection,
    schema_version: u32,
    migrations_applied: u32,
}

/// Synchronous DB open + migration step. Runs inside `spawn_blocking` so
/// `rusqlite`'s blocking I/O never starves the dispatch task or the
/// notification writer. Per-step migration progress is pumped via
/// `boot_progress::emit` from the migration runner's callback (try_send is
/// safe from blocking threads since `mpsc::Sender::try_send` is non-async).
fn open_db_and_migrate(
    app_data_dir: &std::path::Path,
    out_tx: &mpsc::Sender<Vec<u8>>,
) -> Result<MigrateOutcome, String> {
    reconcile_velo_rename(app_data_dir)?;
    let db_path = app_data_dir.join("ratatoskr.db");
    let conn = Connection::open(&db_path)
        .map_err(|e| format!("open db {}: {e}", db_path.display()))?;
    apply_standard_pragmas(&conn)?;

    let mut progress = |current: u32, total: u32| {
        // Populate the human-readable message so the splash always has
        // text to render even on a fresh-DB single-migration run where the
        // before-COMMIT and after-COMMIT frames flicker quickly. The UI
        // also derives "Migration N of M" from the structured `current` /
        // `total`; the message is the wire-side label the splash uses
        // when no localised override is wired in.
        let message = if current == 0 {
            format!("Starting migration 1 of {total}")
        } else {
            format!("Applied migration {current} of {total}")
        };
        boot_progress::emit(
            out_tx,
            BootPhase::Migrating { current, total },
            Some(message),
        );
    };
    let migrations_applied = migrations::run_all_with_progress(&conn, &mut progress)?;
    let schema_version = migrations::current_schema_version(&conn)?;
    Ok(MigrateOutcome {
        conn,
        schema_version,
        migrations_applied,
    })
}

/// Phase 7-1 stub, fleshed out in 7-9c. Compares the persisted
/// active search index `.version` to `search::INDEX_SCHEMA_VERSION`:
///
/// - Absent (first-ever boot): write the current version, no rebuild.
///   The writer task creates the index from scratch.
/// - Match: no-op.
/// - Mismatch: leave the `.version` file untouched (sentinel-write
///   ordering: only update after a successful rebuild) and mark a
///   pending rebuild on `BootSharedState`. The post-ready spawn
///   dispatches a PreserveExisting rebuild against a staging slot and
///   rewrites `.version` on success.
///
/// v1 only handles additive schema changes (new fields). The existing
/// index can be opened with the new schema; pre-existing docs simply
/// don't have the new fields, and the rebuild backfills them. A
/// non-additive change would require deleting the index directory
/// during boot (before the writer task opens it); that path is not
/// implemented here. Document if a future bump is non-additive.
///
pub(crate) fn check_schema_version_and_dispatch(
    app_data_dir: &std::path::Path,
    state: &Arc<BootSharedState>,
) -> Result<(), String> {
    let index_dir = search::active_search_index_dir(app_data_dir);
    std::fs::create_dir_all(&index_dir)
        .map_err(|e| format!("create_dir_all {}: {e}", index_dir.display()))?;
    let version_path = index_dir.join(".version");

    let stored: Option<u32> = match std::fs::read_to_string(&version_path) {
        Ok(s) => s.trim().parse::<u32>().ok(),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => return Err(format!("read {}: {e}", version_path.display())),
    };

    match stored {
        None => {
            log::info!(
                "search index .version absent; writing current version {}",
                search::INDEX_SCHEMA_VERSION
            );
            write_search_index_version(&version_path)?;
        }
        Some(v) if v == search::INDEX_SCHEMA_VERSION => {
            log::debug!("search index .version matches current ({v})");
        }
        Some(v) => {
            log::warn!(
                "search index .version mismatch: stored={v}, current={}. \
                 Marking pending rebuild; .version will be rewritten after \
                 successful PreserveExisting rebuild post-boot.ready.",
                search::INDEX_SCHEMA_VERSION
            );
            state.mark_pending_schema_rebuild();
        }
    }
    Ok(())
}

/// Phase 7-9c: Public-to-the-crate write helper. Used by the
/// post-rebuild path to update `.version` only after the rebuild
/// task has emitted `IndexRebuildCompleted`. Pinning the write to
/// after success preserves the sentinel-write ordering invariant: a
/// mid-rebuild crash leaves the OLD `.version` on disk, and the next
/// boot's `check_schema_version_and_dispatch` re-marks the rebuild.
pub(crate) fn write_current_search_index_version(
    app_data_dir: &std::path::Path,
) -> Result<(), String> {
    let index_dir = search::active_search_index_dir(app_data_dir);
    write_search_index_version_at(&index_dir)
}

pub(crate) fn write_search_index_version_at(
    index_dir: &std::path::Path,
) -> Result<(), String> {
    let version_path = index_dir.join(".version");
    write_search_index_version(&version_path)
}

fn write_search_index_version(path: &std::path::Path) -> Result<(), String> {
    std::fs::write(path, search::INDEX_SCHEMA_VERSION.to_string())
        .map_err(|e| format!("write {}: {e}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Smoke test that constructs a `BootContext` and reads every field.
    /// The struct is `#[allow(dead_code)]` on the encryption key + DB
    /// connection until Phase 2's action service starts consuming them; if
    /// a future PR drops a field thinking it was unused, the dead_code
    /// allow would silence the unused-field warning AND this test would
    /// fail to compile, surfacing the loss before it lands. Locks the
    /// scaffold-for-Phase-2 contract.
    #[test]
    fn boot_context_constructs_with_every_field_readable() {
        let conn = Connection::open_in_memory().expect("open in-memory db");
        let ctx = BootContext {
            encryption_key: SecretKey::from_bytes([9u8; 32]),
            db_conn: Arc::new(Mutex::new(conn)),
            schema_version: 100,
            migrations_applied: 1,
            recovery_warnings: vec!["pending-ops recovery".to_string()],
        };
        // Read every field; if Phase 2 drops one of these, the test stops
        // compiling.
        assert_eq!(ctx.encryption_key.expose()[0], 9);
        assert_eq!(ctx.encryption_key.expose().len(), 32);
        assert_eq!(ctx.schema_version, 100);
        assert_eq!(ctx.migrations_applied, 1);
        assert_eq!(ctx.recovery_warnings.len(), 1);
        assert_eq!(ctx.recovery_warnings[0], "pending-ops recovery");
        // The DB connection is held under Arc<Mutex<>>; verify we can
        // acquire it (the shape Phase 2's ActionContext expects).
        let _guard = ctx.db_conn.lock().expect("db_conn mutex");
    }

    /// `signal_ready` symmetry: the success path must populate both
    /// `result` (Some(Ok)) and `context` (Some(_)); the failure path
    /// must populate `result` (Some(Err)) and leave `context` empty.
    /// A future refactor that swaps the args, drops the `if let Some(ctx)`
    /// guard, or otherwise breaks the pairing would only show up in a
    /// downstream Phase 2 test today; this catches it at the boot crate.
    #[test]
    fn signal_ready_symmetry_success_populates_context() {
        let state = BootSharedState::new(PathBuf::new(), DispatchConfig::default());
        let conn = Connection::open_in_memory().expect("open in-memory db");
        let ctx = BootContext {
            encryption_key: SecretKey::from_bytes([1u8; 32]),
            db_conn: Arc::new(Mutex::new(conn)),
            schema_version: 100,
            migrations_applied: 0,
            recovery_warnings: Vec::new(),
        };
        let response = BootReadyResponse {
            ready: true,
            schema_version: 100,
            migrations_applied: 0,
            recovery_warnings: Vec::new(),
        };
        state.signal_ready(Ok(response.clone()), Some(ctx));
        let got_result = state
            .result
            .lock()
            .expect("result mutex")
            .clone();
        assert!(matches!(got_result, Some(Ok(_))));
        let context_present = state
            .context
            .lock()
            .expect("context mutex")
            .is_some();
        assert!(context_present, "success must populate context");
    }

    #[test]
    fn signal_ready_symmetry_failure_leaves_context_empty() {
        let state = BootSharedState::new(PathBuf::new(), DispatchConfig::default());
        state.signal_ready(Err(BootFailure::KeyLoadFailure), None);
        let got_result = state
            .result
            .lock()
            .expect("result mutex")
            .clone();
        assert!(matches!(got_result, Some(Err(BootFailure::KeyLoadFailure))));
        let context_present = state
            .context
            .lock()
            .expect("context mutex")
            .is_some();
        assert!(!context_present, "failure must NOT populate context");
    }

    /// Second `signal_ready` is a no-op: the first result wins, the
    /// second is logged at warn (see implementation) and dropped.
    /// Locks the contract that prevents a Phase 2 retry path from
    /// silently flipping a Service from "ready" to "failed" (or vice
    /// versa) after the first signal.
    #[test]
    fn signal_ready_second_call_is_no_op() {
        let state = BootSharedState::new(PathBuf::new(), DispatchConfig::default());
        let response = BootReadyResponse {
            ready: true,
            schema_version: 100,
            migrations_applied: 0,
            recovery_warnings: Vec::new(),
        };
        state.signal_ready(Ok(response.clone()), None);
        // Second call with a different result: must not overwrite.
        state.signal_ready(Err(BootFailure::MigrationFailure), None);
        let got_result = state
            .result
            .lock()
            .expect("result mutex")
            .clone();
        assert!(matches!(got_result, Some(Ok(_))));
    }

    /// Phase 7-1: schema-version sentinel writes the current version on a
    /// fresh app-data directory (absent `.version` file) and is a no-op on
    /// a subsequent boot at the same version. A mismatch case marks the
    /// pending-rebuild flag rather than rewriting `.version` immediately.
    #[test]
    fn check_schema_version_writes_on_absent_and_no_ops_on_match() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_data = tmp.path();
        let version_path = app_data.join("search_index").join(".version");
        let state = BootSharedState::new(app_data.to_path_buf(), DispatchConfig::default());

        // First call: file is absent, function writes the current version.
        check_schema_version_and_dispatch(app_data, &state).expect("first call");
        let stored = std::fs::read_to_string(&version_path).expect("read .version");
        assert_eq!(
            stored.trim(),
            search::INDEX_SCHEMA_VERSION.to_string(),
            "first call must persist current version"
        );
        assert!(
            !state.take_pending_schema_rebuild(),
            "absent path must not mark a rebuild"
        );

        // Second call: file matches; function is a no-op.
        check_schema_version_and_dispatch(app_data, &state).expect("second call");
        let stored2 = std::fs::read_to_string(&version_path).expect("read .version");
        assert_eq!(stored2, stored, "second call must not change the file");
        assert!(!state.take_pending_schema_rebuild());
    }

    #[test]
    fn check_schema_version_marks_pending_rebuild_on_mismatch() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let app_data = tmp.path();
        let index_dir = app_data.join("search_index");
        std::fs::create_dir_all(&index_dir).expect("create index_dir");
        let version_path = index_dir.join(".version");

        // Seed a deliberately wrong version.
        std::fs::write(&version_path, "999").expect("seed .version");
        let state = BootSharedState::new(app_data.to_path_buf(), DispatchConfig::default());

        check_schema_version_and_dispatch(app_data, &state).expect("dispatch");

        // Sentinel-write ordering: .version stays at the OLD value
        // until a successful rebuild rewrites it.
        let stored = std::fs::read_to_string(&version_path).expect("read .version");
        assert_eq!(stored.trim(), "999", ".version must not be overwritten yet");
        assert!(
            state.take_pending_schema_rebuild(),
            "mismatch must mark a pending rebuild"
        );
        // take_ is single-use.
        assert!(!state.take_pending_schema_rebuild());
    }
}
