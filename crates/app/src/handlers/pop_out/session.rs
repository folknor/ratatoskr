use iced::{Point, Size, Task};

use crate::pop_out::message_view::MessageViewState;
use crate::pop_out::session::{MessageViewSessionEntry, SessionState};
use crate::pop_out::PopOutWindow;
use crate::ui::layout::{MESSAGE_VIEW_MIN_HEIGHT, MESSAGE_VIEW_MIN_WIDTH};
use crate::{APP_DATA_DIR, App, Message};

impl App {
    /// Save the full session state (main window + all pop-out windows) to disk.
    pub(crate) fn save_session_state(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");

        let message_views: Vec<MessageViewSessionEntry> = self
            .pop_out_windows
            .values()
            .filter_map(|w| match w {
                PopOutWindow::MessageView(state) => Some(MessageViewSessionEntry {
                    message_id: state.message_id.clone(),
                    thread_id: state.thread_id.clone(),
                    account_id: state.account_id.clone(),
                    width: state.width,
                    height: state.height,
                    x: state.x,
                    y: state.y,
                }),
                PopOutWindow::Compose(_) | PopOutWindow::Calendar => None,
            })
            .collect();

        let session = SessionState {
            main_window: self.window.clone(),
            message_views,
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
            let position = match (entry.x, entry.y) {
                (Some(x), Some(y)) if x >= 0.0 && y >= 0.0 => {
                    iced::window::Position::Specific(Point::new(x, y))
                }
                _ => iced::window::Position::default(),
            };

            let settings = iced::window::Settings {
                size: Size::new(entry.width, entry.height),
                position,
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

        tasks
    }
}
