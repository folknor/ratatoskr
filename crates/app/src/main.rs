// ── ARCHITECTURE NOTE ───────────────────────────────────
//
// This file is a THIN DISPATCH LAYER. It contains:
//   - The `Message` enum
//   - The `App` struct definition
//   - `boot()`, `title()`, `theme()`, `subscription()`
//   - `update()` — which dispatches to handler methods
//   - `view()` / `view_main_window()` — layout assembly
//   - Component delegation (handle_sidebar, handle_thread_list, etc.)
//   - Navigation/thread loading helpers
//
// ALL FEATURE LOGIC lives in `handlers/` modules. Each handler file
// adds `impl App` methods that `update()` dispatches to. When adding
// new functionality:
//
//   1. Add the `Message` variant here
//   2. Add a ONE-LINE dispatch arm: `Message::Foo(x) => self.handle_foo(x)`
//   3. Implement `handle_foo()` in the appropriate `handlers/*.rs` file
//
// Do NOT put multi-line handler logic, free functions, or match arms
// with business logic in this file. See `handlers/mod.rs` for the
// module index and `UI.md` for the full module map.
// ────────────────────────────────────────────────────────

mod appearance;
mod command_dispatch;
mod command_resolver;
mod component;
mod db;
mod display;
mod font;
mod handlers;
mod icon;
mod pop_out;
mod ui;
mod window_state;

use command_dispatch::{
    ComposeAction, EmailAction, KeyEventMessage, NavigationTarget,
    ReadingPanePosition, TaskAction,
};
use component::Component;
use db::{Db, Thread};
use iced::widget::{column, container, mouse_area, row, stack, Space};
use iced::{Element, Length, Point, Size, Task, Theme};
use pop_out::{PopOutMessage, PopOutWindow};
use pop_out::compose::ComposeMode;
use ui::palette::{Palette, PaletteMessage};
use ratatoskr_command_palette::{
    BindingTable, Chord, CommandId, CommandRegistry,
    FocusedRegion, UndoStack, current_platform,
};
use ratatoskr_core::db::queries_extra::navigation::{
    NavigationState, get_navigation_state,
};
use ratatoskr_core::db::queries_extra::get_threads_scoped;
use ratatoskr_core::db::types::{AccountScope, DbThread};
use ui::layout::{
    RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH, SIDEBAR_MIN_WIDTH, THREAD_LIST_MIN_WIDTH,
};
use ui::add_account::{AddAccountMessage, AddAccountWizard};
use ui::undoable::UndoableText;
use ui::calendar::{
    CalendarMessage, CalendarOverlay, CalendarState, CalendarView,
};
use ui::reading_pane::{ReadingPane, ReadingPaneEvent, ReadingPaneMessage};
use ui::settings::{Settings, SettingsEvent, SettingsMessage};
use ui::sidebar::{Sidebar, SidebarEvent, SidebarMessage};
use ui::status_bar::{
    AccountWarning, StatusBar, StatusBarEvent, StatusBarMessage, SyncEvent,
    SyncProgressReceiver, WarningKind, create_sync_progress_channel, shared_receiver,
    sync_progress_subscription,
};
use ui::thread_list::{ThreadList, ThreadListEvent, ThreadListMessage};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

static DB: std::sync::OnceLock<Arc<Db>> = std::sync::OnceLock::new();
static APP_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static DEFAULT_SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();

/// How long to wait for the second chord of a sequence.
const CHORD_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);

fn main() -> iced::Result {
    env_logger::init();
    log::info!("Ratatoskr starting");

    let app_data_dir = dirs::data_dir()
        .expect("no data dir")
        .join("com.velo.app");

    let db = Db::open(&app_data_dir)
        .map_err(|e| iced::Error::WindowCreationFailed(e.into()))?;
    let _ = DB.set(Arc::new(db));

    let detected_scale = display::detect_default_scale();
    let _ = DEFAULT_SCALE.set(detected_scale);

    let system_font_family = {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .ok();
        rt.and_then(|rt| {
            let fonts = rt.block_on(ratatoskr_system_fonts::SystemFonts::detect());
            fonts.ui.map(|f| f.family)
        })
    };
    font::set_system_ui_font(system_font_family);

    let _ = APP_DATA_DIR.set(app_data_dir);

    let mut app = iced::daemon(App::boot, App::update, App::view)
        .title(App::title)
        .theme(App::daemon_theme)
        .scale_factor(|app, _window| app.settings.scale)
        .subscription(App::subscription)
        .default_font(font::text());

    for f in font::load() {
        app = app.font(f);
    }

    app.run()
}

/// Whether the app is showing mail or calendar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppMode {
    Mail,
    Calendar,
}

/// Which vertical divider is being dragged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Divider {
    Sidebar,
    ThreadList,
}

/// Drag handle width in logical pixels.
const DIVIDER_WIDTH: f32 = 2.0;

/// Pending chord state for two-key sequence bindings.
struct PendingChord {
    first: Chord,
}

#[derive(Debug, Clone)]
pub enum Message {
    // Existing component messages
    Sidebar(SidebarMessage),
    ThreadList(ThreadListMessage),
    ReadingPane(ReadingPaneMessage),
    Settings(SettingsMessage),
    StatusBar(StatusBarMessage),

    // Existing data loading
    AccountsLoaded(u64, Result<Vec<db::Account>, String>),
    NavigationLoaded(u64, Result<NavigationState, String>),
    ThreadsLoaded(u64, Result<Vec<Thread>, String>),

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
    SetDateDisplay(db::DateDisplay),
    WindowCloseRequested(iced::window::Id),

    // Command system (Slice 6a)
    KeyEvent(KeyEventMessage),
    ExecuteCommand(CommandId),
    ExecuteParameterized(CommandId, ratatoskr_command_palette::CommandArgs),
    NavigateTo(NavigationTarget),
    Escape,
    EmailAction(EmailAction),
    /// Archive completed via action service — carries per-thread outcomes.
    ArchiveCompleted(Vec<ratatoskr_core::actions::ActionOutcome>),
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
    SearchResultsLoaded(u64, Result<Vec<Thread>, String>),
    SearchClear,
    FocusSearchBar,
    SearchBlur,
    SearchHistoryLoaded(Result<Vec<String>, String>),

    // Pinned searches
    PinnedSearchesLoaded(Result<Vec<db::PinnedSearch>, String>),
    SelectPinnedSearch(i64),
    PinnedSearchThreadIdsLoaded(u64, i64, Result<Vec<(String, String)>, String>),
    PinnedSearchThreadsLoaded(u64, Result<Vec<Thread>, String>),
    DismissPinnedSearch(i64),
    PinnedSearchDismissed(i64, Result<(), String>),
    PinnedSearchSaved(Result<i64, String>),
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

    // Thread detail via core
    ThreadDetailLoaded(u64, Result<db::AppThreadDetail, String>),

    // Pinned search management
    ClearAllPinnedSearches,

    // Sync progress pipeline
    SyncProgress(SyncEvent),

    // Signature operations
    SignatureOp(handlers::SignatureResult),

    // Undo
    Undo,

    // Shared mailboxes & public folders
    SharedMailboxesLoaded(Result<Vec<db::SharedMailbox>, String>),
    PinnedPublicFoldersLoaded(Result<Vec<db::PinnedPublicFolder>, String>),

    // GAL (organization directory) cache
    GalRefreshTick,
    GalCacheRefreshed(Result<usize, String>),

    // Keyboard modifier tracking (for Ctrl+click, Shift+click)
    ModifiersChanged(iced::keyboard::Modifiers),
}

struct App {
    db: Arc<Db>,
    sidebar: Sidebar,
    thread_list: ThreadList,
    reading_pane: ReadingPane,
    settings: Settings,
    status_bar: StatusBar,
    status: String,
    mode: appearance::Mode,
    app_mode: AppMode,
    calendar: CalendarState,
    sidebar_width: f32,
    thread_list_width: f32,
    dragging: Option<Divider>,
    hovered_divider: Option<Divider>,
    right_sidebar_open: bool,
    show_settings: bool,
    window: window_state::WindowState,

    main_window_id: iced::window::Id,
    pop_out_windows: HashMap<iced::window::Id, PopOutWindow>,
    pop_out_generation: u64,
    nav_generation: u64,
    thread_generation: u64,

    // Command palette infrastructure
    registry: CommandRegistry,
    binding_table: BindingTable,
    focused_region: Option<FocusedRegion>,
    is_online: bool,
    composer_is_open: bool,
    pending_chord: Option<PendingChord>,
    palette: Palette,
    undo_stack: UndoStack,

    // Search state
    search_state: Option<Arc<ratatoskr_core::search::SearchState>>,
    search_generation: u64,
    search_query: UndoableText,
    search_debounce_deadline: Option<iced::time::Instant>,
    /// Whether the user was in a folder view before entering search.
    /// When search is cleared, threads are reloaded from the current
    /// navigation state instead of restoring a stale clone.
    was_in_folder_view: bool,

    // Search history (recent queries from pinned_searches)
    search_history: Vec<String>,

    // Pinned searches
    pinned_searches: Vec<db::PinnedSearch>,
    active_pinned_search: Option<i64>,
    editing_pinned_search: Option<i64>,
    expiry_ran: bool,

