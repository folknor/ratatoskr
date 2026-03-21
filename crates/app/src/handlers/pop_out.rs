//! Pop-out window handler methods for `App`.
//!
//! All pop-out window logic lives here. `main.rs` dispatches to these methods
//! via one-line match arms. This file owns:
//! - Message view update routing
//! - Compose update routing
//! - Window open/close for pop-outs
//! - Session save/restore
//! - Save As (.eml / .txt) flow
//! - Compose send, draft save, signature resolution, attachment handling

use std::sync::Arc;

use iced::{Point, Size, Task};
use rusqlite::params;

use crate::db::Db;
use crate::pop_out::compose::{
    ComposeMessage, ComposeMode, ComposeState, tokens_to_csv,
};
use crate::pop_out::message_view::{
    MessageViewMessage, MessageViewState, RenderingMode,
};
use crate::pop_out::session::{MessageViewSessionEntry, SessionState};
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::ui::layout::{
    COMPOSE_DEFAULT_HEIGHT, COMPOSE_DEFAULT_WIDTH, COMPOSE_MIN_HEIGHT,
    COMPOSE_MIN_WIDTH, MESSAGE_VIEW_DEFAULT_HEIGHT,
    MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_MIN_HEIGHT,
    MESSAGE_VIEW_MIN_WIDTH,
};
use crate::{App, Message, APP_DATA_DIR};

/// Duration between auto-save ticks for compose windows.
pub const DRAFT_AUTO_SAVE_INTERVAL: std::time::Duration =
    std::time::Duration::from_secs(30);

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
            // Reply / ReplyAll / Forward from message view -> open compose
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
            // Compose send — needs App context for finalization
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(ComposeMessage::Send),
            ) => self.handle_compose_send(window_id),
            // Compose attach files — needs to open file dialog
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(ComposeMessage::AttachFiles),
            ) => self.handle_compose_attach_files(window_id),
            // Compose draft save result — needs to clear dirty flag
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(ComposeMessage::DraftSaved(_)),
            ) => {
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                if let Some(PopOutWindow::Compose(state)) =
                    self.pop_out_windows.get_mut(&window_id)
                {
                    crate::pop_out::compose::update_compose(state, msg);
                }
                Task::none()
            }
            // All other compose messages — pure state update
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

    /// Returns true if any compose window has `draft_dirty` set.
    pub(crate) fn has_dirty_compose_drafts(&self) -> bool {
        self.pop_out_windows.values().any(|w| {
            matches!(w, PopOutWindow::Compose(s) if s.draft_dirty)
        })
    }

    /// Auto-save all dirty compose drafts. Called from subscription tick.
    pub(crate) fn auto_save_compose_drafts(
        &mut self,
    ) -> Task<Message> {
        let mut tasks = Vec::new();
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

        for win_id in dirty_windows {
            tasks.push(self.save_compose_draft(win_id));
        }
        Task::batch(tasks)
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
        let state = MessageViewState::from_thread_message(&msg, generation);
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

        // Resolve signature for the compose window
        let sig_task = self.resolve_signature_for_compose(window_id, &state);

        self.pop_out_windows
            .insert(window_id, PopOutWindow::Compose(state));
        self.composer_is_open = true;

        Task::batch([open_task.discard(), sig_task])
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

// ── Compose: signature resolution ───────────────────────

impl App {
    /// Resolve the appropriate signature for a compose window and dispatch
    /// a `SignatureResolved` message with the result.
    fn resolve_signature_for_compose(
        &self,
        window_id: iced::window::Id,
        state: &ComposeState,
    ) -> Task<Message> {
        let Some(ref from) = state.from_account else {
            return Task::none();
        };

        let db = Arc::clone(&self.db);
        let account_id = from.id.clone();
        let from_email = Some(from.email.clone());
        let is_reply = state.mode.is_reply();

        Task::perform(
            async move {
                resolve_signature_async(
                    db, account_id, from_email, is_reply,
                )
                .await
            },
            move |result| {
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(
                        ComposeMessage::SignatureResolved(result),
                    ),
                )
            },
        )
    }
}

