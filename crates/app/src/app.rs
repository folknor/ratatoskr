use crate::appearance;
use crate::command_resolver;
use crate::db::{self, Db};
use crate::handlers::provider::{JmapPushReceiver, create_jmap_push_channel};
use crate::message::Message;
use crate::pop_out::{self, PopOutWindow};
use crate::service_client::{ServiceClient, ServiceNotificationReceiver};
use crate::ui::add_account::AddAccountWizard;
use crate::ui::calendar::{CalendarState, CalendarView};
use crate::ui::palette::Palette;
use crate::ui::reading_pane::ReadingPane;
use crate::ui::settings::Settings;
use crate::ui::sidebar::Sidebar;
use crate::ui::status_bar::{
    StatusBar, SyncProgressReceiver, create_sync_progress_channel, shared_receiver,
};
use crate::ui::thread_list::ThreadList;
use crate::ui::undoable::UndoableText;
use crate::window_state;
use cmdk::{BindingTable, Chord, CommandRegistry, FocusedRegion, UndoStack, current_platform};
use iced::{Task, Theme};
use rtsk::db::queries::{get_settings_bootstrap_snapshot, get_ui_bootstrap_snapshot};
use rtsk::db::queries_extra::get_calendar_default_view_sync;
use rtsk::generation::{GenerationCounter, Nav, PopOut, Search, ThreadDetail};
use std::collections::HashMap;
use std::sync::Arc;

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
pub(crate) const DIVIDER_WIDTH: f32 = 2.0;

/// How long to wait for the second chord of a sequence.
pub(crate) const CHORD_TIMEOUT: std::time::Duration = std::time::Duration::from_millis(1000);

/// Pending chord state for two-key sequence bindings.
pub(crate) struct PendingChord {
    pub(crate) first: Chord,
}

pub struct ReadyApp {
    pub(crate) db: Arc<Db>,
    pub(crate) sidebar: Sidebar,
    pub(crate) thread_list: ThreadList,
    pub(crate) reading_pane: ReadingPane,
    pub(crate) settings: Settings,
    pub(crate) status_bar: StatusBar,
    pub(crate) status: String,
    pub(crate) mode: appearance::Mode,
    pub(crate) app_mode: AppMode,
    pub(crate) calendar: CalendarState,
    pub(crate) sidebar_width: f32,
    pub(crate) thread_list_width: f32,
    pub(crate) dragging: Option<Divider>,
    pub(crate) hovered_divider: Option<Divider>,
    pub(crate) right_sidebar_open: bool,
    pub(crate) show_settings: bool,
    pub(crate) window: window_state::WindowState,

    pub(crate) main_window_id: iced::window::Id,
    pub(crate) pop_out_windows: HashMap<iced::window::Id, PopOutWindow>,
    pub(crate) pop_out_generation: GenerationCounter<PopOut>,
    pub(crate) nav_generation: GenerationCounter<Nav>,
    pub(crate) thread_generation: GenerationCounter<ThreadDetail>,

    // Command palette infrastructure
    pub(crate) registry: CommandRegistry,
    pub(crate) binding_table: BindingTable,
    pub(crate) focused_region: Option<FocusedRegion>,
    pub(crate) is_online: bool,
    pub(crate) pending_chord: Option<PendingChord>,
    pub(crate) palette: Palette,
    pub(crate) undo_stack: UndoStack<crate::action_resolve::MailUndoPayload>,

    // Chat state
    pub(crate) chat_timeline: Option<crate::ui::chat_timeline::ChatTimeline>,
    pub(crate) chat_generation: GenerationCounter<rtsk::generation::Chat>,
    pub(crate) chat_list_generation: GenerationCounter<rtsk::generation::ChatList>,

    // Search state
    pub(crate) search_state: Option<Arc<rtsk::search::SearchState>>,
    pub(crate) search_generation: GenerationCounter<Search>,
    pub(crate) search_query: UndoableText,
    pub(crate) search_debounce_deadline: Option<iced::time::Instant>,
    /// Whether the user was in a folder view before entering search.
    /// When search is cleared, threads are reloaded from the current
    /// navigation state instead of restoring a stale clone.
    pub(crate) was_in_folder_view: bool,

    // Search history (recent queries from pinned_searches)
    pub(crate) search_history: Vec<String>,

