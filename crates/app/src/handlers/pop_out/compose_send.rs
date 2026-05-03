use iced::Task;

use crate::pop_out::PopOutWindow;
use crate::{Message, ReadyApp};

use rtsk::actions::SendAttachment;

impl ReadyApp {
    /// Build a MIME message from the compose state, save it to the draft row
    /// as base64url in the `attachments` column, mark the draft `'queued'`,
    /// Validate compose state, build a SendRequest, and dispatch to the
    /// action service. The compose window stays open with a "Sending..." status
    /// until SendCompleted arrives.
    pub(crate) fn handle_compose_send(&mut self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) else {
            return Task::none();
        };

        // Prevent double-send
        if state.sending {
            return Task::none();
        }

        // Validate recipients
        let has_recipients = !state.to.tokens.is_empty()
            || !state.cc.tokens.is_empty()
            || !state.bcc.tokens.is_empty();
        if !has_recipients {
            state.status = Some("Add at least one recipient".to_string());
            return Task::none();
        }

        // Build SendRequest from compose state
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

        let to: Vec<String> = state.to.tokens.iter().map(|t| t.email.clone()).collect();
        let cc: Vec<String> = state.cc.tokens.iter().map(|t| t.email.clone()).collect();
        let bcc: Vec<String> = state.bcc.tokens.iter().map(|t| t.email.clone()).collect();

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

        let send_req = rtsk::actions::SendRequest {
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

        // Set sending state and dispatch
        state.sending = true;
        state.status = Some("Sending\u{2026}".to_string());

        self.dispatch_send(window_id, send_req)
    }

    /// Dispatch send_email through the action service.
    fn dispatch_send(
        &mut self,
        window_id: iced::window::Id,
        request: rtsk::actions::SendRequest,
    ) -> Task<Message> {
        let Some(ctx) = self.action_ctx() else {
            if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id) {
                state.sending = false;
                state.status =
                    Some("Send unavailable \u{2014} action service not initialized".to_string());
            }
            return Task::none();
        };
        Task::perform(
            async move {
                let outcome = rtsk::actions::send_email(&ctx, request).await;
                (window_id, outcome)
            },
            move |(window_id, outcome)| Message::SendCompleted { window_id, outcome },
        )
    }

    /// Handle send completion: close compose on success, restore on failure.
    pub(crate) fn handle_send_completed(
        &mut self,
        window_id: iced::window::Id,
        outcome: &rtsk::actions::ActionOutcome,
    ) -> Task<Message> {
        match outcome {
            rtsk::actions::ActionOutcome::Success | rtsk::actions::ActionOutcome::NoOp => {
                self.pop_out_windows.remove(&window_id);
                self.status_bar
                    .show_confirmation("Message sent".to_string());
                iced::window::close(window_id)
            }
            // LocalOnly should not occur for send (send uses Failed for all
            // failures), but handle it defensively as failure for safety.
            rtsk::actions::ActionOutcome::Failed { error }
            | rtsk::actions::ActionOutcome::LocalOnly { reason: error, .. } => {
                if let Some(PopOutWindow::Compose(state)) = self.pop_out_windows.get_mut(&window_id)
                {
                    state.sending = false;
                    state.status = Some(format!("Send failed: {}", error.user_message()));
                }
                Task::none()
            }
        }
    }
}