    no_accounts: bool,
    add_account_wizard: Option<AddAccountWizard>,

    /// Currently held keyboard modifiers (for Ctrl+click, Shift+click).
    current_modifiers: iced::keyboard::Modifiers,

    /// The current navigation target, set by `Message::NavigateTo`.
    navigation_target: Option<NavigationTarget>,

    // Sync progress pipeline
    sync_receiver: SyncProgressReceiver,
    #[allow(dead_code)]
    sync_reporter: Arc<ui::status_bar::IcedProgressReporter>,

    /// Body store for loading decompressed message bodies via core.
    body_store: Option<ratatoskr_core::body_store::BodyStoreState>,
    /// Inline image store for CID image resolution.
    inline_image_store: Option<ratatoskr_stores::inline_image_store::InlineImageStoreState>,
    /// Encryption key for decrypting provider credentials (OAuth tokens, passwords).
    encryption_key: Option<[u8; 32]>,
    /// Action service context — the authoritative write path for email mutations.
    /// `None` if stores failed to initialize at boot (degraded mode).
    action_ctx: Option<ratatoskr_core::actions::ActionContext>,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let db = Arc::clone(DB.get().expect("DB not initialized"));
        let db_ref = Arc::clone(&db);
        let db_ref2 = Arc::clone(&db);
        let db_ref3 = Arc::clone(&db);
        let db_ref4 = Arc::clone(&db);
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let window = window_state::WindowState::load(data_dir);

        let (main_window_id, open_task) =
            iced::window::open(window.to_window_settings());

        let mut registry = CommandRegistry::new();
        let mut binding_table = BindingTable::new(&registry, current_platform());
        let keybindings_path = data_dir.join("keybindings.json");
        if let Err(e) = binding_table.load_overrides_from_file(&keybindings_path) {
            eprintln!("warning: failed to load keybinding overrides: {e}");
        }
        // Load persisted usage counts for command ranking
        let usage_path = data_dir.join("command_usage.json");
        if let Ok(json) = std::fs::read_to_string(&usage_path) {
            if let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, u32>>(&json) {
                registry.usage.load_from_map(&map);
            }
        }
        let resolver = Arc::new(command_resolver::AppInputResolver::new(Arc::clone(&db)));

        let (rx, reporter) = create_sync_progress_channel();
        let sync_receiver = shared_receiver(rx);
        let sync_reporter = Arc::new(reporter);

        let body_store = match db::threads::init_body_store() {
            Ok(bs) => Some(bs),
            Err(e) => {
                log::error!("Failed to init body store: {e}");
                None
            }
        };

        let inline_image_store = match ratatoskr_stores::inline_image_store::InlineImageStoreState::init(data_dir) {
            Ok(store) => Some(store),
            Err(e) => {
                log::error!("Failed to init inline image store: {e}");
                None
            }
        };

        let encryption_key = match ratatoskr_provider_utils::crypto::load_encryption_key(data_dir) {
            Ok(key) => Some(key),
            Err(e) => {
                log::error!("Failed to load encryption key: {e}");
                None
            }
        };

        let action_ctx = match (&body_store, &inline_image_store, encryption_key) {
            (Some(bs), Some(iis), Some(key)) => {
                // SearchState is optional for the action service — actions don't
                // use search. If it's unavailable, init a fresh one for ProviderCtx.
                let search = ratatoskr_core::search::SearchState::init(data_dir).ok();
                if let Some(ss) = search {
                    Some(ratatoskr_core::actions::ActionContext {
                        db: ratatoskr_core::db::DbState::from_arc(db.write_conn_arc()),
                        body_store: bs.clone(),
                        inline_images: iis.clone(),
                        search: ss,
                        encryption_key: key,
                    })
                } else {
                    log::error!("Action service unavailable: search state failed to initialize");
                    None
                }
            }
            _ => {
                log::error!("Action service unavailable: one or more stores failed to initialize");
                None
            }
        };

        let session = pop_out::session::SessionState::load(data_dir);

        let calendar_default_view = db.with_conn_sync(|conn| {
            let view_name = conn.query_row(
                "SELECT value FROM settings WHERE key = 'calendar_default_view'",
                [],
                |row| row.get::<_, String>(0),
            ).unwrap_or_else(|_| "month".to_string());
            Ok(CalendarState::parse_view_name(&view_name))
        }).unwrap_or(CalendarView::Month);

        let mut app = Self {
            db,
            sidebar: Sidebar::new(),
            thread_list: ThreadList::new(),
            reading_pane: ReadingPane::new(),
            settings: Settings::with_scale(
                *DEFAULT_SCALE.get().unwrap_or(&1.0),
            ),
            status_bar: StatusBar::new(),
            status: "Loading...".to_string(),
            mode: appearance::Mode::Dark,
            app_mode: AppMode::Mail,
            calendar: CalendarState::with_default_view(calendar_default_view),
            sidebar_width: window.sidebar_width,
            thread_list_width: window.thread_list_width,
            dragging: None,
            hovered_divider: None,
            right_sidebar_open: window.right_sidebar_open,
            show_settings: false,
            window,
            main_window_id,
            pop_out_windows: HashMap::new(),
            pop_out_generation: 0,
            nav_generation: 1,
            thread_generation: 0,
            registry,
            binding_table,
            focused_region: None,
            is_online: true,
            composer_is_open: false,
            pending_chord: None,
            palette: Palette::new(
                CommandRegistry::new(),
                resolver,
            ),
            undo_stack: UndoStack::default(),
            search_state: ratatoskr_core::search::SearchState::init(data_dir)
                .map(Arc::new)
                .ok(),
            search_generation: 0,
            search_query: UndoableText::new(),
            search_debounce_deadline: None,
            was_in_folder_view: false,
            search_history: Vec::new(),
            pinned_searches: Vec::new(),
            active_pinned_search: None,
            editing_pinned_search: None,
            expiry_ran: false,
            no_accounts: false,
            add_account_wizard: None,
            current_modifiers: iced::keyboard::Modifiers::empty(),
            navigation_target: None,
            sync_receiver,
            sync_reporter,
            body_store,
            inline_image_store,
            encryption_key,
            action_ctx,
        };

        // Restore pop-out windows from previous session
        let mut session_tasks = app.restore_pop_out_windows(&session);

