use iced::Point;

use crate::ui::token_input::{self, TokenId, TokenInputMessage, TokenInputValue};

use super::state::ComposeState;
use super::types::{
    BccNudgeBanner, BulkPasteBanner, ComposeTokenDrag, ContextMenuKind, RecipientField,
    TokenContextMenuState,
};

/// Handle a token input message for a specific recipient field.
/// Intercepts autocomplete keyboard events before delegating to the
/// generic token input handler.
pub(super) fn handle_recipient_token_input(
    state: &mut ComposeState,
    field: RecipientField,
    inner: TokenInputMessage,
) {
    match &inner {
        TokenInputMessage::AutocompleteDown => {
            let len = state.autocomplete.results.len();
            if len > 0 {
                let next = state
                    .autocomplete
                    .highlighted
                    .map_or(0, |h| (h + 1).min(len - 1));
                state.autocomplete.highlighted = Some(next);
            }
            return;
        }
        TokenInputMessage::AutocompleteUp => {
            if let Some(h) = state.autocomplete.highlighted {
                state.autocomplete.highlighted = Some(h.saturating_sub(1));
            }
            return;
        }
        TokenInputMessage::AutocompleteAccept => {
            let idx = state.autocomplete.highlighted.unwrap_or(0);
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
                let is_group = match_entry.is_group;
                let token_label = label.clone();
                target.tokens.push(token_input::Token {
                    id,
                    email: match_entry.email,
                    display_text: label,
                    is_group,
                    group_id: match_entry.group_id,
                    member_count: match_entry.member_count,
                });
                target.text.clear();
                state.autocomplete.results.clear();
                state.autocomplete.highlighted = None;
                state.autocomplete.query.clear();
                state.draft_dirty = true;

                let active = state.autocomplete.active_field;
                if is_group && (active == RecipientField::To || active == RecipientField::Cc) {
                    state.bcc_nudges.push(BccNudgeBanner {
                        group_name: token_label,
                        token_id: id,
                        source_field: active,
                    });
                }
            }
            return;
        }
        TokenInputMessage::AutocompleteDismissKey => {
            state.autocomplete.results.clear();
            state.autocomplete.highlighted = None;
            return;
        }
        TokenInputMessage::DragStarted(token_id) => {
            let tid = *token_id;
            let label = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .find(|t| t.id == tid)
            .map(|t| t.display_text.clone())
            .unwrap_or_default();
            state.drag = Some(ComposeTokenDrag {
                token_id: tid,
                source_field: field,
                display_text: label,
                current_position: Point::ORIGIN,
            });
            return;
        }
        TokenInputMessage::TokenContextMenu(token_id, position) => {
            let tid = *token_id;
            let pos = *position;
            let is_group = match field {
                RecipientField::To => &state.to,
                RecipientField::Cc => &state.cc,
                RecipientField::Bcc => &state.bcc,
            }
            .tokens
            .iter()
            .any(|t| t.id == tid && t.is_group);

            state.context_menu = Some(TokenContextMenuState {
                field,
                position: pos,
                kind: ContextMenuKind::Token {
                    token_id: tid,
                    is_group,
                },
            });
            return;
        }
        TokenInputMessage::FieldContextMenu(position) => {
            state.context_menu = Some(TokenContextMenuState {
                field,
                position: *position,
                kind: ContextMenuKind::Field,
            });
            return;
        }
        _ => {}
    }

    if let TokenInputMessage::TextChanged(ref t) = inner {
        state.autocomplete.query = t.clone();
        state.autocomplete.active_field = field;
    }

    let is_paste = matches!(&inner, TokenInputMessage::Paste(_));
    let tokens_before = if is_paste {
        match field {
            RecipientField::To => state.to.tokens.len(),
            RecipientField::Cc => state.cc.tokens.len(),
            RecipientField::Bcc => state.bcc.tokens.len(),
        }
    } else {
        0
    };

    match field {
        RecipientField::To => {
            state.selected_cc_token = None;
            state.selected_bcc_token = None;
        }
        RecipientField::Cc => {
            state.selected_to_token = None;
            state.selected_bcc_token = None;
        }
        RecipientField::Bcc => {
            state.selected_to_token = None;
            state.selected_cc_token = None;
        }
    }
    let (value, selected) = match field {
        RecipientField::To => (&mut state.to, &mut state.selected_to_token),
        RecipientField::Cc => (&mut state.cc, &mut state.selected_cc_token),
        RecipientField::Bcc => (&mut state.bcc, &mut state.selected_bcc_token),
    };
    handle_token_input_message(value, inner, selected);
    state.draft_dirty = true;

    if is_paste {
        let field_tokens = match field {
            RecipientField::To => &state.to.tokens,
            RecipientField::Cc => &state.cc.tokens,
            RecipientField::Bcc => &state.bcc.tokens,
        };
        let tokens_after = field_tokens.len();
        let added = tokens_after.saturating_sub(tokens_before);
        if added >= 10 {
            let token_ids = field_tokens
                .iter()
                .skip(tokens_before)
                .map(|t| t.id)
                .collect();
            state.bulk_paste_banner = Some(BulkPasteBanner {
                count: added,
                field,
                token_ids,
            });
        }
    }
}

