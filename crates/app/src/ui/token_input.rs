//! Token input widget for chip/tag input with inline tokens.
//!
//! Used in compose recipient fields (To/Cc/Bcc), calendar attendee fields,
//! and the contact group editor. The widget is context-agnostic - all
//! compose-specific behavior (autocomplete dropdown, Bcc suggestions,
//! cross-field drag) lives in the parent view, not here.
//!
//! Built as a custom `advanced::Widget` with a wrapping flow layout of
//! token chip backgrounds + text, followed by an inline text input area.
//! The parent manages the token list and text input state via Elm architecture.

use iced::advanced::layout;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::text::Renderer as TextRenderer;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::keyboard;
use iced::mouse;
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, Vector, border};

use crate::font;
use crate::ui::layout::*;

// ── Data types ──────────────────────────────────────────

/// A single token displayed inline in the input field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// Unique ID for this token instance.
    pub id: TokenId,
    /// The email address this token represents.
    pub email: String,
    /// Display label shown on the token chip.
    pub label: String,
    /// Whether this token represents a contact group.
    pub is_group: bool,
    /// Group ID if this is a group token (for expand operations).
    pub group_id: Option<String>,
    /// Member count for group tokens (displayed as suffix).
    pub member_count: Option<i64>,
}

/// Opaque token identifier. Wraps a u64 counter, monotonically increasing
/// per widget instance to guarantee uniqueness across add/remove cycles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TokenId(pub u64);

/// Which recipient field this widget instance represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecipientField {
    To,
    Cc,
    Bcc,
}

/// Persistent state owned by the caller (lives in the compose model).
/// Passed as data to the widget constructor - the widget does not own this.
pub struct TokenInputValue {
    /// Current tokens in this field.
    pub tokens: Vec<Token>,
    /// Current text being typed (after the last token).
    pub text: String,
    /// Next token ID counter.
    pub next_id: u64,
}

impl TokenInputValue {
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            text: String::new(),
            next_id: 0,
        }
    }

    pub fn next_token_id(&mut self) -> TokenId {
        let id = TokenId(self.next_id);
        self.next_id += 1;
        id
    }
}

impl Default for TokenInputValue {
    fn default() -> Self {
        Self::new()
    }
}

/// Messages emitted by the token input widget upward to the caller.
#[derive(Debug, Clone)]
pub enum TokenInputMessage {
    /// The text input content changed.
    TextChanged(String),
    /// A token should be removed.
    RemoveToken(TokenId),
    /// Raw text should be tokenized (triggered by delimiter keys).
    TokenizeText(String),
    /// A token was clicked (selected).
    SelectToken(TokenId),
    /// Click in empty area - deselect any token.
    DeselectTokens,
    /// Focus was gained by this field.
    Focused,
    /// Focus was lost.
    Blurred,
    /// A paste event with raw text. Caller parses and tokenizes.
    Paste(String),
    /// Backspace was pressed at the start of the text input.
    BackspaceAtStart,
    /// Right-click on a token - emit position for context menu.
    TokenContextMenu(TokenId, Point),
    /// Arrow key navigated to select a token by index.
    ArrowSelectToken(TokenId),
    /// Arrow right from last token - deselect and focus text.
    ArrowToText,
    /// A drag was initiated on a token (exceeded 4px threshold).
    DragStarted(TokenId),
    // ── Autocomplete keyboard events (emitted when autocomplete_open) ──
    /// Arrow down when autocomplete dropdown is visible.
    AutocompleteDown,
    /// Arrow up when autocomplete dropdown is visible.
    AutocompleteUp,
    /// Enter/Tab when autocomplete dropdown is visible - accept selection.
    AutocompleteAccept,
    /// Escape when autocomplete dropdown is visible - dismiss dropdown.
    AutocompleteDismissKey,
}

// ── Widget state ────────────────────────────────────────

/// Transient widget state stored in iced's tree.
#[derive(Debug, Default)]
struct TokenInputState {
    /// Cached per-token bounds from the last layout pass (relative to
    /// widget origin).
    token_bounds: Vec<Rectangle>,
    /// Whether the widget is focused.
    is_focused: bool,
    /// Tracking a potential drag: (token_id, mouse_down_origin).
    drag_tracking: Option<(TokenId, Point)>,
}

