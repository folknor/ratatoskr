// ── Phase 1.5 Booting-state audit (per scope item 21 of phase-1.5-plan.md) ──
//
// The App state machine introduced in commit 13 splits into `Booting` and
// `Ready` variants. While in `Booting` (between iced startup and the
// `boot.ready` handshake completing), most messages are not actionable
// because the DB / accounts / sidebar are not yet constructed. The
// dispatcher whitelists a small set of messages and drops everything else
// at debug level.
//
// Behaviour codes:
//   handle  - dispatcher updates `Booting` state.
//   drop    - dispatcher logs at debug and discards.
//   forward - stash on Booting, replay after Booting -> Ready transition
//             (for messages that affect persistent state like settings).
//
// New variants added in commit 13 (the ones that drive the spawn flow):
//   ServiceChildSpawned(Arc<ServiceClient>)        - handle (populate client)
//   ServiceBootReady(BootReadyResponse)            - handle (transition Ready)
//   ServiceBootFailed(BootFailureReason)           - handle (log + iced::exit;
//                                                    AnotherInstanceRunning
//                                                    gets a friendly message,
//                                                    everything else technical)
//
// Existing Service-related:
//   ServiceReady (Result<Arc<ServiceClient>, _>)   - REPLACED in commit 13 by
//                                                    the three variants above
//   ServiceNotification(Notification)              - handle (boot.progress
//                                                    drives splash; other
//                                                    notifications drop while
//                                                    Booting)
//   ServiceShutdownComplete(Result<(), _>)         - drop (no Service exists)
//
// Window / appearance:
//   WindowResized(id, size)                        - drop (BootingApp does
//                                                    not own a WindowState;
//                                                    ReadyApp loads the saved
//                                                    geometry from disk on
//                                                    transition. The splash
//                                                    is short-lived enough
//                                                    that mid-boot resizes
//                                                    not being persisted is
//                                                    acceptable.)
//   WindowMoved(id, point)                         - drop (same reason)
//   WindowCloseRequested(id)                       - handle (iced::exit if
//                                                    id matches main window;
//                                                    drop otherwise. Only
//                                                    the main window exists
//                                                    during Booting, so the
//                                                    drop arm is unreachable
//                                                    in practice.)
//   AppearanceChanged(mode)                        - forward (stash for Ready)
//
// Harmless / no state:
//   Noop                                           - drop (silent)
//   ModifiersChanged(modifiers)                    - drop (no UI to apply to)
//
// Settings (UI not rendered; the rest of the bootstrap snapshot loads
// after Booting -> Ready):
//   Settings(_) | ToggleSettings | SettingsCheckFocus | SetTheme | SetDateDisplay
//   SetReadingPanePosition | ToggleSidebar | ToggleRightSidebar
//                                                  - drop
//
// Data loading and component messages: all dropped while `Booting` since
// they reference state types (Db, Sidebar, ThreadList, etc.) that are not
// constructed until the Booting -> Ready transition. Includes:
//   AccountsLoaded | NavigationLoaded | ThreadsLoaded
//   Sidebar(_) | ThreadList(_) | ReadingPane(_) | StatusBar(_)
//   ThreadDetailLoaded | NavigateTo | EmailAction | ActionCompleted
//   SendCompleted | ComposeAction | TaskAction | ExecuteCommand
//   ExecuteParameterized | KeyEvent | Escape | Compose | FocusSearch
//   FocusSearchBar | SearchBlur | SearchQueryChanged | SearchExecute
//   SearchCompleted | SearchClear | SearchHistoryLoaded | SearchHere
//   SaveAsSmartFolder | SmartFolderCreateAck | ShowHelp
//
// Pinned-search and snooze messages: dropped while Booting (the periodic
// timers will fire again after Booting -> Ready):
//   PinnedSearchesLoaded | SelectPinnedSearch | DismissPinnedSearch
//   PinnedSearchDeleteAck | PinnedSearchCreateOrUpdateAck
//   PinnedSearchUpdateAck | PinnedSearchDeleteAllAck
//   RefreshPinnedSearch | ClearAllPinnedSearches
//   SnoozeTick | SnoozeResurfaceComplete
//
// Sync / account / push: dropped while Booting (Service hasn't reached
// Ready, so any sync attempt would panic on the missing DB):
//   SyncTick | SyncCurrentFolder | SyncComplete | SyncProgress
//   AddAccount | OpenAddAccount | AccountDeleted | AccountUpdated
//   ReloadSignatures | SignatureOp | SharedMailboxesLoaded
//   PinnedPublicFoldersLoaded | AutoReplyChecked
//
// Pop-out windows: dropped (the pop-out dispatch path needs ReadyApp
// state). The session restorer fires its own pop-out tasks after Ready.
//   PopOut | OpenMessageView | ComposeDraftTick | LocalDraftLoaded
//   RestoredComposeLoaded
//
// Calendar: dropped (calendar state is loaded after Ready):
//   Calendar | ToggleAppMode | SetAppMode | SetCalendarView
//   CalendarToday | CalendarSyncComplete
//
// Chat: dropped (chat state requires accounts loaded):
//   ChatTimeline | ChatTimelineLoaded | ChatOlderLoaded | ChatReadMarked
//   ChatContactsLoaded
//
// Palette / divider: dropped (UI not rendered):
//   Palette | DividerDragStart | DividerDragMove | DividerDragEnd
//   DividerHover | DividerUnhover
//
// Undo: dropped (no actions to undo before Ready):
//   Undo | UndoCompleted
//
// Future Message variants must include a Booting-state row in this table.
// The `BootingApp::update` enforces this at runtime by logging unrecognised
// variants at debug level rather than panicking.