        let load_gen = app.nav_generation;
        let mut boot_tasks = vec![
            open_task.discard(),
            Task::perform(
                async move { (load_gen, load_accounts(db_ref).await) },
                |(g, result)| Message::AccountsLoaded(g, result),
            ),
            Task::perform(
                async move { db_ref2.list_pinned_searches().await },
                Message::PinnedSearchesLoaded,
            ),
            // Load shared mailboxes for sidebar
            Task::perform(
                async move { db_ref3.get_shared_mailboxes().await },
                Message::SharedMailboxesLoaded,
            ),
            // Load pinned public folders for sidebar
            Task::perform(
                async move { db_ref4.get_pinned_public_folders().await },
                Message::PinnedPublicFoldersLoaded,
            ),
            // Initial GAL cache population (deferred — provider clients
            // aren't available at boot; the first GalRefreshTick will
            // attempt the actual fetch once accounts are loaded)
            Task::done(Message::GalRefreshTick),
        ];
        boot_tasks.append(&mut session_tasks);
        (app, Task::batch(boot_tasks))
    }

    fn title(&self, window_id: iced::window::Id) -> String {
        if window_id == self.main_window_id {
            return "Ratatoskr".to_string();
        }
        match self.pop_out_windows.get(&window_id) {
            Some(PopOutWindow::MessageView(state)) => {
                let subject = state.subject.as_deref().unwrap_or("(no subject)");
                let sender = state.from_address.as_deref().unwrap_or("unknown");
                format!("{subject} \u{2014} {sender}")
            }
            Some(PopOutWindow::Compose(state)) => state.window_title(),
            Some(PopOutWindow::Calendar) => "Ratatoskr \u{2014} Calendar".to_string(),
            None => "Ratatoskr".to_string(),
        }
    }

    fn daemon_theme(&self, _window_id: iced::window::Id) -> Theme {
        self.theme()
    }

    fn theme(&self) -> Theme {
        match self.settings.theme.as_str() {
            "Light" => Theme::custom(String::from("Light"), iced::theme::palette::Seed::LIGHT),
            "Dark" => Theme::custom(String::from("Dark"), iced::theme::palette::Seed::DARK),
            "Theme" => {
                let idx = self.settings.selected_theme.unwrap_or(0);
                ui::theme::theme_by_index(idx)
            }
            _ => match self.mode {
                appearance::Mode::Light => Theme::custom(String::from("Light"), iced::theme::palette::Seed::LIGHT),
                _ => Theme::custom(String::from("Dark"), iced::theme::palette::Seed::DARK),
            },
        }
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        let mut subs = vec![
            appearance::subscription().map(Message::AppearanceChanged),
            iced::window::resize_events().map(|(id, size)| {
                Message::WindowResized(id, size)
            }),
            iced::window::close_requests().map(Message::WindowCloseRequested),
            iced::event::listen_with(|event, _status, id| {
                if let iced::Event::Window(iced::window::Event::Moved(point)) = event {
                    Some(Message::WindowMoved(id, point))
                } else {
                    None
                }
            }),
            iced::event::listen_with(|event, status, id| {
                match &event {
                    iced::Event::Keyboard(
                        iced::keyboard::Event::KeyPressed { key, modifiers, .. }
                    ) => {
                        Some(Message::KeyEvent(KeyEventMessage::KeyPressed {
                            key: key.clone(),
                            modifiers: *modifiers,
                            status,
                            window_id: id,
                        }))
                    }
                    iced::Event::Keyboard(
                        iced::keyboard::Event::ModifiersChanged(modifiers)
                    ) => {
                        Some(Message::ModifiersChanged(*modifiers))
                    }
                    _ => None,
                }
            }),
            self.sidebar.subscription().map(Message::Sidebar),
            self.thread_list.subscription().map(Message::ThreadList),
            self.reading_pane.subscription().map(Message::ReadingPane),
            self.settings.subscription().map(Message::Settings),
            self.status_bar.subscription().map(Message::StatusBar),
            sync_progress_subscription(&self.sync_receiver)
                .map(Message::SyncProgress),
        ];

        if self.pending_chord.is_some() {
            subs.push(
                iced::time::every(CHORD_TIMEOUT)
                    .map(|_| Message::KeyEvent(KeyEventMessage::PendingChordTimeout)),
            );
        }

        if let Some(deadline) = self.search_debounce_deadline {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(50))
                    .with(deadline)
                    .map(|(_, deadline)| {
                        if iced::time::Instant::now() >= deadline {
                            Message::SearchExecute
                        } else {
                            Message::Noop
                        }
                    }),
            );
        }

        if self.composer_is_open && self.has_dirty_compose_drafts() {
            subs.push(
                iced::time::every(handlers::pop_out::DRAFT_AUTO_SAVE_INTERVAL)
                    .map(|_| Message::ComposeDraftTick),
            );
        }

        // Periodic pinned search expiry — check every hour
        if !self.pinned_searches.is_empty() {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(3600))
                    .map(|_| Message::ExpiryTick),
            );
        }

        // Periodic sync — delta sync all accounts every 5 minutes
        if !self.sidebar.accounts.is_empty() && self.encryption_key.is_some() {
            subs.push(
                iced::time::every(std::time::Duration::from_secs(300))
                    .map(|_| Message::SyncTick),
            );
        }

        // GAL (organization directory) cache refresh — every hour
        subs.push(
            iced::time::every(std::time::Duration::from_secs(3600))
                .map(|_| Message::GalRefreshTick),
        );

        if self.settings.overlay_anim.is_animating(iced::time::Instant::now()) {
            subs.push(
                iced::window::frames()
                    .map(|at| Message::Settings(SettingsMessage::OverlayAnimTick(at))),
            );
        }

        iced::Subscription::batch(subs)
    }

    /// Central message dispatch. Each arm should be a ONE-LINE delegation
    /// to a handler method in `handlers/*.rs`. Do not inline logic here.
    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            // Component delegation
            Message::Sidebar(msg) => self.handle_sidebar(msg),
            Message::ThreadList(msg) => self.handle_thread_list(msg),
            Message::ReadingPane(msg) => self.handle_reading_pane(msg),
            Message::Settings(msg) => self.handle_settings(msg),
            Message::StatusBar(msg) => self.handle_status_bar(msg),

            // Appearance
            Message::AppearanceChanged(mode) => {
                self.mode = mode;
                Task::none()
            }

            // Data loading with generation guards
            Message::AccountsLoaded(g, _) if g != self.nav_generation => Task::none(),
            Message::AccountsLoaded(_, Ok(accounts)) => {
                log::info!("Loaded {} accounts", accounts.len());
                self.handle_accounts_loaded(accounts)
            }
            Message::AccountsLoaded(_, Err(e)) => {
                log::error!("AccountsLoaded error: {e}");
                self.status = format!("Error: {e}");
                Task::none()
            }
            Message::NavigationLoaded(g, _) if g != self.nav_generation => Task::none(),
            Message::NavigationLoaded(_, Ok(nav_state)) => {
                self.sidebar.nav_state = Some(nav_state);
                Task::none()
            }
            Message::NavigationLoaded(_, Err(e)) => {
                log::error!("Navigation load error: {e}");
                self.status = format!("Navigation error: {e}");
                Task::none()
            }
            Message::ThreadsLoaded(g, _) if g != self.nav_generation => Task::none(),
            Message::ThreadsLoaded(_, Ok(threads)) => {
                log::info!("Loaded {} threads", threads.len());
                self.status = format!("{} threads", threads.len());
                self.thread_list.set_threads(threads);
                Task::none()
            }
            Message::ThreadsLoaded(_, Err(e)) => {
                log::error!("ThreadsLoaded error: {e}");
                self.status = format!("Threads error: {e}");
                Task::none()
            }

            // Divider drag
            Message::DividerDragStart(divider) => {
                self.dragging = Some(divider);
                Task::none()
            }
            Message::DividerDragMove(point) => self.handle_divider_drag(point),
            Message::DividerDragEnd => {
                self.dragging = None;
                Task::none()
            }
            Message::DividerHover(divider) => {
                self.hovered_divider = Some(divider);
                Task::none()
            }
            Message::DividerUnhover => {
                self.hovered_divider = None;
                Task::none()
            }

            // Settings and UI toggles
            Message::ToggleSettings => {
                self.show_settings = !self.show_settings;
                if self.show_settings {
                    self.settings.overlay = None;
                    self.settings.overlay_anim.go_mut(false, iced::time::Instant::now());
                    self.settings.active_tab = crate::ui::settings::Tab::General;
                    self.settings.begin_editing();
                }
                Task::none()
            }
            Message::ToggleRightSidebar => {
                self.right_sidebar_open = !self.right_sidebar_open;
                Task::none()
            }
            Message::SetDateDisplay(display) => {
                self.reading_pane.date_display = display;
                Task::none()
            }

            // Window management
            Message::WindowResized(id, size) => {
                if id == self.main_window_id {
                    self.window.set_size(size);
                    if size.width < RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH
                        && self.right_sidebar_open
                    {
                        self.right_sidebar_open = false;
                    }
                } else {
                    match self.pop_out_windows.get_mut(&id) {
                        Some(PopOutWindow::MessageView(state)) => {
                            state.width = size.width;
                            state.height = size.height;
                        }
                        Some(PopOutWindow::Compose(state)) => {
                            state.width = size.width;
                            state.height = size.height;
                        }
                        Some(PopOutWindow::Calendar) => {}
                        None => {}
                    }
                }
                Task::none()
            }
            Message::WindowMoved(id, point) => {
                if id == self.main_window_id {
                    self.window.set_position(point);
                } else if let Some(PopOutWindow::MessageView(state)) =
                    self.pop_out_windows.get_mut(&id)
                {
                    state.x = Some(point.x);
                    state.y = Some(point.y);
                }
                Task::none()
            }
            Message::WindowCloseRequested(id) => self.handle_window_close(id),

            // Compose
            Message::Compose => self.open_compose_window(ComposeMode::New),
            Message::Noop => Task::none(),

            // Command system
            Message::KeyEvent(msg) => self.handle_key_event(msg),
            Message::ExecuteCommand(id) => self.handle_execute_command(id),
            Message::ExecuteParameterized(id, args) => {
                self.handle_execute_parameterized(id, args)
            }
            Message::NavigateTo(target) => self.handle_navigate_to(target),
            Message::Escape => {
                if !matches!(self.calendar.overlay, CalendarOverlay::None) {
                    self.calendar.overlay = CalendarOverlay::None;
                    return Task::none();
                }
                if self.show_settings {
                    self.settings.commit_preferences();
                    self.show_settings = false;
                    return Task::none();
                }
                if !self.search_query.text().is_empty() || self.active_pinned_search.is_some() {
                    self.active_pinned_search = None;
                    self.sidebar.active_pinned_search = None;
                    self.editing_pinned_search = None;
                    return self.update(Message::SearchClear);
                }
                Task::none()
            }
            Message::EmailAction(action) => self.handle_email_action(action),
            Message::ArchiveCompleted(ref outcomes) => self.handle_archive_completed(outcomes),
            Message::ComposeAction(ref action) => self.handle_compose_action(action),
            Message::TaskAction(_action) => Task::none(),
            Message::SetTheme(theme) => {
                self.settings.theme = theme;
                Task::none()
            }
            Message::ToggleSidebar => Task::none(),
            Message::FocusSearch => self.update(Message::FocusSearchBar),
            Message::ShowHelp => Task::none(),
            Message::SyncCurrentFolder => self.sync_all_accounts(),
            Message::SyncTick => self.sync_all_accounts(),
            Message::SyncComplete(account_id, result) => {
                match result {
                    Err(ref e) => {
                        log::error!("Sync failed for {account_id}: {e}");
                        let lower = e.to_lowercase();
                        let is_auth_error = lower.contains("401")
                            || lower.contains("unauthorized")
                            || lower.contains("token")
                            || lower.contains("auth")
                            || lower.contains("expired")
                            || lower.contains("invalid_grant")
                            || lower.contains("refresh");
                        let email = self.email_for_account(&account_id);
                        if is_auth_error {
                            self.status_bar.set_warning(
                                ui::status_bar::AccountWarning {
                                    account_id: account_id.clone(),
                                    email,
                                    kind: ui::status_bar::WarningKind::TokenExpiry,
                                },
                            );
                        } else {
                            self.status_bar.set_warning(
                                ui::status_bar::AccountWarning {
                                    account_id: account_id.clone(),
                                    email,
                                    kind: ui::status_bar::WarningKind::ConnectionFailure {
                                        message: e.clone(),
                                    },
                                },
                            );
                        }
                    }
                    Ok(()) => {
                        // Sync succeeded — clear any previous warning for this account
                        self.status_bar.clear_warning(&account_id);
                    }
                }
                // Reload navigation + threads to reflect any changes from sync
                self.nav_generation += 1;
                self.load_navigation_and_threads()
            }
            Message::SetReadingPanePosition(_pos) => Task::none(),
            Message::Palette(msg) => self.handle_palette(msg),

            // Search — delegated to handlers/search.rs
            Message::SearchQueryChanged(query) => self.handle_search_query_changed(query),
            Message::SearchExecute => self.handle_search_execute(),
            Message::SearchResultsLoaded(g, _) if g != self.search_generation => Task::none(),
            Message::SearchResultsLoaded(_, result) => self.handle_search_results(result),
            Message::SearchClear => self.handle_search_clear(),
            Message::FocusSearchBar => self.handle_focus_search_bar(),
            Message::SearchBlur => {
                self.thread_list.typeahead.visible = false;
                // Focus a non-existent widget to remove focus from the search bar.
                // iced ignores focus operations on unknown IDs, but the act of
                // issuing any focus operation clears the current focus.
                iced::widget::operation::focus::<Message>("blur-sink".to_string())
            }

            Message::SearchHistoryLoaded(Ok(queries)) => {
                self.search_history = queries;
                Task::none()
            }
            Message::SearchHistoryLoaded(Err(_)) => Task::none(),

            // Pinned searches — delegated to handlers/search.rs
            Message::PinnedSearchesLoaded(result) => self.handle_pinned_searches_loaded(result),
            Message::SelectPinnedSearch(id) => self.handle_select_pinned_search(id),
            Message::PinnedSearchThreadIdsLoaded(g, _, _) if g != self.nav_generation => {
                Task::none()
            }
            Message::PinnedSearchThreadIdsLoaded(_, ps_id, result) => {
                self.handle_pinned_search_thread_ids_loaded(ps_id, result)
            }
            Message::PinnedSearchThreadsLoaded(g, _) if g != self.nav_generation => {
                Task::none()
            }
            Message::PinnedSearchThreadsLoaded(_, result) => {
                self.handle_pinned_search_threads_loaded(result)
            }
            Message::DismissPinnedSearch(id) => self.handle_dismiss_pinned_search(id),
            Message::PinnedSearchDismissed(id, result) => {
                self.handle_pinned_search_dismissed(id, result)
            }
            Message::PinnedSearchSaved(result) => self.handle_pinned_search_saved(result),
            Message::PinnedSearchesExpired(result) => {
                self.handle_pinned_searches_expired(result)
            }
            Message::RefreshPinnedSearch(id) => self.handle_refresh_pinned_search(id),
            Message::ExpiryTick => self.handle_expiry_tick(),
            Message::SearchHere(prefix) => self.handle_search_here(prefix),
            Message::SaveAsSmartFolder(name) => self.handle_save_as_smart_folder(name),
            Message::SmartFolderSaved(result) => self.handle_smart_folder_saved(result),

            // Calendar — delegated to handlers/calendar.rs
            Message::Calendar(cal_msg) => self.handle_calendar(*cal_msg),
            Message::ToggleAppMode => {
                // If calendar is popped out and we're in mail, focus the pop-out window
                if self.app_mode == AppMode::Mail {
                    if let Some((&win_id, _)) = self.pop_out_windows.iter().find(|(_, w)| {
                        matches!(w, PopOutWindow::Calendar)
                    }) {
                        return iced::window::gain_focus(win_id);
                    }
                }
                let target = match self.app_mode {
                    AppMode::Mail => AppMode::Calendar,
                    AppMode::Calendar => AppMode::Mail,
                };
                self.update(Message::SetAppMode(target))
            }
            Message::SetAppMode(mode) => {
                if self.app_mode == mode {
                    return Task::none();
                }
                self.app_mode = mode;
                if self.app_mode == AppMode::Calendar {
                    return self.reload_calendar_events();
                }
                Task::none()
            }
            Message::SetCalendarView(view) => {
                if self.app_mode != AppMode::Calendar {
                    self.app_mode = AppMode::Calendar;
                }
                self.update(Message::Calendar(Box::new(CalendarMessage::SetView(view))))
            }
            Message::CalendarToday => {
                self.update(Message::Calendar(Box::new(CalendarMessage::Today)))
            }

            // Account management
            Message::AddAccount(msg) => self.handle_add_account(msg),
            Message::AccountDeleted(Ok(())) | Message::AccountUpdated(Ok(())) => {
                // Reload accounts after delete or update
                let db = Arc::clone(&self.db);
                self.nav_generation += 1;
                let load_gen = self.nav_generation;
                Task::perform(
                    async move { (load_gen, load_accounts(db).await) },
                    |(g, result)| Message::AccountsLoaded(g, result),
                )
            }
            Message::AccountDeleted(Err(e)) => {
                log::error!("Failed to delete account: {e}");
                Task::none()
            }
            Message::AccountUpdated(Err(e)) => {
                log::error!("Failed to update account: {e}");
                Task::none()
            }
            Message::OpenAddAccount => {
                let used_colors = self.sidebar.accounts.iter()
                    .filter_map(|a| a.account_color.clone())
                    .collect();
                self.add_account_wizard =
                    Some(AddAccountWizard::new_add_account(used_colors, Arc::clone(&self.db)));
                Task::none()
            }
            Message::ReloadSignatures => {
                handlers::signatures::load_signatures_async(&self.db)
                    .map(Message::SignatureOp)
            }
            Message::SignatureOp(result) => self.handle_signature_op(result),

            // Pop-out windows — delegated to handlers/pop_out.rs
            Message::PopOut(window_id, pop_out_msg) => {
                self.handle_pop_out_message(window_id, pop_out_msg)
            }
            Message::OpenMessageView(message_index) => {
                self.open_message_view_window(message_index)
            }
            Message::ComposeDraftTick => self.auto_save_compose_drafts(),

            // Thread detail via core (replaces separate messages/attachments loads)
            Message::ThreadDetailLoaded(g, _) if g != self.thread_generation => Task::none(),
            Message::ThreadDetailLoaded(_, Ok(detail)) => {
                self.reading_pane.load_thread_detail(detail);
                Task::none()
            }
            Message::ThreadDetailLoaded(_, Err(e)) => {
                log::error!("ThreadDetailLoaded error: {e}");
                self.status = format!("Thread detail error: {e}");
                Task::none()
            }

            // Clear all pinned searches
            Message::ClearAllPinnedSearches => self.handle_clear_all_pinned_searches(),

            // Sync progress pipeline
            Message::SyncProgress(event) => {
                self.handle_sync_event(event);
                Task::none()
            }
            Message::Undo => {
                if let Some(token) = self.undo_stack.pop() {
                    let desc = token.description();
                    log::info!("Undo: {desc}");
                    // The actual compensation action would be dispatched here
                    // once email actions are wired to providers. For now, log
                    // the intent. The token carries all state needed to reverse
                    // the action (account_id, thread_ids, prior state).
                    self.status_bar
                        .show_confirmation(format!("Undone: {desc}"));
                }
                Task::none()
            }
            Message::SharedMailboxesLoaded(Ok(mailboxes)) => {
                self.sidebar.shared_mailboxes = mailboxes;
                Task::none()
            }
            Message::SharedMailboxesLoaded(Err(e)) => {
                log::warn!("Failed to load shared mailboxes: {e}");
                Task::none()
            }
            Message::PinnedPublicFoldersLoaded(Ok(pins)) => {
                self.sidebar.pinned_public_folders = pins;
                Task::none()
            }
            Message::PinnedPublicFoldersLoaded(Err(e)) => {
                log::warn!("Failed to load pinned public folders: {e}");
                Task::none()
            }
            Message::GalRefreshTick => {
                // Refresh GAL cache for all connected accounts.
                // Currently a placeholder — the actual directory API calls
                // (Graph /users, Google Directory API) require provider
                // clients. When the sync orchestrator provides account-level
                // clients, this dispatches cache_gal_entries() per account.
                log::debug!("GAL refresh tick (directory fetch not yet wired to provider clients)");
                Task::none()
            }
            Message::GalCacheRefreshed(result) => {
                match result {
                    Ok(count) => log::info!("GAL cache refreshed: {count} entries"),
                    Err(e) => log::warn!("GAL cache refresh failed: {e}"),
                }
                Task::none()
            }
            Message::ModifiersChanged(modifiers) => {
                self.current_modifiers = modifiers;
                Task::none()
            }
        }
    }

    fn view(&self, window_id: iced::window::Id) -> Element<'_, Message> {
        if window_id == self.main_window_id {
            return self.view_main_window();
        }

        if let Some(pop_out) = self.pop_out_windows.get(&window_id) {
            return match pop_out {
                PopOutWindow::MessageView(state) => {
                    pop_out::message_view::view_message_window(window_id, state)
                }
                PopOutWindow::Compose(state) => {
                    pop_out::compose::view_compose_window(window_id, state)
                }
                PopOutWindow::Calendar => {
                    ui::calendar::calendar_layout(&self.calendar)
                        .map(|m| Message::Calendar(Box::new(m)))
                }
            };
        }

        ui::widgets::empty_placeholder("Window not found", "").into()
    }

    fn view_main_window(&self) -> Element<'_, Message> {
        if let Some(ref wizard) = self.add_account_wizard {
            if self.no_accounts {
                return self.view_first_launch_modal(wizard);
            }
            return self.view_with_add_account_modal(wizard);
        }

        if self.show_settings {
            let settings_view = self.settings.view().map(Message::Settings);
            return container(settings_view)
                .height(Length::Fill)
                .width(Length::Fill)
                .into();
        }

        let layout = match self.app_mode {
            AppMode::Calendar => {
                let calendar_view = ui::calendar::calendar_layout(&self.calendar)
                    .map(|m| Message::Calendar(Box::new(m)));
                row![calendar_view].height(Length::Fill)
            }
            AppMode::Mail => {
                let sidebar = container(self.sidebar.view().map(Message::Sidebar))
                    .width(SIDEBAR_MIN_WIDTH)
                    .height(Length::Fill);

                let thread_list = container(self.thread_list.view().map(Message::ThreadList))
                    .width(self.thread_list_width)
                    .height(Length::Fill);

                let divider_thread = self.build_divider(Divider::ThreadList);

                let ctx = command_dispatch::build_context(self);
                let reading_pane = container(
                    self.reading_pane.view_with_commands(&self.registry, &self.binding_table, &ctx),
                )
                .width(Length::Fill)
                .height(Length::Fill);

                let rs_data = ui::right_sidebar::RightSidebarData {
                    calendar: &self.calendar,
                    threads: &self.thread_list.threads,
                };
                let right_sidebar = ui::right_sidebar::view(self.right_sidebar_open, &rs_data);

                let status_bar = self.status_bar_view();
                let content_area = column![
                    row![thread_list, divider_thread, reading_pane, right_sidebar]
                        .height(Length::Fill),
                    status_bar,
                ];

                row![sidebar, content_area].height(Length::Fill)
            }
        };

        let full_layout = column![layout];

        let main_layout: Element<'_, Message> = if self.dragging.is_some() {
            mouse_area(full_layout)
                .on_move(Message::DividerDragMove)
                .on_release(Message::DividerDragEnd)
                .interaction(iced::mouse::Interaction::ResizingHorizontally)
                .into()
        } else {
            full_layout.into()
        };

        if self.palette.is_open() {
            let backdrop = mouse_area(
                container("")
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(ui::theme::ContainerClass::PaletteBackdrop.style()),
            )
            .on_press(Message::Palette(PaletteMessage::Close(
                ratatoskr_command_palette::CommandContext::default(),
            )));

            let palette_widget = self.palette.view().map(Message::Palette);

            let palette_positioned = container(palette_widget)
                .width(Length::Fill)
                .padding(iced::Padding {
                    top: ui::layout::PALETTE_TOP_OFFSET,
                    right: 0.0,
                    bottom: 0.0,
                    left: 0.0,
                })
                .align_x(iced::Alignment::Center);

            stack![main_layout, backdrop, palette_positioned].into()
        } else if let Some(ref pending) = self.pending_chord {
            let chord_display = pending.first.display(current_platform());
            let indicator = ui::palette::chord_indicator::<Message>(&chord_display);
            let indicator_positioned = container(indicator)
                .width(Length::Fill)
                .height(Length::Fill)
                .align_y(iced::Alignment::End);
            stack![main_layout, indicator_positioned].into()
        } else {
            main_layout
        }
    }
}

