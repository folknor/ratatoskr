use iced::Point;
use rte::{Action as RteAction, EditAction, InlineStyle};

use crate::ui::token_input::{self, TokenId};

use super::messages::ComposeMessage;
use super::state::ComposeState;
use super::token_handlers::handle_recipient_token_input;
use super::types::{
    ComposeTokenDrag, ContextMenuKind, RecipientField, TokenContextMenuState,
};

/// Update compose state for a given message.
///
/// NOTE: The caller (`handlers/pop_out.rs`) must check
/// `handlers::contacts::should_trigger_autocomplete(&msg)` BEFORE calling
/// this function. If it returns `true`, the caller should call
/// `handlers::contacts::dispatch_autocomplete_search(db, window_id, state)`
/// AFTER this function returns, to fire the async DB search.
pub fn update_compose(state: &mut ComposeState, msg: ComposeMessage) {
    match msg {
        ComposeMessage::SubjectChanged(s) => {
            state.subject = s;
            state.draft_dirty = true;
        }
        ComposeMessage::BodyChanged(action) => {
            state.body.perform(action);
            state.draft_dirty = true;
        }
        ComposeMessage::FromAccountChanged(account) => {
            state.from_account = Some(account);
            state.from_dropdown_open = false;
        }
        ComposeMessage::ToggleFromDropdown => {
            state.from_dropdown_open = !state.from_dropdown_open;
        }
        ComposeMessage::ShowCc => state.show_cc = true,
        ComposeMessage::ShowBcc => state.show_bcc = true,
        ComposeMessage::ToTokenInput(inner) => {
            handle_recipient_token_input(state, RecipientField::To, inner);
        }
        ComposeMessage::CcTokenInput(inner) => {
            handle_recipient_token_input(state, RecipientField::Cc, inner);
        }
        ComposeMessage::BccTokenInput(inner) => {
            handle_recipient_token_input(state, RecipientField::Bcc, inner);
        }
        ComposeMessage::Send => {
            let has_recipients = !state.to.tokens.is_empty()
                || !state.cc.tokens.is_empty()
                || !state.bcc.tokens.is_empty();
            if !has_recipients {
                state.recipients_error =
                    Some("Add at least one recipient.".to_string());
                return;
            }
            state.recipients_error = None;
            state.status = Some("Send not yet wired".to_string());
        }
        ComposeMessage::Discard => {
            // Handled by the caller (close window)
        }
        ComposeMessage::SaveDraftNow => {
            // Handled by the caller (dispatches async save). The reducer
            // does nothing - the dispatcher reuses the auto-save path.
        }
        ComposeMessage::ToggleDiscardConfirm => {
            state.link_dialog_open = false;
            state.context_menu = None;
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
            state.discard_confirm_open = !state.discard_confirm_open;
        }
        ComposeMessage::AutocompleteResults(generation, Ok(results)) => {
            if state.autocomplete.search_generation.is_current(generation) {
                state.autocomplete.results = results;
                state.autocomplete.highlighted = if state.autocomplete.results.is_empty() {
                    None
                } else {
                    Some(0)
                };
            }
        }
        ComposeMessage::AutocompleteResults(_, Err(_)) => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
        }
        ComposeMessage::AutocompleteSelect(idx) => {
            if let Some(match_entry) = state.autocomplete.results.get(idx).cloned() {
                let label = match_entry
                    .display_name
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(&match_entry.email)
                    .to_string();
                let target = match state.autocomplete.active_field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                let id = target.next_token_id();
                target.tokens.push(token_input::Token {
                    id,
                    email: match_entry.email,
                    display_text: label,
                    is_group: match_entry.is_group,
                    group_id: match_entry.group_id,
                    member_count: match_entry.member_count,
                });
                target.text.clear();
                state.autocomplete.results.clear();
                state.autocomplete.highlighted = None;
                state.autocomplete.query.clear();
            }
        }
        ComposeMessage::AutocompleteNavigate(delta) => {
            if let Some(current) = state.autocomplete.highlighted {
                let len = state.autocomplete.results.len();
                if len > 0 {
                    let new_idx = if delta > 0 {
                        (current + 1).min(len - 1)
                    } else if current > 0 {
                        current - 1
                    } else {
                        0
                    };
                    state.autocomplete.highlighted = Some(new_idx);
                }
            }
        }
        ComposeMessage::AutocompleteDismiss => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
        }
        ComposeMessage::ShowTokenContextMenu {
            field,
            token_id,
            position,
        } => {
            let is_group = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .any(|t| t.id == token_id && t.is_group);
            state.context_menu = Some(TokenContextMenuState {
                field,
                position,
                kind: ContextMenuKind::Token { token_id, is_group },
            });
        }
        ComposeMessage::ShowFieldContextMenu { field, position } => {
            state.context_menu = Some(TokenContextMenuState {
                field,
                position,
                kind: ContextMenuKind::Field,
            });
        }
        ComposeMessage::DismissContextMenu => {
            state.context_menu = None;
        }
        ComposeMessage::ContextMenuDelete { field, token_id } => {
            let tokens = match field {
                RecipientField::To => &mut state.to.tokens,
                RecipientField::Cc => &mut state.cc.tokens,
                RecipientField::Bcc => &mut state.bcc.tokens,
            };
            tokens.retain(|t| t.id != token_id);
            state.context_menu = None;
            state.draft_dirty = true;
        }
        ComposeMessage::ContextMenuMoveTo {
            token_id,
            from,
            to_field,
        } => {
            let source_tokens = match from {
                RecipientField::To => &mut state.to.tokens,
                RecipientField::Cc => &mut state.cc.tokens,
                RecipientField::Bcc => &mut state.bcc.tokens,
            };
            if let Some(pos) = source_tokens.iter().position(|t| t.id == token_id) {
                let mut token = source_tokens.remove(pos);
                let target = match to_field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                token.id = target.next_token_id();
                target.tokens.push(token);
                match to_field {
                    RecipientField::Cc => state.show_cc = true,
                    RecipientField::Bcc => state.show_bcc = true,
                    RecipientField::To => {}
                }
            }
            state.context_menu = None;
            state.draft_dirty = true;
        }
        ComposeMessage::ContextMenuExpandGroup { .. } => {
            state.context_menu = None;
        }
        ComposeMessage::ContextMenuCopy { .. } => {
            state.context_menu = None;
        }
        ComposeMessage::ContextMenuCut { field, token_id } => {
            let tokens = match field {
                RecipientField::To => &mut state.to.tokens,
                RecipientField::Cc => &mut state.cc.tokens,
                RecipientField::Bcc => &mut state.bcc.tokens,
            };
            tokens.retain(|t| t.id != token_id);
            state.context_menu = None;
            state.draft_dirty = true;
        }
        ComposeMessage::ContextMenuPaste { .. } => {
            state.context_menu = None;
        }
        ComposeMessage::GroupExpanded {
            field,
            token_id,
            members,
        } => {
            if let Ok(member_list) = members {
                let tokens = match field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                tokens.tokens.retain(|t| t.id != token_id);
                for (email, display_name) in member_list {
                    let label = display_name
                        .as_deref()
                        .filter(|n| !n.is_empty())
                        .unwrap_or(&email)
                        .to_string();
                    let id = tokens.next_token_id();
                    tokens.tokens.push(token_input::Token {
                        id,
                        email,
                        display_text: label,
                        is_group: false,
                        group_id: None,
                        member_count: None,
                    });
                }
                state.draft_dirty = true;
            }
        }
        ComposeMessage::DragStarted { field, token_id } => {
            let label = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .find(|t| t.id == token_id)
            .map(|t| t.display_text.clone())
            .unwrap_or_default();
            state.drag = Some(ComposeTokenDrag {
                token_id,
                source_field: field,
                display_text: label,
                current_position: Point::ORIGIN,
            });
        }
        ComposeMessage::DragMove(pos) => {
            if let Some(ref mut drag) = state.drag {
                drag.current_position = pos;
            }
        }
        ComposeMessage::DragEnd(_pos) => {
            state.drag = None;
        }
        ComposeMessage::DragCancel => {
            state.drag = None;
        }
        ComposeMessage::BccNudgeAccept(token_id) => {
            let source = if state.to.tokens.iter().any(|t| t.id == token_id) {
                Some(RecipientField::To)
            } else if state.cc.tokens.iter().any(|t| t.id == token_id) {
                Some(RecipientField::Cc)
            } else {
                None
            };
            if let Some(from) = source {
                let source_tokens = match from {
                    RecipientField::To => &mut state.to.tokens,
                    RecipientField::Cc => &mut state.cc.tokens,
                    RecipientField::Bcc => &mut state.bcc.tokens,
                };
                if let Some(pos) = source_tokens.iter().position(|t| t.id == token_id) {
                    let mut token = source_tokens.remove(pos);
                    token.id = state.bcc.next_token_id();
                    state.bcc.tokens.push(token);
                    state.show_bcc = true;
                }
            }
            state.bcc_nudges.retain(|n| n.token_id != token_id);
            state.draft_dirty = true;
        }
        ComposeMessage::BccNudgeDismiss(token_id) => {
            state.bcc_nudges.retain(|n| n.token_id != token_id);
        }
        ComposeMessage::BulkPasteDismiss => {
            state.bulk_paste_banner = None;
        }
        ComposeMessage::BulkPasteSaveAsGroup => {
            if state.bulk_paste_banner.is_some() {
                state.save_group_dialog_open = true;
                state.save_group_name = String::new();
                state.save_group_error = None;
                state.save_group_in_flight = false;
            }
        }
        ComposeMessage::GroupSaveNameChanged(s) => {
            state.save_group_name = s;
            state.save_group_error = None;
        }
        ComposeMessage::GroupSaveCancel => {
            state.save_group_dialog_open = false;
            state.save_group_name = String::new();
            state.save_group_error = None;
            state.save_group_in_flight = false;
        }
        ComposeMessage::GroupSaveConfirm => {
            if state.bulk_paste_banner.is_some() && !state.save_group_name.trim().is_empty() {
                state.save_group_in_flight = true;
                state.save_group_error = None;
            }
        }
        ComposeMessage::GroupSaveResult(Err(e)) => {
            state.save_group_in_flight = false;
            state.save_group_error = Some(e);
        }
        ComposeMessage::GroupSaveResult(Ok(success)) => {
            if let Some(banner) = state.bulk_paste_banner.take() {
                let target = match banner.field {
                    RecipientField::To => &mut state.to,
                    RecipientField::Cc => &mut state.cc,
                    RecipientField::Bcc => &mut state.bcc,
                };
                let pasted: std::collections::HashSet<TokenId> =
                    banner.token_ids.iter().copied().collect();
                let insert_at = target
                    .tokens
                    .iter()
                    .position(|t| pasted.contains(&t.id))
                    .unwrap_or(target.tokens.len());
                target.tokens.retain(|t| !pasted.contains(&t.id));
                let id = target.next_token_id();
                target.tokens.insert(
                    insert_at,
                    token_input::Token {
                        id,
                        email: String::new(),
                        display_text: success.name.clone(),
                        is_group: true,
                        group_id: Some(success.group_id),
                        member_count: Some(success.member_count),
                    },
                );
            }
            state.save_group_dialog_open = false;
            state.save_group_name = String::new();
            state.save_group_error = None;
            state.save_group_in_flight = false;
            state.draft_dirty = true;
        }
        ComposeMessage::PasteGroupMatchResult {
            field,
            added_ids,
            group,
        } => {
            let Some(matched) = group else {
                return;
            };
            let pasted: std::collections::HashSet<TokenId> = added_ids.iter().copied().collect();
            let target_now = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            };
            let still_present = target_now
                .tokens
                .iter()
                .filter(|t| pasted.contains(&t.id))
                .count();
            if still_present != pasted.len() {
                return;
            }
            let group_id_str = matched.id.as_str();
            let already_present = state
                .to
                .tokens
                .iter()
                .chain(state.cc.tokens.iter())
                .chain(state.bcc.tokens.iter())
                .any(|t| t.is_group && t.group_id.as_deref() == Some(group_id_str));
            let target = match field {
                RecipientField::To => &mut state.to,
                RecipientField::Cc => &mut state.cc,
                RecipientField::Bcc => &mut state.bcc,
            };
            let insert_at = target
                .tokens
                .iter()
                .position(|t| pasted.contains(&t.id))
                .unwrap_or(target.tokens.len());
            target.tokens.retain(|t| !pasted.contains(&t.id));
            if !already_present {
                let id = target.next_token_id();
                target.tokens.insert(
                    insert_at,
                    token_input::Token {
                        id,
                        email: String::new(),
                        display_text: matched.name,
                        is_group: true,
                        group_id: Some(matched.id),
                        member_count: Some(matched.member_count),
                    },
                );
            }
            if let Some(ref banner) = state.bulk_paste_banner
                && banner.field == field
                && banner.token_ids.iter().any(|t| pasted.contains(t))
            {
                state.bulk_paste_banner = None;
            }
            state.draft_dirty = true;
        }
        ComposeMessage::FormatBold => {
            state
                .body
                .perform(RteAction::Edit(EditAction::ToggleInlineStyle(
                    InlineStyle::BOLD,
                )));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatItalic => {
            state
                .body
                .perform(RteAction::Edit(EditAction::ToggleInlineStyle(
                    InlineStyle::ITALIC,
                )));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatUnderline => {
            state
                .body
                .perform(RteAction::Edit(EditAction::ToggleInlineStyle(
                    InlineStyle::UNDERLINE,
                )));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatStrikethrough => {
            state
                .body
                .perform(RteAction::Edit(EditAction::ToggleInlineStyle(
                    InlineStyle::STRIKETHROUGH,
                )));
            state.draft_dirty = true;
        }
        ComposeMessage::FormatList => {
            state.body.perform(RteAction::Edit(EditAction::SetBlockType(
                rte::BlockKind::ListItem { ordered: false },
            )));
            state.draft_dirty = true;
        }
        ComposeMessage::ToggleEmojiPicker => {
            state.emoji_picker_open = !state.emoji_picker_open;
            if !state.emoji_picker_open {
                state.emoji_picker_query.clear();
            }
        }
        ComposeMessage::EmojiPickerSearchChanged(q) => {
            state.emoji_picker_query = q;
        }
        ComposeMessage::EmojiPickerCategoryChanged(cat) => {
            state.emoji_picker_category = cat;
            state.emoji_picker_query.clear();
        }
        ComposeMessage::EmojiPickerSelected(emoji) => {
            state
                .body
                .perform(RteAction::Edit(EditAction::InsertText(emoji)));
            state.emoji_picker_open = false;
            state.emoji_picker_query.clear();
            state.draft_dirty = true;
        }
        ComposeMessage::FormatLink | ComposeMessage::ToggleLinkDialog => {
            if !state.link_dialog_open {
                state.link_text = state.body.selection_text();
                state.link_url.clear();
            }
            state.discard_confirm_open = false;
            state.context_menu = None;
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
            state.link_dialog_open = !state.link_dialog_open;
        }
        ComposeMessage::LinkUrlChanged(url) => state.link_url = url,
        ComposeMessage::LinkTextChanged(t) => state.link_text = t,
        ComposeMessage::LinkInsert => {
            let url = state.link_url.trim().to_string();
            let display = state.link_text.trim().to_string();
            if !url.is_empty() {
                let link_label = if display.is_empty() {
                    url.clone()
                } else {
                    display
                };
                if !state.body.selection.is_collapsed() {
                    state
                        .body
                        .perform(RteAction::Edit(EditAction::DeleteSelection));
                }
                state
                    .body
                    .perform(RteAction::Edit(EditAction::InsertText(link_label)));
            }
            state.link_dialog_open = false;
            state.link_url.clear();
            state.link_text.clear();
        }
        ComposeMessage::Noop => {}
        ComposeMessage::AttachFiles => {
            // Handled by the pop-out handler (async file picker)
        }
        ComposeMessage::FilesSelected(files) => {
            state.attachments.extend(files);
        }
        ComposeMessage::RemoveAttachment(idx) => {
            if idx < state.attachments.len() {
                state.attachments.remove(idx);
            }
        }
        ComposeMessage::SignatureResolved {
            signature_id,
            signature_html,
        } => {
            use rte::compose::replace_signature;

            state.signature_separator_index = replace_signature(
                &mut state.body.document,
                state.signature_separator_index,
                signature_html.as_deref(),
            );
            state.active_signature_id = signature_id;
            state.draft_dirty = true;
        }
    }

    // After every update, if a recipients-error is showing and any
    // recipient now exists, clear it so the inline error disappears the
    // moment the problem is fixed.
    if state.recipients_error.is_some()
        && (!state.to.tokens.is_empty()
            || !state.cc.tokens.is_empty()
            || !state.bcc.tokens.is_empty())
    {
        state.recipients_error = None;
    }
}
