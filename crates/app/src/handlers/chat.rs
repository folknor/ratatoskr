use iced::Task;

use crate::ui::chat_timeline::{ChatTimeline, ChatTimelineEvent};
use crate::{App, Message};

impl App {
    /// Enter chat view for a contact.
    pub(crate) fn enter_chat_view(&mut self, email: String) -> Task<Message> {
        self.clear_search_state();
        self.clear_pinned_search_context();
        self.active_chat = Some(email.clone());
        self.clear_thread_selection();
        self.chat_timeline = Some(ChatTimeline::new(email.clone()));

        let db_state = self.db.read_db_state();
        let user_emails = self.user_emails();
        let token = self.chat_generation.next();

        Task::perform(
            async move {
                rtsk::chat::get_chat_timeline(&db_state, &email, &user_emails, 50, None).await
            },
            move |result| Message::ChatTimelineLoaded(token, result),
        )
    }

    /// Handle chat timeline data loaded.
    pub(crate) fn handle_chat_timeline_loaded(
        &mut self,
        messages: Vec<rtsk::chat::ChatMessage>,
    ) -> Task<Message> {
        if let Some(ref mut timeline) = self.chat_timeline {
            timeline.messages = messages;
            timeline.loading = false;
            // TODO: snap to bottom once iced fork exposes scroll_to/snap_to
        }
        Task::none()
    }

    /// Handle events from the chat timeline component.
    pub(crate) fn handle_chat_timeline_event(&mut self, event: ChatTimelineEvent) -> Task<Message> {
        match event {
            ChatTimelineEvent::LoadOlderRequested => {
                let Some(ref timeline) = self.chat_timeline else {
                    return Task::none();
                };
                let oldest = timeline
                    .messages
                    .first()
                    .map(|m| (m.date, m.message_id.clone()));
                let Some(before) = oldest else {
                    return Task::none();
                };
                let contact = timeline.contact_email.clone();
                let db_state = self.db.read_db_state();
                let user_emails = self.user_emails();

                let email = contact.clone();
                Task::perform(
                    async move {
                        rtsk::chat::get_chat_timeline(
                            &db_state,
                            &email,
                            &user_emails,
                            50,
                            Some(before),
                        )
                        .await
                    },
                    move |result| Message::ChatOlderLoaded(contact, result),
                )
            }
        }
    }

    /// Prepend older messages to the chat timeline.
    pub(crate) fn handle_chat_older_loaded(
        &mut self,
        messages: Vec<rtsk::chat::ChatMessage>,
    ) -> Task<Message> {
        if let Some(ref mut timeline) = self.chat_timeline {
            let mut older = messages;
            older.append(&mut timeline.messages);
            timeline.messages = older;
        }
        Task::none()
    }

    /// Get all user email addresses across accounts, including send-as aliases.
    pub(crate) fn user_emails(&self) -> Vec<String> {
        let mut emails: Vec<String> = self
            .sidebar
            .accounts
            .iter()
            .map(|a| a.email.clone())
            .collect();

        if let Ok(send_identities) = self.db.get_send_identity_emails_sync() {
            for email in send_identities {
                if !emails.iter().any(|e| e.eq_ignore_ascii_case(&email)) {
                    emails.push(email);
                }
            }
        }

        emails
    }
}