use crate::app::{AppMode, Divider};
use crate::appearance;
use crate::command_dispatch::{
    ComposeAction, KeyEventMessage, MailActionIntent, NavigationTarget, ReadingPanePosition,
    TaskAction,
};
use crate::db::{self, Thread};
use crate::handlers;
use crate::pop_out::PopOutMessage;
use crate::ui::add_account::AddAccountMessage;
use crate::ui::calendar::{CalendarMessage, CalendarView};
use crate::ui::palette::PaletteMessage;
use crate::ui::reading_pane::ReadingPaneMessage;
use crate::ui::settings::SettingsMessage;
use crate::ui::sidebar::SidebarMessage;
use crate::ui::status_bar::{StatusBarMessage, SyncEvent};
use crate::ui::thread_list::ThreadListMessage;
use cmdk::CommandId;
use iced::{Point, Size};
use rtsk::db::queries_extra::navigation::NavigationState;
use rtsk::generation::{GenerationToken, Nav, ThreadDetail};

#[derive(Debug, Clone)]
pub enum Message {
    // Existing component messages
    Sidebar(SidebarMessage),
    ThreadList(ThreadListMessage),
    ReadingPane(ReadingPaneMessage),
    Settings(SettingsMessage),
    StatusBar(StatusBarMessage),

    // Existing data loading
    AccountsLoaded(GenerationToken<Nav>, Result<Vec<db::Account>, String>),
    NavigationLoaded(GenerationToken<Nav>, Result<NavigationState, String>),
    ThreadsLoaded(GenerationToken<Nav>, Result<Vec<Thread>, String>),

