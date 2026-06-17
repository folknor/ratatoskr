use iced::Rectangle;
use iced::advanced::{Clipboard, Shell};
use iced::keyboard;
use iced::mouse;

use super::layout::selected_index;
use super::types::TokenInputMessage;
use super::widget::{TokenInputState, TokenInputWidget};

pub(super) fn handle_left_click<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    state: &mut TokenInputState,
    cursor: mouse::Cursor,
    bounds: Rectangle,
    shell: &mut Shell<'_, M>,
) {
    let Some(pos) = cursor.position() else {
        return;
    };

    if bounds.contains(pos) {
        // Hit-test tokens
        for (i, token) in widget.tokens.iter().enumerate() {
            if let Some(chip) = state.token_bounds.get(i) {
                let abs = Rectangle {
                    x: bounds.x + chip.x,
                    y: bounds.y + chip.y,
                    width: chip.width,
                    height: chip.height,
                };
                if abs.contains(pos) {
                    if !state.is_focused {
                        state.is_focused = true;
                        shell.publish((widget.on_message)(TokenInputMessage::Focused));
                    }
                    shell.publish((widget.on_message)(TokenInputMessage::SelectToken(
                        token.id,
                    )));
                    shell.capture_event();
                    return;
                }
            }
        }

        // Clicked in field, not on token - focus
        if !state.is_focused {
            state.is_focused = true;
            shell.publish((widget.on_message)(TokenInputMessage::Focused));
        }
        shell.publish((widget.on_message)(TokenInputMessage::DeselectTokens));
        shell.capture_event();
        return;
    }

    // Clicked outside - blur
    if state.is_focused {
        state.is_focused = false;
        shell.publish((widget.on_message)(TokenInputMessage::Blurred));
    }
}

pub(super) fn handle_key_press<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    key: &keyboard::Key,
    modifiers: &keyboard::Modifiers,
    text: Option<&str>,
    clipboard: &mut dyn Clipboard,
    shell: &mut Shell<'_, M>,
) {
    match key {
        // Paste: Ctrl+V / Cmd+V
        keyboard::Key::Character(c)
            if (c.as_str() == "v" || c.as_str() == "V") && modifiers.command() =>
        {
            if let Some(content) = clipboard.read(iced::advanced::clipboard::Kind::Standard) {
                shell.publish((widget.on_message)(TokenInputMessage::Paste(content)));
                shell.capture_event();
            }
        }

        // Copy: Ctrl+C / Cmd+C - only when a token is selected (text-input
        // copy is handled by the inner text_input). The actual clipboard
        // write happens at the compose layer so group tokens can be
        // expanded asynchronously.
        keyboard::Key::Character(c)
            if (c.as_str() == "c" || c.as_str() == "C") && modifiers.command() =>
        {
            if let Some(selected) = widget.selected_token {
                shell.publish((widget.on_message)(TokenInputMessage::CopyToken(selected)));
                shell.capture_event();
            }
        }

        // Cut: Ctrl+X / Cmd+X - same path as Copy, plus token deletion.
        keyboard::Key::Character(c)
            if (c.as_str() == "x" || c.as_str() == "X") && modifiers.command() =>
        {
            if let Some(selected) = widget.selected_token {
                shell.publish((widget.on_message)(TokenInputMessage::CutToken(selected)));
                shell.capture_event();
            }
        }

        // Delete key: remove selected token
        keyboard::Key::Named(keyboard::key::Named::Delete) => {
            if let Some(selected) = widget.selected_token {
                shell.publish((widget.on_message)(TokenInputMessage::RemoveToken(
                    selected,
                )));
                shell.capture_event();
            }
        }

        // Backspace
        keyboard::Key::Named(keyboard::key::Named::Backspace) => {
            handle_backspace(widget, shell);
        }

        // Arrow Up/Down: autocomplete navigation when dropdown open
        keyboard::Key::Named(keyboard::key::Named::ArrowUp) if widget.autocomplete_open => {
            shell.publish((widget.on_message)(TokenInputMessage::AutocompleteUp));
            shell.capture_event();
        }
        keyboard::Key::Named(keyboard::key::Named::ArrowDown) if widget.autocomplete_open => {
            shell.publish((widget.on_message)(TokenInputMessage::AutocompleteDown));
            shell.capture_event();
        }

        // Left arrow: navigate to previous token
        keyboard::Key::Named(keyboard::key::Named::ArrowLeft) => {
            handle_arrow_left(widget, shell);
        }

        // Right arrow: navigate to next token or text
        keyboard::Key::Named(keyboard::key::Named::ArrowRight) => {
            handle_arrow_right(widget, shell);
        }

        // Escape: dismiss autocomplete if open, otherwise blur
        keyboard::Key::Named(keyboard::key::Named::Escape) => {
            if widget.autocomplete_open {
                shell.publish((widget.on_message)(
                    TokenInputMessage::AutocompleteDismissKey,
                ));
            } else {
                shell.publish((widget.on_message)(TokenInputMessage::Blurred));
            }
            shell.capture_event();
        }

        // Enter / Tab: accept autocomplete if open, otherwise tokenize
        keyboard::Key::Named(keyboard::key::Named::Enter | keyboard::key::Named::Tab) => {
            if widget.autocomplete_open {
                shell.publish((widget.on_message)(TokenInputMessage::AutocompleteAccept));
                shell.capture_event();
            } else if !widget.text.is_empty() {
                let text = widget.text.to_string();
                shell.publish((widget.on_message)(TokenInputMessage::TokenizeText(text)));
                shell.capture_event();
            }
        }

        // Comma / Semicolon: always tokenize
        keyboard::Key::Character(c)
            if !modifiers.command() && (c.as_str() == "," || c.as_str() == ";") =>
        {
            if !widget.text.is_empty() {
                let text = widget.text.to_string();
                shell.publish((widget.on_message)(TokenInputMessage::TokenizeText(text)));
            }
            shell.capture_event();
        }

        // Space: tokenize if looks like email, else append
        keyboard::Key::Named(keyboard::key::Named::Space) if !modifiers.command() => {
            handle_space(widget, shell);
        }

        // Regular character input. Prefer the `text` field from the
        // KeyPressed event (post-modifier, post-IME) over `Key::Character`,
        // which only carries the unmodified logical key. Otherwise typing
        // e.g. Shift+2 on a non-US layout inserts "2" instead of "@".
        keyboard::Key::Character(c) if !modifiers.command() => {
            if widget.selected_token.is_some() {
                shell.publish((widget.on_message)(TokenInputMessage::DeselectTokens));
            }
            let to_append = text.unwrap_or(c.as_str());
            let new_text = format!("{}{}", widget.text, to_append);
            shell.publish((widget.on_message)(TokenInputMessage::TextChanged(
                new_text,
            )));
            shell.capture_event();
        }

        _ => {}
    }
}

