//! Pop-out window handler methods for `App`.
//!
//! All pop-out window logic lives here. `main.rs` dispatches to these methods
//! via one-line match arms. This file owns:
//! - Message view update routing
//! - Compose update routing
//! - Window open/close for pop-outs
//! - Session save/restore
//! - Save As (.eml / .txt) flow

use std::sync::Arc;

use iced::{Point, Size, Task};

use crate::db::Db;
use crate::pop_out::compose::{ComposeMessage, ComposeMode, ComposeState};
use crate::pop_out::message_view::{
    MessageViewMessage, MessageViewState, RenderingMode,
};
use crate::pop_out::session::{MessageViewSessionEntry, SessionState};
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::ui::layout::{
    COMPOSE_DEFAULT_HEIGHT, COMPOSE_DEFAULT_WIDTH, COMPOSE_MIN_HEIGHT, COMPOSE_MIN_WIDTH,
    MESSAGE_VIEW_DEFAULT_HEIGHT, MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_MIN_HEIGHT,
    MESSAGE_VIEW_MIN_WIDTH,
};
use crate::{App, Message, APP_DATA_DIR};

// ── Pop-out message dispatch ────────────────────────────

impl App {
    /// Route a `PopOutMessage` to the correct pop-out window handler.
    pub(crate) fn handle_pop_out_message(
        &mut self,
        window_id: iced::window::Id,
        pop_out_msg: PopOutMessage,
    ) -> Task<Message> {
        let Some(window) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };
        match (window, &pop_out_msg) {
            // Reply / ReplyAll / Forward from message view → open compose
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(
                    MessageViewMessage::Reply
                    | MessageViewMessage::ReplyAll
                    | MessageViewMessage::Forward,
                ),
            ) => {
                let mv_msg = match pop_out_msg {
                    PopOutMessage::MessageView(m) => m,
                    _ => return Task::none(),
                };
                self.open_compose_from_message_view(window_id, mv_msg)
            }
            // Save As — needs db access from App
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(MessageViewMessage::SaveAs),
            ) => self.handle_save_as(window_id),
            // SetRenderingMode to Source — may need lazy load
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(
                    MessageViewMessage::SetRenderingMode(RenderingMode::Source),
                ),
            ) => self.handle_set_source_mode(window_id),
            // All other message view messages
            (PopOutWindow::MessageView(state), PopOutMessage::MessageView(_)) => {
                let PopOutMessage::MessageView(msg) = pop_out_msg else {
                    return Task::none();
                };
                handle_message_view_update(state, msg)
            }
            // Compose discard
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(ComposeMessage::Discard),
            ) => {
                self.pop_out_windows.remove(&window_id);
                self.composer_is_open = false;
                iced::window::close(window_id)
            }
            // All other compose messages
            (PopOutWindow::Compose(state), PopOutMessage::Compose(_)) => {
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                crate::pop_out::compose::update_compose(state, msg);
                Task::none()
            }
            _ => Task::none(),
        }
    }
}

// ── Message view update (free function) ─────────────────

fn handle_message_view_update(
    state: &mut MessageViewState,
    msg: MessageViewMessage,
) -> Task<Message> {
    match msg {
        MessageViewMessage::BodyLoaded(gen, _) if !state.is_current_generation(gen) => {
            Task::none() // Stale load — ignore
        }
        MessageViewMessage::BodyLoaded(_, Ok((body_text, body_html))) => {
            state.body_text = body_text;
            state.body_html = body_html;
            Task::none()
        }
        MessageViewMessage::BodyLoaded(_, Err(e)) => {
            eprintln!("Pop-out body load failed: {e}");
            state.error_banner = Some(
                "This message is no longer available. It may have been \
                 deleted or moved."
                    .to_string(),
            );
            Task::none()
        }
        MessageViewMessage::AttachmentsLoaded(gen, _) if !state.is_current_generation(gen) => {
            Task::none() // Stale load — ignore
        }
        MessageViewMessage::AttachmentsLoaded(_, Ok(attachments)) => {
            state.attachments = attachments;
            Task::none()
        }
        MessageViewMessage::AttachmentsLoaded(_, Err(e)) => {
            eprintln!("Pop-out attachments load failed: {e}");
            Task::none()
        }
        MessageViewMessage::RawSourceLoaded(Ok(source)) => {
            state.raw_source = Some(source);
            Task::none()
        }
        MessageViewMessage::RawSourceLoaded(Err(_)) => {
            state.raw_source = Some("(failed to load source)".to_string());
            Task::none()
        }
        MessageViewMessage::SetRenderingMode(mode) => {
            state.rendering_mode = mode;
            Task::none()
        }
        MessageViewMessage::ToggleOverflowMenu => {
            state.overflow_menu_open = !state.overflow_menu_open;
            Task::none()
        }
        MessageViewMessage::LoadRemoteContent => {
            state.remote_content_loaded = true;
            Task::none()
        }
        MessageViewMessage::Reply
        | MessageViewMessage::ReplyAll
        | MessageViewMessage::Forward
        | MessageViewMessage::Archive
        | MessageViewMessage::Delete
        | MessageViewMessage::Print
        | MessageViewMessage::SaveAs
        | MessageViewMessage::Noop => Task::none(),
    }
}