    // Pinned searches
    pub(crate) pinned_searches: Vec<db::PinnedSearch>,
    pub(crate) editing_pinned_search: Option<i64>,
    pub(crate) expiry_ran: bool,

    pub(crate) no_accounts: bool,
    pub(crate) add_account_wizard: Option<AddAccountWizard>,

    /// Currently held keyboard modifiers (for Ctrl+click, Shift+click).
    pub(crate) current_modifiers: iced::keyboard::Modifiers,

    /// Active chat contact email, set when entering chat view.
    pub(crate) active_chat: Option<String>,

    // Sync progress pipeline
    pub(crate) sync_receiver: SyncProgressReceiver,
    #[allow(dead_code)]
    pub(crate) sync_reporter: Arc<crate::ui::status_bar::IcedProgressReporter>,

    /// In-flight delta-sync handles keyed by account id. Used to
    /// (1) skip dispatch when a sync for the same account is already running
    /// and (2) abort the task on account deletion so a stale sync can't keep
    /// writing to the deleted account's stores.
    pub(crate) sync_handles: HashMap<String, iced::task::Handle>,

    // JMAP push notification pipeline
    pub(crate) jmap_push_tx: tokio::sync::mpsc::UnboundedSender<String>,
    pub(crate) jmap_push_receiver: JmapPushReceiver,

    /// Body store for loading decompressed message bodies via core.
    pub(crate) body_store: Option<rtsk::body_store::BodyStoreState>,
    /// Inline image store for CID image resolution.
    pub(crate) inline_image_store: Option<store::inline_image_store::InlineImageStoreState>,
    /// Encryption key for decrypting provider credentials (OAuth tokens, passwords).
    pub(crate) encryption_key: Option<[u8; 32]>,
    /// Action service context - the authoritative write path for email mutations.
    /// `None` if stores failed to initialize at boot (degraded mode).
    pub(crate) action_ctx: Option<rtsk::actions::ActionContext>,

    // Service process scaffold
    pub(crate) service_client: Option<Arc<ServiceClient>>,
    pub(crate) service_notifications: ServiceNotificationReceiver,
}

