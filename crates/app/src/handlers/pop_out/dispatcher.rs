use std::sync::Arc;

use iced::Task;

use crate::pop_out::compose::{ComposeAttachment, ComposeMessage};
use crate::pop_out::message_view::{MessageViewMessage, RenderingMode};
use crate::pop_out::{PopOutMessage, PopOutWindow};
use crate::{Message, ReadyApp};

use super::compose_clipboard::{
    dispatch_paste_group_match, field_token_count, handle_compose_copy_or_cut,
    paste_field_for_message,
};
use super::message_view::handle_message_view_update;

impl ReadyApp {
    /// Route a `PopOutMessage` to the correct pop-out window handler.
    pub(crate) fn handle_pop_out_message(
        &mut self,
        window_id: iced::window::Id,
        pop_out_msg: PopOutMessage,
    ) -> Task<Message> {
        let db = Arc::clone(&self.db);
        // Phase 6a: clone the service_client up front so the
        // GroupSaveConfirm arm (which fires the contacts.group_save
        // IPC) does not need to borrow `self` mid-match - the
        // pop_out_windows.get_mut below already holds a mutable
        // borrow of self.
        let service_client = self.service_client.clone();
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
                self.open_compose_from_message_view(window_id, &mv_msg)
            }
            // Save As - needs db access from App
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(MessageViewMessage::SaveAs),
            ) => self.handle_save_as(window_id),
            // SetRenderingMode to Source - may need lazy load
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(MessageViewMessage::SetRenderingMode(
                    RenderingMode::Source,
                )),
            ) => self.handle_set_source_mode(window_id),
            // Archive - route through action service for DB + provider dispatch
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(MessageViewMessage::Archive),
            ) => self.dispatch_pop_out_action(
                window_id,
                crate::action_resolve::MailActionIntent::Archive,
            ),
            // Delete - route through action service for DB + provider dispatch
            (
                PopOutWindow::MessageView(_),
                PopOutMessage::MessageView(MessageViewMessage::Delete),
            ) => self.dispatch_pop_out_action(
                window_id,
                crate::action_resolve::MailActionIntent::Trash,
            ),
            // All other message view messages
            (PopOutWindow::MessageView(state), PopOutMessage::MessageView(_)) => {
                let PopOutMessage::MessageView(msg) = pop_out_msg else {
                    return Task::none();
                };
                handle_message_view_update(state, msg)
            }
            // Compose discard
            (PopOutWindow::Compose(_), PopOutMessage::Compose(ComposeMessage::Discard)) => {
                self.pop_out_windows.remove(&window_id);
                iced::window::close(window_id)
            }
            // Compose send - build MIME, queue for outbox, close window
            (PopOutWindow::Compose(_), PopOutMessage::Compose(ComposeMessage::Send)) => {
                self.handle_compose_send(window_id)
            }
            // Compose manual save - flush draft immediately
            (PopOutWindow::Compose(_), PopOutMessage::Compose(ComposeMessage::SaveDraftNow)) => {
                self.save_compose_draft(window_id)
            }
            // Compose from-account changed - swap signature for new account
            (
                PopOutWindow::Compose(_),
                PopOutMessage::Compose(ComposeMessage::FromAccountChanged(_)),
            ) => {
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                self.handle_compose_from_account_changed(window_id, msg)
            }
            // Compose attach files - launch async file picker
            (PopOutWindow::Compose(_), PopOutMessage::Compose(ComposeMessage::AttachFiles)) => {
                handle_compose_attach_files(window_id)
            }
            // Compose expand group - needs DB access
            (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::ContextMenuExpandGroup { .. }),
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
                        crate::pop_out::compose::RecipientField::To => &state.to.tokens,
                        crate::pop_out::compose::RecipientField::Cc => &state.cc.tokens,
                        crate::pop_out::compose::RecipientField::Bcc => &state.bcc.tokens,
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
                    async move { db_clone.expand_contact_group(gid).await },
                    move |result| {
                        Message::PopOut(
                            window_id,
                            PopOutMessage::Compose(ComposeMessage::GroupExpanded {
                                field,
                                token_id,
                                members: result,
                            }),
                        )
                    },
                )
            }
            // Compose save-as-group confirm - DB write + state update
            (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::GroupSaveConfirm),
            ) => {
                crate::pop_out::compose::update_compose(
                    state,
                    ComposeMessage::GroupSaveConfirm,
                );
                if !state.save_group_in_flight {
                    return Task::none();
                }
                let Some(banner) = state.bulk_paste_banner.as_ref() else {
                    return Task::none();
                };
                let field_tokens = match banner.field {
                    crate::pop_out::compose::RecipientField::To => &state.to.tokens,
                    crate::pop_out::compose::RecipientField::Cc => &state.cc.tokens,
                    crate::pop_out::compose::RecipientField::Bcc => &state.bcc.tokens,
                };
                let pasted: std::collections::HashSet<_> =
                    banner.token_ids.iter().copied().collect();
                let emails: Vec<String> = field_tokens
                    .iter()
                    .filter(|t| pasted.contains(&t.id))
                    .map(|t| t.email.clone())
                    .filter(|e| !e.is_empty())
                    .collect();
                if emails.is_empty() {
                    return Task::done(Message::PopOut(
                        window_id,
                        PopOutMessage::Compose(ComposeMessage::GroupSaveResult(Err(
                            "No valid addresses to save.".to_string(),
                        ))),
                    ));
                }
                let name = state.save_group_name.trim().to_string();
                let group_id = uuid::Uuid::new_v4().to_string();
                let now = chrono::Utc::now().timestamp();
                #[allow(clippy::cast_possible_wrap)]
                let member_count = emails.len() as i64;
                let entry = crate::db::GroupEntry {
                    id: group_id.clone(),
                    name: name.clone(),
                    member_count,
                    created_at: now,
                    updated_at: now,
                };
                let success = crate::pop_out::compose::GroupSaveSuccess {
                    group_id,
                    name,
                    member_count,
                };
                // Phase 6a: contacts.group_save IPC.
                let Some(client) = service_client else {
                    log::warn!(
                        "contacts.group_save: no ServiceClient yet; surfacing error"
                    );
                    return Task::done(Message::PopOut(
                        window_id,
                        PopOutMessage::Compose(ComposeMessage::GroupSaveResult(Err(
                            "Service not ready".to_string(),
                        ))),
                    ));
                };
                let emails_for_save = emails.clone();
                let params = service_api::ContactGroupSaveParams {
                    id: entry.id,
                    name: entry.name,
                    member_emails: emails_for_save,
                    created_at: entry.created_at,
                    updated_at: entry.updated_at,
                    member_count: entry.member_count,
                };
                Task::perform(
                    async move {
                        client
                            .save_contact_group(params)
                            .await
                            .map_err(|e| e.to_string())
                    },
                    move |result| {
                        let payload = result.map(|()| success.clone());
                        Message::PopOut(
                            window_id,
                            PopOutMessage::Compose(ComposeMessage::GroupSaveResult(payload)),
                        )
                    },
                )
            }
            // Compose context-menu Copy / Cut / Paste - clipboard side
            // effects, plus the standard reducer for state changes.
            (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::ContextMenuCopy { .. }),
            )
            | (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::ContextMenuCut { .. }),
            ) => {
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                let (field, token_id, is_cut) = match &msg {
                    ComposeMessage::ContextMenuCopy { field, token_id } => {
                        (*field, *token_id, false)
                    }
                    ComposeMessage::ContextMenuCut { field, token_id } => {
                        (*field, *token_id, true)
                    }
                    _ => return Task::none(),
                };
                handle_compose_copy_or_cut(state, &db, field, token_id, is_cut)
            }
            // Ctrl+X / Ctrl+C from the token input - same path as the
            // context-menu Copy/Cut entries; figure out which field the
            // message came from and forward to the shared helper.
            (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::ToTokenInput(
                    crate::ui::token_input::TokenInputMessage::CopyToken(_)
                    | crate::ui::token_input::TokenInputMessage::CutToken(_),
                )),
            )
            | (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::CcTokenInput(
                    crate::ui::token_input::TokenInputMessage::CopyToken(_)
                    | crate::ui::token_input::TokenInputMessage::CutToken(_),
                )),
            )
            | (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::BccTokenInput(
                    crate::ui::token_input::TokenInputMessage::CopyToken(_)
                    | crate::ui::token_input::TokenInputMessage::CutToken(_),
                )),
            ) => {
                use crate::pop_out::compose::RecipientField;
                use crate::ui::token_input::TokenInputMessage;
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                let (field, inner) = match msg {
                    ComposeMessage::ToTokenInput(inner) => (RecipientField::To, inner),
                    ComposeMessage::CcTokenInput(inner) => (RecipientField::Cc, inner),
                    ComposeMessage::BccTokenInput(inner) => (RecipientField::Bcc, inner),
                    _ => return Task::none(),
                };
                let (token_id, is_cut) = match inner {
                    TokenInputMessage::CopyToken(id) => (id, false),
                    TokenInputMessage::CutToken(id) => (id, true),
                    _ => return Task::none(),
                };
                handle_compose_copy_or_cut(state, &db, field, token_id, is_cut)
            }
            (
                PopOutWindow::Compose(state),
                PopOutMessage::Compose(ComposeMessage::ContextMenuPaste { .. }),
            ) => {
                let PopOutMessage::Compose(ComposeMessage::ContextMenuPaste { field }) =
                    pop_out_msg
                else {
                    return Task::none();
                };
                crate::pop_out::compose::update_compose(
                    state,
                    ComposeMessage::ContextMenuPaste { field },
                );
                iced::clipboard::read().map(move |opt| {
                    let Some(content) = opt else {
                        return Message::PopOut(
                            window_id,
                            PopOutMessage::Compose(ComposeMessage::Noop),
                        );
                    };
                    let inner = crate::ui::token_input::TokenInputMessage::Paste(content);
                    let outer = match field {
                        crate::pop_out::compose::RecipientField::To => {
                            ComposeMessage::ToTokenInput(inner)
                        }
                        crate::pop_out::compose::RecipientField::Cc => {
                            ComposeMessage::CcTokenInput(inner)
                        }
                        crate::pop_out::compose::RecipientField::Bcc => {
                            ComposeMessage::BccTokenInput(inner)
                        }
                    };
                    Message::PopOut(window_id, PopOutMessage::Compose(outer))
                })
            }
            // All other compose messages
            (PopOutWindow::Compose(state), PopOutMessage::Compose(_)) => {
                let PopOutMessage::Compose(msg) = pop_out_msg else {
                    return Task::none();
                };
                let trigger_autocomplete =
                    crate::handlers::contacts::should_trigger_autocomplete(&msg);
                let paste_field = paste_field_for_message(&msg);
                let tokens_before = paste_field.map(|f| field_token_count(state, f));
                crate::pop_out::compose::update_compose(state, msg);
                let group_match_task =
                    if let (Some(field), Some(before)) = (paste_field, tokens_before) {
                        dispatch_paste_group_match(state, &db, window_id, field, before)
                    } else {
                        Task::none()
                    };
                let autocomplete_task = if trigger_autocomplete {
                    crate::handlers::contacts::dispatch_autocomplete_search(
                        &db, window_id, state,
                    )
                } else {
                    Task::none()
                };
                Task::batch([group_match_task, autocomplete_task])
            }
            _ => Task::none(),
        }
    }
}

/// Launch an async file picker and return the selected files as attachments.
fn handle_compose_attach_files(window_id: iced::window::Id) -> Task<Message> {
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
                let mime_type = crate::pop_out::compose::mime_from_extension(&name);
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
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::FilesSelected(Vec::new())),
                )
            } else {
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::FilesSelected(files)),
                )
            }
        },
    )
}
