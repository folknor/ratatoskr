mod appearance;
mod db;
mod font;
mod icon;
mod ui;
mod window_state;

use db::{Account, Db, Label, Thread};
use iced::widget::{container, mouse_area, row};
use iced::{Element, Length, Point, Size, Task, Theme};
use ui::layout::{SIDEBAR_MIN_WIDTH, SIDEBAR_WIDTH, THREAD_LIST_MIN_WIDTH, THREAD_LIST_WIDTH};
use std::path::PathBuf;
use std::sync::Arc;

static DB: std::sync::OnceLock<Arc<Db>> = std::sync::OnceLock::new();
static APP_DATA_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

fn main() -> iced::Result {
    let app_data_dir = dirs::data_dir()
        .expect("no data dir")
        .join("com.velo.app");

    let db = Db::open(&app_data_dir).expect("failed to open database");
    let _ = DB.set(Arc::new(db));

    let window = window_state::WindowState::load(&app_data_dir);
    let _ = APP_DATA_DIR.set(app_data_dir);

    let mut app = iced::application(App::boot, App::update, App::view)
        .title("Ratatoskr (iced prototype)")
        .theme(App::theme)
        .scale_factor(|app| app.settings.scale)
        .subscription(App::subscription)
        .default_font(font::TEXT)
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
    AccountsLoaded(Result<Vec<Account>, String>),
    SelectAccount(usize),
    SelectAllAccounts,
    CycleAccount,
    LabelsLoaded(Result<Vec<Label>, String>),
    SelectLabel(Option<String>),
    ThreadsLoaded(Result<Vec<Thread>, String>),
    SelectThread(usize),
    Compose,
    Noop,
    ToggleSettings,
    Settings(ui::settings::SettingsMessage),
    AppearanceChanged(appearance::Mode),
    ToggleScopeDropdown,
    ToggleLabelsSection,
    ToggleSmartFoldersSection,
    DividerDragStart(Divider),
    DividerDragMove(Point),
    DividerDragEnd,
    DividerHover(Divider),
    DividerUnhover,
    WindowResized(Size),
    WindowMoved(Point),
    WindowCloseRequested(iced::window::Id),
}

