mod appearance;
mod command_dispatch;
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
use db::{Db, Label, Thread};
use iced::widget::{container, mouse_area, row};
use iced::{Element, Length, Point, Size, Task, Theme};
use ratatoskr_command_palette::{
    BindingTable, Chord, CommandArgs, CommandId, CommandRegistry, FocusedRegion, ResolveResult,
    current_platform,
};
use ui::layout::{RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH, SIDEBAR_MIN_WIDTH, THREAD_LIST_MIN_WIDTH};
use ui::reading_pane::{ReadingPane, ReadingPaneMessage};
use ui::settings::{Settings, SettingsEvent, SettingsMessage};
use ui::sidebar::{Sidebar, SidebarEvent, SidebarMessage};
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

    // Existing data loading
    AccountsLoaded(u64, Result<Vec<db::Account>, String>),
    LabelsLoaded(u64, Result<Vec<Label>, String>),
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
}

struct App {
    db: Arc<Db>,
    sidebar: Sidebar,
    thread_list: ThreadList,
    reading_pane: ReadingPane,
    settings: Settings,
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
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let db = Arc::clone(DB.get().expect("DB not initialized"));
        let db_ref = Arc::clone(&db);
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let window = window_state::WindowState::load(data_dir);

        let registry = CommandRegistry::new();
        let binding_table = BindingTable::new(&registry, current_platform());

        let app = Self {
            db,
            sidebar: Sidebar::new(),
            thread_list: ThreadList::new(),
            reading_pane: ReadingPane::new(),
            settings: Settings::with_scale(
                *DEFAULT_SCALE.get().unwrap_or(&1.0),
            ),
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
        };
        let load_gen = app.nav_generation;
        (app, Task::perform(
            async move { (load_gen, load_accounts(db_ref).await) },
            |(g, result)| Message::AccountsLoaded(g, result),
        ))
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
        ];

        // Pending chord timeout
        if self.pending_chord.is_some() {
            subs.push(
                iced::time::every(CHORD_TIMEOUT)
                    .map(|_| Message::KeyEvent(KeyEventMessage::PendingChordTimeout)),
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
            Message::LabelsLoaded(g, _) if g != self.nav_generation => Task::none(),
            Message::LabelsLoaded(_, Ok(labels)) => {
                self.sidebar.labels = labels;
                Task::none()
            }
            Message::LabelsLoaded(_, Err(e)) => {
                self.status = format!("Labels error: {e}");
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
                // Stub: search focus not yet implemented.
                Task::none()
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
            Message::Palette(PaletteMessage::Open) => {
                // Stub: palette UI is Slice 6b.
                Task::none()
            }
            Message::Palette(PaletteMessage::Close) => Task::none(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.show_settings {
            return self.settings.view().map(Message::Settings);
        }

        let sidebar = container(self.sidebar.view().map(Message::Sidebar))
            .width(self.sidebar_width)
            .height(Length::Fill);

        let divider_sidebar = self.build_divider(Divider::Sidebar);

        let thread_list = container(self.thread_list.view().map(Message::ThreadList))
            .width(self.thread_list_width)
            .height(Length::Fill);

        let divider_thread = self.build_divider(Divider::ThreadList);

        let reading_pane = container(self.reading_pane.view().map(Message::ReadingPane))
            .width(Length::Fill)
            .height(Length::Fill);

        let right_sidebar = ui::right_sidebar::view::<Message>(self.right_sidebar_open);

        let layout = row![sidebar, divider_sidebar, thread_list, divider_thread, reading_pane, right_sidebar]
            .height(Length::Fill);

        // Wrap in a mouse_area to track drag movement across the full window
        if self.dragging.is_some() {
            mouse_area(layout)
                .on_move(Message::DividerDragMove)
                .on_release(Message::DividerDragEnd)
                .interaction(iced::mouse::Interaction::ResizingHorizontally)
                .into()
        } else {
            layout.into()
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
        // 1. If a text input or other widget captured the event, skip
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
                return self.update(Message::ExecuteCommand(id));
            }
            // Second chord didn't match any sequence -- discard
            return Task::none();
        }

        // 4. Resolve single chord
        match self.binding_table.resolve_chord(&chord) {
            ResolveResult::Command(id) => self.update(Message::ExecuteCommand(id)),
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
            SidebarEvent::AccountSelected(idx) => {
                self.thread_list.selected_thread = None;
                self.nav_generation += 1;
                self.thread_generation += 1;
                self.update_thread_list_context_from_sidebar();
                if let Some(account) = self.sidebar.accounts.get(idx) {
                    let id = account.id.clone();
                    return self.load_labels_and_threads(&id);
                }
                Task::none()
            }
            SidebarEvent::AllAccountsSelected => {
                self.thread_list.selected_thread = None;
                self.thread_list.threads.clear();
                self.sidebar.labels.clear();
                self.update_thread_list_context_from_sidebar();
                Task::none()
            }
            SidebarEvent::CycleAccount => Task::none(),
            SidebarEvent::LabelSelected(label_id) => {
                self.update_thread_list_context_from_sidebar();
                self.handle_label_selected(label_id)
            }
            SidebarEvent::Compose => Task::none(),
            SidebarEvent::ToggleSettings => {
                self.show_settings = !self.show_settings;
                Task::none()
            }
        }
    }

    fn handle_label_selected(&mut self, label_id: Option<String>) -> Task<Message> {
        self.thread_list.selected_thread = None;
        self.nav_generation += 1;
        self.thread_generation += 1;
        if let Some(idx) = self.sidebar.selected_account
            && let Some(account) = self.sidebar.accounts.get(idx)
        {
            let db = Arc::clone(&self.db);
            let id = account.id.clone();
            let load_gen = self.nav_generation;
            return Task::perform(
                async move {
                    let r = load_threads(db, id, label_id).await;
                    (load_gen, r)
                },
                |(g, result)| Message::ThreadsLoaded(g, result),
            );
        }
        Task::none()
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
        }
    }

    fn handle_reading_pane(&mut self, msg: ReadingPaneMessage) -> Task<Message> {
        let (task, _event) = self.reading_pane.update(msg);
        task.map(Message::ReadingPane)
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
        }
    }
}