    // Existing UI
    Compose,
    Noop,
    /// Attachments roadmap Phase 5: a Save / Save All dialog returned
    /// a chosen folder; remember it for the next Save inside the same
    /// thread. The tuple key is `(account_id, thread_id)`.
    AttachmentSaveFolderRemembered((String, String), std::path::PathBuf),
    /// Phase 1 of the two-phase spawn (commit 11). Subprocess is up, the
    /// version-check ping succeeded, and the App can now hold the
    /// ServiceClient so it can subscribe to notifications (esp.
    /// `boot.progress` for the splash).
    ServiceChildSpawned(std::sync::Arc<crate::ServiceClient>),
    /// Phase 2 of the two-phase spawn. The Service has migrated, loaded
    /// the encryption key, recovered pending ops, swept queued drafts, and
    /// backfilled thread participants. The App transitions Booting -> Ready.
    ServiceBootReady(service_api::BootReadyResponse),
    /// Spawn or boot.ready failed - or, after the App has reached Ready,
    /// `handle_crash`'s respawn attempt itself failed. Both paths land
    /// here. The App logs the user-visible message via
    /// `service_client::surface_terminal_failure` and exits cleanly via
    /// `iced::exit()`. `BootFailureReason::Classified(...AnotherInstanceRunning)`
    /// is the one case that gets a user-friendly message; everything else
    /// gets a technical message per scope item 16 of phase-1.5-plan.md.
    ServiceBootFailed(crate::service_client::BootFailureReason),
    /// Phase 8-1: coarse Service health for the status-bar indicator.
    /// Emitted by `ServiceClient` on transitions (booting -> healthy,
    /// healthy -> respawning, etc.); the rendering policy is in
    /// `crates/app/src/ui/status_bar.rs`. Distinct from
    /// `ServiceBootFailed` (terminal) and from `ServiceNotification`
    /// (the in-band BootProgress / SyncProgress / etc. stream).
    ServiceHealthChanged(crate::service_client::ServiceHealth),
    /// Phase 8-1: async-init completion for the body store. The
    /// boot-time path now constructs `ReadyApp` with `body_store: None`
    /// and dispatches a `Task::perform` that fires this message when
    /// `BodyStoreReadState::init` completes. The result is
    /// `Result<BodyStoreReadState, String>` - on Err the field stays
    /// None and reading-pane requests for body text show "loading..."
    /// until the next launch retries init.
    BodyStoreReady(Result<rtsk::body_store::BodyStoreReadState, String>),
    /// Phase 8-1: async-init completion for the inline image store.
    /// Same pattern as `BodyStoreReady`.
    InlineImageStoreReady(Result<store::inline_image_store::InlineImageStoreReadState, String>),
    /// Phase 8-1: async-init completion for the search read state.
    /// Same pattern as `BodyStoreReady`. The result is wrapped in
    /// `Arc` because multiple UI surfaces (reading-pane, search-bar)
    /// share read access to the same Tantivy reader handle.
    SearchStateReady(Result<std::sync::Arc<rtsk::search::SearchReadState>, String>),
    ServiceNotification(service_api::Notification),
    ServiceShutdownComplete(Result<(), String>),
    ToggleSettings,
    AppearanceChanged(appearance::Mode),
    DividerDragStart(Divider),
    DividerDragMove(Point),
    DividerDragEnd,
    DividerHover(Divider),
    DividerUnhover,
    WindowResized(iced::window::Id, Size),
    WindowMoved(iced::window::Id, Point),
    ToggleRightSidebar,
    /// Run after each mouse press while settings is open: queries the widget
    /// tree to find which filter input (if any) currently owns focus, then
    /// updates `Settings.focused_filter` accordingly.
    SettingsCheckFocus,
    SetDateDisplay(db::DateDisplay),
    WindowCloseRequested(iced::window::Id),

    // Command system (Slice 6a)
    KeyEvent(KeyEventMessage),
    ExecuteCommand(CommandId),
    ExecuteParameterized(CommandId, cmdk::CommandArgs),
    NavigateTo(NavigationTarget),
    Escape,
    EmailAction(MailActionIntent),
    /// Action service completed - carries action kind, outcomes, rollback, thread IDs, and params.
    ActionCompleted {
        plan: crate::action_resolve::ActionExecutionPlan,
        outcomes: Vec<service_api::actions::ActionOutcome>,
    },
    /// Synchronous response from the IPC `action.execute_plan`
    /// round-trip, classified into the tri-state per Phase 2 plan
    /// scope item 14:
    ///
    /// - `Acked`: Service journaled the plan; outcomes will stream on
    ///   the `ServiceNotification` channel.
    /// - `AckUnknown`: ack lost on the wire (`ServiceCrashed` /
    ///   `Timeout` / wire-corruption). Optimistic state is held; the
    ///   post-`boot.ready` reconciliation flow fires
    ///   `action.job_status` to resolve.
    /// - `Failed`: dispatch never reached the journal (validation
    ///   failure, terminal client error). Roll back and toast.
    ActionDispatched {
        plan_id: service_api::PlanId,
        outcome: crate::service_client::DispatchOutcome,
    },
    /// Result of a post-respawn `action.job_status` query for a plan
    /// that was in `AckUnknown` state when the Service crashed. Drives
    /// the reconciliation arm of Phase 2 plan scope item 14: `Journaled`
    /// promotes the plan to `Acked` (worker replay drives completion);
    /// `NotFound` rolls back optimistic state and removes the plan.
    /// `Err(_)` keeps the plan in `AckUnknown` and logs - the next
    /// respawn will retry.
    JobStatusResolved {
        plan_id: service_api::PlanId,
        result: Result<service_api::JobStatusResponse, String>,
    },
    /// Send completed - carries compose window ID and outcome.
    /// Separate from ActionCompleted because send operates on a compose window,
    /// not a thread list selection.
    SendCompleted {
        window_id: iced::window::Id,
        outcome: service_api::actions::ActionOutcome,
    },
    ComposeAction(ComposeAction),
    TaskAction(TaskAction),
    SetTheme(String),
    ToggleSidebar,
    FocusSearch,
    ShowHelp,
    SyncCurrentFolder,
    /// Periodic sync timer tick.
    SyncTick,
    /// A sync operation completed (success or failure). Reload navigation.
    SyncComplete(String, Result<(), String>),
    SetReadingPanePosition(ReadingPanePosition),