// ── Component event handlers ───────────────────────────

impl App {
    fn handle_sidebar(&mut self, msg: SidebarMessage) -> Task<Message> {
        let (task, event) = self.sidebar.update(msg);
        let mut tasks = vec![task.map(Message::Sidebar)];
        if let Some(evt) = event {
            tasks.push(self.handle_sidebar_event(evt));
        }
        Task::batch(tasks)
    }

    fn handle_sidebar_event(&mut self, event: SidebarEvent) -> Task<Message> {
        match event {
            SidebarEvent::AccountSelected(_idx) => {
                self.clear_search_state();
                self.clear_pinned_search_context();
                self.navigation_target = None;
                self.thread_list.selected_thread = None;
                self.nav_generation += 1;
                self.thread_generation += 1;
                self.update_thread_list_context_from_sidebar();
                self.load_navigation_and_threads()
            }
            SidebarEvent::AllAccountsSelected => {
                self.clear_search_state();
                self.clear_pinned_search_context();
                self.navigation_target = None;
                self.thread_list.selected_thread = None;
                self.nav_generation += 1;
                self.thread_generation += 1;
                self.update_thread_list_context_from_sidebar();
                self.load_navigation_and_threads()
            }
            SidebarEvent::LabelSelected(label_id) => {
                self.clear_search_state();
                self.clear_pinned_search_context();
                self.navigation_target = None;
                self.update_thread_list_context_from_sidebar();
                self.handle_label_selected(label_id)
            }
            SidebarEvent::Compose => self.update(Message::Compose),
            SidebarEvent::ToggleSettings => {
                self.show_settings = !self.show_settings;
                if self.show_settings {
                    self.settings.overlay = None;
                    self.settings.overlay_anim.go_mut(false, iced::time::Instant::now());
                    self.settings.active_tab = crate::ui::settings::Tab::General;
                    self.settings.begin_editing();
                }
                Task::none()
            }
            SidebarEvent::PinnedSearchSelected(id) => {
                self.update(Message::SelectPinnedSearch(id))
            }
            SidebarEvent::PinnedSearchDismissed(id) => {
                self.update(Message::DismissPinnedSearch(id))
            }
            SidebarEvent::ModeToggled => {
                self.update(Message::ToggleAppMode)
            }
            SidebarEvent::SearchHere { query_prefix } => {
                self.update(Message::SearchHere(query_prefix))
            }
            SidebarEvent::SmartFolderSelected { id, query } => {
                self.handle_smart_folder_selected(id, query)
            }
            SidebarEvent::PinnedSearchRefreshed(id) => {
                self.update(Message::RefreshPinnedSearch(id))
            }
            SidebarEvent::SharedMailboxSelected(_) | SidebarEvent::PublicFolderSelected(_) => {
                Task::none()
            }
        }
    }

