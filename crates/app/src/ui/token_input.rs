//! Token input widget for chip/tag input with inline tokens.
//!
//! Used in compose recipient fields (To/Cc/Bcc), calendar attendee fields,
//! and the contact group editor. The widget is context-agnostic — all
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
use iced::{border, Element, Event, Length, Rectangle, Size, Theme, Vector};

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
/// Passed as data to the widget constructor — the widget does not own this.
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
    /// Click in empty area — deselect any token.
    DeselectTokens,
    /// Focus was gained by this field.
    Focused,
    /// Focus was lost.
    Blurred,
    /// A paste event with raw text. Caller parses and tokenizes.
    Paste(String),
    /// Backspace was pressed at the start of the text input.
    BackspaceAtStart,
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
}

// ── Widget struct ───────────────────────────────────────

struct TokenInputWidget<'a, M> {
    tokens: &'a [Token],
    text: &'a str,
    placeholder: &'a str,
    on_message: Box<dyn Fn(TokenInputMessage) -> M + 'a>,
    /// Which token the parent considers selected (for backspace flow).
    selected_token: Option<TokenId>,
}

/// Creates a token input field element.
///
/// This is a custom `advanced::Widget` that renders token chips in a
/// wrapping flow layout followed by a text input area.
///
/// # Arguments
/// * `tokens` — current tokens (from the model)
/// * `text` — current input text (from the model)
/// * `placeholder` — placeholder text when empty
/// * `selected_token` — which token is selected (for backspace-delete flow)
/// * `on_message` — callback converting `TokenInputMessage` to the caller's
///   message type
pub fn token_input_field<'a, M: Clone + 'a>(
    tokens: &'a [Token],
    text: &'a str,
    placeholder: &'a str,
    selected_token: Option<TokenId>,
    on_message: impl Fn(TokenInputMessage) -> M + 'a,
) -> Element<'a, M> {
    TokenInputWidget {
        tokens,
        text,
        placeholder,
        on_message: Box::new(on_message),
        selected_token,
    }
    .into()
}

// ── Layout helpers ──────────────────────────────────────

/// Estimate token chip width from label length.
/// Uses a rough character-width heuristic since precise text measurement
/// requires a paragraph. Adequate for layout — chips are visually padded.
fn estimate_token_width(label: &str) -> f32 {
    // Average character width at TEXT_MD (12px) with Inter is ~6.5px.
    let avg_char_width = TEXT_MD * 0.54;
    #[allow(clippy::cast_precision_loss)]
    let text_width = label.len() as f32 * avg_char_width;
    text_width + PAD_TOKEN.left + PAD_TOKEN.right
}

