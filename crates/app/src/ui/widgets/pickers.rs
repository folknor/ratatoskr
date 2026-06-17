#![allow(dead_code)]

use iced::widget::{
    Canvas, Space, button, canvas, column, container, row, scrollable, text, text_input,
};
use iced::{Alignment, Color, Element, Length, Rectangle, Renderer, Theme, mouse};

use crate::ui::emoji_picker::{EMOJI_TABLE, EmojiCategory, EmojiEntry};
use crate::ui::layout::{
    COLOR_SWATCH_CHECK_SCALE, COLOR_SWATCH_DIMMED_ALPHA, COLOR_SWATCH_SIZE, EMOJI_BUTTON_SIZE,
    EMOJI_GRID_COLUMNS, EMOJI_PICKER_MAX_HEIGHT, EMOJI_PICKER_WIDTH, PAD_COLOR_SWATCH, PAD_INPUT,
    SCROLLBAR_SPACING, SPACE_XS, SPACE_XXS, SPACE_XXXS, TEXT_MD, TEXT_TITLE,
};
use crate::ui::theme;

/// Builds the emoji picker widget. The caller owns visibility state and positioning.
///
/// - `search_query`: current text in the search field
/// - `selected_category`: which category tab is active
/// - `on_select`: called with the emoji string when a user clicks one
/// - `on_category_changed`: called when the user clicks a category tab
/// - `on_search_changed`: called when the search input text changes
pub fn emoji_picker<'a, M: 'a + Clone>(
    search_query: &str,
    selected_category: EmojiCategory,
    on_select: impl Fn(&'static str) -> M + 'a,
    on_category_changed: impl Fn(EmojiCategory) -> M + 'a,
    on_search_changed: impl Fn(String) -> M + 'a,
) -> Element<'a, M> {
    let filtered: Vec<&EmojiEntry> = if search_query.is_empty() {
        EMOJI_TABLE
            .iter()
            .filter(|e| e.category == selected_category)
            .collect()
    } else {
        let query = search_query.to_lowercase();
        EMOJI_TABLE
            .iter()
            .filter(|e| e.name.contains(&query))
            .collect()
    };

    let search = text_input("Search emoji...", search_query)
        .on_input(on_search_changed)
        .padding(PAD_INPUT)
        .size(TEXT_MD)
        .style(theme::TextInputClass::Settings.style());

    let mut tab_row = row![].spacing(SPACE_XXXS).align_y(Alignment::Center);
    for &cat in EmojiCategory::ALL {
        let is_active = cat == selected_category;
        let tab = button(
            container(text(cat.tab_emoji()).size(TEXT_TITLE))
                .width(EMOJI_BUTTON_SIZE)
                .height(EMOJI_BUTTON_SIZE)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(on_category_changed(cat))
        .style(theme::ButtonClass::Chip { active: is_active }.style());
        tab_row = tab_row.push(tab);
    }

    let mut grid_col = column![].spacing(SPACE_XXXS);
    let mut current_row = row![].spacing(SPACE_XXXS);
    let mut col_idx = 0;

    for entry in &filtered {
        let emoji_btn = button(
            container(text(entry.emoji).size(TEXT_TITLE))
                .width(EMOJI_BUTTON_SIZE)
                .height(EMOJI_BUTTON_SIZE)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(on_select(entry.emoji))
        .style(theme::ButtonClass::BareIcon.style());

        current_row = current_row.push(emoji_btn);
        col_idx += 1;

        if col_idx >= EMOJI_GRID_COLUMNS {
            grid_col = grid_col.push(current_row);
            current_row = row![].spacing(SPACE_XXXS);
            col_idx = 0;
        }
    }

    if col_idx > 0 {
        for _ in col_idx..EMOJI_GRID_COLUMNS {
            current_row = current_row.push(
                Space::new()
                    .width(EMOJI_BUTTON_SIZE)
                    .height(EMOJI_BUTTON_SIZE),
            );
        }
        grid_col = grid_col.push(current_row);
    }

    let grid_scrollable = scrollable(container(grid_col).padding([SPACE_XXS, 0.0]))
        .spacing(SCROLLBAR_SPACING)
        .height(Length::Fill);

    container(column![search, tab_row, grid_scrollable,].spacing(SPACE_XS))
        .padding(SPACE_XS)
        .width(EMOJI_PICKER_WIDTH)
        .height(EMOJI_PICKER_MAX_HEIGHT)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
}

/// Swatch canvas painter for the color palette grid.
struct SwatchPainter {
    color: Color,
    selected: bool,
    used: bool,
}

impl<M> canvas::Program<M> for SwatchPainter {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let radius = bounds.width.min(bounds.height) / 2.0;
        let center = iced::Point::new(bounds.width / 2.0, bounds.height / 2.0);

        let circle = canvas::path::Path::circle(center, radius);

        let draw_color = if self.used && !self.selected {
            Color {
                a: COLOR_SWATCH_DIMMED_ALPHA,
                ..self.color
            }
        } else {
            self.color
        };

        frame.fill(&circle, draw_color);

        if self.used {
            swatch_check_mark(&mut frame, bounds, radius);
        }

        vec![frame.into_geometry()]
    }
}

/// Draw a small check-mark inside a swatch circle.
fn swatch_check_mark(frame: &mut canvas::Frame<Renderer>, bounds: Rectangle, radius: f32) {
    let check_color = Color::WHITE;
    let check = canvas::path::Path::new(|b| {
        let cx = bounds.width / 2.0;
        let cy = bounds.height / 2.0;
        let s = radius * COLOR_SWATCH_CHECK_SCALE;
        b.move_to(iced::Point::new(cx - s * 0.5, cy));
        b.line_to(iced::Point::new(cx - s * 0.1, cy + s * 0.4));
        b.line_to(iced::Point::new(cx + s * 0.5, cy - s * 0.3));
    });
    frame.stroke(
        &check,
        canvas::Stroke::default()
            .with_color(check_color)
            .with_width(2.0),
    );
}

/// Build a reusable color palette grid that flows to fit its container.
///
/// `selected` is the currently selected preset index (if any).
/// `used_colors` are background hex strings of already-assigned colors;
/// they render dimmed with a check mark and are NOT clickable.
/// `on_select` maps a preset index to the caller's message type and is
/// only called for available swatches.
///
/// The grid uses iced's `Responsive` widget to compute how many swatches
/// fit per row from the parent's width on every layout pass, so the grid
/// reflows when the section narrows. Selected swatches render as a
/// non-button container with a circular focus ring (matching the Theme
/// tab's selected-ring pattern).
pub fn color_palette_grid<'a, M: Clone + 'a>(
    selected: Option<usize>,
    used_colors: &[String],
    on_select: impl Fn(usize) -> M + 'a,
    on_custom: Option<M>,
) -> Element<'a, M> {
    let used_colors: Vec<String> = used_colors.to_vec();
    iced::widget::Responsive::new(move |size| {
        let presets = label_colors::preset_colors::all_presets();
        // Width consumed by one swatch button (Canvas + per-side padding)
        // plus the inter-swatch gap.
        let swatch_outer = COLOR_SWATCH_SIZE + 2.0 * PAD_COLOR_SWATCH;
        let stride = swatch_outer + SPACE_XS;
        // Adding one stride's worth of imaginary trailing gap to the
        // available width lets the last swatch in a row fit without a gap
        // pushing the count down by one.
        // Width is non-negative and bounded by parent layout; the floor of
        // (width / stride) is well within usize range. Clamp to >=1 so an
        // unexpectedly tiny container still renders one column instead of
        // dividing by zero.
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let cols = ((size.width + SPACE_XS) / stride).floor().max(1.0) as usize;

        let mut grid = column![].spacing(SPACE_XS);
        let mut current_row = row![].spacing(SPACE_XS);
        let mut count: usize = 0;

        for (i, &(_name, bg_hex, _fg_hex)) in presets.iter().enumerate() {
            let is_selected = selected == Some(i);
            let is_used = used_colors.iter().any(|c| c == bg_hex);
            let color = theme::hex_to_color(bg_hex);

            let swatch = Canvas::new(SwatchPainter {
                color,
                selected: is_selected,
                used: is_used,
            })
            .width(COLOR_SWATCH_SIZE)
            .height(COLOR_SWATCH_SIZE);

            let cell: Element<'_, M> = if is_selected {
                // Selected: not a button (already-selected swatches are
                // not actionable). Outer container draws the focus ring.
                container(container(swatch).padding(PAD_COLOR_SWATCH))
                    .style(theme::ContainerClass::ColorSwatchSelectedRing.style())
                    .into()
            } else if is_used {
                // Used by another account: dimmed by SwatchPainter, no
                // press handler so the user can't pick it.
                container(swatch).padding(PAD_COLOR_SWATCH).into()
            } else {
                button(swatch)
                    .on_press(on_select(i))
                    .padding(PAD_COLOR_SWATCH)
                    .style(theme::ButtonClass::BareTransparent.style())
                    .into()
            };

            current_row = current_row.push(cell);
            count += 1;

            if count.is_multiple_of(cols) {
                grid = grid.push(current_row);
                current_row = row![].spacing(SPACE_XS);
            }
        }

        // Optional "+" tile for opening a custom-colour picker. Rendered
        // as the final cell in the grid so it wraps naturally with the
        // swatches above.
        if let Some(msg) = on_custom.clone() {
            let plus_cell: Element<'_, M> = button(
                container(text("+").size(TEXT_TITLE).style(text::base))
                    .width(COLOR_SWATCH_SIZE)
                    .height(COLOR_SWATCH_SIZE)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            )
            .on_press(msg)
            .padding(PAD_COLOR_SWATCH)
            .style(theme::ButtonClass::BareTransparent.style())
            .into();
            current_row = current_row.push(plus_cell);
            count += 1;
            if count.is_multiple_of(cols) {
                grid = grid.push(current_row);
                current_row = row![].spacing(SPACE_XS);
            }
        }

        if !count.is_multiple_of(cols) {
            grid = grid.push(current_row);
        }

        grid.into()
    })
    .into()
}