    fn handle_label_selected(&mut self, _label_id: Option<String>) -> Task<Message> {
        self.thread_list.selected_thread = None;
        self.nav_generation += 1;
        self.thread_generation += 1;
        self.load_threads_for_current_view()
    }

    fn handle_thread_list(&mut self, msg: ThreadListMessage) -> Task<Message> {
        let (task, event) = self.thread_list.update(msg);
        let mut tasks = vec![task.map(Message::ThreadList)];
        if let Some(evt) = event {
            tasks.push(self.handle_thread_list_event(evt));
        }
        Task::batch(tasks)
    }

    fn handle_thread_list_event(&mut self, event: ThreadListEvent) -> Task<Message> {
        match event {
            ThreadListEvent::ThreadSelected(idx) => {
                // Check modifier keys for multi-select behavior.
                if self.current_modifiers.control() {
                    return self.handle_thread_list(
                        ThreadListMessage::ToggleThread(idx),
                    );
                }
                if self.current_modifiers.shift() {
                    return self.handle_thread_list(
                        ThreadListMessage::RangeSelectThread(idx),
                    );
                }
                // Plain click: clear multi-select, single-select.
                self.thread_list.clear_multi_select();
                self.handle_select_thread(idx)
            }
            ThreadListEvent::SearchQueryChanged(query) => {
                self.update(Message::SearchQueryChanged(query))
            }
            ThreadListEvent::SearchExecute => {
                self.update(Message::SearchExecute)
            }
            ThreadListEvent::SearchUndo => {
                if let Some(text) = self.search_query.undo() {
                    let query = text.to_owned();
                    self.thread_list.search_query.clone_from(&query);
                    return self.apply_search_debounce();
                }
                Task::none()
            }
            ThreadListEvent::SearchRedo => {
                if let Some(text) = self.search_query.redo() {
                    let query = text.to_owned();
                    self.thread_list.search_query.clone_from(&query);
                    return self.apply_search_debounce();
                }
                Task::none()
            }
            ThreadListEvent::ThreadDeselected => {
                self.thread_list.selected_thread = None;
                self.reading_pane.set_thread(None);
                Task::none()
            }
            ThreadListEvent::WidenSearchScope => {
                // Widen search scope to all accounts
                self.sidebar.selected_account = None;
                self.nav_generation += 1;
                self.update_thread_list_context_from_sidebar();
                self.update(Message::SearchExecute)
            }
            ThreadListEvent::TypeaheadQuery { .. } => {
                Task::none()
            }
            ThreadListEvent::TypeaheadSelected(idx) => {
                self.handle_typeahead_select(idx)
            }
            ThreadListEvent::MultiSelectionChanged(_count) => {
                // Selection count changed — no action needed yet.
                Task::none()
            }
            ThreadListEvent::AutoAdvance { new_index } => {
                if let Some(idx) = new_index {
                    self.handle_select_thread(idx)
                } else {
                    self.thread_list.selected_thread = None;
                    self.reading_pane.set_thread(None);
                    Task::none()
                }
            }
            ThreadListEvent::BatchAction(_indices) => {
                // Batch email actions not yet wired to providers.
                Task::none()
            }
        }
    }

