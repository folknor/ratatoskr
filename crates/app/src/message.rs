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
//   WindowResized(id, size)                        - handle (apply to single
//                                                    main window if id matches)
//   WindowMoved(id, point)                         - handle (same)
//   WindowCloseRequested(id)                       - handle (iced::exit if main)
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
//   SaveAsSmartFolder | SmartFolderSaved | ShowHelp
//
// Pinned-search and snooze messages: dropped while Booting (the periodic
// timers will fire again after Booting -> Ready):
//   PinnedSearchesLoaded | SelectPinnedSearch | DismissPinnedSearch
//   PinnedSearchDismissed | PinnedSearchPersisted | PinnedSearchesExpired
//   RefreshPinnedSearch | ExpiryTick | ClearAllPinnedSearches
//   SnoozeTick | SnoozeResurfaceComplete
//
// Sync / account / push: dropped while Booting (Service hasn't reached
// Ready, so any sync attempt would panic on the missing DB):
//   SyncTick | SyncCurrentFolder | SyncComplete | SyncProgress
//   AddAccount | OpenAddAccount | AccountDeleted | AccountUpdated
//   ReloadSignatures | SignatureOp | SharedMailboxesLoaded
//   PinnedPublicFoldersLoaded | GalRefreshTick | GalCacheRefreshed
//   AutoReplyChecked
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
        outcomes: Vec<rtsk::actions::ActionOutcome>,
    },
    /// Send completed - carries compose window ID and outcome.
    /// Separate from ActionCompleted because send operates on a compose window,
    /// not a thread list selection.
    SendCompleted {
        window_id: iced::window::Id,
        outcome: rtsk::actions::ActionOutcome,
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
    PinnedSearchDismissed(i64, Result<(), String>),
    PinnedSearchPersisted(
        crate::handlers::search::SearchCompletionBehavior,
        Result<i64, String>,
    ),
    PinnedSearchesExpired(Result<u64, String>),
    RefreshPinnedSearch(i64),
    ExpiryTick,

    // Search extras
    SearchHere(String),
    SaveAsSmartFolder(String),
    SmartFolderSaved(Result<i64, String>),

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

    // Undo
    Undo,
    /// Undo compensation completed.
    UndoCompleted {
        desc: String,
        outcomes: Vec<rtsk::actions::ActionOutcome>,
    },

    // Shared mailboxes & public folders
    SharedMailboxesLoaded(Result<Vec<db::SharedMailbox>, String>),
    PinnedPublicFoldersLoaded(Result<Vec<db::PinnedPublicFolder>, String>),

    // Snooze resurface - periodic check for due snoozed threads
    SnoozeTick,
    SnoozeResurfaceComplete(Result<usize, String>),

    // GAL (organization directory) cache
    GalRefreshTick,
    GalCacheRefreshed(Result<usize, String>),

    // Keyboard modifier tracking (for Ctrl+click, Shift+click)
    ModifiersChanged(iced::keyboard::Modifiers),

    // Auto-reply status check result
    AutoReplyChecked(bool),
}