// ── Widget struct ───────────────────────────────────────

struct TokenInputWidget<'a, M> {
    tokens: &'a [Token],
    text: &'a str,
    placeholder: &'a str,
    on_message: Box<dyn Fn(TokenInputMessage) -> M + 'a>,
    /// Which token the parent considers selected (for backspace flow).
    selected_token: Option<TokenId>,
    /// Whether the autocomplete dropdown is currently visible.
    /// When true, ArrowUp/Down/Enter/Tab/Escape emit autocomplete messages
    /// instead of normal token navigation.
    autocomplete_open: bool,
}

/// Creates a token input field element.
///
/// This is a custom `advanced::Widget` that renders token chips in a
/// wrapping flow layout followed by a text input area.
///
/// # Arguments
/// * `tokens` - current tokens (from the model)
/// * `text` - current input text (from the model)
/// * `placeholder` - placeholder text when empty
/// * `selected_token` - which token is selected (for backspace-delete flow)
/// * `on_message` - callback converting `TokenInputMessage` to the caller's
///   message type
pub fn token_input_field<'a, M: Clone + 'a>(
    tokens: &'a [Token],
    text: &'a str,
    placeholder: &'a str,
    selected_token: Option<TokenId>,
    autocomplete_open: bool,
    on_message: impl Fn(TokenInputMessage) -> M + 'a,
) -> Element<'a, M> {
    TokenInputWidget {
        tokens,
        text,
        placeholder,
        on_message: Box::new(on_message),
        selected_token,
        autocomplete_open,
    }
    .into()
}

// ── Layout helpers ──────────────────────────────────────

/// Build the display label for a token chip. Group tokens with a known member
/// count get a " (N)" suffix.
fn chip_display_label(token: &Token) -> String {
    match (token.is_group, token.member_count) {
        (true, Some(n)) => format!("{} ({n})", token.label),
        _ => token.label.clone(),
    }
}

/// Estimate token chip width from label text.
///
/// Uses character count (not byte count) for correct non-ASCII width.
/// Group tokens include space for the people icon prefix.
fn estimate_token_width(token: &Token) -> f32 {
    let avg_char_width = TEXT_MD * TOKEN_AVG_CHAR_WIDTH_FACTOR;
    let display = chip_display_label(token);
    #[allow(clippy::cast_precision_loss)]
    let text_width = display.chars().count() as f32 * avg_char_width;
    let icon_width = if token.is_group {
        TOKEN_GROUP_ICON_SIZE + SPACE_XXS
    } else {
        0.0
    };
    text_width + icon_width + PAD_TOKEN.left + PAD_TOKEN.right
}

/// Compute the text area origin from the token bounds.
fn text_area_origin(token_bounds: &[Rectangle], field_width: f32) -> (f32, f32) {
    if let Some(last) = token_bounds.last() {
        let next_x = last.x + last.width + TOKEN_SPACING;
        let inner_width = field_width - PAD_TOKEN_INPUT.left - PAD_TOKEN_INPUT.right;
        let remaining = inner_width - (next_x - PAD_TOKEN_INPUT.left);
        if remaining < TOKEN_TEXT_MIN_WIDTH {
            (
                PAD_TOKEN_INPUT.left,
                last.y + TOKEN_HEIGHT + TOKEN_ROW_SPACING,
            )
        } else {
            (next_x, last.y)
        }
    } else {
        (PAD_TOKEN_INPUT.left, PAD_TOKEN_INPUT.top)
    }
}

// ── Group icon drawing ──────────────────────────────────

