mod appearance;
mod command_dispatch;
mod command_resolver;
mod component;
mod db;
mod display;
mod font;
mod icon;
mod ui;
mod window_state;

use command_dispatch::{
    ComposeAction, EmailAction, KeyEventMessage, NavigationTarget, PaletteMessage,
    ReadingPanePosition, TaskAction,
};
use component::Component;
use db::{Db, Thread};
use iced::widget::{column, container, mouse_area, row, stack};
use iced::{Element, Length, Point, Size, Task, Theme};
use ui::palette::PaletteState;
use ratatoskr_command_palette::{
    BindingTable, Chord, CommandArgs, CommandId, CommandInputResolver, CommandRegistry,
    FocusedRegion, OptionItem, ResolveResult, current_platform,
};
use ratatoskr_core::db::queries_extra::navigation::{
    NavigationState, get_navigation_state,
};
use ratatoskr_core::db::queries_extra::get_threads_scoped;
use ratatoskr_core::db::types::{AccountScope, DbThread};
use ui::layout::{RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH, SIDEBAR_MIN_WIDTH, THREAD_LIST_MIN_WIDTH};
use ui::add_account::{AddAccountEvent, AddAccountMessage, AddAccountWizard};
use ui::reading_pane::{ReadingPane, ReadingPaneMessage};
use ui::settings::{Settings, SettingsEvent, SettingsMessage};
use ui::sidebar::{Sidebar, SidebarEvent, SidebarMessage};
use ui::status_bar::{StatusBar, StatusBarEvent, StatusBarMessage};
use ui::thread_list::{ThreadList, ThreadListEvent, ThreadListMessage};
use std::path::PathBuf;
use std::sync::Arc;

static DB: std::sync::OnceLock<Arc<Db>> = std::sync::OnceLock::new();
static APP_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static DEFAULT_SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();

/// How long to wait for the second chord of a sequence.
const CHORD_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);

