mod appearance;
mod component;
mod db;
mod display;
mod font;
mod icon;
mod ui;
mod window_state;

use component::Component;
use db::{DateDisplay, Db, Label, Thread, ThreadAttachment, ThreadMessage};
use iced::widget::{container, mouse_area, row};
use iced::{Element, Length, Point, Size, Task, Theme};
use ui::layout::{RIGHT_SIDEBAR_AUTO_COLLAPSE_WIDTH, SIDEBAR_MIN_WIDTH, THREAD_LIST_MIN_WIDTH};
use ui::sidebar::{Sidebar, SidebarEvent, SidebarMessage};
use std::path::PathBuf;
use std::sync::Arc;

static DB: std::sync::OnceLock<Arc<Db>> = std::sync::OnceLock::new();
static APP_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
static DEFAULT_SCALE: std::sync::OnceLock<f32> = std::sync::OnceLock::new();

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

#[derive(Debug, Clone)]
pub enum Message {
    Sidebar(SidebarMessage),
    AccountsLoaded(u64, Result<Vec<db::Account>, String>),
    LabelsLoaded(u64, Result<Vec<Label>, String>),
    ThreadsLoaded(u64, Result<Vec<Thread>, String>),
    SelectThread(usize),
    Compose,
    Noop,
    ToggleSettings,
    Settings(ui::settings::SettingsMessage),
    AppearanceChanged(appearance::Mode),
    DividerDragStart(Divider),
    DividerDragMove(Point),
    DividerDragEnd,
    DividerHover(Divider),
    DividerUnhover,
    WindowResized(Size),
    WindowMoved(Point),
    ToggleRightSidebar,
    ThreadMessagesLoaded(u64, Result<Vec<ThreadMessage>, String>),
    ThreadAttachmentsLoaded(u64, Result<Vec<ThreadAttachment>, String>),
    ToggleMessageExpanded(usize),
    ToggleAllMessages,
    ToggleAttachmentsCollapsed,
    SetDateDisplay(DateDisplay),
    WindowCloseRequested(iced::window::Id),
}

struct App {
    db: Arc<Db>,
    sidebar: Sidebar,
    threads: Vec<Thread>,
    selected_thread: Option<usize>,
    status: String,
    mode: appearance::Mode,
    sidebar_width: f32,
    thread_list_width: f32,
    dragging: Option<Divider>,
    hovered_divider: Option<Divider>,
    right_sidebar_open: bool,
    thread_messages: Vec<ThreadMessage>,
    thread_attachments: Vec<ThreadAttachment>,
    message_expanded: Vec<bool>,
    attachments_collapsed: bool,
    attachment_collapse_cache: std::collections::HashMap<String, bool>,
    show_settings: bool,
    settings: ui::settings::SettingsState,
    window: window_state::WindowState,
    /// Incremented on every navigation load (accounts, labels, threads).
    nav_generation: u64,
    /// Incremented on every thread detail load (messages, attachments).
    thread_generation: u64,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let db = Arc::clone(DB.get().expect("DB not initialized"));
        let db_ref = Arc::clone(&db);
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let window = window_state::WindowState::load(data_dir);
        let app = Self {
            db,
            sidebar: Sidebar::new(),
            threads: Vec::new(),
            selected_thread: None,
            status: "Loading...".to_string(),
            mode: appearance::Mode::Dark,
            sidebar_width: window.sidebar_width,
            thread_list_width: window.thread_list_width,
            dragging: None,
            hovered_divider: None,
            right_sidebar_open: window.right_sidebar_open,
            thread_messages: Vec::new(),
            thread_attachments: Vec::new(),
            message_expanded: Vec::new(),
            attachments_collapsed: false,
            attachment_collapse_cache: std::collections::HashMap::new(),
            show_settings: false,
            settings: ui::settings::SettingsState::with_scale(
                *DEFAULT_SCALE.get().unwrap_or(&1.0),
            ),
            window,
            nav_generation: 1,
            thread_generation: 0,
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
            // System — follow OS
            _ => match self.mode {
                appearance::Mode::Light => Theme::custom(String::from("Light"), iced::theme::palette::Seed::LIGHT),
                _ => Theme::custom(String::from("Dark"), iced::theme::palette::Seed::DARK),
            },
        }
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        let mut subs = vec![
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
        ];