// ── Open windows ────────────────────────────────────────

impl App {
    /// Open a message view pop-out for the message at `message_index` in the
    /// reading pane's thread messages list.
    pub(crate) fn open_message_view_window(
        &mut self,
        message_index: usize,
    ) -> Task<Message> {
        let Some(msg) = self
            .reading_pane
            .thread_messages
            .get(message_index)
        else {
            return Task::none();
        };

        let generation = self.next_pop_out_generation();
        let state = MessageViewState::from_thread_message(msg, generation);
        let account_id = state.account_id.clone();
        let message_id = state.message_id.clone();

        let settings = iced::window::Settings {
            size: Size::new(
                MESSAGE_VIEW_DEFAULT_WIDTH,
                MESSAGE_VIEW_DEFAULT_HEIGHT,
            ),
            min_size: Some(Size::new(
                MESSAGE_VIEW_MIN_WIDTH,
                MESSAGE_VIEW_MIN_HEIGHT,
            )),
            exit_on_close_request: false,
            ..Default::default()
        };

        let (window_id, open_task) = iced::window::open(settings);
        self.pop_out_windows
            .insert(window_id, PopOutWindow::MessageView(state));

        self.dispatch_message_view_loads(
            window_id,
            generation,
            account_id,
            message_id,
            open_task,
        )
    }

    /// Open a compose window with the given mode.
    pub(crate) fn open_compose_window(
        &mut self,
        mode: ComposeMode,
    ) -> Task<Message> {
        let state = ComposeState::new(&self.sidebar.accounts);
        self.open_compose_window_with_state(state, mode)
    }

    /// Open a compose window with pre-built state and mode.
    pub(crate) fn open_compose_window_with_state(
        &mut self,
        mut state: ComposeState,
        mode: ComposeMode,
    ) -> Task<Message> {
        state.mode = mode;
        state.subject = state.mode.prefixed_subject();

        let settings = iced::window::Settings {
            size: Size::new(COMPOSE_DEFAULT_WIDTH, COMPOSE_DEFAULT_HEIGHT),
            min_size: Some(Size::new(COMPOSE_MIN_WIDTH, COMPOSE_MIN_HEIGHT)),
            exit_on_close_request: false,
            ..Default::default()
        };

        let (window_id, open_task) = iced::window::open(settings);
        self.pop_out_windows
            .insert(window_id, PopOutWindow::Compose(state));
        self.composer_is_open = true;

        open_task.discard()
    }

    /// Open a compose window from a message view's Reply/ReplyAll/Forward.
    pub(crate) fn open_compose_from_message_view(
        &mut self,
        window_id: iced::window::Id,
        action: MessageViewMessage,
    ) -> Task<Message> {
        let Some(PopOutWindow::MessageView(mv)) =
            self.pop_out_windows.get(&window_id)
        else {
            return Task::none();
        };

        let subject = mv.subject.clone().unwrap_or_default();
        let mode = match action {
            MessageViewMessage::Reply => ComposeMode::Reply {
                original_subject: subject,
            },
            MessageViewMessage::ReplyAll => ComposeMode::ReplyAll {
                original_subject: subject,
            },
            MessageViewMessage::Forward => ComposeMode::Forward {
                original_subject: subject,
            },
            _ => return Task::none(),
        };

        let state = ComposeState::new_reply(
            &self.sidebar.accounts,
            mode.clone(),
            mv.from_address.as_deref(),
            mv.from_name.as_deref(),
            mv.cc_addresses.as_deref(),
            mv.body_text.as_deref().or(mv.snippet.as_deref()),
            Some(&mv.thread_id),
            Some(&mv.message_id),
        );

        self.open_compose_window_with_state(state, mode)
    }