fn main() -> iced::Result {
    let app_data_dir = dirs::data_dir()
        .expect("no data dir")
        .join("com.velo.app");

    let db = Db::open(&app_data_dir)
        .map_err(|e| iced::Error::WindowCreationFailed(e.into()))?;
    let _ = DB.set(Arc::new(db));

    let detected_scale = display::detect_default_scale();
    let _ = DEFAULT_SCALE.set(detected_scale);

    // Detect system UI font before launching the app. The detection is async
    // (D-Bus on Linux), so we block briefly on a throwaway tokio runtime.
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

    let window = window_state::WindowState::load(&app_data_dir);
    let _ = APP_DATA_DIR.set(app_data_dir);

    let mut app = iced::application(App::boot, App::update, App::view)
        .title("Ratatoskr (iced prototype)")
        .theme(App::theme)
        .scale_factor(|app| app.settings.scale)
        .subscription(App::subscription)
        .default_font(font::text())
        .window(window.to_window_settings());

    for f in font::load() {
        app = app.font(f);
    }

    app.run()
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
    #[allow(dead_code)]
    started: iced::time::Instant,
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
    ThreadMessagesLoaded(u64, Result<Vec<db::ThreadMessage>, String>),
    ThreadAttachmentsLoaded(u64, Result<Vec<db::ThreadAttachment>, String>),

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
    WindowResized(Size),
    WindowMoved(Point),
    ToggleRightSidebar,
    SetDateDisplay(db::DateDisplay),
    WindowCloseRequested(iced::window::Id),

    // Command system (Slice 6a)
    KeyEvent(KeyEventMessage),
    ExecuteCommand(CommandId),
    ExecuteParameterized(CommandId, CommandArgs),
    NavigateTo(NavigationTarget),
    Escape,
    EmailAction(EmailAction),
    ComposeAction(ComposeAction),
    TaskAction(TaskAction),
    SetTheme(String),
    ToggleSidebar,
    FocusSearch,
    ShowHelp,
    SyncCurrentFolder,
    SetReadingPanePosition(ReadingPanePosition),

    // Palette placeholder (Slice 6b)
    Palette(PaletteMessage),

    // Search
    /// The search bar text changed (debounced).
    SearchQueryChanged(String),
    /// Debounce fired or Enter pressed — execute search.
    SearchExecute,
    /// Async search results arrived. u64 is the generation for staleness detection.
    SearchResultsLoaded(u64, Result<Vec<Thread>, String>),
    /// Clear search and restore folder view.
    SearchClear,
    /// Focus the search bar (e.g. `/` shortcut).
    FocusSearchBar,
    /// Unfocus the search bar without clearing.
    SearchBlur,

    // Pinned searches
    PinnedSearchesLoaded(Result<Vec<db::PinnedSearch>, String>),
    SelectPinnedSearch(i64),
    PinnedSearchThreadIdsLoaded(u64, i64, Result<Vec<(String, String)>, String>),
    PinnedSearchThreadsLoaded(u64, Result<Vec<Thread>, String>),
    DismissPinnedSearch(i64),
    PinnedSearchDismissed(i64, Result<(), String>),
    PinnedSearchSaved(Result<i64, String>),
    PinnedSearchesExpired(Result<u64, String>),

    // Account management
    AddAccount(AddAccountMessage),
    OpenAddAccount,
    ReloadSignatures,
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
    sidebar_width: f32,
    thread_list_width: f32,
    dragging: Option<Divider>,
    hovered_divider: Option<Divider>,
    right_sidebar_open: bool,
    show_settings: bool,
    window: window_state::WindowState,
    /// Incremented on every navigation load (accounts, labels, threads).
    nav_generation: u64,
    /// Incremented on every thread detail load (messages, attachments).
    thread_generation: u64,

    // Command palette infrastructure
    registry: CommandRegistry,
    binding_table: BindingTable,
    /// Which panel currently has focus. Updated on click/tab.
    focused_region: Option<FocusedRegion>,
    /// Network connectivity state.
    is_online: bool,
    /// Whether the compose window/panel is open.
    composer_is_open: bool,
    /// Pending chord for two-key sequence bindings.
    pending_chord: Option<PendingChord>,
    /// Command palette overlay state.
    palette: PaletteState,
    /// Resolver for parameterized command options (stage 2).
    resolver: Arc<command_resolver::AppInputResolver>,

    // Search state
    /// Monotonically increasing counter for search result freshness.
    search_generation: u64,
    /// The query string currently in the search bar.
    search_query: String,
    /// When set, a search execution is pending after this instant (debounce).
    search_debounce_deadline: Option<iced::time::Instant>,
    /// Threads displayed before the current search, restored on SearchClear.
    pre_search_threads: Option<Vec<Thread>>,

    // Pinned searches
    /// All pinned searches, loaded at boot. Ordered by updated_at DESC.
    pinned_searches: Vec<db::PinnedSearch>,
    /// The currently selected pinned search, if any.
    active_pinned_search: Option<i64>,
    /// Tracks which pinned search to update on next search execution.
    editing_pinned_search: Option<i64>,
    /// Whether auto-expiry has run this session.
    expiry_ran: bool,

    /// True when the app has no configured accounts.
    no_accounts: bool,
    /// The add-account wizard state. Some when the modal is open.
    add_account_wizard: Option<AddAccountWizard>,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let db = Arc::clone(DB.get().expect("DB not initialized"));
        let db_ref = Arc::clone(&db);
        let db_ref2 = Arc::clone(&db);
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let window = window_state::WindowState::load(data_dir);

        let registry = CommandRegistry::new();
        let binding_table = BindingTable::new(&registry, current_platform());
        let resolver = Arc::new(command_resolver::AppInputResolver::new(Arc::clone(&db)));

        let app = Self {
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
            sidebar_width: window.sidebar_width,
            thread_list_width: window.thread_list_width,
            dragging: None,
            hovered_divider: None,
            right_sidebar_open: window.right_sidebar_open,
            show_settings: false,
            window,
            nav_generation: 1,
            thread_generation: 0,
            registry,
            binding_table,
            focused_region: None,
            is_online: true,
            composer_is_open: false,
            pending_chord: None,
            palette: PaletteState::new(),
            resolver,
            search_generation: 0,
            search_query: String::new(),
            search_debounce_deadline: None,
            pre_search_threads: None,
            pinned_searches: Vec::new(),
            active_pinned_search: None,
            editing_pinned_search: None,
            expiry_ran: false,
            no_accounts: false,
            add_account_wizard: None,
        };
        let load_gen = app.nav_generation;
        (app, Task::batch([
            Task::perform(
                async move { (load_gen, load_accounts(db_ref).await) },
                |(g, result)| Message::AccountsLoaded(g, result),
            ),
            Task::perform(
                async move { db_ref2.list_pinned_searches().await },
                Message::PinnedSearchesLoaded,
            ),
        ]))
    }

    fn theme(&self) -> Theme {
        match self.settings.theme.as_str() {
            "Light" => Theme::custom(String::from("Light"), iced::theme::palette::Seed::LIGHT),
            "Dark" => Theme::custom(String::from("Dark"), iced::theme::palette::Seed::DARK),
            "Theme" => {
                let idx = self.settings.selected_theme.unwrap_or(0);
                ui::theme::theme_by_index(idx)
            }
            // System -- follow OS
            _ => match self.mode {
                appearance::Mode::Light => Theme::custom(String::from("Light"), iced::theme::palette::Seed::LIGHT),
                _ => Theme::custom(String::from("Dark"), iced::theme::palette::Seed::DARK),
            },
        }
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        let mut subs = vec![
            // App-level subscriptions
            appearance::subscription().map(Message::AppearanceChanged),
            iced::window::resize_events().map(|(_id, size)| Message::WindowResized(size)),
            iced::window::close_requests().map(Message::WindowCloseRequested),
            iced::event::listen_with(|event, _status, _id| {
                if let iced::Event::Window(iced::window::Event::Moved(point)) = event {
                    Some(Message::WindowMoved(point))
                } else {
                    None
                }
            }),
            // Global keyboard dispatch
            iced::event::listen_with(|event, status, _id| {
                if let iced::Event::Keyboard(
                    iced::keyboard::Event::KeyPressed { key, modifiers, .. }
                ) = &event {
                    Some(Message::KeyEvent(KeyEventMessage::KeyPressed {
                        key: key.clone(),
                        modifiers: *modifiers,
                        status,
                    }))
                } else {
                    None
                }
            }),
            // Component subscriptions
            self.sidebar.subscription().map(Message::Sidebar),
            self.thread_list.subscription().map(Message::ThreadList),
            self.reading_pane.subscription().map(Message::ReadingPane),
            self.settings.subscription().map(Message::Settings),
            self.status_bar.subscription().map(Message::StatusBar),
        ];

        // Pending chord timeout
        if self.pending_chord.is_some() {
            subs.push(
                iced::time::every(CHORD_TIMEOUT)
                    .map(|_| Message::KeyEvent(KeyEventMessage::PendingChordTimeout)),
            );
        }

        // Search debounce timer — polls every 50ms while a deadline is set
        if let Some(deadline) = self.search_debounce_deadline {
            subs.push(
                iced::time::every(std::time::Duration::from_millis(50))
                    .map(move |_| {
                        if iced::time::Instant::now() >= deadline {
                            Message::SearchExecute
                        } else {
                            Message::Noop
                        }
                    }),
            );
        }

        // Drive overlay slide animation
        if self.settings.overlay_anim.is_animating(iced::time::Instant::now()) {
            subs.push(
                iced::window::frames()
                    .map(|at| Message::Settings(SettingsMessage::OverlayAnimTick(at))),
            );
        }

        iced::Subscription::batch(subs)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Sidebar(msg) => self.handle_sidebar(msg),
            Message::ThreadList(msg) => self.handle_thread_list(msg),
            Message::ReadingPane(msg) => self.handle_reading_pane(msg),
            Message::Settings(msg) => self.handle_settings(msg),
            Message::StatusBar(msg) => self.handle_status_bar(msg),
            Message::AppearanceChanged(mode) => {
                self.mode = mode;
                Task::none()
            }
            Message::AccountsLoaded(g, _) if g != self.nav_generation => Task::none(),
            Message::AccountsLoaded(_, Ok(accounts)) => self.handle_accounts_loaded(accounts),
            Message::AccountsLoaded(_, Err(e)) => {
                self.status = format!("Error: {e}");
                Task::none()
            }
            Message::NavigationLoaded(g, _) if g != self.nav_generation => Task::none(),
            Message::NavigationLoaded(_, Ok(nav_state)) => {
                self.sidebar.nav_state = Some(nav_state);
                Task::none()
            }
            Message::NavigationLoaded(_, Err(e)) => {
                self.status = format!("Navigation error: {e}");
                Task::none()
            }
            Message::ThreadsLoaded(g, _) if g != self.nav_generation => Task::none(),
            Message::ThreadsLoaded(_, Ok(threads)) => {
                self.status = format!("{} threads", threads.len());
                self.thread_list.set_threads(threads);
                Task::none()
            }
            Message::ThreadsLoaded(_, Err(e)) => {
                self.status = format!("Threads error: {e}");
                Task::none()
            }
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
            Message::ToggleSettings => {
                self.show_settings = !self.show_settings;
                Task::none()
            }
            Message::ToggleRightSidebar => {
                self.right_sidebar_open = !self.right_sidebar_open;
                Task::none()
            }
            Message::WindowResized(size) => {
                self.window.set_size(size);
                if size.width < RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH && self.right_sidebar_open {
                    self.right_sidebar_open = false;
                }
                Task::none()
            }
            Message::WindowMoved(point) => {
                self.window.set_position(point);
                Task::none()
            }
            Message::WindowCloseRequested(id) => self.handle_window_close(id),
            Message::ThreadMessagesLoaded(g, _) if g != self.thread_generation => Task::none(),
            Message::ThreadMessagesLoaded(_, Ok(messages)) => {
                self.reading_pane.apply_message_expansion(&messages);
                self.reading_pane.thread_messages = messages;
                Task::none()
            }
            Message::ThreadMessagesLoaded(_, Err(e)) => {
                self.status = format!("Messages error: {e}");
                Task::none()
            }
            Message::ThreadAttachmentsLoaded(g, _) if g != self.thread_generation => Task::none(),
            Message::ThreadAttachmentsLoaded(_, Ok(attachments)) => {
                self.reading_pane.thread_attachments = attachments;
                Task::none()
            }
            Message::ThreadAttachmentsLoaded(_, Err(e)) => {
                self.status = format!("Attachments error: {e}");
                Task::none()
            }
            Message::SetDateDisplay(display) => {
                self.reading_pane.date_display = display;
                Task::none()
            }
            Message::Compose | Message::Noop => Task::none(),

            // Command system
            Message::KeyEvent(msg) => self.handle_key_event(msg),
            Message::ExecuteCommand(id) => self.handle_execute_command(id),
            Message::ExecuteParameterized(id, args) => {
                self.handle_execute_parameterized(id, args)
            }
            Message::NavigateTo(_target) => {
                // Stub: navigation targets will be wired as sidebar
                // selection and view state are normalized.
                Task::none()
            }
            Message::Escape => {
                // Close settings overlay, deselect thread, etc.
                if self.show_settings {
                    self.show_settings = false;
                    return Task::none();
                }
                // If search has content, clear it and deselect pinned search
                // (but don't dismiss the pinned search).
                if !self.search_query.is_empty() || self.active_pinned_search.is_some() {
                    self.active_pinned_search = None;
                    self.sidebar.active_pinned_search = None;
                    self.editing_pinned_search = None;
                    return self.update(Message::SearchClear);
                }
                Task::none()
            }
            Message::EmailAction(_action) => {
                // Stub: email actions will be wired to core when
                // the action backend is implemented.
                Task::none()
            }
            Message::ComposeAction(_action) => {
                // Stub: compose actions not yet implemented.
                Task::none()
            }
            Message::TaskAction(_action) => {
                // Stub: task actions not yet implemented.
                Task::none()
            }
            Message::SetTheme(theme) => {
                self.settings.theme = theme;
                Task::none()
            }
            Message::ToggleSidebar => {
                // Stub: sidebar toggle not yet implemented.
                Task::none()
            }
            Message::FocusSearch => {
                // Delegate to the real FocusSearchBar handler
                self.update(Message::FocusSearchBar)
            }
            Message::ShowHelp => {
                // Stub: help overlay not yet implemented.
                Task::none()
            }
            Message::SyncCurrentFolder => {
                // Stub: sync not yet implemented.
                Task::none()
            }
            Message::SetReadingPanePosition(_pos) => {
                // Stub: reading pane position not yet implemented.
                Task::none()
            }
            Message::Palette(msg) => self.handle_palette(msg),

            // Search
            Message::SearchQueryChanged(query) => {
                self.search_query = query;
                self.thread_list.search_query.clone_from(&self.search_query);
                if self.search_query.trim().is_empty() {
                    self.search_debounce_deadline = None;
                    // Empty query while in search mode → restore folder view
                    // immediately so the user doesn't see stale search results.
                    if self.thread_list.mode == ui::thread_list::ThreadListMode::Search {
                        return self.restore_folder_view();
                    }
                } else {
                    self.search_debounce_deadline = Some(
                        iced::time::Instant::now()
                            + std::time::Duration::from_millis(150),
                    );
                }
                Task::none()
            }
            Message::SearchExecute => {
                self.search_debounce_deadline = None;
                let query = self.search_query.trim().to_string();
                if query.is_empty() {
                    return self.restore_folder_view();
                }

                // Store pre-search threads on first search from folder mode
                if self.thread_list.mode == ui::thread_list::ThreadListMode::Folder {
                    self.pre_search_threads = Some(self.thread_list.threads.clone());
                }

                self.search_generation += 1;
                let generation = self.search_generation;
                let db = Arc::clone(&self.db);

                Task::perform(
                    async move {
                        let result = execute_search(db, query).await;
                        (generation, result)
                    },
                    |(g, result)| Message::SearchResultsLoaded(g, result),
                )
            }
            Message::SearchResultsLoaded(g, _) if g != self.search_generation => {
                // Stale results — silently drop.
                Task::none()
            }
            Message::SearchResultsLoaded(_, Ok(threads)) => {
                self.thread_list.mode = ui::thread_list::ThreadListMode::Search;
                self.status = format!("{} results", threads.len());

                // Collect thread IDs for pinned search snapshot
                let thread_ids: Vec<(String, String)> = threads
                    .iter()
                    .map(|t| (t.id.clone(), t.account_id.clone()))
                    .collect();
                let query = self.search_query.clone();

                self.thread_list.set_threads(threads);
                self.thread_list.selected_thread = None;

                // Create or update pinned search
                if !query.trim().is_empty() {
                    let db = Arc::clone(&self.db);
                    if let Some(editing_id) = self.editing_pinned_search {
                        return Task::perform(
                            async move {
                                db.update_pinned_search(editing_id, query, thread_ids)
                                    .await
                                    .map(|()| editing_id)
                            },
                            Message::PinnedSearchSaved,
                        );
                    }
                    return Task::perform(
                        async move {
                            db.create_or_update_pinned_search(query, thread_ids).await
                        },
                        Message::PinnedSearchSaved,
                    );
                }
                Task::none()
            }
            Message::SearchResultsLoaded(_, Err(e)) => {
                self.status = format!("Search error: {e}");
                Task::none()
            }
            Message::SearchClear => {
                self.search_query.clear();
                self.thread_list.search_query.clear();
                self.search_debounce_deadline = None;
                self.search_generation += 1; // Invalidate in-flight searches
                self.restore_folder_view()
            }
            Message::FocusSearchBar => {
                iced::widget::operation::focus::<Message>("search-bar".to_string())
            }
            Message::SearchBlur => {
                // Unfocus the search bar without clearing search state.
                Task::none()
            }

            // Pinned searches
            Message::PinnedSearchesLoaded(Ok(searches)) => {
                self.pinned_searches = searches;
                self.sidebar.pinned_searches.clone_from(&self.pinned_searches);

                if !self.expiry_ran {
                    self.expiry_ran = true;
                    let db = Arc::clone(&self.db);
                    // 14 days in seconds
                    return Task::perform(
                        async move { db.expire_stale_pinned_searches(1_209_600).await },
                        Message::PinnedSearchesExpired,
                    );
                }
                Task::none()
            }
            Message::PinnedSearchesLoaded(Err(e)) => {
                self.status = format!("Pinned searches error: {e}");
                Task::none()
            }
            Message::SelectPinnedSearch(id) => {
                self.handle_select_pinned_search(id)
            }
            Message::PinnedSearchThreadIdsLoaded(g, _, _) if g != self.nav_generation => {
                Task::none()
            }
            Message::PinnedSearchThreadIdsLoaded(_, ps_id, Ok(ids)) => {
                // Populate search bar with the pinned search query
                if let Some(ps) = self.pinned_searches.iter().find(|p| p.id == ps_id) {
                    self.search_query.clone_from(&ps.query);
                    self.thread_list.search_query.clone_from(&ps.query);
                }

                let db = Arc::clone(&self.db);
                let load_gen = self.nav_generation;
                Task::perform(
                    async move {
                        let result = db.get_threads_by_ids(ids).await;
                        (load_gen, result)
                    },
                    |(g, result)| Message::PinnedSearchThreadsLoaded(g, result),
                )
            }
            Message::PinnedSearchThreadIdsLoaded(_, _, Err(e)) => {
                self.status = format!("Error loading pinned search: {e}");
                Task::none()
            }
            Message::PinnedSearchThreadsLoaded(g, _) if g != self.nav_generation => {
                Task::none()
            }
            Message::PinnedSearchThreadsLoaded(_, Ok(threads)) => {
                self.thread_list.mode = ui::thread_list::ThreadListMode::Search;
                self.status = format!("{} threads (pinned search)", threads.len());
                self.thread_list.set_threads(threads);
                self.thread_list.selected_thread = None;
                Task::none()
            }
            Message::PinnedSearchThreadsLoaded(_, Err(e)) => {
                self.status = format!("Threads error: {e}");
                Task::none()
            }
            Message::DismissPinnedSearch(id) => {
                let db = Arc::clone(&self.db);
                Task::perform(
                    async move {
                        let result = db.delete_pinned_search(id).await;
                        (id, result)
                    },
                    |(id, result)| Message::PinnedSearchDismissed(id, result),
                )
            }
            Message::PinnedSearchDismissed(id, Ok(())) => {
                self.pinned_searches.retain(|ps| ps.id != id);
                self.sidebar.pinned_searches.retain(|ps| ps.id != id);
                if self.active_pinned_search == Some(id) {
                    self.active_pinned_search = None;
                    self.sidebar.active_pinned_search = None;
                    self.editing_pinned_search = None;
                    // Restore previous folder view
                    return self.restore_folder_view();
                }
                Task::none()
            }
            Message::PinnedSearchDismissed(_, Err(e)) => {
                self.status = format!("Dismiss error: {e}");
                Task::none()
            }
            Message::PinnedSearchSaved(Ok(id)) => {
                self.active_pinned_search = Some(id);
                self.sidebar.active_pinned_search = Some(id);
                self.editing_pinned_search = Some(id);

                let db = Arc::clone(&self.db);
                Task::perform(
                    async move { db.list_pinned_searches().await },
                    Message::PinnedSearchesLoaded,
                )
            }
            Message::PinnedSearchSaved(Err(e)) => {
                self.status = format!("Save pinned search error: {e}");
                Task::none()
            }
            Message::PinnedSearchesExpired(Ok(count)) => {
                if count > 0 {
                    let db = Arc::clone(&self.db);
                    Task::perform(
                        async move { db.list_pinned_searches().await },
                        Message::PinnedSearchesLoaded,
                    )
                } else {
                    Task::none()
                }
            }
            Message::PinnedSearchesExpired(Err(e)) => {
                self.status = format!("Expiry warning: {e}");
                Task::none()
            }

            // Account management
            Message::AddAccount(msg) => self.handle_add_account(msg),
            Message::OpenAddAccount => {
                let used_colors = self.sidebar.accounts.iter()
                    .filter_map(|a| a.account_color.clone())
                    .collect();
                self.add_account_wizard =
                    Some(AddAccountWizard::new_add_account(used_colors, Arc::clone(&self.db)));
                Task::none()
            }
            Message::ReloadSignatures => {
                self.load_signatures_into_settings();
                Task::none()
            }
        }
    }

    fn view(&self) -> Element<'_, Message> {
        // Add-account wizard modal takes precedence
        if let Some(ref wizard) = self.add_account_wizard {
            if self.no_accounts {
                return self.view_first_launch_modal(wizard);
            }
            return self.view_with_add_account_modal(wizard);
        }

        if self.show_settings {
            let settings_view = self.settings.view().map(Message::Settings);
            let status_bar = self.status_bar.view().map(Message::StatusBar);
            return column![
                container(settings_view).height(Length::Fill),
                status_bar,
            ]
            .into();
        }

        let sidebar = container(self.sidebar.view().map(Message::Sidebar))
            .width(self.sidebar_width)
            .height(Length::Fill);

        let divider_sidebar = self.build_divider(Divider::Sidebar);

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

        let right_sidebar = ui::right_sidebar::view::<Message>(self.right_sidebar_open);

        let layout = row![sidebar, divider_sidebar, thread_list, divider_thread, reading_pane, right_sidebar]
            .height(Length::Fill);

        let status_bar = self.status_bar.view().map(Message::StatusBar);
        let full_layout = column![layout, status_bar];

        // Wrap in a mouse_area to track drag movement across the full window
        let main_layout: Element<'_, Message> = if self.dragging.is_some() {
            mouse_area(full_layout)
                .on_move(Message::DividerDragMove)
                .on_release(Message::DividerDragEnd)
                .interaction(iced::mouse::Interaction::ResizingHorizontally)
                .into()
        } else {
            full_layout.into()
        };

        // Palette overlay
        if self.palette.is_open() {
            let backdrop = mouse_area(
                container("")
                    .width(Length::Fill)
                    .height(Length::Fill)
                    .style(ui::theme::ContainerClass::PaletteBackdrop.style()),
            )
            .on_press(Message::Palette(PaletteMessage::Close));

            let palette_widget = ui::palette::palette_card(
                &self.palette,
                &self.registry,
                |q| Message::Palette(PaletteMessage::QueryChanged(q)),
                Message::Palette(PaletteMessage::Confirm),
                |idx| Message::Palette(PaletteMessage::ClickResult(idx)),
                |idx| Message::Palette(PaletteMessage::ClickOption(idx)),
            );

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
        } else {
            main_layout
        }
    }
}

