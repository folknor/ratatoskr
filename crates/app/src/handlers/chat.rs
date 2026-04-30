use iced::Task;

use crate::ui::chat_timeline::{
    CHAT_SCROLLABLE_ID, CHAT_TIMELINE_PAGE, ChatTimeline, ChatTimelineEvent,
};
use crate::{App, Message};

impl App {
    /// Enter chat view for a contact.
    pub(crate) fn enter_chat_view(&mut self, email: String) -> Task<Message> {
        self.clear_search_state();
        self.clear_pinned_search_context();
        self.active_chat = Some(email.clone());
        self.sidebar.active_chat = Some(email.clone());
        self.clear_thread_selection();
        self.chat_timeline = Some(ChatTimeline::new(email.clone()));

        let db_state = self.db.read_db_state();
        let user_emails = self.user_emails();
        let token = self.chat_generation.next();
        let chat_list_token = self.chat_list_generation.next();

        let Some(body_store) = self.body_store.clone() else {
            log::warn!("enter_chat_view: body store unavailable");
            return self.fire_chat_contacts_load(chat_list_token);
        };
        let Some(inline_image_store) = self.inline_image_store.clone() else {
            log::warn!("enter_chat_view: inline image store unavailable");
            return self.fire_chat_contacts_load(chat_list_token);
        };

        let timeline_load = Task::perform(
            async move {
                rtsk::chat::get_chat_timeline(
                    &db_state,
                    &body_store,
                    &inline_image_store,
                    &email,
                    &user_emails,
                    CHAT_TIMELINE_PAGE,
                    None,
                )
                .await
            },
            move |result| Message::ChatTimelineLoaded(token, result),
        );
        // Refresh the sidebar's chat-contacts list so the unread count clears
        // on the active row once mark-read lands. No-op until then.
        let contacts_reload = self.fire_chat_contacts_load(chat_list_token);
        Task::batch([timeline_load, contacts_reload])
    }

    /// Handle chat timeline data loaded.
    pub(crate) fn handle_chat_timeline_loaded(
        &mut self,
        messages: Vec<rtsk::chat::ChatMessage>,
    ) -> Task<Message> {
        let Some(timeline) = self.chat_timeline.as_mut() else {
            return Task::none();
        };
        // A short page means we've reached the start of history.
        timeline.has_more = messages.len() >= CHAT_TIMELINE_PAGE;
        timeline.messages = messages;
        timeline.loading = false;
        // Pre-build image handles so iced's GPU image cache stays stable
        // across view cycles. (Re-creating handles each frame thrashes the
        // cache and causes flicker / driver pressure.)
        timeline.refresh_image_handles();
        // Snap to bottom so the most recent message is visible on entry.
        iced::widget::operation::snap_to_end::<Message>(CHAT_SCROLLABLE_ID.to_string())
    }

    /// Handle events from the chat timeline component.
    #[allow(clippy::needless_pass_by_value)]
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
                let Some(body_store) = self.body_store.clone() else {
                    return Task::none();
                };
                let Some(inline_image_store) = self.inline_image_store.clone() else {
                    return Task::none();
                };

                let email = contact.clone();
                Task::perform(
                    async move {
                        rtsk::chat::get_chat_timeline(
                            &db_state,
                            &body_store,
                            &inline_image_store,
                            &email,
                            &user_emails,
                            CHAT_TIMELINE_PAGE,
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
            let got = messages.len();
            let mut older = messages;
            older.append(&mut timeline.messages);
            timeline.messages = older;
            // A short page means we just hit the start of history.
            if got < CHAT_TIMELINE_PAGE {
                timeline.has_more = false;
            }
            // Build handles for the newly prepended messages.
            timeline.refresh_image_handles();
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