/// Resolve signature from the database (runs on background thread).
async fn resolve_signature_async(
    db: Arc<Db>,
    account_id: String,
    from_email: Option<String>,
    is_reply: bool,
) -> Option<(String, String, Option<String>)> {
    let result = db
        .with_conn(move |conn| {
            // Resolution order:
            // 1. Alias-level signature override
            // 2. Reply-default or default signature
            let sig_from_alias = from_email.as_ref().and_then(|email| {
                conn.query_row(
                    "SELECT signature_id FROM send_as_aliases \
                     WHERE account_id = ?1 AND email = ?2 \
                     AND signature_id IS NOT NULL",
                    params![account_id, email],
                    |row| row.get::<_, String>(0),
                )
                .ok()
            });

            let sig_id = if let Some(ref alias_id) = sig_from_alias {
                alias_id.clone()
            } else {
                let sql = if is_reply {
                    "SELECT id FROM signatures \
                     WHERE account_id = ?1 \
                     AND (is_reply_default = 1 OR is_default = 1) \
                     ORDER BY is_reply_default DESC LIMIT 1"
                } else {
                    "SELECT id FROM signatures \
                     WHERE account_id = ?1 AND is_default = 1 LIMIT 1"
                };
                match conn.query_row(sql, params![account_id], |row| {
                    row.get::<_, String>(0)
                }) {
                    Ok(id) => id,
                    Err(_) => return Ok(None),
                }
            };

            // Fetch the full signature
            let result = conn.query_row(
                "SELECT id, body_html FROM signatures WHERE id = ?1",
                params![sig_id],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                    ))
                },
            );
            match result {
                Ok((id, html)) => Ok(Some((html, id))),
                Err(_) => Ok(None),
            }
        })
        .await
        .ok()
        .flatten();

    result.map(|(html, id)| (html, id, None))
}

// ── Compose: send path ──────────────────────────────────

impl App {
    /// Handle the Send action from a compose window.
    fn handle_compose_send(
        &mut self,
        window_id: iced::window::Id,
    ) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) =
            self.pop_out_windows.get_mut(&window_id)
        else {
            return Task::none();
        };

        // Validate recipients
        let has_recipients = !state.to.tokens.is_empty()
            || !state.cc.tokens.is_empty()
            || !state.bcc.tokens.is_empty();
        if !has_recipients {
            state.status =
                Some("Add at least one recipient".to_string());
            return Task::none();
        }

        state.status = Some("Preparing message...".to_string());

        // Extract what we need from state before the async boundary
        let db = Arc::clone(&self.db);
        let draft_id = state.draft_id.clone();
        let account_id = state
            .from_account
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        let from_email = state
            .from_account
            .as_ref()
            .map(|a| a.email.clone());
        let to_csv = tokens_to_csv(&state.to);
        let cc_csv = tokens_to_csv(&state.cc);
        let bcc_csv = tokens_to_csv(&state.bcc);
        let subject = if state.subject.is_empty() {
            None
        } else {
            Some(state.subject.clone())
        };
        let body_html = state.editor.to_html();
        let sig_id = state.active_signature_id.clone();
        let reply_msg_id = state.reply_message_id.clone();
        let thread_id = state.reply_thread_id.clone();

        // Finalize the HTML (wrap signature in identifying div)
        let finalized_html = ratatoskr_core::db::queries_extra::finalize_compose_html(&body_html);

        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    conn.execute(
                        "INSERT INTO local_drafts \
                         (id, account_id, to_addresses, cc_addresses, \
                          bcc_addresses, subject, body_html, \
                          reply_to_message_id, thread_id, from_email, \
                          signature_id, updated_at, sync_status) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, \
                                 ?10, ?11, unixepoch(), 'finalized') \
                         ON CONFLICT(id) DO UPDATE SET \
                           to_addresses = ?3, cc_addresses = ?4, \
                           bcc_addresses = ?5, subject = ?6, \
                           body_html = ?7, reply_to_message_id = ?8, \
                           thread_id = ?9, from_email = ?10, \
                           signature_id = ?11, \
                           updated_at = unixepoch(), \
                           sync_status = 'finalized'",
                        params![
                            draft_id,
                            account_id,
                            to_csv,
                            cc_csv,
                            bcc_csv,
                            subject,
                            finalized_html,
                            reply_msg_id,
                            thread_id,
                            from_email,
                            sig_id,
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(())
                })
                .await
            },
            move |result| {
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::SendFinalized(result)),
                )
            },
        )
    }
}