// ── Key event handling ─────────────────────────────────

impl App {
    fn handle_key_event(&mut self, msg: KeyEventMessage) -> Task<Message> {
        match msg {
            KeyEventMessage::KeyPressed {
                key,
                modifiers,
                status,
            } => self.handle_key_pressed(key, modifiers, status),
            KeyEventMessage::PendingChordTimeout => {
                self.pending_chord = None;
                Task::none()
            }
        }
    }

    fn handle_key_pressed(
        &mut self,
        key: iced::keyboard::Key,
        modifiers: iced::keyboard::Modifiers,
        status: iced::event::Status,
    ) -> Task<Message> {
        // 1. If palette is open, route to palette-specific handler
        if self.palette.is_open() {
            return self.handle_palette_key(&key);
        }

        // 2. If a text input or other widget captured the event, skip
        //    (unless it's a modifier-chord like Ctrl+K)
        if status == iced::event::Status::Captured
            && !command_dispatch::has_command_modifier(&modifiers)
        {
            return Task::none();
        }

        // 2. Convert iced key to command-palette Chord
        let Some(chord) = command_dispatch::iced_key_to_chord(&key, &modifiers) else {
            return Task::none();
        };

        // 3. If we're in pending chord state, resolve the sequence
        if let Some(pending) = self.pending_chord.take() {
            if let Some(id) = self.binding_table.resolve_sequence(
                &pending.first, &chord,
            ) {
                if self.is_command_available(id) {
                    return self.update(Message::ExecuteCommand(id));
                }
                return Task::none();
            }
            // Second chord didn't match any sequence — re-process as fresh first chord
            return self.try_resolve_single_chord(chord);
        }

        // 4. Resolve single chord
        self.try_resolve_single_chord(chord)
    }