    fn handle_reading_pane(&mut self, msg: ReadingPaneMessage) -> Task<Message> {
        let (task, event) = self.reading_pane.update(msg);
        let mut tasks = vec![task.map(Message::ReadingPane)];
        if let Some(evt) = event {
            tasks.push(self.handle_reading_pane_event(evt));
        }
        Task::batch(tasks)
    }

    fn handle_reading_pane_event(&mut self, event: ReadingPaneEvent) -> Task<Message> {
        match event {
            ReadingPaneEvent::AttachmentCollapseChanged { thread_key, collapsed } => {
                // thread_key format is "account_id:thread_id"
                if let Some((account_id, thread_id)) = thread_key.split_once(':') {
                    let db = Arc::clone(&self.db);
                    let account_id = account_id.to_string();
                    let thread_id = thread_id.to_string();
                    Task::perform(
                        async move {
                            db::threads::persist_attachments_collapsed(
                                &db, account_id, thread_id, collapsed,
                            )
                            .await
                        },
                        |result| {
                            if let Err(e) = result {
                                log::error!("Failed to persist attachment collapse: {e}");
                            }
                            Message::Noop
                        },
                    )
                } else {
                    Task::none()
                }
            }
            ReadingPaneEvent::OpenMessagePopOut { message_index } => {
                self.open_message_view_window(message_index)
            }
            ReadingPaneEvent::ReplyToMessage { message_index } => {
                self.handle_reading_pane_compose(message_index, ComposeMode::Reply {
                    original_subject: self.current_subject(),
                })
            }
            ReadingPaneEvent::ReplyAllToMessage { message_index } => {
                self.handle_reading_pane_compose(message_index, ComposeMode::ReplyAll {
                    original_subject: self.current_subject(),
                })
            }
            ReadingPaneEvent::ForwardMessage { message_index } => {
                self.handle_reading_pane_compose(message_index, ComposeMode::Forward {
                    original_subject: self.current_subject(),
                })
            }
            ReadingPaneEvent::EditContact { email } => {
                // Open the contact editor in settings for this email.
                // Find or create the contact, then open settings with editor.
                self.open_contact_editor_for_email(email)
            }
            ReadingPaneEvent::CreateEventFromEmail { message_index } => {
                self.create_event_from_email(message_index)
            }
            ReadingPaneEvent::ToggleStar => {
                self.update(Message::ExecuteCommand(CommandId::EmailStar))
            }
        }
    }

    /// Create a calendar event pre-filled from the given email message.
    fn create_event_from_email(&mut self, message_index: usize) -> Task<Message> {
        use chrono::Timelike;
        use crate::ui::calendar::{CalendarEventData, CalendarMessage, CalendarOverlay};

        let msg = self.reading_pane.thread_messages.get(message_index);
        let Some(msg) = msg else { return Task::none() };

        let title = msg.subject.clone().unwrap_or_default();
        let description = msg.snippet.clone().unwrap_or_default();

        // Pre-fill attendees from To/Cc addresses.
        let today = chrono::Local::now().date_naive();
        let hour = chrono::Local::now().time().hour();
        let mut event = CalendarEventData::new_at(today, hour.min(22));
        event.title = title;
        event.description = description;

        // Set the account_id from the current context.
        if let Some(account) = self.sidebar.accounts.first() {
            event.account_id = Some(account.id.clone());
        }

        self.calendar.reset_editor_undo(&event);
        let original_title = event.title.clone();
        self.calendar.overlay = CalendarOverlay::EventEditor {
            event,
            is_new: true,
            original_title,
        };

        // If calendar is popped out, focus that window instead of switching main to calendar.
        if let Some((&win_id, _)) = self.pop_out_windows.iter().find(|(_, w)| {
            matches!(w, crate::pop_out::PopOutWindow::Calendar)
        }) {
            return iced::window::gain_focus(win_id);
        }

        // Otherwise switch to calendar mode to show the editor.
        self.app_mode = AppMode::Calendar;
        self.reload_calendar_events()
    }

    /// Open the contact editor in settings for a specific email address.
    /// Navigates to Settings > People and opens the editor, creating a
    /// new local contact if none exists for that email.
    fn open_contact_editor_for_email(&mut self, email: String) -> Task<Message> {
        use crate::ui::settings::SettingsMessage;

        // Open settings and switch to People tab
        self.show_settings = true;
        self.settings.active_tab = crate::ui::settings::types::Tab::People;

        // Look up existing contact or create new editor state
        let found = self.settings.contacts.iter().find(|c| {
            c.email.eq_ignore_ascii_case(&email)
        });

        if let Some(contact) = found {
            let id = contact.id.clone();
            self.settings.open_contact_editor(&id);
        } else {
            // Create a new editor pre-populated with the email
            self.settings.open_new_contact_editor();
            if let Some(ref mut editor) = self.settings.contact_editor {
                editor.email = email;
            }
        }

        // Load contacts for the settings view
        self.handle_load_contacts(self.settings.contact_filter.clone())
    }

    /// Get the subject of the currently selected thread.
    fn current_subject(&self) -> String {
        self.thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx))
            .and_then(|t| t.subject.clone())
            .unwrap_or_default()
    }

    /// Open a compose window from a reading pane Reply/ReplyAll/Forward action.
    fn handle_reading_pane_compose(
        &mut self,
        message_index: usize,
        mode: ComposeMode,
    ) -> Task<Message> {
        // Clone all data upfront to avoid borrow checker conflicts with &mut self.
        let msg = self.reading_pane.thread_messages.get(message_index).cloned();
        let to_email = msg.as_ref().and_then(|m| m.from_address.clone());
        let to_name = msg.as_ref().and_then(|m| m.from_name.clone());
        let cc_emails = msg.as_ref().and_then(|m| m.cc_addresses.clone());
        let thread_id = self
            .thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx))
            .map(|t| t.id.clone());
        let message_id = msg.as_ref().map(|m| m.id.clone());
        let snippet = msg.as_ref().and_then(|m| m.snippet.clone());

        let state = pop_out::compose::ComposeState::new_reply(
            &self.sidebar.accounts,
            &mode,
            to_email.as_deref(),
            to_name.as_deref(),
            cc_emails.as_deref(),
            snippet.as_deref(),
            thread_id.as_deref(),
            message_id.as_deref(),
        );

        self.open_compose_window_with_state(state, mode)
    }

    fn handle_status_bar(&mut self, msg: StatusBarMessage) -> Task<Message> {
        let (task, event) = self.status_bar.update(msg);
        let mut tasks = vec![task.map(Message::StatusBar)];
        if let Some(evt) = event {
            tasks.push(self.handle_status_bar_event(evt));
        }
        Task::batch(tasks)
    }

    fn handle_status_bar_event(&mut self, event: StatusBarEvent) -> Task<Message> {
        match event {
            StatusBarEvent::RequestReauth { account_id } => {
                self.handle_open_reauth_wizard(account_id)
            }
        }
    }

    fn handle_sync_event(&mut self, event: SyncEvent) {
        match event {
            SyncEvent::Progress { account_id, phase, current, total } => {
                log::info!("Sync progress: account={account_id} phase={phase} {current}/{total}");
                let email = self.email_for_account(&account_id);
                self.status_bar.report_sync_progress(
                    account_id, email, current, total, phase,
                );
            }
            SyncEvent::Complete { account_id } => {
                log::info!("Sync complete: account={account_id}");
                self.status_bar.report_sync_complete(&account_id);
                // Clear connection failure warnings on successful sync.
                self.status_bar.clear_warning(&account_id);
            }
            SyncEvent::Error { account_id, error } => {
                log::warn!("Sync error: account={account_id} error={error}");
                let email = self.email_for_account(&account_id);
                self.status_bar.set_warning(AccountWarning {
                    account_id,
                    email,
                    kind: WarningKind::ConnectionFailure { message: error },
                });
            }
        }
    }

    /// Look up the email address for an account ID from the sidebar's
    /// account list. Returns the account ID itself if not found.
    fn email_for_account(&self, account_id: &str) -> String {
        self.sidebar.accounts.iter()
            .find(|a| a.id == account_id)
            .map(|a| a.email.clone())
            .unwrap_or_else(|| account_id.to_string())
    }

    /// Render the status bar, respecting the `sync_status_bar` setting.
    /// When the setting is off, returns an empty zero-height element.
    fn status_bar_view(&self) -> Element<'_, Message> {
        if self.settings.sync_status_bar {
            self.status_bar.view().map(Message::StatusBar)
        } else {
            Space::new().width(0).height(0).into()
        }
    }

    fn handle_settings(&mut self, msg: SettingsMessage) -> Task<Message> {
        let (task, event) = self.settings.update(msg);
        let mut tasks = vec![task.map(Message::Settings)];
        if let Some(evt) = event {
            tasks.push(self.handle_settings_event(evt));
        }
        Task::batch(tasks)
    }

    fn handle_settings_event(&mut self, event: SettingsEvent) -> Task<Message> {
        match event {
            SettingsEvent::Closed => {
                self.show_settings = false;
                Task::none()
            }
            SettingsEvent::PreferencesCommitted | SettingsEvent::PreferencesDiscarded => {
                // Preferences have been committed or discarded within Settings.
                // The live fields are already updated — no additional action needed.
                Task::none()
            }
            SettingsEvent::DateDisplayChanged(display) => {
                self.reading_pane.date_display = display;
                Task::none()
            }
            SettingsEvent::OpenAddAccountWizard => self.handle_open_add_account_wizard(),
            SettingsEvent::DeleteAccount(account_id) => {
                self.handle_delete_account(account_id)
            }
            SettingsEvent::SaveAccountChanges { account_id, params } => {
                self.handle_save_account_changes(account_id, params)
            }
            SettingsEvent::SaveSignature(req) => {
                handlers::signatures::handle_save_signature(&self.db, req)
                    .map(Message::SignatureOp)
            }
            SettingsEvent::DeleteSignature(id) => {
                handlers::signatures::handle_delete_signature(&self.db, id)
                    .map(Message::SignatureOp)
            }
            SettingsEvent::ReorderSignatures(ordered_ids) => {
                handlers::signatures::handle_reorder_signatures(&self.db, ordered_ids)
                    .map(Message::SignatureOp)
            }
            SettingsEvent::LoadContacts(filter) => self.handle_load_contacts(filter),
            SettingsEvent::LoadGroups(filter) => self.handle_load_groups(filter),
            SettingsEvent::SaveContact(entry) => self.handle_save_contact(entry),
            SettingsEvent::DeleteContact(id) => self.handle_delete_contact(id),
            SettingsEvent::SaveGroup(group, members) => self.handle_save_group(group, members),
            SettingsEvent::DeleteGroup(id) => self.handle_delete_group(id),
            SettingsEvent::LoadGroupMembers(group_id) => self.handle_load_group_members(group_id),
            SettingsEvent::ExecuteContactImport { contacts, account_id, update_existing } => {
                self.handle_import_contacts(contacts, account_id, update_existing)
            }
            SettingsEvent::ReorderAccounts(orders) => {
                self.handle_reorder_accounts(orders)
            }
            SettingsEvent::ReauthenticateAccount(account_id) => {
                self.handle_open_reauth_wizard(account_id)
            }
        }
    }

    fn handle_add_account(&mut self, msg: AddAccountMessage) -> Task<Message> {
        let wizard = match self.add_account_wizard.as_mut() {
            Some(w) => w,
            None => return Task::none(),
        };

        let (task, event) = wizard.update(msg);
        let mut tasks = vec![task.map(Message::AddAccount)];

        if let Some(evt) = event {
            tasks.push(self.handle_add_account_event(evt));
        }
        Task::batch(tasks)
    }
}