// ── Helper methods ─────────────────────────────────────

impl App {
    fn load_labels_and_threads(&mut self, account_id: &str) -> Task<Message> {
        self.nav_generation += 1;
        let db = Arc::clone(&self.db);
        let db2 = Arc::clone(&self.db);
        let id = account_id.to_string();
        let id2 = id.clone();
        let load_gen = self.nav_generation;
        Task::batch([
            Task::perform(
                async move {
                    let r = load_labels(db, id).await;
                    (load_gen, r)
                },
                |(g, result)| Message::LabelsLoaded(g, result),
            ),
            Task::perform(
                async move {
                    let r = load_threads(db2, id2, None).await;
                    (load_gen, r)
                },
                |(g, result)| Message::ThreadsLoaded(g, result),
            ),
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
        if !self.sidebar.accounts.is_empty() {
            self.sidebar.selected_account = Some(0);
            let id = self.sidebar.accounts[0].id.clone();
            self.status = format!("Loaded {} accounts", self.sidebar.accounts.len());
            return self.load_labels_and_threads(&id);
        }
        self.status = "No accounts found".to_string();
        Task::none()
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

    fn update_thread_list_context_from_sidebar(&mut self) {
        let folder_name = self
            .sidebar.selected_label
            .as_ref()
            .and_then(|lid| {
                self.sidebar.labels.iter()
                    .find(|l| l.id == *lid)
                    .map(|l| l.name.clone())
            })
            .unwrap_or_else(|| "Inbox".to_string());
        let scope_name = self
            .sidebar.selected_account
            .and_then(|idx| self.sidebar.accounts.get(idx))
            .and_then(|a| a.display_name.as_deref().or(Some(a.email.as_str())))
            .unwrap_or("All")
            .to_string();
        self.thread_list.set_context(folder_name, scope_name);
    }
}

async fn load_accounts(db: Arc<Db>) -> Result<Vec<db::Account>, String> {
    db.get_accounts().await
}

async fn load_labels(
    db: Arc<Db>,
    account_id: String,
) -> Result<Vec<Label>, String> {
    db.get_labels(account_id).await
}

async fn load_threads(
    db: Arc<Db>,
    account_id: String,
    label_id: Option<String>,
) -> Result<Vec<Thread>, String> {
    db.get_threads(account_id, label_id, 1000).await
}