    /// Check if a command is currently available given app context.
    fn is_command_available(&self, id: CommandId) -> bool {
        let ctx = command_dispatch::build_context(self);
        self.registry.get(id).is_some_and(|desc| (desc.is_available)(&ctx))
    }

    /// When the palette is open, intercept Escape/ArrowUp/ArrowDown/Enter.
    /// All other keys flow to the text_input naturally.
    fn handle_palette_key(&mut self, key: &iced::keyboard::Key) -> Task<Message> {
        match key {
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => {
                self.update(Message::Palette(PaletteMessage::Close))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowDown) => {
                self.update(Message::Palette(PaletteMessage::SelectNext))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::ArrowUp) => {
                self.update(Message::Palette(PaletteMessage::SelectPrev))
            }
            iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter) => {
                self.update(Message::Palette(PaletteMessage::Confirm))
            }
            _ => Task::none(),
        }
    }

    /// Try to resolve a single chord, checking availability before dispatch.
    fn try_resolve_single_chord(&mut self, chord: Chord) -> Task<Message> {
        match self.binding_table.resolve_chord(&chord) {
            ResolveResult::Command(id) => {
                if self.is_command_available(id) {
                    self.update(Message::ExecuteCommand(id))
                } else {
                    Task::none()
                }
            }
            ResolveResult::Pending => {
                self.pending_chord = Some(PendingChord {
                    first: chord,
                    started: iced::time::Instant::now(),
                });
                Task::none()
            }
            ResolveResult::NoMatch => Task::none(),
        }
    }
}