// ── Compose: draft auto-save ────────────────────────────

impl App {
    /// Save the current compose state as a local draft.
    fn save_compose_draft(
        &mut self,
        window_id: iced::window::Id,
    ) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) =
            self.pop_out_windows.get(&window_id)
        else {
            return Task::none();
        };

        if !state.draft_dirty {
            return Task::none();
        }

        let db = Arc::clone(&self.db);
        let draft_id = state.draft_id.clone();
        let account_id = state
            .from_account
            .as_ref()
            .map(|a| a.id.clone())
            .unwrap_or_default();
        let from_email = state
            .from_account
            .as_ref()
            .map(|a| a.email.clone());
        let to_csv = tokens_to_csv(&state.to);
        let cc_csv = tokens_to_csv(&state.cc);
        let bcc_csv = tokens_to_csv(&state.bcc);
        let subject = if state.subject.is_empty() {
            None
        } else {
            Some(state.subject.clone())
        };
        let body_html = state.editor.to_html();
        let sig_id = state.active_signature_id.clone();
        let reply_msg_id = state.reply_message_id.clone();
        let thread_id = state.reply_thread_id.clone();

        Task::perform(
            async move {
                db.with_write_conn(move |conn| {
                    conn.execute(
                        "INSERT INTO local_drafts \
                         (id, account_id, to_addresses, cc_addresses, \
                          bcc_addresses, subject, body_html, \
                          reply_to_message_id, thread_id, from_email, \
                          signature_id, updated_at, sync_status) \
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, \
                                 ?10, ?11, unixepoch(), 'pending') \
                         ON CONFLICT(id) DO UPDATE SET \
                           to_addresses = ?3, cc_addresses = ?4, \
                           bcc_addresses = ?5, subject = ?6, \
                           body_html = ?7, reply_to_message_id = ?8, \
                           thread_id = ?9, from_email = ?10, \
                           signature_id = ?11, \
                           updated_at = unixepoch(), \
                           sync_status = 'pending'",
                        params![
                            draft_id,
                            account_id,
                            to_csv,
                            cc_csv,
                            bcc_csv,
                            subject,
                            body_html,
                            reply_msg_id,
                            thread_id,
                            from_email,
                            sig_id,
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                    Ok(())
                })
                .await
            },
            move |result| {
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::DraftSaved(result)),
                )
            },
        )
    }
}

// ── Compose: attachment handling ────────────────────────

impl App {
    /// Handle the AttachFiles action — stub file picker.
    fn handle_compose_attach_files(
        &self,
        window_id: iced::window::Id,
    ) -> Task<Message> {
        // rfd is not yet a dependency. Use a stub that reads from
        // a well-known location or simply reports that file picking
        // is not yet available. When rfd is added, this will use
        // AsyncFileDialog.
        Task::perform(
            async move {
                // Stub: in a real implementation, this would open a native
                // file picker dialog via rfd::AsyncFileDialog.
                // For now, return an empty list.
                Vec::<(String, std::path::PathBuf, u64)>::new()
            },
            move |files| {
                if files.is_empty() {
                    // No files selected (or stub)
                    Message::PopOut(
                        window_id,
                        PopOutMessage::Compose(ComposeMessage::FilesAttached(
                            Vec::new(),
                        )),
                    )
                } else {
                    Message::PopOut(
                        window_id,
                        PopOutMessage::Compose(ComposeMessage::FilesAttached(
                            files,
                        )),
                    )
                }
            },
        )
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