    /// Handle compose actions from command dispatch (Reply/ReplyAll/Forward
    /// from the main window's reading pane context).
    pub(crate) fn handle_compose_action(
        &mut self,
        action: crate::command_dispatch::ComposeAction,
    ) -> Task<Message> {
        let selected_thread = self
            .thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx));
        let last_message = self.reading_pane.thread_messages.first();

        let subject = selected_thread
            .and_then(|t| t.subject.clone())
            .unwrap_or_default();

        let mode = match action {
            crate::command_dispatch::ComposeAction::Reply => {
                ComposeMode::Reply {
                    original_subject: subject,
                }
            }
            crate::command_dispatch::ComposeAction::ReplyAll => {
                ComposeMode::ReplyAll {
                    original_subject: subject,
                }
            }
            crate::command_dispatch::ComposeAction::Forward => {
                ComposeMode::Forward {
                    original_subject: subject,
                }
            }
        };

        let to_email = last_message.and_then(|m| m.from_address.as_deref());
        let to_name = last_message.and_then(|m| m.from_name.as_deref());
        let cc_emails = last_message.and_then(|m| m.cc_addresses.as_deref());
        let thread_id = selected_thread.map(|t| t.id.as_str());
        let message_id = last_message.map(|m| m.id.as_str());
        let snippet = last_message.and_then(|m| m.snippet.as_deref());

        let state = ComposeState::new_reply(
            &self.sidebar.accounts,
            mode.clone(),
            to_email,
            to_name,
            cc_emails,
            snippet,
            thread_id,
            message_id,
        );

        self.open_compose_window_with_state(state, mode)
    }
}

// ── Async data loads ────────────────────────────────────

impl App {
    /// Dispatch body + attachment loads for a message view window.
    fn dispatch_message_view_loads(
        &self,
        window_id: iced::window::Id,
        generation: u64,
        account_id: String,
        message_id: String,
        open_task: Task<iced::window::Id>,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let db2 = Arc::clone(&self.db);
        let account_id2 = account_id.clone();
        let message_id2 = message_id.clone();

        Task::batch([
            open_task.discard(),
            Task::perform(
                async move {
                    db.load_message_body(account_id, message_id).await
                },
                move |result| {
                    Message::PopOut(
                        window_id,
                        PopOutMessage::MessageView(
                            MessageViewMessage::BodyLoaded(generation, result),
                        ),
                    )
                },
            ),
            Task::perform(
                async move {
                    db2.load_message_attachments(account_id2, message_id2)
                        .await
                },
                move |result| {
                    Message::PopOut(
                        window_id,
                        PopOutMessage::MessageView(
                            MessageViewMessage::AttachmentsLoaded(
                                generation, result,
                            ),
                        ),
                    )
                },
            ),
        ])
    }

    /// Handle switching to Source rendering mode — lazy-loads raw source if needed.
    fn handle_set_source_mode(
        &mut self,
        window_id: iced::window::Id,
    ) -> Task<Message> {
        let Some(PopOutWindow::MessageView(state)) =
            self.pop_out_windows.get_mut(&window_id)
        else {
            return Task::none();
        };

        let needs_source = state.raw_source.is_none();
        state.rendering_mode = RenderingMode::Source;

        if needs_source {
            let db = Arc::clone(&self.db);
            let account_id = state.account_id.clone();
            let message_id = state.message_id.clone();

            Task::perform(
                async move {
                    db.load_raw_source(account_id, message_id).await
                },
                move |result| {
                    Message::PopOut(
                        window_id,
                        PopOutMessage::MessageView(
                            MessageViewMessage::RawSourceLoaded(result),
                        ),
                    )
                },
            )
        } else {
            Task::none()
        }
    }