// ── Command execution ──────────────────────────────────

impl App {
    fn handle_execute_command(&mut self, id: CommandId) -> Task<Message> {
        self.registry.usage.record_usage(id);
        match command_dispatch::dispatch_command(id, self) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }

    fn handle_execute_parameterized(
        &mut self,
        id: CommandId,
        args: CommandArgs,
    ) -> Task<Message> {
        self.registry.usage.record_usage(id);
        match command_dispatch::dispatch_parameterized(id, args) {
            Some(msg) => self.update(msg),
            None => Task::none(),
        }
    }
}

// ── Palette handling ───────────────────────────────────

impl App {
    fn handle_palette(&mut self, msg: PaletteMessage) -> Task<Message> {
        match msg {
            PaletteMessage::Open => {
                // Don't open palette when settings overlay is showing —
                // the palette can't render there, creating a hidden-modal state.
                if self.show_settings {
                    return Task::none();
                }
                let ctx = command_dispatch::build_context(self);
                let results = self.registry.query(&ctx, "");
                self.palette.open = true;
                self.palette.query.clear();
                self.palette.results = results;
                self.palette.selected_index = 0;
                self.palette.stage = ui::palette::PaletteStage::CommandSearch;
                iced::widget::operation::focus::<Message>("palette-input".to_string())
            }
            PaletteMessage::Close => {
                // In stage 2, Escape goes back to stage 1 instead of closing.
                if self.palette.is_option_pick() {
                    let ctx = command_dispatch::build_context(self);
                    self.palette.back_to_stage1();
                    self.palette.results = self.registry.query(&ctx, "");
                    return iced::widget::operation::focus::<Message>(
                        "palette-input".to_string(),
                    );
                }
                self.palette.close();
                Task::none()
            }
            PaletteMessage::QueryChanged(query) => {
                if self.palette.is_option_pick() {
                    // Stage 2: filter options with fuzzy search
                    self.palette.option_matches = ratatoskr_command_palette::search_options(
                        &self.palette.option_items,
                        &query,
                    );
                    self.palette.query = query;
                    self.palette.selected_index = 0;
                } else {
                    // Stage 1: query the registry
                    let ctx = command_dispatch::build_context(self);
                    self.palette.results = self.registry.query(&ctx, &query);
                    self.palette.query = query;
                    self.palette.selected_index = 0;
                }
                Task::none()
            }
            PaletteMessage::SelectNext => {
                let len = if self.palette.is_option_pick() {
                    self.palette.option_matches.len()
                } else {
                    self.palette.results.len()
                };
                if len > 0 {
                    self.palette.selected_index = (self.palette.selected_index + 1)
                        .min(len - 1);
                }
                Task::none()
            }
            PaletteMessage::SelectPrev => {
                self.palette.selected_index = self.palette.selected_index.saturating_sub(1);
                Task::none()
            }
            PaletteMessage::Confirm => {
                if self.palette.is_option_pick() {
                    self.palette_confirm_option()
                } else {
                    self.palette_confirm()
                }
            }
            PaletteMessage::ClickResult(idx) => {
                if idx < self.palette.results.len() {
                    self.palette.selected_index = idx;
                    self.palette_confirm()
                } else {
                    Task::none()
                }
            }
            PaletteMessage::ClickOption(idx) => {
                if idx < self.palette.option_matches.len() {
                    self.palette.selected_index = idx;
                    self.palette_confirm_option()
                } else {
                    Task::none()
                }
            }
            PaletteMessage::OptionsLoaded(generation, command_id, result) => {
                self.handle_options_loaded(generation, command_id, result)
            }
        }
    }

