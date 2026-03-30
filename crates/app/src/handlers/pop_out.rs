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
use std::time::Duration;

use iced::{Point, Size, Task};

pub const DRAFT_AUTO_SAVE_INTERVAL: Duration = Duration::from_secs(30);

use crate::db::Db;
use crate::pop_out::compose::{
    ComposeAttachment, ComposeMessage, ComposeMode, ComposeState,
};
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

use ratatoskr_core::actions::SendAttachment;
use rusqlite::OptionalExtension;

// ── Pop-out message dispatch ────────────────────────────

impl App {
    /// Route a `PopOutMessage` to the correct pop-out window handler.
    pub(crate) fn handle_pop_out_message(
        &mut self,
        window_id: iced::window::Id,
        pop_out_msg: PopOutMessage,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
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
                self.open_compose_from_message_view(window_id, &mv_msg)
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
            // Archive — route through action service for DB + provider dispatch
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(MessageViewMessage::Archive),
            ) => {
                return self.dispatch_pop_out_action(window_id, crate::CompletedAction::Archive);
            }
            // Delete — route through action service for DB + provider dispatch
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(MessageViewMessage::Delete),
            ) => {
                return self.dispatch_pop_out_action(window_id, crate::CompletedAction::Trash);
            }
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
                iced::window::close(window_id)
            }
            // Compose send — build MIME, queue for outbox, close window
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(ComposeMessage::Send),
            ) => self.handle_compose_send(window_id),
            // Compose from-account changed — swap signature for new account
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(ComposeMessage::FromAccountChanged(_)),
            ) => {
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                self.handle_compose_from_account_changed(window_id, msg)
            }
            // Compose attach files — launch async file picker
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(ComposeMessage::AttachFiles),
            ) => handle_compose_attach_files(window_id),
            // Compose expand group — needs DB access
            (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::ContextMenuExpandGroup {
                    ..
                }),
            ) => {
                let PopOutMessage::Compose(ComposeMessage::ContextMenuExpandGroup {
                    field,
                    token_id,
                }) = pop_out_msg
                else {
                    return Task::none();
                };
                // Find the group_id from the token
                let group_id = {
                    let tokens = match field {
                        crate::pop_out::compose::RecipientField::To => {
                            &state.to.tokens
                        }
                        crate::pop_out::compose::RecipientField::Cc => {
                            &state.cc.tokens
                        }
                        crate::pop_out::compose::RecipientField::Bcc => {
                            &state.bcc.tokens
                        }
                    };
                    tokens
                        .iter()
                        .find(|t| t.id == token_id)
                        .and_then(|t| t.group_id.clone())
                };
                let Some(gid) = group_id else {
                    return Task::none();
                };
                let db_clone = Arc::clone(&db);
                Task::perform(
                    async move {
                        db_clone
                            .expand_contact_group(gid)
                            .await
                    },
                    move |result| {
                        Message::PopOut(
                            window_id,
                            PopOutMessage::Compose(
                                ComposeMessage::GroupExpanded {
                                    field,
                                    token_id,
                                    members: result,
                                },
                            ),
                        )
                    },
                )
            }
            // All other compose messages
            (PopOutWindow::Compose(state), PopOutMessage::Compose(_)) => {
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                let trigger_autocomplete =
                    crate::handlers::contacts::should_trigger_autocomplete(&msg);
                crate::pop_out::compose::update_compose(state, msg);
                if trigger_autocomplete {
                    crate::handlers::contacts::dispatch_autocomplete_search(
                        &db, window_id, state,
                    )
                } else {
                    Task::none()
                }
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
        MessageViewMessage::BodyLoaded(generation, _) if !state.is_current_generation(generation) => {
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
        MessageViewMessage::AttachmentsLoaded(generation, _) if !state.is_current_generation(generation) => {
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

// ── Compose: attach files ───────────────────────────────

/// Launch an async file picker and return the selected files as attachments.
fn handle_compose_attach_files(
    window_id: iced::window::Id,
) -> Task<Message> {
    Task::perform(
        async {
            let handles = rfd::AsyncFileDialog::new()
                .set_title("Attach Files")
                .pick_files()
                .await;

            let Some(handles) = handles else {
                return Vec::new();
            };

            let mut attachments = Vec::new();
            for handle in handles {
                let name = handle.file_name();
                let data = handle.read().await;
                let mime_type =
                    crate::pop_out::compose::mime_from_extension(&name);
                attachments.push(ComposeAttachment {
                    name,
                    mime_type,
                    data: Arc::new(data),
                });
            }
            attachments
        },
        move |files| {
            if files.is_empty() {
                // User cancelled — no-op
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::FilesSelected(
                        Vec::new(),
                    )),
                )
            } else {
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::FilesSelected(
                        files,
                    )),
                )
            }
        },
    )
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
            .cloned()
        else {
            return Task::none();
        };

        let generation = self.next_pop_out_generation();
        let source_label_id = self.sidebar.selected_label.clone();
        let state = MessageViewState::from_thread_message(&msg, generation, source_label_id, self.settings.default_rendering_mode);
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
            .insert(window_id, PopOutWindow::MessageView(Box::new(state)));

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
    /// If a compose window already exists for the same reply thread,
    /// focuses it instead of opening a duplicate.
    pub(crate) fn open_compose_window_with_state(
        &mut self,
        mut state: ComposeState,
        mode: ComposeMode,
    ) -> Task<Message> {
        // Dedup: if replying to a thread that already has a compose window, focus it
        if let Some(ref tid) = state.reply_thread_id {
            if let Some((&existing_id, _)) = self.pop_out_windows.iter().find(|(_, w)| {
                matches!(w, PopOutWindow::Compose(s) if s.reply_thread_id.as_deref() == Some(tid))
            }) {
                return iced::window::gain_focus(existing_id);
            }
        }

        // Auto-select shared mailbox identity when replying from shared
        // mailbox context.  Query the cached email address synchronously.
        if let ratatoskr_core::scope::ViewScope::SharedMailbox {
            ref account_id,
            ref mailbox_id,
        } = self.sidebar.selected_scope
        {
            if let Ok(Some(shared_email)) = self.db.with_conn_sync(|conn| {
                conn.query_row(
                    "SELECT email_address FROM shared_mailbox_sync_state \
                     WHERE account_id = ?1 AND mailbox_id = ?2",
                    rusqlite::params![account_id, mailbox_id],
                    |row| row.get::<_, Option<String>>(0),
                )
                .optional()
                .map_err(|e| format!("shared mailbox email: {e}"))
                .map(|opt| opt.flatten())
            }) {
                state.set_shared_mailbox_from(account_id, &shared_email);
            }
        }

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
            .insert(window_id, PopOutWindow::Compose(Box::new(state)));

        // Dispatch initial signature resolution for the default From account.
        let sig_task = self.resolve_compose_signature(window_id);

        Task::batch([open_task.discard(), sig_task])
    }

    /// Open a compose window from a message view's Reply/ReplyAll/Forward.
    pub(crate) fn open_compose_from_message_view(
        &mut self,
        window_id: iced::window::Id,
        action: &MessageViewMessage,
    ) -> Task<Message> {
        let Some(PopOutWindow::MessageView(mv)) =
            self.pop_out_windows.get(&window_id)
        else {
            return Task::none();
        };

        let subject = mv.subject.clone().unwrap_or_default();
        let mode = match *action {
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
            &mode,
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
        action: &crate::command_dispatch::ComposeAction,
    ) -> Task<Message> {
        let selected_thread = self
            .thread_list
            .selected_thread
            .and_then(|idx| self.thread_list.threads.get(idx));
        let last_message = self.reading_pane.thread_messages.first();

        let subject = selected_thread
            .and_then(|t| t.subject.clone())
            .unwrap_or_default();

        let mode = match *action {
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
            &mode,
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
        generation: ratatoskr_core::generation::GenerationToken<ratatoskr_core::generation::PopOut>,
        account_id: String,
        message_id: String,
        open_task: Task<iced::window::Id>,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        let db2 = Arc::clone(&self.db);
        let account_id2 = account_id.clone();
        let message_id2 = message_id.clone();
        let body_store = self.body_store.clone();

        Task::batch([
            open_task.discard(),
            Task::perform(
                async move {
                    // Try body store first (has full zstd-decompressed bodies),
                    // fall back to DB snippet if body store unavailable.
                    if let Some(bs) = body_store {
                        if let Ok(Some(body)) = bs.get(message_id.clone()).await {
                            return Ok((body.body_text, body.body_html));
                        }
                    }
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

    /// Dispatch an action-service operation for the thread shown in a pop-out
    /// message view. Extracts thread context from the pop-out state, closes the
    /// overflow menu, then delegates to the action service.
    fn dispatch_pop_out_action(
        &mut self,
        window_id: iced::window::Id,
        action: crate::CompletedAction,
    ) -> Task<Message> {
        let Some(PopOutWindow::MessageView(state)) = self.pop_out_windows.get_mut(&window_id)
        else {
            return Task::none();
        };
        let threads = vec![(state.account_id.clone(), state.thread_id.clone())];
        let source_label_id = state.source_label_id.clone();
        state.overflow_menu_open = false;
        drop(state);

        // I5: pop-out only reaches Archive/Trash/PermanentDelete — no optimistic toggles.
        use crate::action_resolve::{self as ar, MailActionIntent, UiContext};
        let intent = match action {
            crate::CompletedAction::Archive => MailActionIntent::Archive,
            crate::CompletedAction::Trash => MailActionIntent::Trash,
            crate::CompletedAction::PermanentDelete => MailActionIntent::PermanentDelete,
            other => {
                log::error!("dispatch_pop_out_action: unexpected action {other:?}");
                return Task::none();
            }
        };
        let ui_ctx = UiContext { selected_label: source_label_id };
        let outcome = ar::resolve_intent(intent, &ui_ctx);
        let Some(plan) = ar::build_execution_plan(
            outcome,
            &threads,
            &mut self.thread_list,
        ) else {
            return Task::none();
        };
        self.dispatch_plan(plan)
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

    /// Increment and return the next pop-out generation token.
    fn next_pop_out_generation(&mut self) -> ratatoskr_core::generation::GenerationToken<ratatoskr_core::generation::PopOut> {
        self.pop_out_generation.next()
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
            let state = MessageViewState::from_session_entry(entry, generation, self.settings.default_rendering_mode);
            let account_id = entry.account_id.clone();
            let message_id = entry.message_id.clone();

            self.pop_out_windows
                .insert(window_id, PopOutWindow::MessageView(Box::new(state)));

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

/// Open a native file picker and save the message as .eml or .txt.
async fn save_message_dialog(
    db: Arc<Db>,
    account_id: String,
    message_id: String,
    subject: String,
) -> Result<(), String> {
    let safe_name = sanitize_filename(&subject);

    let file_handle = rfd::AsyncFileDialog::new()
        .set_title("Save Message As")
        .set_file_name(format!("{safe_name}.eml"))
        .add_filter("Email Message (.eml)", &["eml"])
        .add_filter("Plain Text (.txt)", &["txt"])
        .save_file()
        .await;

    let Some(handle) = file_handle else {
        return Ok(()); // user cancelled
    };

    let path = handle.path().to_path_buf();
    let extension = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("eml");

    match extension {
        "txt" => {
            let (body_text, _body_html) = db
                .load_message_body(account_id, message_id)
                .await?;
            let txt_content = body_text.unwrap_or_default();
            std::fs::write(&path, txt_content.as_bytes())
                .map_err(|e| format!("Write failed: {e}"))?;
        }
        _ => {
            // Default to .eml
            let raw = db
                .load_raw_source(account_id, message_id)
                .await?;
            std::fs::write(&path, raw.as_bytes())
                .map_err(|e| format!("Write failed: {e}"))?;
        }
    }

    Ok(())
}

// ── Draft auto-save helpers ─────────────────────────────

/// Convert token input tokens into a comma-separated string of email addresses.
fn tokens_to_csv(tokens: &[crate::ui::token_input::Token]) -> Option<String> {
    if tokens.is_empty() {
        return None;
    }
    Some(
        tokens
            .iter()
            .map(|t| t.email.as_str())
            .collect::<Vec<_>>()
            .join(","),
    )
}

/// Data extracted from a `ComposeState` for draft persistence.
struct DraftData {
    draft_id: String,
    account_id: String,
    to_csv: Option<String>,
    cc_csv: Option<String>,
    bcc_csv: Option<String>,
    subject: Option<String>,
    body_html: Option<String>,
    from_email: Option<String>,
    reply_to_message_id: Option<String>,
    thread_id: Option<String>,
    signature_id: Option<String>,
    signature_separator_index: Option<i64>,
}

impl DraftData {
    fn from_compose(state: &crate::pop_out::compose::ComposeState) -> Self {
        let account_id = state
            .from_account
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        let body_text = state.body.to_html();
        #[allow(clippy::cast_possible_wrap)]
        let sep_idx = state.signature_separator_index.map(|i| i as i64);
        Self {
            draft_id: state.draft_id.clone(),
            account_id,
            to_csv: tokens_to_csv(&state.to.tokens),
            cc_csv: tokens_to_csv(&state.cc.tokens),
            bcc_csv: tokens_to_csv(&state.bcc.tokens),
            subject: if state.subject.is_empty() {
                None
            } else {
                Some(state.subject.clone())
            },
            body_html: if body_text.trim().is_empty() {
                None
            } else {
                Some(body_text)
            },
            from_email: state.from_account.as_ref().map(|a| a.email.clone()),
            reply_to_message_id: state.reply_message_id.clone(),
            thread_id: state.reply_thread_id.clone(),
            signature_id: state.active_signature_id.clone(),
            signature_separator_index: sep_idx,
        }
    }

    fn execute(self, conn: &rusqlite::Connection) -> Result<(), String> {
        conn.execute(
            "INSERT INTO local_drafts \
             (id, account_id, to_addresses, cc_addresses, bcc_addresses, \
              subject, body_html, reply_to_message_id, thread_id, \
              from_email, signature_id, signature_separator_index, \
              updated_at, sync_status) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, \
                     unixepoch(), 'pending') \
             ON CONFLICT(id) DO UPDATE SET \
               account_id = ?2, \
               to_addresses = ?3, cc_addresses = ?4, bcc_addresses = ?5, \
               subject = ?6, body_html = ?7, reply_to_message_id = ?8, \
               thread_id = ?9, from_email = ?10, signature_id = ?11, \
               signature_separator_index = ?12, \
               updated_at = unixepoch(), sync_status = 'pending'",
            rusqlite::params![
                self.draft_id,
                self.account_id,
                self.to_csv,
                self.cc_csv,
                self.bcc_csv,
                self.subject,
                self.body_html,
                self.reply_to_message_id,
                self.thread_id,
                self.from_email,
                self.signature_id,
                self.signature_separator_index,
            ],
        )
        .map_err(|e| e.to_string())?;
        Ok(())
    }
}

impl App {
    /// Returns true if at least one compose pop-out window exists.
    /// Computed from `pop_out_windows` — no manual bookkeeping needed.
    pub(crate) fn composer_is_open(&self) -> bool {
        self.pop_out_windows
            .values()
            .any(|w| matches!(w, PopOutWindow::Compose(_)))
    }

    /// Returns true if any compose window has `draft_dirty` set.
    pub(crate) fn has_dirty_compose_drafts(&self) -> bool {
        self.pop_out_windows.values().any(|w| {
            matches!(w, PopOutWindow::Compose(s) if s.draft_dirty)
        })
    }

    /// Save a single compose window's state as a local draft (async).
    /// Used by the periodic auto-save timer.
    fn save_compose_draft(&mut self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };
        if state.from_account.is_none() {
            return Task::none();
        }
        state.draft_dirty = false;
        let data = DraftData::from_compose(state);
        let db = Arc::clone(&self.db);

        Task::perform(
            async move { db.with_write_conn(move |conn| data.execute(&conn)).await },
            move |result| {
                if let Err(e) = result {
                    log::error!("Failed to auto-save compose draft: {e}");
                }
                Message::Noop
            },
        )
    }

    /// Synchronously save a compose window's draft before the window is
    /// destroyed. Used on window close where an async Task would race
    /// against `iced::exit()`. A single-row INSERT is sub-millisecond.
    ///
    /// Returns `true` if the draft was saved (or didn't need saving),
    /// `false` if the write failed and the draft is still dirty.
    pub(crate) fn save_compose_draft_sync(&mut self, window_id: iced::window::Id) -> bool {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return true;
        };
        if !state.draft_dirty || state.from_account.is_none() {
            return true;
        }
        let data = DraftData::from_compose(state);

        let result = self.db.with_write_conn_sync(|conn| data.execute(conn));
        match result {
            Ok(()) => {
                if let Some(PopOutWindow::Compose(state)) =
                    self.pop_out_windows.get_mut(&window_id)
                {
                    state.draft_dirty = false;
                }
                true
            }
            Err(e) => {
                log::error!("Failed to save compose draft on close: {e}");
                false
            }
        }
    }

    // ── Compose send ─────────────────────────────────────

    /// Build a MIME message from the compose state, save it to the draft row
    /// as base64url in the `attachments` column, mark the draft `'queued'`,
    /// Validate compose state, build a SendRequest, and dispatch to the
    /// action service. The compose window stays open with a "Sending..." status
    /// until SendCompleted arrives.
    fn handle_compose_send(
        &mut self,
        window_id: iced::window::Id,
    ) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) =
            self.pop_out_windows.get_mut(&window_id)
        else {
            return Task::none();
        };

        // Prevent double-send
        if state.sending {
            return Task::none();
        }

        // ── Validate recipients ─────────────────────────
        let has_recipients = !state.to.tokens.is_empty()
            || !state.cc.tokens.is_empty()
            || !state.bcc.tokens.is_empty();
        if !has_recipients {
            state.status =
                Some("Add at least one recipient".to_string());
            return Task::none();
        }

        // ── Build SendRequest from compose state ────────
        let account_info = match state.from_account.as_ref() {
            Some(a) => a.clone(),
            None => {
                state.status = Some("No sending account selected".to_string());
                return Task::none();
            }
        };

        let from = if let Some(ref name) = account_info.display_name {
            format!("{name} <{}>", account_info.email)
        } else {
            account_info.email.clone()
        };

        let to: Vec<String> = state
            .to
            .tokens
            .iter()
            .map(|t| t.email.clone())
            .collect();
        let cc: Vec<String> = state
            .cc
            .tokens
            .iter()
            .map(|t| t.email.clone())
            .collect();
        let bcc: Vec<String> = state
            .bcc
            .tokens
            .iter()
            .map(|t| t.email.clone())
            .collect();

        let subject = if state.subject.is_empty() {
            None
        } else {
            Some(state.subject.clone())
        };

        let body_html = state.body.to_html();
        let body_text = state.body.document.flattened_text();

        let attachments: Vec<SendAttachment> = state
            .attachments
            .iter()
            .map(|a| SendAttachment {
                filename: a.name.clone(),
                mime_type: a.mime_type.clone(),
                data: a.data.as_ref().clone(),
                content_id: None,
            })
            .collect();

        // Reuse draft_id on retry so the action updates the existing
        // 'failed' row instead of creating a new one.
        let draft_id = state
            .send_draft_id
            .get_or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();

        let send_req = ratatoskr_core::actions::SendRequest {
            draft_id,
            account_id: account_info.id.clone(),
            from,
            to,
            cc,
            bcc,
            subject,
            body_html,
            body_text,
            attachments,
            in_reply_to: state.reply_message_id.clone(),
            references: state.reply_message_id.clone(),
            thread_id: state.reply_thread_id.clone(),
        };

        // ── Set sending state and dispatch ──────────────
        state.sending = true;
        state.status = Some("Sending\u{2026}".to_string());

        self.dispatch_send(window_id, send_req)
    }

    /// Dispatch send_email through the action service.
    fn dispatch_send(
        &mut self,
        window_id: iced::window::Id,
        request: ratatoskr_core::actions::SendRequest,
    ) -> Task<Message> {
        let Some(ctx) = self.action_ctx() else {
            if let Some(PopOutWindow::Compose(state)) =
                self.pop_out_windows.get_mut(&window_id)
            {
                state.sending = false;
                state.status = Some(
                    "Send unavailable \u{2014} action service not initialized"
                        .to_string(),
                );
            }
            return Task::none();
        };
        Task::perform(
            async move {
                let outcome =
                    ratatoskr_core::actions::send_email(&ctx, request).await;
                (window_id, outcome)
            },
            move |(window_id, outcome)| Message::SendCompleted {
                window_id,
                outcome,
            },
        )
    }

    /// Handle send completion: close compose on success, restore on failure.
    pub(crate) fn handle_send_completed(
        &mut self,
        window_id: iced::window::Id,
        outcome: &ratatoskr_core::actions::ActionOutcome,
    ) -> Task<Message> {
        match outcome {
            ratatoskr_core::actions::ActionOutcome::Success
            | ratatoskr_core::actions::ActionOutcome::NoOp => {
                self.pop_out_windows.remove(&window_id);
                self.status_bar
                    .show_confirmation("Message sent".to_string());
                iced::window::close(window_id)
            }
            // LocalOnly should not occur for send (send uses Failed for all
            // failures), but handle it defensively as failure for safety.
            ratatoskr_core::actions::ActionOutcome::Failed { error }
            | ratatoskr_core::actions::ActionOutcome::LocalOnly {
                reason: error, ..
            } => {
                if let Some(PopOutWindow::Compose(state)) =
                    self.pop_out_windows.get_mut(&window_id)
                {
                    state.sending = false;
                    state.status =
                        Some(format!("Send failed: {}", error.user_message()));
                }
                Task::none()
            }
        }
    }

    // ── Compose signature resolution ───────────────────────

    /// When the user changes the "From" account, update the compose state
    /// and dispatch a signature resolution task for the new account.
    fn handle_compose_from_account_changed(
        &mut self,
        window_id: iced::window::Id,
        msg: ComposeMessage,
    ) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) =
            self.pop_out_windows.get_mut(&window_id)
        else {
            return Task::none();
        };

        // Let the standard update handler process the account change first
        // (sets state.from_account to the new account).
        crate::pop_out::compose::update_compose(state, msg);

        self.resolve_compose_signature(window_id)
    }

    /// Resolve the default signature for the current From account of a
    /// compose window and dispatch a `SignatureResolved` message.
    ///
    /// Used both on initial compose window open and when the user switches
    /// the From account.
    fn resolve_compose_signature(
        &self,
        window_id: iced::window::Id,
    ) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) =
            self.pop_out_windows.get(&window_id)
        else {
            return Task::none();
        };

        let Some(ref account) = state.from_account else {
            return Task::none();
        };

        let account_id = account.id.clone();
        let from_email = Some(account.email.clone());
        let is_reply = matches!(
            state.mode,
            ComposeMode::Reply { .. }
                | ComposeMode::ReplyAll { .. }
        );

        let db = Arc::clone(&self.db);

        Task::perform(
            async move {
                let core_db =
                    ratatoskr_core::db::DbState::from_arc(db.conn_arc());
                let sig = ratatoskr_core::db::queries_extra::db_resolve_signature_for_compose(
                    &core_db,
                    account_id,
                    from_email,
                    is_reply,
                )
                .await?;
                Ok::<_, String>(sig)
            },
            move |result| {
                let (sig_id, sig_html) = match result {
                    Ok(Some(sig)) => (Some(sig.id), Some(sig.body_html)),
                    Ok(None) => (None, None),
                    Err(e) => {
                        log::error!("Signature resolution failed: {e}");
                        (None, None)
                    }
                };
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::SignatureResolved {
                        signature_id: sig_id,
                        signature_html: sig_html,
                    }),
                )
            },
        )
    }

    /// Auto-save all dirty compose drafts. Called from subscription tick.
    pub(crate) fn auto_save_compose_drafts(&mut self) -> Task<Message> {
        let dirty_windows: Vec<iced::window::Id> = self
            .pop_out_windows
            .iter()
            .filter_map(|(&id, w)| {
                if let PopOutWindow::Compose(s) = w {
                    if s.draft_dirty { Some(id) } else { None }
                } else {
                    None
                }
            })
            .collect();

        let mut tasks = Vec::new();
        for win_id in dirty_windows {
            tasks.push(self.save_compose_draft(win_id));
        }
        Task::batch(tasks)
    }
}