    // Palette placeholder (Slice 6b)
    Palette(PaletteMessage),

    // Search
    SearchQueryChanged(String),
    SearchExecute,
    SearchCompleted(crate::handlers::search::SearchExecutionResult),
    SearchClear,
    FocusSearchBar,
    SearchBlur,
    SearchHistoryLoaded(Result<Vec<String>, String>),

    // Pinned searches
    PinnedSearchesLoaded(Result<Vec<db::PinnedSearch>, String>),
    SelectPinnedSearch(i64),
    DismissPinnedSearch(i64),
    /// `pinned_search.delete` IPC ack. Phase 6a-part-2: dedicated
    /// per-method ack variant (renamed from `PinnedSearchDismissed`)
    /// matching the signature CRUD precedent of one ack per IPC.
    PinnedSearchDeleteAck(i64, Result<(), String>),
    /// `pinned_search.create_or_update` IPC ack. Phase 6a-part-2:
    /// dedicated variant covering the create-snapshot and first-save
    /// path. Routes into the same downstream
    /// `handle_pinned_search_persisted` helper as
    /// `PinnedSearchUpdateAck` so behavioural divergence remains
    /// possible without duplicating dispatch logic.
    PinnedSearchCreateOrUpdateAck(
        crate::handlers::search::SearchCompletionBehavior,
        Result<i64, String>,
    ),
    /// `pinned_search.update` IPC ack. Phase 6a-part-2: dedicated
    /// variant covering the id-keyed update path (refresh / re-pin).
    PinnedSearchUpdateAck(
        crate::handlers::search::SearchCompletionBehavior,
        Result<i64, String>,
    ),
    /// `pinned_search.delete_all` IPC ack. Phase 6a-part-2: dedicated
    /// variant replacing the prior funnel into `Message::Noop` so the
    /// closing-6a CI lockdown script does not flag it as a missing
    /// rollback path. UI clears local state pre-IPC; on Err we log and
    /// rely on next reload to reconcile (staleness-tolerant per the
    /// signature-reorder precedent).
    PinnedSearchDeleteAllAck(Result<u64, String>),
    RefreshPinnedSearch(i64),

    // Search extras
    SearchHere(String),
    SaveAsSmartFolder(String),
    /// `smart_folder.create` IPC ack. Phase 6a-part-2: dedicated
    /// per-method ack variant (renamed from `SmartFolderSaved`).
    /// Carries the minted UUID String returned by the Service-side
    /// handler instead of the previous always-zero `i64`.
    SmartFolderCreateAck(Result<String, String>),

    // Calendar
    Calendar(Box<CalendarMessage>),
    ToggleAppMode,
    SetAppMode(AppMode),
    SetCalendarView(CalendarView),
    CalendarToday,
    /// Calendar sync completed - refresh in-memory calendar state.
    CalendarSyncComplete,

    // Account management
    AddAccount(AddAccountMessage),
    OpenAddAccount,
    AccountDeleted(Result<(), String>),
    AccountUpdated(Result<(), String>),
    ReloadSignatures,

    // Pop-out windows
    PopOut(iced::window::Id, PopOutMessage),
    OpenMessageView(usize),
    ComposeDraftTick,
    /// A local draft was loaded from DB - open it in a compose window.
    LocalDraftLoaded(Result<Option<rtsk::db::types::DbLocalDraft>, String>),
    /// Session-restore draft load completed for an already-open compose
    /// pop-out. The window opened at boot with default geometry; this
    /// fills the `ComposeState` in place (or closes the window if the
    /// draft was deleted between sessions).
    RestoredComposeLoaded {
        window_id: iced::window::Id,
        width: f32,
        height: f32,
        x: Option<f32>,
        y: Option<f32>,
        result: Result<Option<rtsk::db::types::DbLocalDraft>, String>,
    },

