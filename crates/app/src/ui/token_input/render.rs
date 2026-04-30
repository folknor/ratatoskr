use iced::advanced::renderer::Renderer as _;
use iced::advanced::text::Renderer as _;
use iced::{Point, Rectangle, Size};

use crate::font;
use crate::ui::layout::*;

use super::layout::text_area_origin;
use super::widget::{TokenInputState, TokenInputWidget};

/// Draw the people icon glyph for group tokens using the icon font. The
/// caller positions `position` at the top-left of the icon's square box;
/// `bounds` and `line_height` are set to exactly the icon size so the
/// glyph occupies the full box without extra line-height padding shifting
/// it.
pub(super) fn draw_group_icon(
    renderer: &mut iced::Renderer,
    position: Point,
    color: iced::Color,
    clip: Rectangle,
) {
    // Lucide "users" icon: U+E1A4
    renderer.fill_text(
        iced::advanced::text::Text {
            content: "\u{e1a4}".to_string(),
            bounds: Size::new(TOKEN_GROUP_ICON_SIZE, TOKEN_GROUP_ICON_SIZE),
            size: iced::Pixels(TOKEN_GROUP_ICON_SIZE),
            line_height: iced::advanced::text::LineHeight::Absolute(iced::Pixels(
                TOKEN_GROUP_ICON_SIZE,
            )),
            font: crate::font::ICON,
            align_x: iced::advanced::text::Alignment::Left,
            align_y: iced::alignment::Vertical::Top,
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

pub(super) fn draw_text_area(
    widget: &TokenInputWidget<'_, impl Clone>,
    state: &TokenInputState,
    renderer: &mut iced::Renderer,
    palette: &iced::theme::Palette,
    bounds: Rectangle,
) {
    let (text_x, text_y) =
        text_area_origin(&state.token_bounds, bounds.width, state.chip_v_offset);

    let display_text = if widget.text.is_empty() && widget.tokens.is_empty() {
        widget.placeholder
    } else {
        widget.text
    };

    // Match iced's default text_input: placeholder uses
    // `palette.secondary.base.color`, value uses `palette.background.base.text`.
    let text_color = if widget.text.is_empty() && widget.tokens.is_empty() {
        palette.secondary.base.color
    } else {
        palette.background.base.text
    };

    if !display_text.is_empty() {
        let text_area_width = bounds.width - text_x - PAD_TOKEN_INPUT.right;
        // With align_y::Center the renderer treats `position.y` as the
        // vertical center of the text. Anchor to the slot's vertical
        // midpoint so the text actually sits in the middle of the row.
        renderer.fill_text(
            iced::advanced::text::Text {
                content: display_text.to_string(),
                bounds: Size::new(text_area_width, TOKEN_HEIGHT),
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
                bounds.x + text_x,
                bounds.y + text_y + TOKEN_HEIGHT / 2.0,
            ),
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
                widget.text.chars().count() as f32 * TEXT_LG * TOKEN_AVG_CHAR_WIDTH_FACTOR;
            bounds.x + text_x + text_width
        };
        let cursor_y = bounds.y + text_y + SPACE_XXXS;
        let cursor_height = TOKEN_HEIGHT - SPACE_XXS;

        renderer.fill_quad(
            iced::advanced::renderer::Quad {
                bounds: Rectangle {
                    x: cursor_x,
                    y: cursor_y,
                    width: 1.0,
                    height: cursor_height,
                },
                ..iced::advanced::renderer::Quad::default()
            },
            palette.background.base.text,
        );
    }
}