// ── Signature result handler ────────────────────────────

impl App {
    fn handle_signature_op(&mut self, result: handlers::SignatureResult) -> Task<Message> {
        match result {
            handlers::SignatureResult::Loaded(Ok(sigs)) => {
                self.settings.signatures = sigs;
                Task::none()
            }
            handlers::SignatureResult::Loaded(Err(e)) => {
                log::error!("Failed to load signatures: {e}");
                Task::none()
            }
            handlers::SignatureResult::Saved(_) | handlers::SignatureResult::Deleted(_) => {
                handlers::signatures::load_signatures_async(&self.db)
                    .map(Message::SignatureOp)
            }
        }
    }

    fn handle_delete_account(&mut self, account_id: String) -> Task<Message> {
        // If this was the selected account, revert to All Accounts
        if let Some(idx) = self.sidebar.selected_account {
            if self.sidebar.accounts.get(idx).is_some_and(|a| a.id == account_id) {
                self.sidebar.selected_account = None;
            }
        }

        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    ratatoskr_core::account::delete::delete_account_row(conn, &account_id)
                })
                .await
            },
            Message::AccountDeleted,
        )
    }

    fn handle_save_account_changes(
        &mut self,
        account_id: String,
        params: ratatoskr_core::db::queries_extra::UpdateAccountParams,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    ratatoskr_core::db::queries_extra::update_account_sync(
                        conn, &account_id, params,
                    )
                })
                .await
            },
            Message::AccountUpdated,
        )
    }

    fn handle_reorder_accounts(
        &mut self,
        orders: Vec<(String, i64)>,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    let mut stmt = conn
                        .prepare(
                            "UPDATE accounts SET sort_order = ?1 WHERE id = ?2",
                        )
                        .map_err(|e| e.to_string())?;
                    for (account_id, sort_order) in &orders {
                        stmt.execute(rusqlite::params![sort_order, account_id])
                            .map_err(|e| e.to_string())?;
                    }
                    Ok(())
                })
                .await
            },
            Message::AccountUpdated,
        )
    }
}

// ── Helper methods ─────────────────────────────────────

impl App {
    fn current_scope(&self) -> AccountScope {
        match self.sidebar.selected_account {
            Some(idx) => {
                let Some(account) = self.sidebar.accounts.get(idx) else {
                    return AccountScope::All;
                };
                AccountScope::Single(account.id.clone())
            }
            None => AccountScope::All,
        }
    }