    // Thread detail via core
    ThreadDetailLoaded(
        GenerationToken<ThreadDetail>,
        Result<db::AppThreadDetail, String>,
    ),
    // Chat
    ChatTimeline(crate::ui::chat_timeline::ChatTimelineMessage),
    ChatTimelineLoaded(
        GenerationToken<rtsk::generation::Chat>,
        Result<Vec<rtsk::chat::ChatMessage>, String>,
    ),
    ChatOlderLoaded(String, Result<Vec<rtsk::chat::ChatMessage>, String>),
    ChatReadMarked,
    ChatContactsLoaded(
        GenerationToken<rtsk::generation::ChatList>,
        Result<Vec<rtsk::chat::ChatContactSummary>, String>,
    ),

    // Pinned search management
    ClearAllPinnedSearches,

    // Sync progress pipeline
    SyncProgress(SyncEvent),

    // Signature operations
    SignatureOp(handlers::SignatureResult),

    // Cross-account label operations (Mail Rules > Labels)
    LabelOp(handlers::LabelOp),

    // Undo
    Undo,
    /// Undo compensation completed.
    UndoCompleted {
        desc: String,
        outcomes: Vec<service_api::actions::ActionOutcome>,
    },

    // Shared mailboxes & public folders
    SharedMailboxesLoaded(Result<Vec<db::SharedMailbox>, String>),
    PinnedPublicFoldersLoaded(Result<Vec<db::PinnedPublicFolder>, String>),

    // Snooze resurface - periodic check for due snoozed threads
    SnoozeTick,
    SnoozeResurfaceComplete(Result<usize, String>),

    /// Phase 7-6: hourly fan-out for `extract.backfill_kick`. The
    /// Service-side handler scans `attachments` JOINed against
    /// `attachment_blobs` for live (non-tombstoned) bytes with
    /// `text_indexed_at IS NULL` and enqueues each into the
    /// `ExtractRuntime`. Drop class - missed ticks self-heal on the
    /// next hour. Also fired once on `ServiceBootReady` to catch up
    /// after a Service crash mid-extraction.
    ExtractBackfillTick,

    /// Phase 7-9d: dispatched from the palette
    /// `app.rebuildSearchIndex` command. Sends a Wipe rebuild
    /// request to the Service. Progress flows back via
    /// `IndexRebuildProgress` notifications routed through the
    /// service-notification subscription.
    RebuildSearchIndex,
    /// Phase 7-9d: rebuild started; carry the rebuild_id so the
    /// status bar / progress UI can correlate against incoming
    /// `IndexRebuildProgress` notifications.
    RebuildSearchIndexDispatched(Result<String, String>),

    // Phase 5 task 10: GalRefreshTick / GalCacheRefreshed deleted. GAL
    // refresh now rides on `Message::SyncTick -> kick_gal_refresh`
    // (fire-and-forget IPC notification); the Service handler iterates
    // accounts and the 24 h cache gate inside fetch_gal_for_account_if_stale
    // self-throttles.

    // Phase 3 task 17: debounced reader reload after `index.committed`
    // notifications. The notification handler stamps
    // `App.pending_reader_reload`; a 200 ms tick subscription emits
    // `ReaderReloadTick`; the handler calls `SearchReadState::reload()`
    // when the stamp has aged past one tick.
    ReaderReloadTick,

    /// Phase 5 task 11: debounced calendar reload after a
    /// `Notification::CalendarChanged` arrival. The CalendarChanged
    /// arm stamps `App.pending_calendar_reload`; a 250 ms tick
    /// subscription emits this; the handler calls
    /// `reload_calendar_events()` once the stamp has aged past one
    /// tick. Collapses an N-account kick batch into a single reload.
    CalendarReloadTick,

    // Keyboard modifier tracking (for Ctrl+click, Shift+click)
    ModifiersChanged(iced::keyboard::Modifiers),

    // Auto-reply status check result
    AutoReplyChecked(bool),

    /// Phase 6a-part-2 (encryption-key handle): cold-boot bootstrap
    /// snapshots arrived from `internal.read_bootstrap_snapshots`.
    /// Handler applies them via `Settings::apply_bootstrap`. The IPC
    /// fires once during `from_boot_ready`; failure logs and the UI
    /// continues with default preferences (today's silent-fallback
    /// behaviour).
    BootstrapSnapshotsLoaded(
        Result<service_api::ReadBootstrapSnapshotsAck, String>,
    ),
}