impl ReadyApp {
    /// Construct the `Ready` half of the app state machine. Called from
    /// `BootingApp::update` when `Message::ServiceBootReady` arrives. Does
    /// the heavy bootstrap work (DB open, accounts/sidebar/calendar load,
    /// action_ctx construction) the original `App::boot` did synchronously
    /// before Phase 1.5; the Service has already migrated the schema by
    /// this point so the UI's local DB open is fast.
    pub(crate) fn from_boot_ready(
        boot_response: &service_api::BootReadyResponse,
        main_window_id: iced::window::Id,
        service_client: Arc<ServiceClient>,
        service_notifications: ServiceNotificationReceiver,
    ) -> (Self, Task<Message>) {
        log::info!(
            "Service boot.ready: schema_version={}, migrations_applied={}",
            boot_response.schema_version,
            boot_response.migrations_applied,
        );
        let data_dir = crate::APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let db = Arc::new(
            Db::open(data_dir).expect("UI-side DB open after Service handshake"),
        );
        let db_ref = Arc::clone(&db);
        let db_ref2 = Arc::clone(&db);
        let db_ref3 = Arc::clone(&db);
        let db_ref4 = Arc::clone(&db);
        let window = window_state::WindowState::load(data_dir);

        let mut registry = CommandRegistry::new();
        let mut binding_table = BindingTable::new(&registry, current_platform());
        let keybindings_path = data_dir.join("keybindings.json");
        if let Err(e) = binding_table.load_overrides_from_file(&keybindings_path) {
            eprintln!("warning: failed to load keybinding overrides: {e}");
        }
        // Load persisted usage counts for command ranking
        let usage_path = data_dir.join("command_usage.json");
        if let Ok(json) = std::fs::read_to_string(&usage_path)
            && let Ok(map) = serde_json::from_str::<std::collections::HashMap<String, u32>>(&json)
        {
            registry.usage.load_from_map(&map);
        }
        let resolver = Arc::new(command_resolver::AppInputResolver::new(Arc::clone(&db)));

        let (rx, reporter) = create_sync_progress_channel();
        let sync_receiver = shared_receiver(rx);
        let sync_reporter = Arc::new(reporter);

        let (jmap_push_tx, jmap_push_receiver) = create_jmap_push_channel();

        let body_store = match db::threads::init_body_store() {
            Ok(bs) => Some(bs),
            Err(e) => {
                log::error!("Failed to init body store: {e}");
                None
            }
        };

        let inline_image_store =
            match store::inline_image_store::InlineImageStoreState::init(data_dir) {
                Ok(store) => Some(store),
                Err(e) => {
                    log::error!("Failed to init inline image store: {e}");
                    None
                }
            };

        let encryption_key = match rtsk::load_encryption_key(data_dir) {
            Ok(key) => Some(key),
            Err(e) => {
                log::error!("Failed to load encryption key: {e}");
                None
            }
        };

        // Initialize search state once - shared between the app and action service.
        let search_state: Option<Arc<rtsk::search::SearchState>> =
            rtsk::search::SearchState::init(data_dir).map(Arc::new).ok();

        let action_ctx = match (
            &body_store,
            &inline_image_store,
            &search_state,
            encryption_key,
        ) {
            (Some(bs), Some(iis), Some(ss), Some(key)) => Some(rtsk::actions::ActionContext {
                db: db.write_db_state(),
                body_store: bs.clone(),
                inline_images: iis.clone(),
                search: (**ss).clone(),
                encryption_key: key,
                suppress_pending_enqueue: false,
                in_flight: std::sync::Arc::new(std::sync::Mutex::new(
                    std::collections::HashSet::new(),
                )),
            }),
            _ => {
                log::error!("Action service unavailable: one or more stores failed to initialize");
                None
            }
        };

        let session = pop_out::session::SessionState::load(data_dir);

        let calendar_default_view = db
            .read_db_state()
            .with_conn_sync(get_calendar_default_view_sync)
            .ok()
            .flatten()
            .map(|view_name| CalendarState::parse_view_name(&view_name))
            .unwrap_or(CalendarView::Month);

        // Load persisted preferences. The bootstrap snapshots don't decrypt
        // anything (they only cover non-secure keys), so a zero key is fine
        // when the real key is missing.
        let snapshot_key = encryption_key.unwrap_or([0u8; 32]);
        let bootstrap = db
            .read_db_state()
            .with_conn_sync(|conn| {
                let ui = get_ui_bootstrap_snapshot(conn, &snapshot_key)?;
                let settings = get_settings_bootstrap_snapshot(conn, &snapshot_key)?;
                Ok((ui, settings))
            })
            .ok();

        let bimi_cache = Arc::new(rtsk::bimi::BimiLruCache::new());

        let mut app = Self {
            db,
            sidebar: Sidebar::new(),
            thread_list: ThreadList::new(Arc::clone(&bimi_cache)),
            reading_pane: ReadingPane::new(),
            settings: Settings::with_scale(*crate::DEFAULT_SCALE.get().unwrap_or(&1.0)),
            status_bar: StatusBar::new(),
            status: "Service ready".to_string(),
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
            pop_out_generation: GenerationCounter::new(),
            nav_generation: GenerationCounter::new(),
            thread_generation: GenerationCounter::new(),
            registry,
            binding_table,
            focused_region: None,
            is_online: true,
            pending_chord: None,
            palette: Palette::new(CommandRegistry::new(), resolver),
            undo_stack: UndoStack::default(),
            search_state,
            chat_timeline: None,
            chat_generation: GenerationCounter::new(),
            chat_list_generation: GenerationCounter::new(),
            search_generation: GenerationCounter::new(),
            search_query: UndoableText::new(),
            search_debounce_deadline: None,
            was_in_folder_view: false,
            search_history: Vec::new(),
            pinned_searches: Vec::new(),
            editing_pinned_search: None,
            expiry_ran: false,
            no_accounts: false,
            add_account_wizard: None,
            current_modifiers: iced::keyboard::Modifiers::empty(),
            active_chat: None,
            sync_receiver,
            sync_reporter,
            sync_handles: HashMap::new(),
            jmap_push_tx,
            jmap_push_receiver,
            body_store,
            inline_image_store,
            encryption_key,
            action_ctx,
            service_client: Some(service_client),
            service_notifications,
        };

        if let Some((ui_snap, settings_snap)) = bootstrap {
            app.settings.apply_bootstrap(&ui_snap, &settings_snap);
        }

        // Note: the queued-drafts sweep, pending-ops boot recovery, and
        // per-account thread_participants backfill that used to run from
        // here all relocated to the Service's boot sequence in Phase 1.5.
        // The UI ActionContext below stays UI-side until Phase 2 moves the
        // action service across the boundary.

        // Restore pop-out windows from previous session
        let mut session_tasks = app.restore_pop_out_windows(&session);

        let load_gen = app.nav_generation.next();
        let mut boot_tasks = vec![
            Task::perform(
                async move { (load_gen, crate::helpers::load_accounts(db_ref).await) },
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
            // Initial GAL cache population (deferred - provider clients
            // aren't available at boot; the first GalRefreshTick will
            // attempt the actual fetch once accounts are loaded)
            Task::done(Message::GalRefreshTick),
        ];

        // Pending-ops crash recovery, queued-drafts sweep, and thread-
        // participants backfill all run Service-side now (Phase 1.5
        // commits 7-9).

        // Snooze resurface on boot: unsnooze threads that became due while the app was closed.
        boot_tasks.push(Task::done(Message::SnoozeTick));

        boot_tasks.append(&mut session_tasks);
        (app, Task::batch(boot_tasks))
    }

    pub(crate) fn title(&self, window_id: iced::window::Id) -> String {
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
            Some(PopOutWindow::Calendar(_)) => "Ratatoskr \u{2014} Calendar".to_string(),
            None => "Ratatoskr".to_string(),
        }
    }

