mod appearance;
mod db;
mod font;
mod icon;
mod ui;
mod window_state;

use db::{Account, Db, Label, Thread};
use iced::widget::pane_grid::{self, Configuration, PaneGrid};
use iced::{Element, Point, Size, Task, Theme};
use ui::layout::{READING_PANE_MIN_WIDTH, SIDEBAR_MIN_WIDTH, THREAD_LIST_MIN_WIDTH};
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneKind {
    Sidebar,
    ThreadList,
    ReadingPane,
}

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
    PaneResized(pane_grid::ResizeEvent),
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
    panes: pane_grid::State<PaneKind>,
    show_settings: bool,
    settings: ui::settings::SettingsState,
    window: window_state::WindowState,
}

fn pane_configuration() -> Configuration<PaneKind> {
    // Sidebar | ThreadList | ReadingPane
    //  ~15%       ~22%         ~63%
    Configuration::Split {
        axis: pane_grid::Axis::Vertical,
        ratio: 0.15,
        a: Box::new(Configuration::Pane(PaneKind::Sidebar)),
        b: Box::new(Configuration::Split {
            axis: pane_grid::Axis::Vertical,
            ratio: 0.26,
            a: Box::new(Configuration::Pane(PaneKind::ThreadList)),
            b: Box::new(Configuration::Pane(PaneKind::ReadingPane)),
        }),
    }
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
            panes: pane_grid::State::with_configuration(pane_configuration()),
            show_settings: false,
            settings: ui::settings::SettingsState::default(),
            window,
        };
        (app, Task::perform(load_accounts(db_ref), Message::AccountsLoaded))
    }

    fn theme(&self) -> Theme {
        let accent = ui::theme::ACCENT_COLORS
            .get(self.settings.accent_color_index)
            .copied()
            .unwrap_or(ui::theme::ACCENT_COLORS[0]);
        match self.settings.theme.as_str() {
            "Light" => ui::theme::light_with_accent(accent),
            "Dark" => ui::theme::dark_with_accent(accent),
            _ => self.mode.theme(accent),
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
            Message::PaneResized(pane_grid::ResizeEvent { split, ratio }) => {
                let clamped = self.clamp_pane_ratio(split, ratio);
                self.panes.resize(split, clamped);
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
                self.reclamp_all_panes();
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

    /// Clamp a pane split ratio so each pane respects its minimum width.
    ///
    /// The layout is a nested split:
    ///   Outer: Sidebar (ratio) | Rest (1-ratio)
    ///   Inner: ThreadList (ratio) | ReadingPane (1-ratio)  [within Rest]
    ///
    /// We identify which split is being dragged by walking the layout tree:
    /// the root split is "outer", and any split nested inside it is "inner".
    fn clamp_pane_ratio(&self, split: pane_grid::Split, ratio: f32) -> f32 {
        let total_width = self.window.width;
        if total_width <= 0.0 {
            return ratio;
        }

        let layout = self.panes.layout();
        let (outer_split, outer_ratio) = match layout {
            pane_grid::Node::Split {
                id,
                ratio: current_ratio,
                b,
                ..
            } => {
                let inner_split = match b.as_ref() {
                    pane_grid::Node::Split { id, .. } => Some(*id),
                    _ => None,
                };
                if split == *id {
                    // Dragging the outer split (Sidebar | Rest)
                    let min_ratio = SIDEBAR_MIN_WIDTH / total_width;
                    // Rest must fit ThreadList + ReadingPane minimums
                    let rest_min = THREAD_LIST_MIN_WIDTH + READING_PANE_MIN_WIDTH;
                    let max_ratio = 1.0 - (rest_min / total_width);
                    return ratio.clamp(min_ratio, max_ratio.max(min_ratio));
                }
                (inner_split, *current_ratio)
            }
            _ => return ratio,
        };

        if let Some(inner_id) = outer_split {
            if split == inner_id {
                // Dragging the inner split (ThreadList | ReadingPane)
                let rest_width = total_width * (1.0 - outer_ratio);
                if rest_width <= 0.0 {
                    return ratio;
                }
                let min_ratio = THREAD_LIST_MIN_WIDTH / rest_width;
                let max_ratio = 1.0 - (READING_PANE_MIN_WIDTH / rest_width);
                return ratio.clamp(min_ratio, max_ratio.max(min_ratio));
            }
        }

        ratio
    }

    /// Re-clamp all split ratios to enforce per-pane minimums at the current
    /// window size. Called on window resize so panes can't end up below their
    /// minimum when the window shrinks.
    fn reclamp_all_panes(&mut self) {
        if self.window.width <= 0.0 {
            return;
        }

        // Extract split IDs and ratios before mutating (avoids borrow conflict).
        let (outer, inner) = {
            let layout = self.panes.layout();
            match layout {
                pane_grid::Node::Split {
                    id, ratio, b, ..
                } => {
                    let inner = match b.as_ref() {
                        pane_grid::Node::Split { id, ratio, .. } => Some((*id, *ratio)),
                        _ => None,
                    };
                    Some((*id, *ratio, inner))
                }
                _ => None,
            }
        }.map_or(
            (None, None),
            |(id, ratio, inner)| (Some((id, ratio)), inner),
        );

        // Clamp outer split (Sidebar | Rest) first — inner depends on it.
        if let Some((id, ratio)) = outer {
            let clamped = self.clamp_pane_ratio(id, ratio);
            self.panes.resize(id, clamped);
        }

        // Clamp inner split (ThreadList | ReadingPane).
        if let Some((id, ratio)) = inner {
            let clamped = self.clamp_pane_ratio(id, ratio);
            self.panes.resize(id, clamped);
        }
    }

    fn view(&self) -> Element<'_, Message> {
        if self.show_settings {
            return ui::settings::view(&self.settings).map(Message::Settings);
        }

        let label_name = self
            .selected_label
            .as_deref()
            .unwrap_or("Inbox");

        let selected_thread = self
            .selected_thread
            .and_then(|idx| self.threads.get(idx));

        PaneGrid::new(&self.panes, |_pane, kind, _maximized| {
            let content: Element<'_, Message> = match kind {
                PaneKind::Sidebar => {
                    let sidebar_model = ui::sidebar::SidebarModel {
                        accounts: &self.accounts,
                        selected_account: self.selected_account,
                        labels: &self.labels,
                        selected_label: &self.selected_label,
                        scope_dropdown_open: self.scope_dropdown_open,
                        labels_expanded: self.labels_expanded,
                        smart_folders_expanded: self.smart_folders_expanded,
                    };
                    ui::sidebar::view(sidebar_model)
                }
                PaneKind::ThreadList => {
                    ui::thread_list::view(
                        &self.threads,
                        self.selected_thread,
                        &self.status,
                        label_name,
                    )
                }
                PaneKind::ReadingPane => {
                    ui::reading_pane::view(selected_thread)
                }
            };
            pane_grid::Content::new(content)
        })
        .spacing(1)
        .min_size(SIDEBAR_MIN_WIDTH)
        .on_resize(4, Message::PaneResized)
        .into()
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