fn handle_token_input_message(
    value: &mut TokenInputValue,
    msg: TokenInputMessage,
    selected: &mut Option<TokenId>,
) {
    match msg {
        TokenInputMessage::TextChanged(text) => value.text = text,
        TokenInputMessage::RemoveToken(id) => {
            value.tokens.retain(|t| t.id != id);
            *selected = None;
        }
        TokenInputMessage::TokenizeText(text) => {
            let parsed =
                import::parse_recipient_paste(&import::RecipientPastePayload::from_plain_text(text));
            if push_parsed_recipients(value, parsed.recipients) > 0 {
                value.text.clear();
            }
        }
        TokenInputMessage::SelectToken(id) => *selected = Some(id),
        TokenInputMessage::DeselectTokens => *selected = None,
        TokenInputMessage::BackspaceAtStart => {
            if let Some(last) = value.tokens.last() {
                *selected = Some(last.id);
            }
        }
        TokenInputMessage::Focused | TokenInputMessage::Blurred => {}
        TokenInputMessage::TokenContextMenu(_, _)
        | TokenInputMessage::FieldContextMenu(_) => {
            // Handled at the compose level via handle_recipient_token_input
        }
        TokenInputMessage::CopyToken(_) | TokenInputMessage::CutToken(_) => {
            // Handled at the app layer (handlers/pop_out.rs) which translates
            // these to ContextMenuCopy/Cut so the same async DB-expand-then-
            // clipboard-write logic runs whether the trigger is a key chord
            // or the right-click menu.
        }
        TokenInputMessage::ArrowSelectToken(_) => {}
        TokenInputMessage::ArrowToText => {}
        TokenInputMessage::AutocompleteDown
        | TokenInputMessage::AutocompleteUp
        | TokenInputMessage::AutocompleteAccept
        | TokenInputMessage::AutocompleteDismissKey
        | TokenInputMessage::DragStarted(_) => {}
        TokenInputMessage::Paste(content) => {
            let parsed = import::parse_recipient_paste(
                &import::RecipientPastePayload::from_plain_text(content),
            );
            let mut recipients = parsed.recipients;
            let existing: std::collections::HashSet<String> = value
                .tokens
                .iter()
                .map(|t| t.email.to_lowercase())
                .collect();
            recipients.retain(|addr| !existing.contains(&addr.email.to_lowercase()));
            if push_parsed_recipients(value, recipients) > 0 {
                value.text.clear();
            }
        }
    }
}

fn push_parsed_recipients(
    value: &mut TokenInputValue,
    parsed: Vec<import::ParsedRecipient>,
) -> usize {
    let mut added = 0usize;
    for addr in parsed {
        let label = addr
            .display_name
            .as_deref()
            .filter(|n| !n.is_empty())
            .unwrap_or(&addr.email)
            .to_string();
        let id = value.next_token_id();
        value.tokens.push(token_input::Token {
            id,
            email: addr.email,
            display_text: label,
            is_group: false,
            group_id: None,
            member_count: None,
        });
        added += 1;
    }
    added
}