    // (helper hoisted above; impl block continues)
    pub(crate) fn daemon_theme(&self, _window_id: iced::window::Id) -> Theme {
        self.theme()
    }

    pub(crate) fn theme(&self) -> Theme {
        match self.settings.theme.as_str() {
            "Light" => Theme::custom(String::from("Light"), iced::theme::palette::Seed::LIGHT),
            "Dark" => Theme::custom(String::from("Dark"), iced::theme::palette::Seed::DARK),
            "Theme" => {
                let idx = self.settings.selected_theme.unwrap_or(0);
                crate::ui::theme::theme_by_index(idx)
            }
            _ => match self.mode {
                appearance::Mode::Light => {
                    Theme::custom(String::from("Light"), iced::theme::palette::Seed::LIGHT)
                }
                _ => Theme::custom(String::from("Dark"), iced::theme::palette::Seed::DARK),
            },
        }
    }
}

/// Adapter for the two-phase spawn event receiver. Converts each
/// `SpawnEvent` into the matching `Message` and feeds an iced `Task`
/// stream. Lives at module scope so the type inference for
/// `iced::stream::channel` lands on a concrete signature rather than the
/// closure-context inference that was failing inside `App::boot`.
fn spawn_event_stream(
    rx: tokio::sync::mpsc::Receiver<crate::service_client::SpawnEvent>,
) -> Task<Message> {
    Task::stream(iced::stream::channel(
        8,
        move |mut output: iced::futures::channel::mpsc::Sender<Message>| {
            let mut rx = rx;
            async move {
                while let Some(event) = rx.recv().await {
                    let msg = match event {
                        crate::service_client::SpawnEvent::ChildSpawned(client) => {
                            Message::ServiceChildSpawned(client)
                        }
                        crate::service_client::SpawnEvent::BootReady(response) => {
                            Message::ServiceBootReady(response)
                        }
                        crate::service_client::SpawnEvent::Terminal(error) => {
                            Message::ServiceBootFailed(error.to_string())
                        }
                    };
                    if output.try_send(msg).is_err() {
                        break;
                    }
                }
            }
        },
    ))
}

/// Splash state rendered while in `Booting`. Updated on each
/// `Message::ServiceNotification(BootProgress)` so the user sees migration
/// progress during the slow boot.ready round-trip.
#[derive(Debug, Default)]
pub(crate) struct SplashState {
    /// Current boot phase reported by the Service. None means "awaiting
    /// first BootProgress" (typically very brief).
    pub(crate) phase: Option<service_api::BootPhase>,
    /// Optional human-readable message from the Service.
    pub(crate) message: Option<String>,
}

impl SplashState {
    fn apply(&mut self, progress: service_api::BootProgress) {
        self.phase = Some(progress.phase);
        self.message = progress.message;
    }

