use std::sync::Arc;

use iced::{Point, Size, Task};

use crate::pop_out::message_view::MessageViewState;
use crate::pop_out::session::{
    CalendarSessionEntry, ComposeSessionEntry, MessageViewSessionEntry, SessionState,
};
use crate::pop_out::{CalendarPopOutGeometry, PopOutWindow};
use crate::ui::layout::{
    COMPOSE_MIN_HEIGHT, COMPOSE_MIN_WIDTH, MESSAGE_VIEW_MIN_HEIGHT, MESSAGE_VIEW_MIN_WIDTH,
};
use crate::{APP_DATA_DIR, Message, ReadyApp};

impl ReadyApp {
    /// Save the full session state (main window + all pop-out windows) to disk.
    pub(crate) fn save_session_state(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");

        let mut message_views = Vec::new();
        let mut compose_windows = Vec::new();
        let mut calendar_window = None;
        for window in self.pop_out_windows.values() {
            match window {
                PopOutWindow::MessageView(state) => {
                    message_views.push(MessageViewSessionEntry {
                        message_id: state.message_id.clone(),
                        thread_id: state.thread_id.clone(),
                        account_id: state.account_id.clone(),
                        width: state.width,
                        height: state.height,
                        x: state.x,
                        y: state.y,
                    });
                }
                PopOutWindow::Compose(state) => {
                    compose_windows.push(ComposeSessionEntry {
                        draft_id: state.draft_id.clone(),
                        width: state.width,
                        height: state.height,
                        x: state.x,
                        y: state.y,
                    });
                }
                PopOutWindow::Calendar(geom) => {
                    calendar_window = Some(CalendarSessionEntry {
                        width: geom.width,
                        height: geom.height,
                        x: geom.x,
                        y: geom.y,
                    });
                }
            }
        }

        let session = SessionState {
            main_window: self.window.clone(),
            message_views,
            compose_windows,
            calendar_window,
        };

        let path = data_dir.join("session.json");
        if let Ok(json) = serde_json::to_string_pretty(&session) {
            let _ = std::fs::write(path, json);
        }
    }

    /// Restore pop-out windows from session state during boot.
    /// Returns tasks to open restored windows and load their data.
    pub(crate) fn restore_pop_out_windows(&mut self, session: &SessionState) -> Vec<Task<Message>> {
        let mut tasks = Vec::new();

        for entry in &session.message_views {
            let settings = iced::window::Settings {
                size: Size::new(entry.width, entry.height),
                position: position_for(entry.x, entry.y),
                min_size: Some(Size::new(MESSAGE_VIEW_MIN_WIDTH, MESSAGE_VIEW_MIN_HEIGHT)),
                exit_on_close_request: false,
                ..Default::default()
            };

            let (window_id, open_task) = iced::window::open(settings);

            let generation = self.next_pop_out_generation();
            let state = MessageViewState::from_session_entry(
                entry,
                generation,
                self.settings.default_rendering_mode,
            );
            let account_id = entry.account_id.clone();
            let message_id = entry.message_id.clone();

            self.pop_out_windows
                .insert(window_id, PopOutWindow::MessageView(Box::new(state)));

            tasks.push(self.dispatch_message_view_loads(
                window_id, generation, account_id, message_id, open_task,
            ));
        }

        for entry in &session.compose_windows {
            let settings = iced::window::Settings {
                size: Size::new(entry.width, entry.height),
                position: position_for(entry.x, entry.y),
                min_size: Some(Size::new(COMPOSE_MIN_WIDTH, COMPOSE_MIN_HEIGHT)),
                exit_on_close_request: false,
                ..Default::default()
            };

            let db = Arc::clone(&self.db);
            let draft_id = entry.draft_id.clone();
            let width = entry.width;
            let height = entry.height;
            let x = entry.x;
            let y = entry.y;

            // Open the window now (so the user sees it appear at boot) and
            // hydrate the compose state from the persisted draft async. The
            // `RestoredComposeLoaded` arm fills in the state on the
            // already-open window, or closes it if the draft is gone.
            let (window_id, open_task) = iced::window::open(settings);

            let load_task = Task::perform(
                async move {
                    let core_db = db.read_db_state();
                    rtsk::db::queries_extra::db_get_local_draft(&core_db, draft_id).await
                },
                move |result| Message::RestoredComposeLoaded {
                    window_id,
                    width,
                    height,
                    x,
                    y,
                    result,
                },
            );

            tasks.push(open_task.discard());
            tasks.push(load_task);
        }

        if let Some(entry) = &session.calendar_window {
            let settings = iced::window::Settings {
                size: Size::new(entry.width, entry.height),
                position: position_for(entry.x, entry.y),
                exit_on_close_request: false,
                ..Default::default()
            };
            let (window_id, open_task) = iced::window::open(settings);
            self.pop_out_windows.insert(
                window_id,
                PopOutWindow::Calendar(CalendarPopOutGeometry {
                    width: entry.width,
                    height: entry.height,
                    x: entry.x,
                    y: entry.y,
                }),
            );
            tasks.push(open_task.discard());
        }

        tasks
    }
}

impl ReadyApp {
    /// Hydrate a session-restored compose pop-out with its draft state, or
    /// close the window if the draft no longer exists. The window is
    /// already open at this point - `restore_pop_out_windows` opened it at
    /// boot so the user sees the window appear before the async DB load
    /// finishes.
    pub(crate) fn handle_restored_compose_loaded(
        &mut self,
        window_id: iced::window::Id,
        width: f32,
        height: f32,
        x: Option<f32>,
        y: Option<f32>,
        result: Result<Option<rtsk::db::types::DbLocalDraft>, String>,
    ) -> Task<Message> {
        let draft = match result {
            Ok(Some(draft)) => draft,
            Ok(None) => {
                log::info!("Session-restored compose draft missing; closing window");
                return iced::window::close(window_id);
            }
            Err(e) => {
                log::error!("Failed to load session-restored compose draft: {e}");
                return iced::window::close(window_id);
            }
        };

        let mut state =
            crate::pop_out::compose::ComposeState::from_local_draft(&self.sidebar.accounts, &draft);
        state.width = width;
        state.height = height;
        state.x = x;
        state.y = y;

        self.pop_out_windows
            .insert(window_id, PopOutWindow::Compose(Box::new(state)));

        // Resolve the signature for the From account (matches
        // `open_compose_window_with_state`).
        self.resolve_compose_signature(window_id)
    }
}

fn position_for(x: Option<f32>, y: Option<f32>) -> iced::window::Position {
    match (x, y) {
        (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => {
            iced::window::Position::Specific(Point::new(x, y))
        }
        _ => iced::window::Position::default(),
    }
}
