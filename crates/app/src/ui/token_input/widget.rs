use iced::advanced::layout;
use iced::advanced::renderer::{self, Renderer as _};
use iced::advanced::text::Renderer as _;
use iced::advanced::widget::tree::{self, Tree};
use iced::advanced::{Clipboard, Layout, Shell, Widget};
use iced::keyboard;
use iced::mouse;
use iced::{Element, Event, Length, Point, Rectangle, Size, Theme, border};

use crate::font;
use crate::ui::layout::*;

use super::handlers::{handle_key_press, handle_left_click};
use super::layout::{chip_display_label, estimate_token_width};
use super::render::{draw_group_icon, draw_text_area};
use super::types::{Token, TokenId, TokenInputMessage};

/// Transient widget state stored in iced's tree.
#[derive(Debug, Default)]
pub(super) struct TokenInputState {
    /// Cached per-token bounds from the last layout pass (relative to
    /// widget origin).
    pub(super) token_bounds: Vec<Rectangle>,
    /// Whether the widget is focused.
    pub(super) is_focused: bool,
    /// Whether the cursor is currently over the field. Tracked via
    /// `CursorMoved` events in `update()` so the hover-border draw is
    /// stable across frames (reading `cursor.is_over()` directly in
    /// `draw()` misses repaints when nothing else changes).
    pub(super) is_hovered: bool,
    /// Tracking a potential drag: (token_id, mouse_down_origin).
    pub(super) drag_tracking: Option<(TokenId, Point)>,
    /// Vertical offset applied to chips and the text area when the field
    /// is taller than the chips need (to match iced text_input's height
    /// for the same `TEXT_LG`). Computed in `layout()`.
    pub(super) chip_v_offset: f32,
}

