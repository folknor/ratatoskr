use iced::widget::{column, container, mouse_area, row, scrollable, text, text_input};
use iced::{Element, Length};
use ratatoskr_command_palette::{CommandMatch, CommandRegistry, InputMode};

use super::layout::*;
use super::theme::{ContainerClass, TextClass};

/// Palette overlay state.
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub results: Vec<CommandMatch>,
    pub selected_index: usize,
}

impl PaletteState {
    pub fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }
}

/// Build the palette overlay widget.
///
/// Returns an `Element` that should be layered on top of the main layout
/// via `iced::widget::stack![]`. The caller provides the backdrop click
/// message externally (in `App::view()`), so this function only builds
/// the palette card itself.
pub fn palette_card<'a, M: 'a + Clone>(
    state: &PaletteState,
    registry: &CommandRegistry,
    on_query_changed: impl Fn(String) -> M + 'a,
    on_confirm: M,
    on_click_result: impl Fn(usize) -> M + 'a,
) -> Element<'a, M> {
    let _ = registry; // Used for future enrichment; results are pre-queried

    let input = text_input("Type a command...", &state.query)
        .on_input(on_query_changed)
        .on_submit(on_confirm.clone())
        .id("palette-input")
        .padding(PAD_INPUT)
        .size(TEXT_LG);

    let results_column = build_results_column(
        &state.results,
        state.selected_index,
        on_click_result,
    );

    let results_scrollable = scrollable(results_column)
        .height(Length::Shrink);

    let card_content = column![input, results_scrollable]
        .spacing(SPACE_XXS);

    container(card_content)
        .width(PALETTE_WIDTH)
        .height(Length::Shrink)
        .padding(SPACE_XS)
        .style(ContainerClass::PaletteCard.style())
        .into()
}

fn build_results_column<'a, M: 'a + Clone>(
    results: &[CommandMatch],
    selected_index: usize,
    on_click_result: impl Fn(usize) -> M + 'a,
) -> Element<'a, M> {
    let mut col = column![].spacing(SPACE_XXXS);

    for (i, result) in results.iter().enumerate() {
        let is_selected = i == selected_index;
        let row_element = palette_result_row(
            result.category,
            result.label,
            result.keybinding.clone(),
            result.available,
            result.input_mode,
            is_selected,
            on_click_result(i),
        );
        col = col.push(row_element);
    }

    col.into()
}

fn palette_result_row<'a, M: 'a + Clone>(
    category_str: &'static str,
    label_str: &'static str,
    keybinding_str: Option<String>,
    available: bool,
    input_mode: InputMode,
    is_selected: bool,
    on_click: M,
) -> Element<'a, M> {
    let text_style = if available {
        TextClass::Muted.style()
    } else {
        TextClass::Tertiary.style()
    };

    let label_style: fn(&iced::Theme) -> text::Style = if available {
        |_theme: &iced::Theme| text::Style { color: None }
    } else {
        TextClass::Tertiary.style()
    };

    // Category badge (left, fixed width)
    let category = container(
        text(category_str)
            .size(TEXT_SM)
            .style(text_style),
    )
    .width(PALETTE_CATEGORY_WIDTH)
    .align_y(iced::Alignment::Center);

    // Command label (center, fills)
    let label_text = if matches!(input_mode, InputMode::Parameterized { .. }) {
        format!("{label_str}...")
    } else {
        label_str.to_string()
    };
    let label = container(
        text(label_text)
            .size(TEXT_MD)
            .style(label_style),
    )
    .width(Length::Fill)
    .align_y(iced::Alignment::Center);

    // Keybinding hint (right, fixed width, pill style)
    let keybinding: Element<'_, M> = match keybinding_str {
        Some(kb) => container(
            container(
                text(kb)
                    .size(TEXT_XS)
                    .style(TextClass::Tertiary.style()),
            )
            .padding(PAD_BADGE)
            .style(ContainerClass::KeyBadge.style()),
        )
        .width(PALETTE_KEYBINDING_WIDTH)
        .align_x(iced::Alignment::End)
        .align_y(iced::Alignment::Center)
        .into(),
        None => container("")
            .width(PALETTE_KEYBINDING_WIDTH)
            .into(),
    };

    let row_content = row![category, label, keybinding]
        .align_y(iced::Alignment::Center);

    let row_container = if is_selected {
        container(row_content)
            .height(PALETTE_RESULT_HEIGHT)
            .padding([0.0, SPACE_XS])
            .style(ContainerClass::PaletteSelectedRow.style())
    } else {
        container(row_content)
            .height(PALETTE_RESULT_HEIGHT)
            .padding([0.0, SPACE_XS])
    };

    mouse_area(row_container)
        .on_press(on_click)
        .into()
}