    fn palette_confirm(&mut self) -> Task<Message> {
        let Some(result) = self.palette.results.get(self.palette.selected_index) else {
            return Task::none();
        };
        if !result.available {
            return Task::none();
        }
        let id = result.id;
        let input_mode = result.input_mode;

        match input_mode {
            ratatoskr_command_palette::InputMode::Direct => {
                self.palette.close();
                self.update(Message::ExecuteCommand(id))
            }
            ratatoskr_command_palette::InputMode::Parameterized { schema } => {
                // Get the param label for the placeholder text
                let param_label = schema
                    .param_at(0)
                    .map(|p| match p {
                        ratatoskr_command_palette::ParamDef::ListPicker { label } => label,
                        ratatoskr_command_palette::ParamDef::DateTime { label } => label,
                        ratatoskr_command_palette::ParamDef::Enum { label, .. } => label,
                        ratatoskr_command_palette::ParamDef::Text { label, .. } => label,
                    })
                    .unwrap_or("option");

                // Skip DateTime commands for now (snooze picker is complex)
                if matches!(
                    schema.param_at(0),
                    Some(ratatoskr_command_palette::ParamDef::DateTime { .. })
                ) {
                    self.palette.close();
                    return Task::none();
                }

                // Transition to stage 2
                self.palette.stage = ui::palette::PaletteStage::OptionPick;
                self.palette.query.clear();
                self.palette.selected_index = 0;
                self.palette.stage2_command_id = Some(id);
                self.palette.stage2_label = param_label.to_string();
                self.palette.option_items.clear();
                self.palette.option_matches.clear();
                self.palette.options_loading = true;
                self.palette.option_load_generation += 1;
                let generation = self.palette.option_load_generation;

                // Dispatch async resolver call
                let resolver = Arc::clone(&self.resolver);
                let ctx = command_dispatch::build_context(self);
                let focus_task = iced::widget::operation::focus::<Message>(
                    "palette-input".to_string(),
                );
                let load_task = Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            resolver.get_options(id, 0, &[], &ctx)
                        })
                        .await
                        .unwrap_or_else(|e| Err(format!("spawn_blocking: {e}")))
                    },
                    move |result| {
                        Message::Palette(PaletteMessage::OptionsLoaded(
                            generation, id, result,
                        ))
                    },
                );
                Task::batch([focus_task, load_task])
            }
        }
    }

    fn handle_options_loaded(
        &mut self,
        generation: u64,
        command_id: CommandId,
        result: Result<Vec<ratatoskr_command_palette::OptionItem>, String>,
    ) -> Task<Message> {
        // Discard stale results
        if generation < self.palette.option_load_generation {
            return Task::none();
        }
        // Verify we're still in the right stage for this command
        if self.palette.stage2_command_id != Some(command_id) {
            return Task::none();
        }

        self.palette.options_loading = false;

        match result {
            Ok(items) => {
                self.palette.option_matches =
                    ratatoskr_command_palette::search_options(&items, &self.palette.query);
                self.palette.option_items = items;
                self.palette.selected_index = 0;
            }
            Err(msg) => {
                // On error, show empty list. The placeholder text
                // changes from "Loading..." to the search prompt,
                // making it clear no options were found.
                self.palette.option_items.clear();
                self.palette.option_matches.clear();
                self.status = format!("Palette error: {msg}");
            }
        }
        Task::none()
    }

    fn palette_confirm_option(&mut self) -> Task<Message> {
        let Some(option_match) = self
            .palette
            .option_matches
            .get(self.palette.selected_index)
        else {
            return Task::none();
        };
        if option_match.item.disabled {
            return Task::none();
        }

        let Some(command_id) = self.palette.stage2_command_id else {
            return Task::none();
        };

        let Some(args) = build_command_args(command_id, &option_match.item) else {
            return Task::none();
        };

        self.palette.close();
        self.update(Message::ExecuteParameterized(command_id, args))
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
                self.thread_list.selected_thread = None;
                self.nav_generation += 1;
                self.thread_generation += 1;
                self.update_thread_list_context_from_sidebar();
                self.load_navigation_and_threads()
            }
            SidebarEvent::AllAccountsSelected => {
                self.clear_search_state();
                self.clear_pinned_search_context();
                self.thread_list.selected_thread = None;
                self.nav_generation += 1;
                self.thread_generation += 1;
                self.update_thread_list_context_from_sidebar();
                self.load_navigation_and_threads()
            }
            SidebarEvent::CycleAccount => Task::none(),
            SidebarEvent::LabelSelected(label_id) => {
                self.clear_search_state();
                self.clear_pinned_search_context();
                self.update_thread_list_context_from_sidebar();
                self.handle_label_selected(label_id)
            }
            SidebarEvent::Compose => Task::none(),
            SidebarEvent::ToggleSettings => {
                self.show_settings = !self.show_settings;
                Task::none()
            }
            SidebarEvent::PinnedSearchSelected(id) => {
                self.update(Message::SelectPinnedSearch(id))
            }
            SidebarEvent::PinnedSearchDismissed(id) => {
                self.update(Message::DismissPinnedSearch(id))
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
            ThreadListEvent::ThreadSelected(idx) => self.handle_select_thread(idx),
            ThreadListEvent::SearchQueryChanged(query) => {
                self.update(Message::SearchQueryChanged(query))
            }
            ThreadListEvent::SearchExecute => {
                self.update(Message::SearchExecute)
            }
        }
    }

    fn handle_reading_pane(&mut self, msg: ReadingPaneMessage) -> Task<Message> {
        let (task, _event) = self.reading_pane.update(msg);
        task.map(Message::ReadingPane)
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
                // TODO: Open re-authentication flow for this account.
                // This will be wired when the accounts UI is implemented.
                let _ = account_id;
                Task::none()
            }
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
            SettingsEvent::DateDisplayChanged(display) => {
                self.reading_pane.date_display = display;
                Task::none()
            }
            SettingsEvent::OpenAddAccountWizard => {
                let used_colors = self.sidebar.accounts.iter()
                    .filter_map(|a| a.account_color.clone())
                    .collect();
                self.add_account_wizard =
                    Some(AddAccountWizard::new_add_account(used_colors, Arc::clone(&self.db)));
                self.show_settings = false;
                Task::none()
            }
            SettingsEvent::SaveSignature(req) => {
                let db = Arc::clone(&self.db);
                Task::perform(
                    async move {
                        db.with_write_conn(move |conn| {
                            if let Some(ref id) = req.id {
                                conn.execute(
                                    "UPDATE signatures SET name = ?1, body_html = ?2, \
                                     is_default = ?3, is_reply_default = ?4 WHERE id = ?5",
                                    rusqlite::params![
                                        req.name,
                                        req.body_html,
                                        if req.is_default { 1 } else { 0 },
                                        if req.is_reply_default { 1 } else { 0 },
                                        id,
                                    ],
                                )
                                .map_err(|e| e.to_string())?;
                            } else {
                                let id = uuid::Uuid::new_v4().to_string();
                                conn.execute(
                                    "INSERT INTO signatures (id, account_id, name, body_html, \
                                     is_default, is_reply_default) \
                                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                                    rusqlite::params![
                                        id,
                                        req.account_id,
                                        req.name,
                                        req.body_html,
                                        if req.is_default { 1 } else { 0 },
                                        if req.is_reply_default { 1 } else { 0 },
                                    ],
                                )
                                .map_err(|e| e.to_string())?;
                            }
                            Ok(())
                        })
                        .await
                    },
                    |result| {
                        if let Err(e) = result {
                            eprintln!("Failed to save signature: {e}");
                        }
                        Message::ReloadSignatures
                    },
                )
            }
            SettingsEvent::DeleteSignature(sig_id) => {
                let db = Arc::clone(&self.db);
                Task::perform(
                    async move {
                        db.with_write_conn(move |conn| {
                            conn.execute(
                                "DELETE FROM signatures WHERE id = ?1",
                                rusqlite::params![sig_id],
                            )
                            .map_err(|e| e.to_string())?;
                            Ok(())
                        })
                        .await
                    },
                    |result| {
                        if let Err(e) = result {
                            eprintln!("Failed to delete signature: {e}");
                        }
                        Message::ReloadSignatures
                    },
                )
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

    fn handle_add_account_event(&mut self, event: AddAccountEvent) -> Task<Message> {
        match event {
            AddAccountEvent::AccountAdded(_account_id) => {
                self.add_account_wizard = None;
                self.no_accounts = false;
                // Reload accounts list
                let db = Arc::clone(&self.db);
                self.nav_generation += 1;
                let load_gen = self.nav_generation;
                Task::perform(
                    async move { (load_gen, load_accounts(db).await) },
                    |(g, result)| Message::AccountsLoaded(g, result),
                )
            }
            AddAccountEvent::Cancelled => {
                // Only allow cancel if there are existing accounts
                if !self.no_accounts {
                    self.add_account_wizard = None;
                }
                Task::none()
            }
        }
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

    fn load_threads_for_current_view(&self) -> Task<Message> {
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
        self.reading_pane.set_thread(thread);
        self.thread_generation += 1;
        if let Some(thread) = thread {
            let db = Arc::clone(&self.db);
            let account_id = thread.account_id.clone();
            let thread_id = thread.id.clone();
            let db2 = Arc::clone(&self.db);
            let account_id2 = account_id.clone();
            let thread_id2 = thread_id.clone();
            let load_gen = self.thread_generation;
            return Task::batch([
                Task::perform(
                    async move {
                        let r = db.get_thread_messages(account_id, thread_id).await;
                        (load_gen, r)
                    },
                    |(g, result)| Message::ThreadMessagesLoaded(g, result),
                ),
                Task::perform(
                    async move {
                        let r = db2.get_thread_attachments(account_id2, thread_id2).await;
                        (load_gen, r)
                    },
                    |(g, result)| Message::ThreadAttachmentsLoaded(g, result),
                ),
            ]);
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
        // Sync managed accounts for settings tab
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
            })
            .collect();
        // Load signatures for the settings UI
        self.load_signatures_into_settings();
        self.sidebar.selected_account = Some(0);
        self.status = format!("Loaded {} accounts", self.sidebar.accounts.len());
        self.load_navigation_and_threads()
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

        // Event blocker between base and modal
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

    /// Build the main three-panel layout without modal overlays.
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
        let right_sidebar =
            ui::right_sidebar::view::<Message>(self.right_sidebar_open);
        let layout = row![
            sidebar,
            divider_sidebar,
            thread_list,
            divider_thread,
            reading_pane,
            right_sidebar
        ]
        .height(Length::Fill);
        let status_bar = self.status_bar.view().map(Message::StatusBar);
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

    fn handle_window_close(&mut self, id: iced::window::Id) -> Task<Message> {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        self.window.sidebar_width = self.sidebar_width;
        self.window.thread_list_width = self.thread_list_width;
        self.window.right_sidebar_open = self.right_sidebar_open;
        self.window.save(data_dir);
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

    /// Handle selecting a pinned search from the sidebar.
    fn handle_select_pinned_search(&mut self, id: i64) -> Task<Message> {
        // Save pre-search threads on first activation from folder mode
        if self.active_pinned_search.is_none()
            && self.thread_list.mode == ui::thread_list::ThreadListMode::Folder
        {
            self.pre_search_threads = Some(self.thread_list.threads.clone());
        }

        self.active_pinned_search = Some(id);
        self.sidebar.active_pinned_search = Some(id);
        self.editing_pinned_search = Some(id);
        self.sidebar.selected_label = None;

        self.nav_generation += 1;
        self.thread_generation += 1;
        self.thread_list.selected_thread = None;

        // Update thread list context
        if let Some(ps) = self.pinned_searches.iter().find(|p| p.id == id) {
            let label = truncate_query(&ps.query, 30);
            self.thread_list
                .set_context(format!("Search: {label}"), "All Accounts".to_string());
        }

        let db = Arc::clone(&self.db);
        let load_gen = self.nav_generation;
        Task::perform(
            async move {
                let ids = db.get_pinned_search_thread_ids(id).await;
                (load_gen, id, ids)
            },
            |(g, id, result)| Message::PinnedSearchThreadIdsLoaded(g, id, result),
        )
    }

    /// Clear pinned search context on navigate-away.
    fn clear_pinned_search_context(&mut self) {
        self.active_pinned_search = None;
        self.sidebar.active_pinned_search = None;
        self.editing_pinned_search = None;
    }

    /// Clear all search-related state without restoring pre-search threads.
    /// Used when navigating via sidebar (new threads will be loaded).
    fn clear_search_state(&mut self) {
        self.search_query.clear();
        self.thread_list.search_query.clear();
        self.search_debounce_deadline = None;
        self.search_generation += 1;
        self.thread_list.mode = ui::thread_list::ThreadListMode::Folder;
        self.pre_search_threads = None;
    }

    /// Restore the thread list to folder view after clearing search.
    fn restore_folder_view(&mut self) -> Task<Message> {
        self.thread_list.mode = ui::thread_list::ThreadListMode::Folder;
        self.search_query.clear();
        self.thread_list.search_query.clear();
        // Clear selection to avoid stale state: the selected index from
        // search results may point at a different thread in the restored
        // folder list. Also clear the reading pane's messages.
        self.thread_list.selected_thread = None;
        self.reading_pane.thread_messages.clear();
        self.reading_pane.thread_attachments.clear();
        self.reading_pane.message_expanded.clear();
        if let Some(threads) = self.pre_search_threads.take() {
            self.status = format!("{} threads", threads.len());
            self.thread_list.set_threads(threads);
        }
        Task::none()
    }

    /// Load signatures from DB into settings.signatures synchronously.
    /// Called after accounts load so the settings UI has signature data.
    fn load_signatures_into_settings(&mut self) {
        let result = self.db.with_conn_sync(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, name, body_html, is_default,
                            is_reply_default, sort_order
                     FROM signatures ORDER BY account_id, sort_order, name",
                )
                .map_err(|e| e.to_string())?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(ui::settings::SignatureEntry {
                        id: row.get("id")?,
                        account_id: row.get("account_id")?,
                        name: row.get("name")?,
                        body_html: row.get::<_, Option<String>>("body_html")?
                            .unwrap_or_default(),
                        body_text: None,
                        is_default: row.get::<_, i64>("is_default").unwrap_or(0) != 0,
                        is_reply_default: row.get::<_, i64>("is_reply_default")
                            .unwrap_or(0)
                            != 0,
                    })
                })
                .map_err(|e| e.to_string())?;
            rows.collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())
        });
        match result {
            Ok(sigs) => self.settings.signatures = sigs,
            Err(e) => eprintln!("Failed to load signatures: {e}"),
        }
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

