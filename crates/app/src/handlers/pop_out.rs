use std::sync::Arc;

use iced::{Size, Task};

use crate::pop_out::compose::{ComposeMode, ComposeState};
use crate::pop_out::message_view::{MessageViewMessage, MessageViewState};
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::ui::layout::{
    COMPOSE_DEFAULT_HEIGHT, COMPOSE_DEFAULT_WIDTH, COMPOSE_MIN_HEIGHT, COMPOSE_MIN_WIDTH,
    MESSAGE_VIEW_DEFAULT_HEIGHT, MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_MIN_HEIGHT,
    MESSAGE_VIEW_MIN_WIDTH,
};
use crate::{App, Message};
use crate::command_dispatch::ComposeAction;

impl App {
    pub(crate) fn handle_pop_out_message(
        &mut self,
        window_id: iced::window::Id,
        pop_out_msg: PopOutMessage,
    ) -> Task<Message> {
        let Some(window) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };
        match (window, &pop_out_msg) {
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
            (
                PopOutWindow::MessageView(state),
                PopOutMessage::MessageView(_),
            ) => {
                let PopOutMessage::MessageView(msg) = pop_out_msg else {
                    return Task::none();
                };
                handle_message_view_update(state, msg)
            }
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(crate::pop_out::compose::ComposeMessage::Discard),
            ) => {
                self.pop_out_windows.remove(&window_id);
                self.composer_is_open = false;
                iced::window::close(window_id)
            }
            (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(_),
            ) => {
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                crate::pop_out::compose::update_compose(state, msg);
                Task::none()
            }
            _ => Task::none(),
        }
    }

    pub(crate) fn open_message_view_window(
        &mut self,
        message_index: usize,
    ) -> Task<Message> {
        let Some(msg) = self.reading_pane.thread_messages.get(message_index)
        else {
            return Task::none();
        };

        let state = MessageViewState::from_thread_message(msg);
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
                            MessageViewMessage::BodyLoaded(result),
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
                            MessageViewMessage::AttachmentsLoaded(result),
                        ),
                    )
                },
            ),
        ])
    }

    pub(crate) fn open_compose_window(&mut self, mode: ComposeMode) -> Task<Message> {
        let state = ComposeState::new(&self.sidebar.accounts);
        self.open_compose_window_with_state(state, mode)
    }

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
            None,
            mv.body_text.as_deref().or(mv.snippet.as_deref()),
            Some(&mv.thread_id),
            Some(&mv.message_id),
        );

        self.open_compose_window_with_state(state, mode)
    }

    pub(crate) fn handle_compose_action(
        &mut self,
        action: ComposeAction,
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
            ComposeAction::Reply => ComposeMode::Reply {
                original_subject: subject,
            },
            ComposeAction::ReplyAll => ComposeMode::ReplyAll {
                original_subject: subject,
            },
            ComposeAction::Forward => ComposeMode::Forward {
                original_subject: subject,
            },
        };

        let to_email = last_message.and_then(|m| m.from_address.as_deref());
        let to_name = last_message.and_then(|m| m.from_name.as_deref());
        let cc_emails: Option<&str> = None;
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

fn handle_message_view_update(
    state: &mut MessageViewState,
    msg: MessageViewMessage,
) -> Task<Message> {
    match msg {
        MessageViewMessage::BodyLoaded(Ok((body_text, body_html))) => {
            state.body_text = body_text;
            state.body_html = body_html;
            Task::none()
        }
        MessageViewMessage::BodyLoaded(Err(e)) => {
            eprintln!("Pop-out body load failed: {e}");
            Task::none()
        }
        MessageViewMessage::AttachmentsLoaded(Ok(attachments)) => {
            state.attachments = attachments;
            Task::none()
        }
        MessageViewMessage::AttachmentsLoaded(Err(e)) => {
            eprintln!("Pop-out attachments load failed: {e}");
            Task::none()
        }
        MessageViewMessage::Reply
        | MessageViewMessage::ReplyAll
        | MessageViewMessage::Forward
        | MessageViewMessage::Noop => Task::none(),
    }
}