        // Drive overlay slide animation
        if self.settings.overlay_anim.is_animating(iced::time::Instant::now()) {
            subs.push(
                iced::window::frames()
                    .map(|at| Message::Settings(ui::settings::SettingsMessage::OverlayAnimTick(at))),
            );
        }

        iced::Subscription::batch(subs)
    }

    fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Sidebar(msg) => self.handle_sidebar(msg),
            Message::AppearanceChanged(mode) => {
                self.mode = mode;
                Task::none()
            }
            Message::AccountsLoaded(g, _) if g != self.nav_generation => Task::none(),
            Message::AccountsLoaded(_, Ok(accounts)) => {
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
                self.threads = threads;
                Task::none()
            }
            Message::ThreadsLoaded(_, Err(e)) => {
                self.status = format!("Threads error: {e}");
                Task::none()
            }
            Message::SelectThread(idx) => self.handle_select_thread(idx),
            Message::DividerDragStart(divider) => {
                self.dragging = Some(divider);
                Task::none()
            }
            Message::DividerDragMove(point) => {
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
            Message::Settings(ui::settings::SettingsMessage::Close) => {
                self.show_settings = false;
                Task::none()
            }
            Message::Settings(msg) => {
                self.settings.update(msg).map(Message::Settings)
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
            Message::WindowCloseRequested(id) => {
                let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
                self.window.sidebar_width = self.sidebar_width;
                self.window.thread_list_width = self.thread_list_width;
                self.window.right_sidebar_open = self.right_sidebar_open;
                self.window.save(data_dir);
                iced::window::close(id)
            }
            Message::ThreadMessagesLoaded(g, _) if g != self.thread_generation => Task::none(),
            Message::ThreadMessagesLoaded(_, Ok(messages)) => {
                self.apply_message_expansion(&messages);
                self.thread_messages = messages;
                Task::none()
            }
            Message::ThreadMessagesLoaded(_, Err(e)) => {
                self.status = format!("Messages error: {e}");
                Task::none()
            }
            Message::ThreadAttachmentsLoaded(g, _) if g != self.thread_generation => Task::none(),
            Message::ThreadAttachmentsLoaded(_, Ok(attachments)) => {
                self.thread_attachments = attachments;
                Task::none()
            }
            Message::ThreadAttachmentsLoaded(_, Err(e)) => {
                self.status = format!("Attachments error: {e}");
                Task::none()
            }
            Message::ToggleMessageExpanded(index) => {
                if let Some(e) = self.message_expanded.get_mut(index) {
                    *e = !*e;
                }
                Task::none()
            }
            Message::ToggleAllMessages => {
                let all_expanded = self.message_expanded.iter().all(|&e| e);
                for e in &mut self.message_expanded {
                    *e = !all_expanded;
                }
                Task::none()
            }
            Message::ToggleAttachmentsCollapsed => {
                self.attachments_collapsed = !self.attachments_collapsed;
                if let Some(thread) = self.selected_thread.and_then(|i| self.threads.get(i)) {
                    let key = format!("{}:{}", thread.account_id, thread.id);
                    self.attachment_collapse_cache
                        .insert(key, self.attachments_collapsed);
                }
                Task::none()
            }
            Message::SetDateDisplay(display) => {
                self.settings.date_display = display;
                Task::none()
            }
            Message::Compose | Message::Noop => Task::none(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.show_settings {
            return ui::settings::view(&self.settings).map(Message::Settings);
        }

        let selected_thread = self
            .selected_thread
            .and_then(|idx| self.threads.get(idx));

        let sidebar = container(self.sidebar.view().map(Message::Sidebar))
            .width(self.sidebar_width)
            .height(Length::Fill);

        let sidebar_divider_style = if self.hovered_divider == Some(Divider::Sidebar)
            || self.dragging == Some(Divider::Sidebar)
        {
            ui::theme::divider_hover_container as fn(&Theme) -> _
        } else {
            ui::theme::divider_container
        };
        let divider_sidebar = mouse_area(
            container("")
                .width(DIVIDER_WIDTH)
                .height(Length::Fill)
                .style(sidebar_divider_style),
        )
        .on_press(Message::DividerDragStart(Divider::Sidebar))
        .on_release(Message::DividerDragEnd)
        .on_enter(Message::DividerHover(Divider::Sidebar))
        .on_exit(Message::DividerUnhover)
        .interaction(iced::mouse::Interaction::ResizingHorizontally);

        let folder_name = self
            .sidebar.selected_label
            .as_ref()
            .and_then(|lid| {
                self.sidebar.labels.iter()
                    .find(|l| l.id == *lid)
                    .map(|l| l.name.as_str())
            })
            .unwrap_or("Inbox");
        let scope_name = self
            .sidebar.selected_account
            .and_then(|idx| self.sidebar.accounts.get(idx))
            .and_then(|a| a.display_name.as_deref().or(Some(a.email.as_str())))
            .unwrap_or("All");

        let thread_list = container(ui::thread_list::view(
            &self.threads,
            self.selected_thread,
            folder_name,
            scope_name,
        ))
        .width(self.thread_list_width)
        .height(Length::Fill);

        let thread_divider_style = if self.hovered_divider == Some(Divider::ThreadList)
            || self.dragging == Some(Divider::ThreadList)
        {
            ui::theme::divider_hover_container as fn(&Theme) -> _
        } else {
            ui::theme::divider_container
        };
        let divider_thread = mouse_area(
            container("")
                .width(DIVIDER_WIDTH)
                .height(Length::Fill)
                .style(thread_divider_style),
        )
        .on_press(Message::DividerDragStart(Divider::ThreadList))
        .on_release(Message::DividerDragEnd)
        .on_enter(Message::DividerHover(Divider::ThreadList))
        .on_exit(Message::DividerUnhover)
        .interaction(iced::mouse::Interaction::ResizingHorizontally);

        let reading_pane = container(ui::reading_pane::view(
            selected_thread,
            &self.thread_messages,
            &self.message_expanded,
            &self.thread_attachments,
            self.attachments_collapsed,
            self.settings.date_display,
        ))
            .width(Length::Fill)
            .height(Length::Fill);

        let right_sidebar = ui::right_sidebar::view(self.right_sidebar_open);

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

// ── Sidebar event handling ─────────────────────────────

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
                self.selected_thread = None;
                self.nav_generation += 1;
                self.thread_generation += 1;
                if let Some(account) = self.sidebar.accounts.get(idx) {
                    let id = account.id.clone();
                    return self.load_labels_and_threads(&id);
                }
                Task::none()
            }
            SidebarEvent::AllAccountsSelected => {
                self.selected_thread = None;
                self.threads.clear();
                self.sidebar.labels.clear();
                Task::none()
            }
            SidebarEvent::CycleAccount => {
                // Already handled inside sidebar.update() — no extra work needed
                Task::none()
            }
            SidebarEvent::LabelSelected(label_id) => {
                self.selected_thread = None;
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
            SidebarEvent::Compose => Task::none(),
            SidebarEvent::ToggleSettings => {
                self.show_settings = !self.show_settings;
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
        self.selected_thread = Some(idx);
        self.thread_messages.clear();
        self.thread_attachments.clear();
        self.message_expanded.clear();
        self.thread_generation += 1;
        // Restore attachment collapse state from cache
        self.attachments_collapsed = self.threads.get(idx)
            .map(|t| {
                let key = format!("{}:{}", t.account_id, t.id);
                self.attachment_collapse_cache.get(&key).copied().unwrap_or(false)
            })
            .unwrap_or(false);
        if let Some(thread) = self.threads.get(idx) {
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

    fn apply_message_expansion(&mut self, messages: &[ThreadMessage]) {
        let len = messages.len();
        let mut expanded = vec![false; len];

        for (i, msg) in messages.iter().enumerate() {
            if !msg.is_read || i == 0 || i == len - 1 {
                expanded[i] = true;
            }
        }

        self.message_expanded = expanded;
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