struct App {
    db: Arc<Db>,
    accounts: Vec<Account>,
    labels: Vec<Label>,
    threads: Vec<Thread>,
    selected_account: Option<usize>,
    selected_label: Option<String>,
    selected_thread: Option<usize>,
    status: String,
    mode: appearance::Mode,
    scope_dropdown_open: bool,
    labels_expanded: bool,
    smart_folders_expanded: bool,
    sidebar_width: f32,
    thread_list_width: f32,
    dragging: Option<Divider>,
    hovered_divider: Option<Divider>,
    show_settings: bool,
    settings: ui::settings::SettingsState,
    window: window_state::WindowState,
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let db = Arc::clone(DB.get().expect("DB not initialized"));
        let db_ref = Arc::clone(&db);
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let window = window_state::WindowState::load(data_dir);
        let app = Self {
            db,
            accounts: Vec::new(),
            labels: Vec::new(),
            threads: Vec::new(),
            selected_account: None,
            selected_label: None,
            selected_thread: None,
            status: "Loading...".to_string(),
            mode: appearance::Mode::Dark,
            scope_dropdown_open: false,
            labels_expanded: true,
            smart_folders_expanded: true,
            sidebar_width: SIDEBAR_WIDTH,
            thread_list_width: THREAD_LIST_WIDTH,
            dragging: None,
            hovered_divider: None,
            show_settings: false,
            settings: ui::settings::SettingsState::default(),
            window,
        };
        (app, Task::perform(load_accounts(db_ref), Message::AccountsLoaded))
    }

    fn theme(&self) -> Theme {
        match self.settings.theme.as_str() {
            "Light" => Theme::custom(String::from("Light"), iced::theme::Palette::LIGHT),
            "Dark" => Theme::custom(String::from("Dark"), iced::theme::Palette::DARK),
            "Theme" => {
                let idx = self.settings.selected_theme.unwrap_or(0);
                ui::theme::theme_by_index(idx)
            }
            // System — follow OS
            _ => match self.mode {
                appearance::Mode::Light => Theme::custom(String::from("Light"), iced::theme::Palette::LIGHT),
                _ => Theme::custom(String::from("Dark"), iced::theme::Palette::DARK),
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
            Message::AppearanceChanged(mode) => {
                self.mode = mode;
                Task::none()
            }
            Message::AccountsLoaded(Ok(accounts)) => {
                self.accounts = accounts;
                if !self.accounts.is_empty() {
                    self.selected_account = Some(0);
                    let db = Arc::clone(&self.db);
                    let id = self.accounts[0].id.clone();
                    self.status = format!("Loaded {} accounts", self.accounts.len());
                    return Task::batch([
                        Task::perform(
                            load_labels(db.clone(), id.clone()),
                            Message::LabelsLoaded,
                        ),
                        Task::perform(
                            load_threads(db, id, None),
                            Message::ThreadsLoaded,
                        ),
                    ]);
                }
                self.status = "No accounts found".to_string();
                Task::none()
            }
            Message::AccountsLoaded(Err(e)) => {
                self.status = format!("Error: {e}");
                Task::none()
            }
            Message::SelectAllAccounts => {
                self.selected_account = None;
                self.selected_label = None;
                self.selected_thread = None;
                self.scope_dropdown_open = false;
                self.threads.clear();
                self.labels.clear();
                Task::none()
            }
            Message::SelectAccount(idx) => {
                self.selected_account = Some(idx);
                self.selected_label = None;
                self.selected_thread = None;
                self.scope_dropdown_open = false;
                if let Some(account) = self.accounts.get(idx) {
                    let db = Arc::clone(&self.db);
                    let id = account.id.clone();
                    Task::batch([
                        Task::perform(
                            load_labels(db.clone(), id.clone()),
                            Message::LabelsLoaded,
                        ),
                        Task::perform(
                            load_threads(db, id, None),
                            Message::ThreadsLoaded,
                        ),
                    ])
                } else {
                    Task::none()
                }
            }
            Message::CycleAccount => {
                if self.accounts.len() > 1 {
                    let next = match self.selected_account {
                        Some(idx) => (idx + 1) % self.accounts.len(),
                        None => 0,
                    };
                    self.update(Message::SelectAccount(next))
                } else {
                    Task::none()
                }
            }
            Message::LabelsLoaded(Ok(labels)) => {
                self.labels = labels;
                Task::none()
            }
            Message::LabelsLoaded(Err(e)) => {
                self.status = format!("Labels error: {e}");
                Task::none()
            }
            Message::SelectLabel(label_id) => {
                self.selected_label = label_id.clone();
                self.selected_thread = None;
                if let Some(idx) = self.selected_account {
                    if let Some(account) = self.accounts.get(idx) {
                        let db = Arc::clone(&self.db);
                        let id = account.id.clone();
                        return Task::perform(
                            load_threads(db, id, label_id),
                            Message::ThreadsLoaded,
                        );
                    }
                }
                Task::none()
            }
            Message::ThreadsLoaded(Ok(threads)) => {
                self.status = format!("{} threads", threads.len());
                self.threads = threads;
                Task::none()
            }
            Message::ThreadsLoaded(Err(e)) => {
                self.status = format!("Threads error: {e}");
                Task::none()
            }
            Message::SelectThread(idx) => {
                self.selected_thread = Some(idx);
                Task::none()
            }
            Message::ToggleScopeDropdown => {
                self.scope_dropdown_open = !self.scope_dropdown_open;
                Task::none()
            }
            Message::ToggleLabelsSection => {
                self.labels_expanded = !self.labels_expanded;
                Task::none()
            }
            Message::ToggleSmartFoldersSection => {
                self.smart_folders_expanded = !self.smart_folders_expanded;
                Task::none()
            }
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
            Message::WindowResized(size) => {
                self.window.set_size(size);
                Task::none()
            }
            Message::WindowMoved(point) => {
                self.window.set_position(point);
                Task::none()
            }
            Message::WindowCloseRequested(id) => {
                let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
                self.window.save(data_dir);
                iced::window::close(id)
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

        let sidebar_model = ui::sidebar::SidebarModel {
            accounts: &self.accounts,
            selected_account: self.selected_account,
            labels: &self.labels,
            selected_label: &self.selected_label,
            scope_dropdown_open: self.scope_dropdown_open,
            labels_expanded: self.labels_expanded,
            smart_folders_expanded: self.smart_folders_expanded,
        };

        let sidebar = container(ui::sidebar::view(sidebar_model))
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

        let thread_list = container(ui::thread_list::view(
            &self.threads,
            self.selected_thread,
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

        let reading_pane = container(ui::reading_pane::view(selected_thread))
            .width(Length::Fill)
            .height(Length::Fill);

        let layout = row![sidebar, divider_sidebar, thread_list, divider_thread, reading_pane]
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

async fn load_accounts(db: Arc<Db>) -> Result<Vec<Account>, String> {
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