/// Draw the people icon glyph for group tokens using the icon font.
fn draw_group_icon(
    renderer: &mut iced::Renderer,
    position: Point,
    color: iced::Color,
    clip: Rectangle,
) {
    // Lucide "users" icon: U+E1A4
    renderer.fill_text(
        iced::advanced::text::Text {
            content: "\u{e1a4}".to_string(),
            bounds: Size::new(TOKEN_GROUP_ICON_SIZE, TOKEN_HEIGHT),
            size: iced::Pixels(TOKEN_GROUP_ICON_SIZE),
            line_height: iced::advanced::text::LineHeight::default(),
            font: crate::font::ICON,
            align_x: iced::advanced::text::Alignment::Left,
            align_y: iced::alignment::Vertical::Center,
            shaping: iced::advanced::text::Shaping::Advanced,
            wrapping: iced::advanced::text::Wrapping::None,
            ellipsis: iced::advanced::text::Ellipsis::None,
            hint_factor: None,
        },
        position,
        color,
        clip,
    );
}

// ── Arrow key helpers ───────────────────────────────────

/// Find the index of the currently selected token.
fn selected_index(tokens: &[Token], selected: Option<TokenId>) -> Option<usize> {
    let sel = selected?;
    tokens.iter().position(|t| t.id == sel)
}

// ── Widget implementation ───────────────────────────────

