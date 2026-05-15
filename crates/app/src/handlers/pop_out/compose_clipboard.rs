use std::sync::Arc;

use iced::Task;

use crate::db::Db;
use crate::pop_out::compose::ComposeMessage;
use crate::pop_out::PopOutMessage;
use crate::Message;

/// Shared implementation for "copy / cut a token" - whether the trigger is
/// the right-click context menu or the Ctrl+C / Ctrl+X key chords. Mutates
/// state via `update_compose` (Cut deletes the token; Copy is a no-op apart
/// from closing the context menu) and returns the clipboard-write task.
/// Group tokens require an async DB expand; regular tokens write
/// synchronously.
pub(super) fn handle_compose_copy_or_cut(
    state: &mut crate::pop_out::compose::ComposeState,
    db: &Arc<Db>,
    field: crate::pop_out::compose::RecipientField,
    token_id: crate::ui::token_input::TokenId,
    is_cut: bool,
) -> Task<Message> {
    use crate::pop_out::compose::RecipientField;
    let tokens = match field {
        RecipientField::To => &state.to.tokens,
        RecipientField::Cc => &state.cc.tokens,
        RecipientField::Bcc => &state.bcc.tokens,
    };
    let token = tokens.iter().find(|t| t.id == token_id);
    let group_id = token.and_then(|t| {
        if t.is_group {
            t.group_id.clone()
        } else {
            None
        }
    });
    let direct_string = token.and_then(token_clipboard_string);
    let mutate_msg = if is_cut {
        ComposeMessage::ContextMenuCut { field, token_id }
    } else {
        ComposeMessage::ContextMenuCopy { field, token_id }
    };
    crate::pop_out::compose::update_compose(state, mutate_msg);
    if let Some(gid) = group_id {
        let db_clone = Arc::clone(db);
        Task::perform(
            async move { db_clone.expand_contact_group(gid).await },
            |result| result,
        )
        .then(|expanded| match expanded {
            Ok(members) if !members.is_empty() => {
                iced::clipboard::write(group_members_clipboard_string(&members))
            }
            _ => Task::none(),
        })
    } else {
        match direct_string {
            Some(s) if !s.is_empty() => iced::clipboard::write(s),
            _ => Task::none(),
        }
    }
}

/// If `msg` is a token-input paste on a recipient field, return that field.
/// Used to detect when a paste just happened so we can look up whether the
/// pasted set matches an existing group.
pub(super) fn paste_field_for_message(
    msg: &ComposeMessage,
) -> Option<crate::pop_out::compose::RecipientField> {
    use crate::pop_out::compose::RecipientField;
    use crate::ui::token_input::TokenInputMessage;
    match msg {
        ComposeMessage::ToTokenInput(TokenInputMessage::Paste(_)) => Some(RecipientField::To),
        ComposeMessage::CcTokenInput(TokenInputMessage::Paste(_)) => Some(RecipientField::Cc),
        ComposeMessage::BccTokenInput(TokenInputMessage::Paste(_)) => Some(RecipientField::Bcc),
        _ => None,
    }
}

pub(super) fn field_token_count(
    state: &crate::pop_out::compose::ComposeState,
    field: crate::pop_out::compose::RecipientField,
) -> usize {
    use crate::pop_out::compose::RecipientField;
    match field {
        RecipientField::To => state.to.tokens.len(),
        RecipientField::Cc => state.cc.tokens.len(),
        RecipientField::Bcc => state.bcc.tokens.len(),
    }
}

/// After a paste reducer has run, look up whether the just-added addresses
/// exactly match an existing contact group; if so, the result message will
/// collapse them into a single group token. Skipped for pastes that added
/// fewer than 2 tokens since a 1-address paste can't sensibly recreate a
/// group.
pub(super) fn dispatch_paste_group_match(
    state: &crate::pop_out::compose::ComposeState,
    db: &Arc<Db>,
    window_id: iced::window::Id,
    field: crate::pop_out::compose::RecipientField,
    tokens_before: usize,
) -> Task<Message> {
    use crate::pop_out::compose::RecipientField;
    let field_tokens = match field {
        RecipientField::To => &state.to.tokens,
        RecipientField::Cc => &state.cc.tokens,
        RecipientField::Bcc => &state.bcc.tokens,
    };
    if field_tokens.len() <= tokens_before + 1 {
        return Task::none();
    }
    let added = &field_tokens[tokens_before..];
    let added_ids: Vec<crate::ui::token_input::TokenId> = added.iter().map(|t| t.id).collect();
    let added_emails: Vec<String> = added
        .iter()
        .map(|t| t.email.clone())
        .filter(|e| !e.is_empty())
        .collect();
    if added_emails.len() < 2 {
        return Task::none();
    }
    let db_clone = Arc::clone(db);
    Task::perform(
        async move { db_clone.find_group_matching_emails(added_emails).await },
        move |result| {
            let group = result.ok().flatten();
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::PasteGroupMatchResult {
                    field,
                    added_ids: added_ids.clone(),
                    group,
                }),
            )
        },
    )
}

/// Format a token for copy/cut to the clipboard. Prefers `Name <email>` so
/// the round-trip via paste rebuilds an equivalent token; falls back to the
/// bare email when there's no display name. Group tokens are handled
/// separately via DB expansion so this returns `None` for them - copying a
/// group must produce a `Name <email>, ...` list of its members, which
/// requires a DB lookup.
fn token_clipboard_string(t: &crate::ui::token_input::Token) -> Option<String> {
    if t.is_group {
        return None;
    }
    if t.email.is_empty() {
        return Some(t.display_text.clone());
    }
    Some(if t.display_text.is_empty() || t.display_text == t.email {
        t.email.clone()
    } else {
        format!("{} <{}>", t.display_text, t.email)
    })
}

/// Format a group's expanded members as a comma-separated `Name <email>`
/// list suitable for the clipboard.
fn group_members_clipboard_string(members: &[(String, Option<String>)]) -> String {
    members
        .iter()
        .map(|(email, name)| match name.as_deref().filter(|n| !n.is_empty()) {
            Some(n) => format!("{n} <{email}>"),
            None => email.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}
