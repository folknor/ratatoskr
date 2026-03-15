mod appearance;
mod db;
mod font;
mod icon;
mod ui;

use db::{Account, Db, Label, Thread};
use iced::widget::row;
use iced::{Element, Task, Theme};
use std::sync::Arc;

static DB: std::sync::OnceLock<Arc<Db>> = std::sync::OnceLock::new();

fn main() -> iced::Result {
    let app_data_dir = dirs::data_dir()
        .expect("no data dir")
        .join("com.velo.app");

    let db = Db::open(&app_data_dir).expect("failed to open database");
    let _ = DB.set(Arc::new(db));

    let mut app = iced::application(App::boot, App::update, App::view)
        .title("Ratatoskr (iced prototype)")
        .theme(App::theme)
        .subscription(App::subscription)
        .default_font(font::TEXT);

    for f in font::load() {
        app = app.font(f);
    }

    app.run()
}

#[derive(Debug, Clone)]
pub enum Message {
    AccountsLoaded(Result<Vec<Account>, String>),
    SelectAccount(usize),
    CycleAccount,
    LabelsLoaded(Result<Vec<Label>, String>),
    SelectLabel(Option<String>),
    ThreadsLoaded(Result<Vec<Thread>, String>),
    SelectThread(usize),
    Compose,
    Noop,
    AppearanceChanged(appearance::Mode),
    ToggleScopeDropdown,
    ToggleLabelsSection,
    ToggleSmartFoldersSection,
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
}

impl App {
    fn boot() -> (Self, Task<Message>) {
        let db = Arc::clone(DB.get().expect("DB not initialized"));
        let db_ref = Arc::clone(&db);
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
        };
        (app, Task::perform(load_accounts(db_ref), Message::AccountsLoaded))
    }

    fn theme(&self) -> Theme {
        self.mode.theme()
    }

    fn subscription(&self) -> iced::Subscription<Message> {
        appearance::subscription().map(Message::AppearanceChanged)
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
            Message::Compose | Message::Noop => Task::none(),
        }
    }

    fn view(&self) -> Element<'_, Message> {
        let label_name = self
            .selected_label
            .as_deref()
            .unwrap_or("Inbox");

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
        let sidebar = ui::sidebar::view(sidebar_model);

        let thread_list = ui::thread_list::view(
            &self.threads,
            self.selected_thread,
            &self.status,
            label_name,
        );

        let reading_pane = ui::reading_pane::view(selected_thread);

        let contact_sidebar = ui::contact_sidebar::view(selected_thread);

        row![sidebar, thread_list, reading_pane, contact_sidebar].into()
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
