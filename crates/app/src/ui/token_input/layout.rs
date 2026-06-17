use iced::{Rectangle, Size};

use crate::font;
use crate::ui::layout::*;

use super::types::{Token, TokenId};

/// Build the display label for a token chip. Group tokens with a known member
/// count get a " (N)" suffix.
pub(super) fn chip_display_label(token: &Token) -> String {
    match (token.is_group, token.member_count) {
        (true, Some(n)) => format!("{} ({n})", token.label),
        _ => token.label.clone(),
    }
}

/// Measure token chip width from the actual rendered label using iced's
/// text shaper. Falls back to a per-character estimate if iced's measurer
/// can't run (which shouldn't happen in practice, but the fallback keeps
/// layout deterministic). Group tokens include the people-icon prefix.
pub(super) fn estimate_token_width(token: &Token) -> f32 {
    use iced::advanced::text::{
        Alignment, Ellipsis, LineHeight, Paragraph as ParagraphTrait, Shaping, Wrapping,
    };
    type IcedParagraph = <iced::Renderer as iced::advanced::text::Renderer>::Paragraph;

    let display = chip_display_label(token);
    let text = iced::advanced::text::Text {
        content: display.as_str(),
        bounds: Size::new(f32::INFINITY, f32::INFINITY),
        size: iced::Pixels(TEXT_LG),
        line_height: LineHeight::default(),
        font: font::text(),
        align_x: Alignment::Left,
        align_y: iced::alignment::Vertical::Top,
        shaping: Shaping::Advanced,
        wrapping: Wrapping::None,
        ellipsis: Ellipsis::None,
        hint_factor: None,
    };
    let paragraph = IcedParagraph::with_text(text);
    let text_width = paragraph.min_bounds().width;
    let icon_width = if token.is_group {
        TOKEN_GROUP_ICON_SIZE + SPACE_XXS
    } else {
        0.0
    };
    text_width + icon_width + PAD_TOKEN.left + PAD_TOKEN.right
}

/// Compute the text area origin from the token bounds.
pub(super) fn text_area_origin(
    token_bounds: &[Rectangle],
    field_width: f32,
    chip_v_offset: f32,
) -> (f32, f32) {
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
        (PAD_TOKEN_INPUT.left, PAD_TOKEN_INPUT.top + chip_v_offset)
    }
}

/// Find the index of the currently selected token.
pub(super) fn selected_index(tokens: &[Token], selected: Option<TokenId>) -> Option<usize> {
    let sel = selected?;
    tokens.iter().position(|t| t.id == sel)
}
