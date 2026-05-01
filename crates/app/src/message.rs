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