/// Truncates a query string for display, adding ellipsis if needed.
fn truncate_query(query: &str, max_chars: usize) -> String {
    if query.len() <= max_chars {
        query.to_string()
    } else {
        format!("{}...", &query[..query.floor_char_boundary(max_chars)])
    }
}

async fn load_accounts(db: Arc<Db>) -> Result<Vec<db::Account>, String> {
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
        Ok(db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect())
    })
    .await
}

/// Build typed `CommandArgs` from the selected option item.
///
/// Maps each parameterized `CommandId` to its corresponding `CommandArgs`
/// variant, extracting the item's ID (and for cross-account commands,
/// splitting the `"account_id:label_id"` encoding).
fn build_command_args(command_id: CommandId, item: &OptionItem) -> Option<CommandArgs> {
    match command_id {
        CommandId::EmailMoveToFolder => Some(CommandArgs::MoveToFolder {
            folder_id: item.id.clone(),
        }),
        CommandId::EmailAddLabel => Some(CommandArgs::AddLabel {
            label_id: item.id.clone(),
        }),
        CommandId::EmailRemoveLabel => Some(CommandArgs::RemoveLabel {
            label_id: item.id.clone(),
        }),
        CommandId::EmailSnooze => {
            // DateTime picker returns a stringified unix timestamp
            item.id
                .parse::<i64>()
                .ok()
                .map(|ts| CommandArgs::Snooze { until: ts })
        }
        _ => None,
    }
}

