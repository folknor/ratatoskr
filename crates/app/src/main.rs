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
    ComposeAction, EmailAction, KeyEventMessage, NavigationTarget, PaletteMessage,
    ReadingPanePosition, TaskAction,
};
use component::Component;
use db::{Db, Thread};
use iced::widget::{column, container, mouse_area, row, stack};
use iced::{Element, Length, Point, Size, Task, Theme};
use pop_out::{PopOutMessage, PopOutWindow};
use pop_out::compose::ComposeMode;
use ui::palette::PaletteState;
use ratatoskr_command_palette::{
    BindingTable, Chord, CommandId, CommandRegistry,
    FocusedRegion, current_platform,
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
use ui::calendar::{
    CalendarMessage, CalendarOverlay, CalendarState, CalendarView,
};
use ui::reading_pane::{ReadingPane, ReadingPaneEvent, ReadingPaneMessage};
use ui::settings::{Settings, SettingsEvent, SettingsMessage};
use ui::sidebar::{Sidebar, SidebarEvent, SidebarMessage};
use ui::status_bar::{StatusBar, StatusBarEvent, StatusBarMessage};
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
    SearchQueryChanged(String),
    SearchExecute,
    SearchResultsLoaded(u64, Result<Vec<Thread>, String>),
    SearchClear,
    FocusSearchBar,
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

    // Calendar
    Calendar(CalendarMessage),
    ToggleAppMode,
    SetCalendarView(CalendarView),
    CalendarToday,

    // Account management
    AddAccount(AddAccountMessage),
    OpenAddAccount,
    ReloadSignatures,

    // Pop-out windows
    PopOut(iced::window::Id, PopOutMessage),
    OpenMessageView(usize),
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
    nav_generation: u64,
    thread_generation: u64,

    // Command palette infrastructure
    registry: CommandRegistry,
    binding_table: BindingTable,
    focused_region: Option<FocusedRegion>,
    is_online: bool,
    composer_is_open: bool,
    pending_chord: Option<PendingChord>,
    palette: PaletteState,
    resolver: Arc<command_resolver::AppInputResolver>,

    // Search state
    search_generation: u64,
    search_query: String,
    search_debounce_deadline: Option<iced::time::Instant>,
    pre_search_threads: Option<Vec<Thread>>,

    // Pinned searches
    pinned_searches: Vec<db::PinnedSearch>,
    active_pinned_search: Option<i64>,
    editing_pinned_search: Option<i64>,
    expiry_ran: bool,

    no_accounts: bool,
    add_account_wizard: Option<AddAccountWizard>,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let db = Arc::clone(DB.get().expect("DB not initialized"));
        let db_ref = Arc::clone(&db);
        let db_ref2 = Arc::clone(&db);
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let window = window_state::WindowState::load(data_dir);

        let (main_window_id, open_task) =
            iced::window::open(window.to_window_settings());

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
            app_mode: AppMode::Mail,
            calendar: CalendarState::new(),
            sidebar_width: window.sidebar_width,
            thread_list_width: window.thread_list_width,
            dragging: None,
            hovered_divider: None,
            right_sidebar_open: window.right_sidebar_open,
            show_settings: false,
            window,
            main_window_id,
            pop_out_windows: HashMap::new(),
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
            open_task.discard(),
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
                if let iced::Event::Keyboard(
                    iced::keyboard::Event::KeyPressed { key, modifiers, .. }
                ) = &event {
                    Some(Message::KeyEvent(KeyEventMessage::KeyPressed {
                        key: key.clone(),
                        modifiers: *modifiers,
                        status,
                        window_id: id,
                    }))
                } else {
                    None
                }
            }),
            self.sidebar.subscription().map(Message::Sidebar),
            self.thread_list.subscription().map(Message::ThreadList),
            self.reading_pane.subscription().map(Message::ReadingPane),
            self.settings.subscription().map(Message::Settings),
            self.status_bar.subscription().map(Message::StatusBar),
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
                    .map(move |_| {
                        if iced::time::Instant::now() >= deadline {
                            Message::SearchExecute
                        } else {
                            Message::Noop
                        }
                    }),
            );
        }

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
                        None => {}
                    }
                }
                Task::none()
            }
            Message::WindowMoved(id, point) => {
                if id == self.main_window_id {
                    self.window.set_position(point);
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
            Message::NavigateTo(_target) => Task::none(),
            Message::Escape => {
                if !matches!(self.calendar.overlay, CalendarOverlay::None) {
                    self.calendar.overlay = CalendarOverlay::None;
                    return Task::none();
                }
                if self.show_settings {
                    self.show_settings = false;
                    return Task::none();
                }
                if !self.search_query.is_empty() || self.active_pinned_search.is_some() {
                    self.active_pinned_search = None;
                    self.sidebar.active_pinned_search = None;
                    self.editing_pinned_search = None;
                    return self.update(Message::SearchClear);
                }
                Task::none()
            }
            Message::EmailAction(_action) => Task::none(),
            Message::ComposeAction(action) => self.handle_compose_action(action),
            Message::TaskAction(_action) => Task::none(),
            Message::SetTheme(theme) => {
                self.settings.theme = theme;
                Task::none()
            }
            Message::ToggleSidebar => Task::none(),
            Message::FocusSearch => self.update(Message::FocusSearchBar),
            Message::ShowHelp => Task::none(),
            Message::SyncCurrentFolder => Task::none(),
            Message::SetReadingPanePosition(_pos) => Task::none(),
            Message::Palette(msg) => self.handle_palette(msg),

            // Search — delegated to handlers/search.rs
            Message::SearchQueryChanged(query) => self.handle_search_query_changed(query),
            Message::SearchExecute => self.handle_search_execute(),
            Message::SearchResultsLoaded(g, _) if g != self.search_generation => Task::none(),
            Message::SearchResultsLoaded(_, result) => self.handle_search_results(result),
            Message::SearchClear => self.handle_search_clear(),
            Message::FocusSearchBar => self.handle_focus_search_bar(),
            Message::SearchBlur => Task::none(),

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

            // Calendar — delegated to handlers/calendar.rs
            Message::Calendar(cal_msg) => self.handle_calendar(cal_msg),
            Message::ToggleAppMode => {
                self.app_mode = match self.app_mode {
                    AppMode::Mail => AppMode::Calendar,
                    AppMode::Calendar => AppMode::Mail,
                };
                self.sidebar.in_calendar_mode = self.app_mode == AppMode::Calendar;
                if self.app_mode == AppMode::Calendar {
                    return self.reload_calendar_events();
                }
                Task::none()
            }
            Message::SetCalendarView(view) => {
                if self.app_mode != AppMode::Calendar {
                    self.app_mode = AppMode::Calendar;
                    self.sidebar.in_calendar_mode = true;
                }
                self.update(Message::Calendar(CalendarMessage::SetView(view)))
            }
            Message::CalendarToday => {
                self.update(Message::Calendar(CalendarMessage::Today))
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

            // Pop-out windows — delegated to handlers/pop_out.rs
            Message::PopOut(window_id, pop_out_msg) => {
                self.handle_pop_out_message(window_id, pop_out_msg)
            }
            Message::OpenMessageView(message_index) => {
                self.open_message_view_window(message_index)
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

        let layout = match self.app_mode {
            AppMode::Calendar => {
                let calendar_view = ui::calendar::calendar_layout(&self.calendar)
                    .map(Message::Calendar);
                row![sidebar, divider_sidebar, calendar_view]
                    .height(Length::Fill)
            }
            AppMode::Mail => {
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

                row![sidebar, divider_sidebar, thread_list, divider_thread, reading_pane, right_sidebar]
                    .height(Length::Fill)
            }
        };

        let status_bar = self.status_bar.view().map(Message::StatusBar);
        let full_layout = column![layout, status_bar];

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
            SidebarEvent::ModeToggled => {
                self.update(Message::ToggleAppMode)
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
        let (task, event) = self.reading_pane.update(msg);
        let mut tasks = vec![task.map(Message::ReadingPane)];
        if let Some(evt) = event {
            tasks.push(self.handle_reading_pane_event(evt));
        }
        Task::batch(tasks)
    }

    fn handle_reading_pane_event(&mut self, event: ReadingPaneEvent) -> Task<Message> {
        match event {
            ReadingPaneEvent::AttachmentCollapseChanged { .. } => Task::none(),
            ReadingPaneEvent::OpenMessagePopOut { message_index } => {
                self.open_message_view_window(message_index)
            }
        }
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
            SettingsEvent::OpenAddAccountWizard => self.handle_open_add_account_wizard(),
            SettingsEvent::SaveSignature(req) => self.handle_save_signature(req),
            SettingsEvent::DeleteSignature(id) => self.handle_delete_signature(id),
            SettingsEvent::LoadContacts(filter) => self.handle_load_contacts(filter),
            SettingsEvent::LoadGroups(filter) => self.handle_load_groups(filter),
            SettingsEvent::SaveContact(entry) => self.handle_save_contact(entry),
            SettingsEvent::DeleteContact(id) => self.handle_delete_contact(id),
            SettingsEvent::SaveGroup(group, members) => self.handle_save_group(group, members),
            SettingsEvent::DeleteGroup(id) => self.handle_delete_group(id),
            SettingsEvent::LoadGroupMembers(group_id) => self.handle_load_group_members(group_id),
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
        if id == self.main_window_id {
            let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
            self.window.sidebar_width = self.sidebar_width;
            self.window.thread_list_width = self.thread_list_width;
            self.window.right_sidebar_open = self.right_sidebar_open;
            self.window.save(data_dir);
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
        Ok(db_threads
            .into_iter()
            .map(db_thread_to_app_thread)
            .collect())
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