    fn fire_navigation_load(&self) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let scope = self.current_scope();
        let load_gen = self.nav_generation;
        Task::perform(
            async move {
                let r = load_navigation(db, scope).await;
                (load_gen, r)
            },
            |(g, result)| Message::NavigationLoaded(g, result),
        )
    }

    pub(crate) fn load_threads_for_current_view(&self) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let scope = self.current_scope();
        let label_id = self.sidebar.selected_label.clone();
        let load_gen = self.nav_generation;
        Task::perform(
            async move {
                let r = load_threads_scoped(db, scope, label_id).await;
                (load_gen, r)
            },
            |(g, result)| Message::ThreadsLoaded(g, result),
        )
    }

    fn load_navigation_and_threads(&self) -> Task<Message> {
        Task::batch([
            self.fire_navigation_load(),
            self.load_threads_for_current_view(),
        ])
    }

    fn handle_select_thread(&mut self, idx: usize) -> Task<Message> {
        let thread = self.thread_list.threads.get(idx);
        if let Some(t) = thread {
            log::debug!("Thread selected: {}", t.id);
        }
        self.reading_pane.set_thread(thread);

        // Set search highlight terms when in search mode
        if self.thread_list.mode == ui::thread_list::ThreadListMode::Search {
            let query = self.search_query.text().to_string();
            let parsed = ratatoskr_smart_folder::parse_query(&query);
            self.reading_pane.search_highlight_terms = parsed
                .free_text
                .split_whitespace()
                .map(String::from)
                .collect();
        } else {
            self.reading_pane.search_highlight_terms.clear();
        }

        self.thread_generation += 1;
        if let Some(thread) = thread {
            let account_id = thread.account_id.clone();
            let thread_id = thread.id.clone();
            let load_gen = self.thread_generation;

            // Use core's thread detail if body store is available,
            // otherwise fall back to the old separate queries.
            if let Some(ref body_store) = self.body_store {
                let db = Arc::clone(&self.db);
                let bs = body_store.clone();
                return Task::perform(
                    async move {
                        let r = db::threads::load_thread_detail(
                            &db, &bs, account_id, thread_id,
                        )
                        .await;
                        (load_gen, r)
                    },
                    |(g, result)| Message::ThreadDetailLoaded(g, result),
                );
            }

        }
        Task::none()
    }

    fn handle_accounts_loaded(&mut self, accounts: Vec<db::Account>) -> Task<Message> {
        self.sidebar.accounts = accounts;
        if self.sidebar.accounts.is_empty() {
            self.no_accounts = true;
            self.add_account_wizard = Some(AddAccountWizard::new_first_launch(
                Arc::clone(&self.db),
            ));
            self.status = "Welcome".to_string();
            return Task::none();
        }
        self.no_accounts = false;
        self.settings.managed_accounts = self
            .sidebar
            .accounts
            .iter()
            .map(|a| ui::settings::ManagedAccount {
                id: a.id.clone(),
                email: a.email.clone(),
                provider: a.provider.clone(),
                account_name: a.account_name.clone(),
                account_color: a.account_color.clone(),
                display_name: a.display_name.clone(),
                last_sync_at: a.last_sync_at,
                health: ui::settings::compute_health(a.last_sync_at, a.token_expires_at, a.is_active),
            })
            .collect();
        self.sidebar.selected_account = Some(0);
        self.status = format!("Loaded {} accounts", self.sidebar.accounts.len());
        let sig_task = handlers::signatures::load_signatures_async(&self.db)
            .map(Message::SignatureOp);
        let sync_task = self.sync_all_accounts();
        let push_task = self.start_jmap_push();
        Task::batch([self.load_navigation_and_threads(), sig_task, sync_task, push_task])
    }

    fn view_first_launch_modal<'a>(
        &'a self,
        wizard: &'a AddAccountWizard,
    ) -> Element<'a, Message> {
        use ui::layout::{ACCOUNT_MODAL_MAX_HEIGHT, ACCOUNT_MODAL_WIDTH};

        let modal_content = wizard.view().map(Message::AddAccount);

        let modal = container(modal_content)
            .width(Length::Fixed(ACCOUNT_MODAL_WIDTH))
            .max_height(ACCOUNT_MODAL_MAX_HEIGHT)
            .padding(ui::layout::PAD_SETTINGS_CONTENT)
            .style(ui::theme::ContainerClass::Elevated.style());

        container(modal)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(ui::theme::ContainerClass::Content.style())
            .into()
    }

    fn view_with_add_account_modal<'a>(
        &'a self,
        wizard: &'a AddAccountWizard,
    ) -> Element<'a, Message> {
        use ui::layout::{ACCOUNT_MODAL_MAX_HEIGHT, ACCOUNT_MODAL_WIDTH};

        let base_layout = self.view_main_layout();

        let modal_content = wizard.view().map(Message::AddAccount);

        let modal = container(modal_content)
            .width(Length::Fixed(ACCOUNT_MODAL_WIDTH))
            .max_height(ACCOUNT_MODAL_MAX_HEIGHT)
            .padding(ui::layout::PAD_SETTINGS_CONTENT)
            .style(ui::theme::ContainerClass::Elevated.style());

        let centered_modal = container(modal)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x(Length::Fill)
            .center_y(Length::Fill);

        let blocker = mouse_area(
            container("")
                .width(Length::Fill)
                .height(Length::Fill)
                .style(ui::theme::ContainerClass::ModalBackdrop.style()),
        )
        .on_press(Message::Noop);

        stack![base_layout, blocker, centered_modal]
            .width(Length::Fill)
            .height(Length::Fill)
            .into()
    }

    fn view_main_layout(&self) -> Element<'_, Message> {
        let sidebar = container(self.sidebar.view().map(Message::Sidebar))
            .width(self.sidebar_width)
            .height(Length::Fill);
        let divider_sidebar = self.build_divider(Divider::Sidebar);
        let thread_list =
            container(self.thread_list.view().map(Message::ThreadList))
                .width(self.thread_list_width)
                .height(Length::Fill);
        let divider_thread = self.build_divider(Divider::ThreadList);
        let ctx = command_dispatch::build_context(self);
        let reading_pane = container(
            self.reading_pane
                .view_with_commands(&self.registry, &self.binding_table, &ctx),
        )
        .width(Length::Fill)
        .height(Length::Fill);
        let rs_data = ui::right_sidebar::RightSidebarData {
            calendar: &self.calendar,
            threads: &self.thread_list.threads,
        };
        let right_sidebar =
            ui::right_sidebar::view(self.right_sidebar_open, &rs_data);
        let layout = row![
            sidebar,
            divider_sidebar,
            thread_list,
            divider_thread,
            reading_pane,
            right_sidebar
        ]
        .height(Length::Fill);
        let status_bar = self.status_bar_view();
        column![layout, status_bar].into()
    }

    fn handle_divider_drag(&mut self, point: Point) -> Task<Message> {
        match self.dragging {
            Some(Divider::Sidebar) => {
                self.sidebar_width = point.x.max(SIDEBAR_MIN_WIDTH);
            }
            Some(Divider::ThreadList) => {
                let new_width = (point.x - self.sidebar_width - DIVIDER_WIDTH)
                    .max(THREAD_LIST_MIN_WIDTH);
                self.thread_list_width = new_width;
            }
            None => {}
        }
        Task::none()
    }

    fn handle_clear_all_pinned_searches(&mut self) -> Task<Message> {
        self.pinned_searches.clear();
        self.active_pinned_search = None;
        self.sidebar.active_pinned_search = None;
        self.sidebar.pinned_searches.clear();
        let db = Arc::clone(&self.db);
        Task::perform(
            async move {
                db.delete_all_pinned_searches().await.map(|_| ())
            },
            |result| {
                if let Err(e) = result {
                    log::error!("Failed to clear pinned searches: {e}");
                }
                Message::Noop
            },
        )
    }

    fn handle_window_close(&mut self, id: iced::window::Id) -> Task<Message> {
        if id == self.main_window_id {
            log::info!("Main window closing, saving state");
            let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
            self.window.sidebar_width = self.sidebar_width;
            self.window.thread_list_width = self.thread_list_width;
            self.window.right_sidebar_open = self.right_sidebar_open;
            self.window.save(data_dir);
            self.save_session_state();
            let mut tasks: Vec<Task<Message>> = self
                .pop_out_windows
                .keys()
                .map(|&win_id| iced::window::close(win_id))
                .collect();
            self.pop_out_windows.clear();
            tasks.push(iced::window::close(id));
            tasks.push(iced::exit());
            return Task::batch(tasks);
        }

        if matches!(self.pop_out_windows.get(&id), Some(PopOutWindow::Compose(_))) {
            self.composer_is_open = false;
        }
        // Calendar pop-out closing — calendar becomes available in main window again.
        // (No state change needed — mode toggle just works.)
        self.pop_out_windows.remove(&id);
        iced::window::close(id)
    }

    fn build_divider(&self, divider: Divider) -> Element<'_, Message> {
        let class = if self.hovered_divider == Some(divider)
            || self.dragging == Some(divider)
        {
            ui::theme::ContainerClass::DividerHover
        } else {
            ui::theme::ContainerClass::Divider
        };
        mouse_area(
            container("")
                .width(DIVIDER_WIDTH)
                .height(Length::Fill)
                .style(class.style()),
        )
        .on_press(Message::DividerDragStart(divider))
        .on_release(Message::DividerDragEnd)
        .on_enter(Message::DividerHover(divider))
        .on_exit(Message::DividerUnhover)
        .interaction(iced::mouse::Interaction::ResizingHorizontally)
        .into()
    }

    fn update_thread_list_context_from_sidebar(&mut self) {
        let folder_name = self
            .sidebar
            .selected_label
            .as_ref()
            .and_then(|lid| {
                self.sidebar.nav_state.as_ref().and_then(|ns| {
                    ns.folders
                        .iter()
                        .find(|f| f.id == *lid)
                        .map(|f| f.name.clone())
                })
            })
            .unwrap_or_else(|| "Inbox".to_string());
        let scope_name = self
            .sidebar
            .selected_account
            .and_then(|idx| self.sidebar.accounts.get(idx))
            .and_then(|a| a.display_name.as_deref().or(Some(a.email.as_str())))
            .unwrap_or("All")
            .to_string();
        self.thread_list.set_context(folder_name, scope_name);
    }
}

// ── Free functions ─────────────────────────────────────

pub(crate) async fn load_accounts(db: Arc<Db>) -> Result<Vec<db::Account>, String> {
    db.get_accounts().await
}

async fn load_navigation(
    db: Arc<Db>,
    scope: AccountScope,
) -> Result<NavigationState, String> {
    db.with_conn(move |conn| get_navigation_state(conn, &scope))
        .await
}

async fn load_threads_scoped(
    db: Arc<Db>,
    scope: AccountScope,
    label_id: Option<String>,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        let db_threads =
            get_threads_scoped(conn, &scope, label_id.as_deref(), Some(1000), None)?;
        let mut threads: Vec<Thread> = db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect();

        // When viewing Drafts, also include local-only drafts
        if label_id.as_deref() == Some("DRAFT") {
            let local = ratatoskr_core::db::queries_extra::get_local_draft_summaries(
                conn, &scope, Some(1000), None,
            )?;
            let local_threads: Vec<Thread> = local
                .into_iter()
                .map(local_draft_to_app_thread)
                .collect();
            threads.extend(local_threads);
            // Sort all drafts together by updated_at DESC
            threads.sort_by_key(|t| std::cmp::Reverse(t.last_message_at));
        }

        Ok(threads)
    })
    .await
}

fn db_thread_to_app_thread(t: DbThread) -> Thread {
    Thread {
        id: t.id,
        account_id: t.account_id,
        subject: t.subject,
        snippet: t.snippet,
        last_message_at: t.last_message_at,
        message_count: t.message_count,
        is_read: t.is_read,
        is_starred: t.is_starred,
        is_pinned: t.is_pinned,
        is_muted: t.is_muted,
        has_attachments: t.has_attachments,
        from_name: t.from_name,
        from_address: t.from_address,
        is_local_draft: false,
    }
}

fn local_draft_to_app_thread(
    d: ratatoskr_core::db::queries_extra::LocalDraftSummary,
) -> Thread {
    Thread {
        id: d.id,
        account_id: d.account_id,
        subject: d.subject,
        snippet: d.snippet,
        last_message_at: Some(d.updated_at),
        message_count: 1,
        is_read: true,
        is_starred: false,
        is_pinned: false,
        is_muted: false,
        has_attachments: false,
        from_name: None,
        from_address: d.from_email,
        is_local_draft: true,
    }
}