fn handle_backspace<M: Clone>(widget: &TokenInputWidget<'_, M>, shell: &mut Shell<'_, M>) {
    if widget.text.is_empty() {
        if let Some(selected) = widget.selected_token {
            shell.publish((widget.on_message)(TokenInputMessage::RemoveToken(
                selected,
            )));
            shell.capture_event();
            return;
        }
        if !widget.tokens.is_empty() {
            shell.publish((widget.on_message)(TokenInputMessage::BackspaceAtStart));
            shell.capture_event();
        }
    } else {
        let mut new_text = widget.text.to_string();
        new_text.pop();
        shell.publish((widget.on_message)(TokenInputMessage::TextChanged(
            new_text,
        )));
        shell.capture_event();
    }
}

fn handle_arrow_left<M: Clone>(widget: &TokenInputWidget<'_, M>, shell: &mut Shell<'_, M>) {
    if widget.tokens.is_empty() {
        return;
    }

    match selected_index(widget.tokens, widget.selected_token) {
        Some(idx) if idx > 0 => {
            // Move selection to previous token
            shell.publish((widget.on_message)(TokenInputMessage::ArrowSelectToken(
                widget.tokens[idx - 1].id,
            )));
            shell.capture_event();
        }
        Some(_) => {
            // Already at first token, do nothing
            shell.capture_event();
        }
        None if widget.text.is_empty() => {
            // At text position 0 with no text: select last token
            if let Some(last) = widget.tokens.last() {
                shell.publish((widget.on_message)(TokenInputMessage::ArrowSelectToken(
                    last.id,
                )));
                shell.capture_event();
            }
        }
        None => {}
    }
}

fn handle_arrow_right<M: Clone>(widget: &TokenInputWidget<'_, M>, shell: &mut Shell<'_, M>) {
    if widget.tokens.is_empty() {
        return;
    }

    if let Some(idx) = selected_index(widget.tokens, widget.selected_token) {
        if idx + 1 < widget.tokens.len() {
            // Move selection to next token
            shell.publish((widget.on_message)(TokenInputMessage::ArrowSelectToken(
                widget.tokens[idx + 1].id,
            )));
        } else {
            // At last token: deselect and focus text
            shell.publish((widget.on_message)(TokenInputMessage::ArrowToText));
        }
        shell.capture_event();
    }
}

fn handle_space<M: Clone>(widget: &TokenInputWidget<'_, M>, shell: &mut Shell<'_, M>) {
    if !widget.text.is_empty() && widget.text.contains('@') {
        let text = widget.text.to_string();
        shell.publish((widget.on_message)(TokenInputMessage::TokenizeText(text)));
    } else if !widget.text.is_empty() {
        let new_text = format!("{} ", widget.text);
        shell.publish((widget.on_message)(TokenInputMessage::TextChanged(
            new_text,
        )));
    }
    shell.capture_event();
}
