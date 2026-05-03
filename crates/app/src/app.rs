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

pub struct App {
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

impl App {
    pub(crate) fn boot() -> (Self, Task<Message>) {
        let db = Arc::clone(crate::DB.get().expect("DB not initialized"));
        let db_ref = Arc::clone(&db);
        let db_ref2 = Arc::clone(&db);
        let db_ref3 = Arc::clone(&db);
        let db_ref4 = Arc::clone(&db);
        let data_dir = crate::APP_DATA_DIR.get().expect("APP_DATA_DIR not set");
        let window = window_state::WindowState::load(data_dir);

        let (main_window_id, open_task) = iced::window::open(window.to_window_settings());

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
        // Placeholder queue until the Service is ready; replaced in
        // Message::ServiceChildSpawned with the spawned client's queue. Cap
        // matches service_client::NOTIFICATION_QUEUE_CAP.
        let service_notifications: ServiceNotificationReceiver =
            Arc::new(crate::notification_queue::NotificationQueue::new(1024));

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
            service_client: None,
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
        // Two-phase Service spawn (Phase 1.5 commit 11). The receiver emits
        // SpawnEvent::ChildSpawned -> Message::ServiceChildSpawned (App now
        // holds the client and can subscribe to notifications), followed by
        // SpawnEvent::BootReady -> Message::ServiceBootReady (boot
        // sequence complete). On any failure, SpawnEvent::Terminal ->
        // Message::ServiceBootFailed -> iced::exit().
        let spawn_stream =
            spawn_event_stream(crate::service_client::ServiceClient::spawn_with_events(
                data_dir.clone(),
            ));
        let mut boot_tasks = vec![
            open_task.discard(),
            spawn_stream,
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

        // Pending-ops crash recovery used to run here. As of Phase 1.5 it
        // runs Service-side during the boot sequence (DB-only variant);
        // the UI no longer dispatches recover_on_boot from App::boot.

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