impl<M: Clone> Widget<M, Theme, iced::Renderer> for TokenInputWidget<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<TokenInputState>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(TokenInputState::default())
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Shrink)
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let state = tree.state.downcast_mut::<TokenInputState>();
        let max_width = limits.max().width;
        let inner_width = max_width - PAD_TOKEN_INPUT.left - PAD_TOKEN_INPUT.right;

        let mut x: f32 = 0.0;
        let mut y: f32 = 0.0;
        state.token_bounds.clear();

        // Layout each token chip in a wrapping flow
        for token in self.tokens {
            let chip_width = estimate_token_width(token);
            if x + chip_width > inner_width && x > 0.0 {
                x = 0.0;
                y += TOKEN_HEIGHT + TOKEN_ROW_SPACING;
            }
            state.token_bounds.push(Rectangle {
                x: PAD_TOKEN_INPUT.left + x,
                y: PAD_TOKEN_INPUT.top + y,
                width: chip_width,
                height: TOKEN_HEIGHT,
            });
            x += chip_width + TOKEN_SPACING;
        }

        // Text input wraps to a new row if not enough space
        let remaining = inner_width - x;
        if remaining < TOKEN_TEXT_MIN_WIDTH && x > 0.0 {
            y += TOKEN_HEIGHT + TOKEN_ROW_SPACING;
        }

        let total_height = PAD_TOKEN_INPUT.top + y + TOKEN_HEIGHT + PAD_TOKEN_INPUT.bottom;

        layout::Node::new(Size::new(max_width, total_height))
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        _style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_ref::<TokenInputState>();
        let bounds = layout.bounds();
        let palette = theme.palette();

        // Field background + border
        let border_color = if state.is_focused {
            palette.primary.base.color
        } else if cursor.is_over(bounds) {
            palette.background.strongest.color.scale_alpha(0.3)
        } else {
            palette.background.strongest.color.scale_alpha(0.15)
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: border::Border {
                    color: border_color,
                    width: 1.0,
                    radius: RADIUS_SM.into(),
                },
                ..renderer::Quad::default()
            },
            palette.background.weak.color,
        );

        // Draw each token chip
        for (i, token) in self.tokens.iter().enumerate() {
            let Some(chip_bounds) = state.token_bounds.get(i) else {
                continue;
            };
            let abs = Rectangle {
                x: bounds.x + chip_bounds.x,
                y: bounds.y + chip_bounds.y,
                width: chip_bounds.width,
                height: chip_bounds.height,
            };

            let is_selected = self.selected_token == Some(token.id);
            let is_hovered = cursor.position().is_some_and(|pos| abs.contains(pos));

            let chip_bg = if is_selected {
                palette.primary.base.color
            } else if is_hovered {
                palette.background.weaker.color
            } else {
                palette.background.weakest.color
            };
            let text_color = if is_selected {
                palette.primary.base.text
            } else {
                palette.background.base.text
            };

            // Chip background
            renderer.fill_quad(
                renderer::Quad {
                    bounds: abs,
                    border: border::Border {
                        color: iced::Color::TRANSPARENT,
                        width: 0.0,
                        radius: TOKEN_RADIUS.into(),
                    },
                    ..renderer::Quad::default()
                },
                chip_bg,
            );

            // Group icon prefix for group tokens
            let label_x_offset = if token.is_group {
                draw_group_icon(
                    renderer,
                    Point::new(abs.x + PAD_TOKEN.left, abs.y),
                    text_color,
                    abs,
                );
                TOKEN_GROUP_ICON_SIZE + SPACE_XXS
            } else {
                0.0
            };

            // Chip label (with "(N)" suffix for group tokens)
            renderer.fill_text(
                iced::advanced::text::Text {
                    content: chip_display_label(token),
                    bounds: Size::new(
                        abs.width - PAD_TOKEN.left - PAD_TOKEN.right - label_x_offset,
                        abs.height,
                    ),
                    size: iced::Pixels(TEXT_MD),
                    line_height: iced::advanced::text::LineHeight::default(),
                    font: font::text(),
                    align_x: iced::advanced::text::Alignment::Left,
                    align_y: iced::alignment::Vertical::Center,
                    shaping: iced::advanced::text::Shaping::Advanced,
                    wrapping: iced::advanced::text::Wrapping::None,
                    ellipsis: iced::advanced::text::Ellipsis::None,
                    hint_factor: None,
                },
                Point::new(abs.x + PAD_TOKEN.left + label_x_offset, abs.y),
                text_color,
                abs,
            );
        }

        // Text area: placeholder or current text
        draw_text_area(self, state, renderer, palette, bounds);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        _viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<TokenInputState>();
        let bounds = layout.bounds();

        match event {
            // ── Mouse: right click ──────────────────────
            Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Right,
                ..
            }) => {
                if let Some(pos) = cursor.position() {
                    if bounds.contains(pos) {
                        for (i, token) in self.tokens.iter().enumerate() {
                            if let Some(chip) = state.token_bounds.get(i) {
                                let abs = Rectangle {
                                    x: bounds.x + chip.x,
                                    y: bounds.y + chip.y,
                                    width: chip.width,
                                    height: chip.height,
                                };
                                if abs.contains(pos) {
                                    shell.publish((self.on_message)(
                                        TokenInputMessage::TokenContextMenu(token.id, pos),
                                    ));
                                    shell.capture_event();
                                    return;
                                }
                            }
                        }
                    }
                }
            }

            // ── Mouse: left click ──────────────────────────
            Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Left,
                ..
            })
            | Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
                // Start drag tracking if clicking on a token
                if let Some(pos) = cursor.position() {
                    if bounds.contains(pos) {
                        for (i, token) in self.tokens.iter().enumerate() {
                            if let Some(chip) = state.token_bounds.get(i) {
                                let abs = Rectangle {
                                    x: bounds.x + chip.x,
                                    y: bounds.y + chip.y,
                                    width: chip.width,
                                    height: chip.height,
                                };
                                if abs.contains(pos) {
                                    state.drag_tracking = Some((token.id, pos));
                                    break;
                                }
                            }
                        }
                    }
                }
                handle_left_click(self, state, cursor, bounds, shell);
            }

            // ── Mouse move: drag detection ──────────────────
            Event::Mouse(mouse::Event::CursorMoved { .. }) => {
                if let (Some((token_id, origin)), Some(pos)) =
                    (state.drag_tracking, cursor.position())
                {
                    let dx = pos.x - origin.x;
                    let dy = pos.y - origin.y;
                    if dx * dx + dy * dy > 16.0 {
                        // 4px threshold
                        state.drag_tracking = None;
                        shell.publish((self.on_message)(TokenInputMessage::DragStarted(token_id)));
                    }
                }
            }

            // ── Mouse up: cancel drag tracking ──────────────
            Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                state.drag_tracking = None;
            }

            // ── Keyboard events (only when focused) ────────
            Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. })
                if state.is_focused =>
            {
                handle_key_press(self, key, modifiers, clipboard, shell);
            }

            _ => {}
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        _viewport: &Rectangle,
        _renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let state = tree.state.downcast_ref::<TokenInputState>();
        let bounds = layout.bounds();

        if let Some(pos) = cursor.position() {
            if bounds.contains(pos) {
                for chip in &state.token_bounds {
                    let abs = Rectangle {
                        x: bounds.x + chip.x,
                        y: bounds.y + chip.y,
                        width: chip.width,
                        height: chip.height,
                    };
                    if abs.contains(pos) {
                        return mouse::Interaction::Pointer;
                    }
                }
                return mouse::Interaction::Text;
            }
        }

        mouse::Interaction::default()
    }
}