/// Execute search off the main thread via spawn_blocking.
///
/// TODO: Wire real SearchState once it is initialized at app startup.
/// Currently stubs the search by querying the DB for threads matching
/// the query as a substring in subject/snippet, since SearchState
/// requires a Tantivy index that may not be available yet.
async fn execute_search(
    db: Arc<Db>,
    query: String,
) -> Result<Vec<Thread>, String> {
    db.with_conn(move |conn| {
        // Stub: use the unified search pipeline if SearchState is available.
        // For now, do a simple SQL LIKE search as a placeholder so the full
        // message flow, debounce, and generational tracking are exercised.
        let pattern = format!("%{query}%");
        let mut stmt = conn
            .prepare(
                "SELECT t.id, t.account_id, t.subject, t.snippet,
                        t.last_message_at, t.message_count,
                        t.is_read, t.is_starred, t.has_attachments,
                        t.from_name, t.from_address
                 FROM threads t
                 WHERE t.subject LIKE ?1 OR t.snippet LIKE ?1
                 ORDER BY t.last_message_at DESC
                 LIMIT 200",
            )
            .map_err(|e| format!("prepare search: {e}"))?;
        let rows = stmt
            .query_map([&pattern], |row| {
                Ok(Thread {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    subject: row.get(2)?,
                    snippet: row.get(3)?,
                    last_message_at: row.get::<_, Option<String>>(4)?
                        .and_then(|s| s.parse().ok()),
                    message_count: row.get(5)?,
                    is_read: row.get(6)?,
                    is_starred: row.get(7)?,
                    has_attachments: row.get(8)?,
                    from_name: row.get(9)?,
                    from_address: row.get(10)?,
                })
            })
            .map_err(|e| format!("search query: {e}"))?;
        let mut threads = Vec::new();
        for row in rows {
            threads.push(row.map_err(|e| format!("search row: {e}"))?);
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
        last_message_at: t.last_message_at.and_then(|s| s.parse().ok()),
        message_count: t.message_count,
        is_read: t.is_read,
        is_starred: t.is_starred,
        has_attachments: t.has_attachments,
        from_name: t.from_name,
        from_address: t.from_address,
    }
}