    /// Default user-visible label per phase.
    fn label(&self) -> &'static str {
        match self.phase {
            None => "Connecting to Service...",
            Some(service_api::BootPhase::LoadingKey) => "Loading encryption key...",
            Some(service_api::BootPhase::OpeningDatabase) => "Opening database...",
            Some(service_api::BootPhase::Migrating { .. }) => "Migrating database...",
            Some(service_api::BootPhase::RecoveringPendingOps) => {
                "Recovering pending operations..."
            }
            Some(service_api::BootPhase::SweepingQueuedDrafts) => "Sweeping queued drafts...",
            Some(service_api::BootPhase::BackfillingThreadParticipants) => {
                "Backfilling thread participants..."
            }
        }
    }
}

/// The `Booting` half of the App state machine. Active from `iced::daemon`
/// startup until the Service answers `boot.ready`. Holds only what's needed
/// to render the splash and process the boot-flow messages whitelisted in
/// `crates/app/src/message.rs`.
///
/// Transitions to `App::Ready` on `Message::ServiceBootReady` via
/// `ReadyApp::from_boot_ready`. The user's deferred preferences (the only
/// non-default state we care about during Booting) carry over via the
/// stashed `appearance_mode`.
pub struct BootingApp {
    pub(crate) main_window_id: iced::window::Id,
    pub(crate) splash: SplashState,
    pub(crate) service_client: Option<Arc<ServiceClient>>,
    pub(crate) service_notifications: Option<ServiceNotificationReceiver>,
    /// AppearanceChanged events that arrive while Booting are stashed here
    /// and applied after ReadyApp construction so the user's first sight of
    /// the real UI matches their system theme.
    pub(crate) appearance_mode: Option<appearance::Mode>,
}

/// Outcome of `BootingApp::update`. The dispatcher uses this to decide
/// whether to stay in `Booting` (returning the task as-is) or transition to
/// `Ready` (replacing `*self` with `App::Ready(box ready)` and firing the
/// boxed task).
pub enum BootingUpdate {
    Stay(Task<Message>),
    Transition(Box<ReadyApp>, Task<Message>),
}

impl BootingApp {
    pub(crate) fn update(&mut self, message: Message) -> BootingUpdate {
        match message {
            Message::ServiceChildSpawned(client) => {
                self.service_notifications = Some(client.notifications());
                self.service_client = Some(client);
                self.splash.message =
                    Some("Service connected, awaiting boot.ready...".to_string());
                BootingUpdate::Stay(Task::none())
            }
            Message::ServiceBootReady(response) => {
                let client = self
                    .service_client
                    .take()
                    .expect("ServiceChildSpawned must precede ServiceBootReady");
                let notifications = self.service_notifications.take().unwrap_or_else(|| {
                    Arc::new(crate::notification_queue::NotificationQueue::new(1024))
                });
                let (ready, task) = ReadyApp::from_boot_ready(
                    &response,
                    self.main_window_id,
                    client,
                    notifications,
                );
                BootingUpdate::Transition(Box::new(ready), task)
            }
            Message::ServiceBootFailed(detail) => {
                log::error!("Service boot failed (fatal): {detail}");
                eprintln!("[ui] fatal: service boot failed: {detail}");
                BootingUpdate::Stay(iced::exit())
            }
            Message::ServiceNotification(service_api::Notification::BootProgress(progress)) => {
                self.splash.apply(progress);
                BootingUpdate::Stay(Task::none())
            }
            Message::WindowCloseRequested(id) if id == self.main_window_id => {
                BootingUpdate::Stay(iced::exit())
            }
            Message::AppearanceChanged(mode) => {
                self.appearance_mode = Some(mode);
                BootingUpdate::Stay(Task::none())
            }
            Message::WindowResized(_, _)
            | Message::WindowMoved(_, _)
            | Message::WindowCloseRequested(_)
            | Message::Noop
            | Message::ModifiersChanged(_) => BootingUpdate::Stay(Task::none()),
            other => {
                log::debug!(
                    "BootingApp dropped message variant {:?} (whitelist per phase-1.5-plan scope item 21)",
                    std::mem::discriminant(&other),
                );
                BootingUpdate::Stay(Task::none())
            }
        }
    }

