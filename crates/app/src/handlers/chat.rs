use std::sync::Arc;

use iced::Task;

use crate::ui::chat_timeline::{ChatTimeline, ChatTimelineEvent};
use crate::{App, Message};
use crate::command_dispatch::NavigationTarget;

impl App {
    /// Enter chat view for a contact.
    pub(crate) fn enter_chat_view(&mut self, email: String) -> Task<Message> {
        self.navigation_target = Some(NavigationTarget::Chat { email: email.clone() });
        self.clear_thread_selection();
        self.chat_timeline = Some(ChatTimeline::new(email.clone()));

        let db_state = ratatoskr_core::db::DbState::from_arc(self.db.conn_arc());
        let user_emails = self.user_emails();
        let token = self.chat_generation.next();

        Task::perform(
            async move {
                ratatoskr_core::chat::get_chat_timeline(
                    &db_state, &email, &user_emails, 50, None,
                )
                .await
            },
            move |result| Message::ChatTimelineLoaded(token, result),
        )
    }

    /// Handle chat timeline data loaded.
    pub(crate) fn handle_chat_timeline_loaded(
        &mut self,
        messages: Vec<ratatoskr_core::chat::ChatMessage>,
    ) -> Task<Message> {
        if let Some(ref mut timeline) = self.chat_timeline {
            timeline.messages = messages;
            timeline.loading = false;
            // TODO: snap to bottom once iced fork exposes scroll_to/snap_to
        }
        Task::none()
    }

    /// Handle events from the chat timeline component.
    pub(crate) fn handle_chat_timeline_event(
        &mut self,
        event: ChatTimelineEvent,
    ) -> Task<Message> {
        match event {
            ChatTimelineEvent::LoadOlderRequested => {
                let Some(ref timeline) = self.chat_timeline else {
                    return Task::none();
                };
                let oldest = timeline.messages.first().map(|m| m.date);
                let Some(before) = oldest else {
                    return Task::none();
                };
                let email = timeline.contact_email.clone();
                let db_state = ratatoskr_core::db::DbState::from_arc(self.db.conn_arc());
                let user_emails = self.user_emails();

                Task::perform(
                    async move {
                        ratatoskr_core::chat::get_chat_timeline(
                            &db_state, &email, &user_emails, 50, Some(before),
                        )
                        .await
                    },
                    |result| Message::ChatOlderLoaded(result),
                )
            }
        }
    }

    /// Prepend older messages to the chat timeline.
    pub(crate) fn handle_chat_older_loaded(
        &mut self,
        messages: Vec<ratatoskr_core::chat::ChatMessage>,
    ) -> Task<Message> {
        if let Some(ref mut timeline) = self.chat_timeline {
            let mut older = messages;
            older.append(&mut timeline.messages);
            timeline.messages = older;
        }
        Task::none()
    }

    /// Get all user email addresses across accounts.
    pub(crate) fn user_emails(&self) -> Vec<String> {
        self.sidebar
            .accounts
            .iter()
            .map(|a| a.email.clone())
            .collect()
    }
}