/// Compute the text area origin from the token bounds.
fn text_area_origin(
    token_bounds: &[Rectangle],
    field_width: f32,
) -> (f32, f32) {
    if let Some(last) = token_bounds.last() {
        let next_x = last.x + last.width + TOKEN_SPACING;
        let inner_width =
            field_width - PAD_TOKEN_INPUT.left - PAD_TOKEN_INPUT.right;
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
        let inner_width =
            max_width - PAD_TOKEN_INPUT.left - PAD_TOKEN_INPUT.right;

        let mut x: f32 = 0.0;
        let mut y: f32 = 0.0;
        state.token_bounds.clear();

        // Layout each token chip in a wrapping flow
        for token in self.tokens {
            let chip_width = estimate_token_width(&token.label);
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

        let total_height =
            PAD_TOKEN_INPUT.top + y + TOKEN_HEIGHT + PAD_TOKEN_INPUT.bottom;

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
            let is_hovered = cursor
                .position()
                .is_some_and(|pos| abs.contains(pos));

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

            // Chip label
            let label = &token.label;
            renderer.fill_text(
                iced::advanced::text::Text {
                    content: label.to_string(),
                    bounds: Size::new(
                        abs.width - PAD_TOKEN.left - PAD_TOKEN.right,
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
                iced::Point::new(abs.x + PAD_TOKEN.left, abs.y),
                text_color,
                abs,
            );
        }

        // Text area: placeholder or current text
        let (text_x, text_y) =
            text_area_origin(&state.token_bounds, bounds.width);

        let display_text =
            if self.text.is_empty() && self.tokens.is_empty() {
                self.placeholder
            } else {
                self.text
            };

        let text_color =
            if self.text.is_empty() && self.tokens.is_empty() {
                palette.background.base.text.scale_alpha(0.4)
            } else {
                palette.background.base.text
            };

        if !display_text.is_empty() {
            let text_area_width =
                bounds.width - text_x - PAD_TOKEN_INPUT.right;
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
                iced::Point::new(bounds.x + text_x, bounds.y + text_y),
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
        if state.is_focused && self.selected_token.is_none() {
            #[allow(clippy::cast_precision_loss)]
            let cursor_x = if self.text.is_empty() {
                bounds.x + text_x
            } else {
                let text_width = self.text.len() as f32 * TEXT_MD * 0.54;
                bounds.x + text_x + text_width
            };
            let cursor_y = bounds.y + text_y + 2.0;
            let cursor_height = TOKEN_HEIGHT - 4.0;

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
            // ── Mouse: left click ──────────────────────────
            Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Left,
                ..
            })
            | Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
                if let Some(pos) = cursor.position() {
                    if bounds.contains(pos) {
                        // Hit-test tokens
                        for (i, token) in self.tokens.iter().enumerate() {
                            if let Some(chip) = state.token_bounds.get(i) {
                                let abs = Rectangle {
                                    x: bounds.x + chip.x,
                                    y: bounds.y + chip.y,
                                    width: chip.width,
                                    height: chip.height,
                                };
                                if abs.contains(pos) {
                                    // Focus the widget so keyboard events
                                    // (Backspace/Delete) work on the selected token
                                    if !state.is_focused {
                                        state.is_focused = true;
                                        shell.publish((self.on_message)(
                                            TokenInputMessage::Focused,
                                        ));
                                    }
                                    shell.publish((self.on_message)(
                                        TokenInputMessage::SelectToken(
                                            token.id,
                                        ),
                                    ));
                                    shell.capture_event();
                                    return;
                                }
                            }
                        }

                        // Clicked in field, not on token — focus
                        if !state.is_focused {
                            state.is_focused = true;
                            shell.publish((self.on_message)(
                                TokenInputMessage::Focused,
                            ));
                        }
                        shell.publish((self.on_message)(
                            TokenInputMessage::DeselectTokens,
                        ));
                        shell.capture_event();
                        return;
                    }

                    // Clicked outside — blur
                    if state.is_focused {
                        state.is_focused = false;
                        shell.publish((self.on_message)(
                            TokenInputMessage::Blurred,
                        ));
                    }
                }
            }

            // ── Keyboard events (only when focused) ────────
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modifiers,
                ..
            }) if state.is_focused => {
                match key {
                    // Paste: Ctrl+V / Cmd+V
                    keyboard::Key::Character(c)
                        if (c.as_str() == "v" || c.as_str() == "V")
                            && modifiers.command() =>
                    {
                        if let Some(content) = clipboard.read(
                            iced::advanced::clipboard::Kind::Standard,
                        ) {
                            shell.publish((self.on_message)(
                                TokenInputMessage::Paste(content),
                            ));
                            shell.capture_event();
                            return;
                        }
                    }

                    // Backspace
                    keyboard::Key::Named(
                        keyboard::key::Named::Backspace,
                    ) => {
                        if self.text.is_empty() {
                            if let Some(selected) = self.selected_token {
                                shell.publish((self.on_message)(
                                    TokenInputMessage::RemoveToken(selected),
                                ));
                                shell.capture_event();
                                return;
                            }
                            if !self.tokens.is_empty() {
                                shell.publish((self.on_message)(
                                    TokenInputMessage::BackspaceAtStart,
                                ));
                                shell.capture_event();
                                return;
                            }
                        } else {
                            let mut new_text = self.text.to_string();
                            new_text.pop();
                            shell.publish((self.on_message)(
                                TokenInputMessage::TextChanged(new_text),
                            ));
                            shell.capture_event();
                            return;
                        }
                    }

                    // Escape: blur
                    keyboard::Key::Named(keyboard::key::Named::Escape) => {
                        state.is_focused = false;
                        shell.publish((self.on_message)(
                            TokenInputMessage::Blurred,
                        ));
                        shell.capture_event();
                        return;
                    }

                    // Enter / Tab: tokenize current text
                    keyboard::Key::Named(
                        keyboard::key::Named::Enter
                        | keyboard::key::Named::Tab,
                    ) => {
                        if !self.text.is_empty() {
                            let text = self.text.to_string();
                            shell.publish((self.on_message)(
                                TokenInputMessage::TokenizeText(text),
                            ));
                            shell.capture_event();
                            return;
                        }
                    }

                    // Comma / Semicolon: always tokenize
                    keyboard::Key::Character(c)
                        if !modifiers.command()
                            && (c.as_str() == ","
                                || c.as_str() == ";") =>
                    {
                        if !self.text.is_empty() {
                            let text = self.text.to_string();
                            shell.publish((self.on_message)(
                                TokenInputMessage::TokenizeText(text),
                            ));
                        }
                        shell.capture_event();
                        return;
                    }

                    // Space: tokenize if looks like email, else append
                    keyboard::Key::Named(keyboard::key::Named::Space)
                        if !modifiers.command() =>
                    {
                        if !self.text.is_empty() && self.text.contains('@')
                        {
                            let text = self.text.to_string();
                            shell.publish((self.on_message)(
                                TokenInputMessage::TokenizeText(text),
                            ));
                        } else if !self.text.is_empty() {
                            let new_text = format!("{} ", self.text);
                            shell.publish((self.on_message)(
                                TokenInputMessage::TextChanged(new_text),
                            ));
                        }
                        shell.capture_event();
                        return;
                    }

                    // Regular character input
                    keyboard::Key::Character(c)
                        if !modifiers.command() =>
                    {
                        if self.selected_token.is_some() {
                            shell.publish((self.on_message)(
                                TokenInputMessage::DeselectTokens,
                            ));
                        }
                        let new_text =
                            format!("{}{}", self.text, c.as_str());
                        shell.publish((self.on_message)(
                            TokenInputMessage::TextChanged(new_text),
                        ));
                        shell.capture_event();
                        return;
                    }

                    _ => {}
                }
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

impl<'a, M: Clone + 'a> From<TokenInputWidget<'a, M>> for Element<'a, M> {
    fn from(widget: TokenInputWidget<'a, M>) -> Self {
        Self::new(widget)
    }
}

// ── Helper: email validation ────────────────────────────

/// Minimal validation — catches obvious typos, not RFC 5321.
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

// ── Contact search result types ─────────────────────────

/// A single result from unified contact search.
#[derive(Debug, Clone)]
pub struct ContactSearchResult {
    /// The email address (None for group results).
    pub email: Option<String>,
    /// Display name, resolved from highest-priority source.
    pub display_name: Option<String>,
    /// Recency score for ranking (higher = more recent).
    pub recency_score: f64,
    /// The kind of result.
    pub kind: ContactSearchKind,
}

/// The kind of contact search result.
#[derive(Debug, Clone)]
pub enum ContactSearchKind {
    /// An individual contact.
    Person,
    /// A contact group.
    Group {
        group_id: String,
        member_count: i64,
    },
}