pub(super) struct TokenInputWidget<'a, M> {
    pub(super) tokens: &'a [Token],
    pub(super) text: &'a str,
    pub(super) placeholder: &'a str,
    pub(super) on_message: Box<dyn Fn(TokenInputMessage) -> M + 'a>,
    /// Which token the parent considers selected (for backspace flow).
    pub(super) selected_token: Option<TokenId>,
    /// Whether the autocomplete dropdown is currently visible.
    /// When true, ArrowUp/Down/Enter/Tab/Escape emit autocomplete messages
    /// instead of normal token navigation.
    pub(super) autocomplete_open: bool,
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

        let raw_chip_height = PAD_TOKEN_INPUT.top + y + TOKEN_HEIGHT + PAD_TOKEN_INPUT.bottom;

        // Match iced text_input(TEXT_LG, PAD_INPUT) for a single-row field
        // so To/Cc/Bcc and Subject end up the same height. iced's formula:
        //   padding.top + LineHeight::default().to_absolute(text_size) + padding.bottom
        // with default LineHeight = Relative(1.3).
        let single_row_height = PAD_INPUT.top + TEXT_LG * 1.3 + PAD_INPUT.bottom;
        let total_height = single_row_height.max(raw_chip_height);

        // If the field is taller than chips need (single-row case), shift
        // chip bounds and the text area down so they sit centered in the
        // taller slot.
        let chip_v_offset = ((total_height - raw_chip_height) / 2.0).max(0.0);
        if chip_v_offset > 0.0 {
            for bound in &mut state.token_bounds {
                bound.y += chip_v_offset;
            }
        }
        state.chip_v_offset = chip_v_offset;

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

        // Field background + border. Match iced's default text_input style
        // so the To/Cc/Bcc inputs look identical to the Subject text_input
        // sitting next to them. Hover state comes from `state.is_hovered`
        // (maintained in `update()`), not a live `cursor.is_over()` read,
        // so the border tracks the cursor reliably across frames.
        let border_color = if state.is_focused {
            palette.primary.strong.color
        } else if state.is_hovered {
            palette.background.base.text
        } else {
            palette.background.strong.color
        };

        renderer.fill_quad(
            renderer::Quad {
                bounds,
                border: border::Border {
                    color: border_color,
                    width: 1.0,
                    radius: 2.0.into(),
                },
                ..renderer::Quad::default()
            },
            palette.background.base.color,
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

            // Group icon prefix for group tokens. Position the icon's bounds
            // box centered against the chip's vertical midline so font
            // line-height padding can't push it off-center.
            let label_x_offset = if token.is_group {
                let icon_y = abs.y + (abs.height - TOKEN_GROUP_ICON_SIZE) / 2.0;
                draw_group_icon(
                    renderer,
                    Point::new(abs.x + PAD_TOKEN.left, icon_y),
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
                    size: iced::Pixels(TEXT_LG),
                    line_height: iced::advanced::text::LineHeight::default(),
                    font: font::text(),
                    align_x: iced::advanced::text::Alignment::Left,
                    align_y: iced::alignment::Vertical::Center,
                    shaping: iced::advanced::text::Shaping::Advanced,
                    wrapping: iced::advanced::text::Wrapping::None,
                    ellipsis: iced::advanced::text::Ellipsis::None,
                    hint_factor: None,
                },
                Point::new(
                    abs.x + PAD_TOKEN.left + label_x_offset,
                    abs.y + abs.height / 2.0,
                ),
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
                if let Some(pos) = cursor.position()
                    && bounds.contains(pos)
                {
                    for (i, token) in self.tokens.iter().enumerate() {
                        if let Some(chip) = state.token_bounds.get(i) {
                            let abs = Rectangle {
                                x: bounds.x + chip.x,
                                y: bounds.y + chip.y,
                                width: chip.width,
                                height: chip.height,
                            };
                            if abs.contains(pos) {
                                // Mirror left-click semantics so the
                                // pill highlights as the active marker
                                // before the context menu opens.
                                if !state.is_focused {
                                    state.is_focused = true;
                                    shell.publish((self.on_message)(TokenInputMessage::Focused));
                                }
                                shell.publish((self.on_message)(TokenInputMessage::SelectToken(
                                    token.id,
                                )));
                                shell.publish((self.on_message)(
                                    TokenInputMessage::TokenContextMenu(token.id, pos),
                                ));
                                shell.capture_event();
                                return;
                            }
                        }
                    }
                    // Right-click landed inside the field but not on a
                    // token - emit a field-context message so the
                    // caller can show a Paste-only menu.
                    if !state.is_focused {
                        state.is_focused = true;
                        shell.publish((self.on_message)(TokenInputMessage::Focused));
                    }
                    shell.publish((self.on_message)(TokenInputMessage::DeselectTokens));
                    shell.publish((self.on_message)(TokenInputMessage::FieldContextMenu(pos)));
                    shell.capture_event();
                    return;
                }
            }

            // ── Mouse: left click ──────────────────────────
            Event::Mouse(mouse::Event::ButtonPressed {
                button: mouse::Button::Left,
                ..
            })
            | Event::Touch(iced::touch::Event::FingerPressed { .. }) => {
                // Start drag tracking if clicking on a token
                if let Some(pos) = cursor.position()
                    && bounds.contains(pos)
                {
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
            Event::Keyboard(keyboard::Event::KeyPressed {
                key,
                modifiers,
                text,
                ..
            }) if state.is_focused => {
                handle_key_press(self, key, modifiers, text.as_deref(), clipboard, shell);
            }

            _ => {}
        }

        // Maintain hover state for the field border. Re-evaluating on every
        // event the widget sees (mouse moves anywhere, button presses, etc.)
        // keeps `state.is_hovered` in sync; the explicit redraw request makes
        // sure the border repaints on the transition.
        let now_hovered = cursor.position().is_some_and(|p| bounds.contains(p));
        if state.is_hovered != now_hovered {
            state.is_hovered = now_hovered;
            shell.request_redraw();
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

        if let Some(pos) = cursor.position()
            && bounds.contains(pos)
        {
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

        mouse::Interaction::default()
    }
}

impl<'a, M: Clone + 'a> From<TokenInputWidget<'a, M>> for Element<'a, M> {
    fn from(widget: TokenInputWidget<'a, M>) -> Self {
        Self::new(widget)
    }
}