// ── Extracted event handlers (keep update() under 100 lines) ──

fn handle_left_click<M: Clone>(
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

fn handle_key_press<M: Clone>(
    widget: &TokenInputWidget<'_, M>,
    key: &keyboard::Key,
    modifiers: &keyboard::Modifiers,
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

        // Regular character input
        keyboard::Key::Character(c) if !modifiers.command() => {
            if widget.selected_token.is_some() {
                shell.publish((widget.on_message)(TokenInputMessage::DeselectTokens));
            }
            let new_text = format!("{}{}", widget.text, c.as_str());
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

fn draw_text_area(
    widget: &TokenInputWidget<'_, impl Clone>,
    state: &TokenInputState,
    renderer: &mut iced::Renderer,
    palette: &iced::theme::Palette,
    bounds: Rectangle,
) {
    let (text_x, text_y) = text_area_origin(&state.token_bounds, bounds.width);

    let display_text = if widget.text.is_empty() && widget.tokens.is_empty() {
        widget.placeholder
    } else {
        widget.text
    };

    let text_color = if widget.text.is_empty() && widget.tokens.is_empty() {
        palette.background.base.text.scale_alpha(0.4)
    } else {
        palette.background.base.text
    };

    if !display_text.is_empty() {
        let text_area_width = bounds.width - text_x - PAD_TOKEN_INPUT.right;
        renderer.fill_text(
            iced::advanced::text::Text {
                content: display_text.to_string(),
                bounds: Size::new(text_area_width, TOKEN_HEIGHT),
                size: iced::Pixels(TEXT_MD),
                line_height: iced::advanced::text::LineHeight::default(),
                font: font::text(),
                align_x: iced::advanced::text::Alignment::Left,
                align_y: iced::alignment::Vertical::Center,
                shaping: iced::advanced::text::Shaping::Advanced,
                wrapping: iced::advanced::text::Wrapping::None,
                ellipsis: iced::advanced::text::Ellipsis::None,
                hint_factor: None,
            },
            Point::new(bounds.x + text_x, bounds.y + text_y),
            text_color,
            Rectangle {
                x: bounds.x + text_x,
                y: bounds.y + text_y,
                width: text_area_width,
                height: TOKEN_HEIGHT,
            },
        );
    }

    // Text cursor when focused and no token selected
    if state.is_focused && widget.selected_token.is_none() {
        #[allow(clippy::cast_precision_loss)]
        let cursor_x = if widget.text.is_empty() {
            bounds.x + text_x
        } else {
            let text_width =
                widget.text.chars().count() as f32 * TEXT_MD * TOKEN_AVG_CHAR_WIDTH_FACTOR;
            bounds.x + text_x + text_width
        };
        let cursor_y = bounds.y + text_y + SPACE_XXXS;
        let cursor_height = TOKEN_HEIGHT - SPACE_XXS;

        renderer.fill_quad(
            renderer::Quad {
                bounds: Rectangle {
                    x: cursor_x,
                    y: cursor_y,
                    width: 1.0,
                    height: cursor_height,
                },
                ..renderer::Quad::default()
            },
            palette.background.base.text,
        );
    }
}

impl<'a, M: Clone + 'a> From<TokenInputWidget<'a, M>> for Element<'a, M> {
    fn from(widget: TokenInputWidget<'a, M>) -> Self {
        Self::new(widget)
    }
}

// ── Helper: email validation ────────────────────────────

/// Minimal validation - catches obvious typos, not RFC 5321.
pub fn is_plausible_email(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    let Some((local, domain)) = trimmed.split_once('@') else {
        return false;
    };
    !local.is_empty() && domain.contains('.')
}