    /// Handle Save As action from the overflow menu.
    fn handle_save_as(
        &self,
        window_id: iced::window::Id,
    ) -> Task<Message> {
        let Some(PopOutWindow::MessageView(state)) =
            self.pop_out_windows.get(&window_id)
        else {
            return Task::none();
        };

        let db = Arc::clone(&self.db);
        let account_id = state.account_id.clone();
        let message_id = state.message_id.clone();
        let subject = state
            .subject
            .clone()
            .unwrap_or_else(|| "message".to_string());

        Task::perform(
            async move {
                save_message_dialog(db, account_id, message_id, subject).await
            },
            move |_result| {
                Message::PopOut(
                    window_id,
                    PopOutMessage::MessageView(MessageViewMessage::Noop),
                )
            },
        )
    }

    /// Increment and return the next pop-out generation counter.
    fn next_pop_out_generation(&mut self) -> u64 {
        self.pop_out_generation += 1;
        self.pop_out_generation
    }
}

// ── Session save/restore ────────────────────────────────

impl App {
    /// Save the full session state (main window + all pop-out windows) to disk.
    pub(crate) fn save_session_state(&self) {
        let data_dir = APP_DATA_DIR.get().expect("APP_DATA_DIR not set");

        let message_views: Vec<MessageViewSessionEntry> = self
            .pop_out_windows
            .values()
            .filter_map(|w| match w {
                PopOutWindow::MessageView(state) => {
                    Some(MessageViewSessionEntry {
                        message_id: state.message_id.clone(),
                        thread_id: state.thread_id.clone(),
                        account_id: state.account_id.clone(),
                        width: state.width,
                        height: state.height,
                        x: state.x,
                        y: state.y,
                    })
                }
                PopOutWindow::Compose(_) => None,
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
    pub(crate) fn restore_pop_out_windows(
        &mut self,
        session: &SessionState,
    ) -> Vec<Task<Message>> {
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
                min_size: Some(Size::new(
                    MESSAGE_VIEW_MIN_WIDTH,
                    MESSAGE_VIEW_MIN_HEIGHT,
                )),
                exit_on_close_request: false,
                ..Default::default()
            };

            let (window_id, open_task) = iced::window::open(settings);

            let generation = self.next_pop_out_generation();
            let state = MessageViewState::from_session_entry(entry, generation);
            let account_id = entry.account_id.clone();
            let message_id = entry.message_id.clone();

            self.pop_out_windows
                .insert(window_id, PopOutWindow::MessageView(state));

            tasks.push(self.dispatch_message_view_loads(
                window_id,
                generation,
                account_id,
                message_id,
                open_task,
            ));
        }

        tasks
    }
}

// ── Save As (.eml / .txt) ───────────────────────────────

/// Sanitize a subject line for use as a filename.
fn sanitize_filename(subject: &str) -> String {
    let safe: String = subject
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = safe.trim().to_string();
    if trimmed.is_empty() {
        "message".to_string()
    } else {
        trimmed
    }
}

/// Open a file picker and save the message as .eml or .txt.
async fn save_message_dialog(
    db: Arc<Db>,
    account_id: String,
    message_id: String,
    subject: String,
) -> Result<(), String> {
    let safe_name = sanitize_filename(&subject);

    // Build a simple save dialog using std::fs since rfd is not yet a dependency.
    // When rfd is added, this will use AsyncFileDialog.
    // For now, fall back to writing to the user's home directory with a prompt-free approach.
    let home = dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| "Could not determine download directory".to_string())?;

    let eml_path = home.join(format!("{safe_name}.eml"));
    let raw = db
        .load_raw_source(account_id.clone(), message_id.clone())
        .await?;
    std::fs::write(&eml_path, raw.as_bytes())
        .map_err(|e| format!("Write failed: {e}"))?;

    // Also write .txt version
    let txt_path = home.join(format!("{safe_name}.txt"));
    let (body_text, _body_html) = db
        .load_message_body(account_id, message_id)
        .await?;
    let txt_content = body_text.unwrap_or_default();
    std::fs::write(&txt_path, txt_content.as_bytes())
        .map_err(|e| format!("Write failed: {e}"))?;

    Ok(())
}