    pub(crate) fn view(&self, window_id: iced::window::Id) -> iced::Element<'_, Message> {
        if window_id != self.main_window_id {
            return crate::ui::widgets::empty_placeholder("", "");
        }
        let label = self.splash.label();
        let detail = self.splash.message.clone().unwrap_or_else(|| {
            if let Some(service_api::BootPhase::Migrating { current, total }) = self.splash.phase {
                format!("Migration {current} of {total}")
            } else {
                "Ratatoskr is starting...".to_string()
            }
        });
        iced::widget::container(
            iced::widget::column![
                iced::widget::text("Ratatoskr").size(28),
                iced::widget::Space::new().height(iced::Length::Fixed(12.0)),
                iced::widget::text(label).size(16),
                iced::widget::Space::new().height(iced::Length::Fixed(4.0)),
                iced::widget::text(detail).size(12),
            ]
            .align_x(iced::Alignment::Center),
        )
        .center_x(iced::Length::Fill)
        .center_y(iced::Length::Fill)
        .into()
    }

    pub(crate) fn title(&self, _window_id: iced::window::Id) -> String {
        "Ratatoskr - Starting".to_string()
    }

    pub(crate) fn subscription(&self) -> iced::Subscription<Message> {
        let mut subs = vec![
            appearance::subscription().map(Message::AppearanceChanged),
            iced::window::resize_events().map(|(id, size)| Message::WindowResized(id, size)),
            iced::window::close_requests().map(Message::WindowCloseRequested),
        ];
        if let Some(notifications) = self.service_notifications.as_ref() {
            subs.push(
                crate::service_subscription::service_notification_subscription(notifications)
                    .map(Message::ServiceNotification),
            );
        }
        iced::Subscription::batch(subs)
    }
}

/// Top-level state machine. `App::Booting` is the initial state; it
/// transitions to `App::Ready` exactly once when the Service answers
/// `boot.ready` (via `Message::ServiceBootReady`).
pub enum App {
    Booting(BootingApp),
    Ready(Box<ReadyApp>),
}

impl App {
    pub(crate) fn boot() -> (Self, Task<Message>) {
        let data_dir = crate::APP_DATA_DIR
            .get()
            .expect("APP_DATA_DIR must be set before iced::daemon::run");
        let window = window_state::WindowState::load(data_dir);
        let (main_window_id, open_task) = iced::window::open(window.to_window_settings());

        let booting = BootingApp {
            main_window_id,
            splash: SplashState::default(),
            service_client: None,
            service_notifications: None,
            appearance_mode: None,
        };

        // Two-phase Service spawn. The receiver emits ChildSpawned ->
        // BootReady (or Terminal on failure). BootingApp::update consumes
        // those messages and triggers the Booting -> Ready transition.
        let spawn_stream =
            spawn_event_stream(crate::service_client::ServiceClient::spawn_with_events(
                data_dir.clone(),
            ));

        (
            App::Booting(booting),
            Task::batch([open_task.discard(), spawn_stream]),
        )
    }

    pub(crate) fn update(&mut self, message: Message) -> Task<Message> {
        match self {
            App::Booting(booting) => match booting.update(message) {
                BootingUpdate::Stay(task) => task,
                BootingUpdate::Transition(mut ready, task) => {
                    if let Some(mode) = booting.appearance_mode.take() {
                        ready.mode = mode;
                    }
                    *self = App::Ready(ready);
                    task
                }
            },
            App::Ready(ready) => ready.update(message),
        }
    }

    pub(crate) fn view(&self, window_id: iced::window::Id) -> iced::Element<'_, Message> {
        match self {
            App::Booting(b) => b.view(window_id),
            App::Ready(r) => r.view(window_id),
        }
    }

    pub(crate) fn title(&self, window_id: iced::window::Id) -> String {
        match self {
            App::Booting(b) => b.title(window_id),
            App::Ready(r) => r.title(window_id),
        }
    }

    pub(crate) fn daemon_theme(&self, window_id: iced::window::Id) -> Theme {
        match self {
            App::Booting(_) => Theme::custom(String::from("Dark"), iced::theme::palette::Seed::DARK),
            App::Ready(r) => r.daemon_theme(window_id),
        }
    }

    pub(crate) fn scale_factor(&self) -> f32 {
        match self {
            App::Booting(_) => *crate::DEFAULT_SCALE.get().unwrap_or(&1.0),
            App::Ready(r) => r.settings.scale,
        }
    }

    pub(crate) fn subscription(&self) -> iced::Subscription<Message> {
        match self {
            App::Booting(b) => b.subscription(),
            App::Ready(r) => r.subscription(),
        }
    }
}
