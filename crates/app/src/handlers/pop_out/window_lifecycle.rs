use iced::{Size, Task};

use crate::pop_out::compose::{ComposeMode, ComposeState};
use crate::pop_out::message_view::{MessageViewMessage, MessageViewState};
use crate::pop_out::PopOutWindow;
use crate::ui::layout::{
    COMPOSE_DEFAULT_HEIGHT, COMPOSE_DEFAULT_WIDTH, COMPOSE_MIN_HEIGHT, COMPOSE_MIN_WIDTH,
    MESSAGE_VIEW_DEFAULT_HEIGHT, MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_MIN_HEIGHT,
    MESSAGE_VIEW_MIN_WIDTH,
};
use crate::{Message, ReadyApp};

impl ReadyApp {
    /// Open a message view pop-out for the message at `message_index` in the
    /// reading pane's thread messages list.
    pub(crate) fn open_message_view_window(&mut self, message_index: usize) -> Task<Message> {
        let Some(msg) = self
            .reading_pane
            .thread_messages
            .get(message_index)
            .cloned()
        else {
            return Task::none();
        };

        let generation = self.next_pop_out_generation();
        let source_selection = Some(self.sidebar.selection.clone());
        let state = MessageViewState::from_thread_message(
            &msg,
            generation,
            source_selection,
            self.settings.default_rendering_mode,
        );
        let account_id = state.account_id.clone();
        let message_id = state.message_id.clone();

        let settings = iced::window::Settings {
            size: Size::new(MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_DEFAULT_HEIGHT),
            min_size: Some(Size::new(MESSAGE_VIEW_MIN_WIDTH, MESSAGE_VIEW_MIN_HEIGHT)),
            exit_on_close_request: false,
            ..Default::default()
        };

        let (window_id, open_task) = iced::window::open(settings);
        self.pop_out_windows
            .insert(window_id, PopOutWindow::MessageView(Box::new(state)));

        self.dispatch_message_view_loads(window_id, generation, account_id, message_id, open_task)
    }

    /// Open a message-view pop-out for a chat-timeline message.
    ///
    /// This is the "View as email" path from chat view: the user wants to
    /// see one of the bubbles rendered in classic email format (full
    /// headers, signatures, quoted history) without leaving the chat
    /// timeline. Works the same as `open_message_view_window` from the
    /// thread reading pane, but pulls header data off `ChatMessage`
    /// rather than `reading_pane.thread_messages`.
    pub(crate) fn open_chat_message_view_window(
        &mut self,
        msg: &rtsk::chat::ChatMessage,
    ) -> Task<Message> {
        let generation = self.next_pop_out_generation();
        // Chat view has no sidebar selection of its own (active_chat sits
        // outside ViewScope), so we pass through whatever was selected
        // before the user entered chat. That keeps any subsequent
        // pop-out-initiated action (archive, etc.) resolving against the
        // user's last folder context, not nothing.
        let source_selection = Some(self.sidebar.selection.clone());
        let state = MessageViewState::from_chat_message(
            msg,
            generation,
            source_selection,
            self.settings.default_rendering_mode,
        );
        let account_id = state.account_id.clone();
        let message_id = state.message_id.clone();

        let settings = iced::window::Settings {
            size: Size::new(MESSAGE_VIEW_DEFAULT_WIDTH, MESSAGE_VIEW_DEFAULT_HEIGHT),
            min_size: Some(Size::new(MESSAGE_VIEW_MIN_WIDTH, MESSAGE_VIEW_MIN_HEIGHT)),
            exit_on_close_request: false,
            ..Default::default()
        };

        let (window_id, open_task) = iced::window::open(settings);
        self.pop_out_windows
            .insert(window_id, PopOutWindow::MessageView(Box::new(state)));

        self.dispatch_message_view_loads(window_id, generation, account_id, message_id, open_task)
    }

    /// Open a compose window with the given mode.
    pub(crate) fn open_compose_window(&mut self, mode: ComposeMode) -> Task<Message> {
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
        if let Some(ref tid) = state.reply_thread_id
            && let Some((&existing_id, _)) = self.pop_out_windows.iter().find(|(_, w)| {
                matches!(w, PopOutWindow::Compose(s) if s.reply_thread_id.as_deref() == Some(tid))
            })
        {
            return iced::window::gain_focus(existing_id);
        }

        // Auto-select shared mailbox identity only for reply/forward flows
        // originating from shared-mailbox scope, and only when mailbox rights
        // do not explicitly deny submit.
        if state.reply_thread_id.is_some()
            && self.current_mailbox_may_submit().unwrap_or(true)
            && let rtsk::scope::ViewScope::SharedMailbox {
                ref account_id,
                ref mailbox_id,
            } = self.sidebar.selected_scope
            && let Ok(Some(shared_email)) =
                self.db.get_shared_mailbox_email(account_id, mailbox_id)
        {
            state.set_shared_mailbox_from(account_id, &shared_email);
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

    fn current_mailbox_may_submit(&self) -> Option<bool> {
        self.sidebar
            .selection
            .navigation_folder_id()
            .and_then(|nav_id| {
                self.sidebar
                    .nav_state
                    .as_ref()?
                    .folders
                    .iter()
                    .find(|folder| folder.id == nav_id)
                    .and_then(|folder| folder.rights.as_ref())
                    .and_then(|rights| rights.may_submit)
            })
    }

    /// Open a compose window from a message view's Reply/ReplyAll/Forward.
    pub(crate) fn open_compose_from_message_view(
        &mut self,
        window_id: iced::window::Id,
        action: &MessageViewMessage,
    ) -> Task<Message> {
        let Some(PopOutWindow::MessageView(mv)) = self.pop_out_windows.get(&window_id) else {
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
            mv.message_id_header.as_deref(),
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
            crate::command_dispatch::ComposeAction::Reply => ComposeMode::Reply {
                original_subject: subject,
            },
            crate::command_dispatch::ComposeAction::ReplyAll => ComposeMode::ReplyAll {
                original_subject: subject,
            },
            crate::command_dispatch::ComposeAction::Forward => ComposeMode::Forward {
                original_subject: subject,
            },
        };

        let to_email = last_message.and_then(|m| m.from_address.as_deref());
        let to_name = last_message.and_then(|m| m.from_name.as_deref());
        let cc_emails = last_message.and_then(|m| m.cc_addresses.as_deref());
        let thread_id = selected_thread.map(|t| t.id.as_str());
        let message_id = last_message.map(|m| m.id.as_str());
        let message_id_header = last_message.and_then(|m| m.message_id_header.as_deref());
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
            message_id_header,
        );

        self.open_compose_window_with_state(state, mode)
    }

    /// Increment and return the next pop-out generation token.
    pub(crate) fn next_pop_out_generation(
        &mut self,
    ) -> rtsk::generation::GenerationToken<rtsk::generation::PopOut> {
        self.pop_out_generation.next()
    }
}
