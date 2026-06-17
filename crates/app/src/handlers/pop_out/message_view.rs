use std::sync::Arc;

use iced::Task;

use crate::pop_out::message_view::{MessageViewMessage, MessageViewState, RenderingMode};
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::{Message, ReadyApp};

pub(super) fn handle_message_view_update(
    state: &mut MessageViewState,
    msg: MessageViewMessage,
) -> Task<Message> {
    match msg {
        MessageViewMessage::BodyLoaded(generation, _)
            if !state.is_current_generation(generation) =>
        {
            Task::none() // Stale load - ignore
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
        MessageViewMessage::AttachmentsLoaded(generation, _)
            if !state.is_current_generation(generation) =>
        {
            Task::none() // Stale load - ignore
        }
        MessageViewMessage::AttachmentsLoaded(_, Ok(attachments)) => {
            state.attachments = attachments;
            Task::none()
        }
        MessageViewMessage::AttachmentsLoaded(_, Err(e)) => {
            eprintln!("Pop-out attachments load failed: {e}");
            Task::none()
        }
        MessageViewMessage::SetRenderingMode(mode) => {
            state.rendering_mode = mode;
            // Mode picker lives inside the overflow menu; close it on select.
            state.context_menu_open = false;
            Task::none()
        }
        MessageViewMessage::OpenContextMenu => {
            state.context_menu_open = true;
            Task::none()
        }
        MessageViewMessage::CloseContextMenu => {
            state.context_menu_open = false;
            Task::none()
        }
        MessageViewMessage::LoadRemoteContent => {
            state.remote_content_loaded = true;
            Task::none()
        }
        MessageViewMessage::HoverAttachment(id) => {
            state.hovered_attachment_id = id;
            Task::none()
        }
        // Attachments roadmap Phase 5: OpenAttachment / SaveAttachment
        // / SaveAllAttachments are intercepted by
        // `dispatcher.rs::handle_pop_out_message` so they can reach
        // the App-level shared handler. If they arrive here it's a
        // routing regression - log and drop.
        MessageViewMessage::OpenAttachment(att_id) | MessageViewMessage::SaveAttachment(att_id) => {
            log::warn!(
                "MessageViewMessage::{{Open,Save}}Attachment({att_id}) reached \
                 message_view.rs - dispatcher routing regression",
            );
            Task::none()
        }
        MessageViewMessage::SaveAllAttachments => {
            log::warn!(
                "MessageViewMessage::SaveAllAttachments reached message_view.rs \
                 - dispatcher routing regression",
            );
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

/// Build a best-effort pseudo `.eml` for the Source view from the headers and
/// body already on `MessageViewState`. Not a faithful RFC 822 reconstruction -
/// the original MIME boundaries, transfer encodings, and Received headers are
/// long gone - but it gives users a useful "underlying content" view in place
/// of the previous "(raw source not available)" stub.
fn synthesize_pseudo_source(state: &MessageViewState) -> String {
    let mut out = String::new();

    let from = match (&state.from_name, &state.from_address) {
        (Some(name), Some(addr)) if !name.is_empty() => format!("{name} <{addr}>"),
        (_, Some(addr)) => addr.clone(),
        (Some(name), None) => name.clone(),
        (None, None) => String::new(),
    };
    if !from.is_empty() {
        out.push_str(&format!("From: {from}\n"));
    }
    if let Some(to) = &state.to_addresses
        && !to.is_empty()
    {
        out.push_str(&format!("To: {to}\n"));
    }
    if let Some(cc) = &state.cc_addresses
        && !cc.is_empty()
    {
        out.push_str(&format!("Cc: {cc}\n"));
    }
    if let Some(subject) = &state.subject
        && !subject.is_empty()
    {
        out.push_str(&format!("Subject: {subject}\n"));
    }
    if let Some(ts) = state.date
        && let Some(dt) = chrono::DateTime::from_timestamp(ts, 0)
    {
        out.push_str(&format!("Date: {}\n", dt.to_rfc2822()));
    }

    let (body, content_type) = match (state.body_html.as_deref(), state.body_text.as_deref()) {
        (Some(html), _) if !html.trim().is_empty() => (html, "text/html; charset=utf-8"),
        (_, Some(text)) if !text.trim().is_empty() => (text, "text/plain; charset=utf-8"),
        _ => ("(empty body)", "text/plain; charset=utf-8"),
    };
    out.push_str(&format!("Content-Type: {content_type}\n"));
    out.push('\n');
    out.push_str(body);

    out
}

impl ReadyApp {
    /// Dispatch body + attachment loads for a message view window.
    pub(crate) fn dispatch_message_view_loads(
        &self,
        window_id: iced::window::Id,
        generation: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
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
                    // Try body store first (has full decompressed bodies),
                    // fall back to DB snippet if body store unavailable.
                    if let Some(bs) = body_store
                        && let Ok(Some(body)) = bs.get(message_id.clone()).await
                    {
                        return Ok((body.body_text, body.body_html));
                    }
                    db.load_message_body(account_id, message_id).await
                },
                move |result| {
                    Message::PopOut(
                        window_id,
                        PopOutMessage::MessageView(MessageViewMessage::BodyLoaded(
                            generation, result,
                        )),
                    )
                },
            ),
            Task::perform(
                async move { db2.load_message_attachments(account_id2, message_id2).await },
                move |result| {
                    Message::PopOut(
                        window_id,
                        PopOutMessage::MessageView(MessageViewMessage::AttachmentsLoaded(
                            generation, result,
                        )),
                    )
                },
            ),
        ])
    }

    /// Handle switching to Source rendering mode.
    ///
    /// True raw source (the .eml as it arrived) isn't stored anywhere right
    /// now - bodies live in the body store, headers live in `messages`, and
    /// neither preserves the original MIME framing. Until that's available
    /// we synthesize a best-effort pseudo-source from the headers and body
    /// already on `MessageViewState`, which is far more useful than the
    /// previous "(raw source not available)" placeholder.
    pub(crate) fn handle_set_source_mode(&mut self, window_id: iced::window::Id) -> Task<Message> {
        let Some(PopOutWindow::MessageView(state)) = self.pop_out_windows.get_mut(&window_id)
        else {
            return Task::none();
        };

        state.rendering_mode = RenderingMode::Source;
        if state.raw_source.is_none() {
            state.raw_source = Some(synthesize_pseudo_source(state));
        }
        Task::none()
    }

    /// Dispatch an action-service operation for the thread shown in a pop-out
    /// message view. Extracts thread context from the pop-out state, closes the
    /// overflow menu, then delegates to the action service.
    pub(crate) fn dispatch_pop_out_action(
        &mut self,
        window_id: iced::window::Id,
        intent: crate::action_resolve::MailActionIntent,
    ) -> Task<Message> {
        let (threads, selection) = {
            let Some(PopOutWindow::MessageView(state)) = self.pop_out_windows.get_mut(&window_id)
            else {
                return Task::none();
            };
            state.context_menu_open = false;
            let threads = vec![(state.account_id.clone(), state.thread_id.clone())];
            let selection = state
                .source_selection
                .clone()
                .unwrap_or(types::SidebarSelection::Inbox);
            (threads, selection)
        };

        use crate::action_resolve::{self as ar, UiContext};
        let ui_ctx = UiContext { selection };
        let outcome = ar::resolve_intent(intent, &ui_ctx);
        let Some(plan) = ar::build_execution_plan(outcome, &threads, &mut self.thread_list) else {
            return Task::none();
        };
        self.dispatch_plan(plan)
    }
}
