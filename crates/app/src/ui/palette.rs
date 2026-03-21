use iced::widget::{column, container, mouse_area, row, scrollable, text, text_input};
use iced::{Element, Length};
use ratatoskr_command_palette::{
    CommandId, CommandMatch, InputMode, OptionItem, OptionMatch,
};

use super::layout::*;
use super::theme::{ContainerClass, TextClass};

/// Which stage the palette is in.
#[derive(Debug, Clone, Default)]
pub enum PaletteStage {
    /// Stage 1: searching commands via `CommandRegistry::query()`.
    #[default]
    CommandSearch,
    /// Stage 2: picking an option for a parameterized command.
    OptionPick,
}

/// Palette overlay state.
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub results: Vec<CommandMatch>,
    pub selected_index: usize,
    // Stage 2 state
    pub stage: PaletteStage,
    /// Raw option items from the resolver (unfiltered).
    pub option_items: Vec<OptionItem>,
    /// Filtered option matches for the current query.
    pub option_matches: Vec<OptionMatch>,
    /// The command ID that entered stage 2.
    pub stage2_command_id: Option<CommandId>,
    /// The param label to display in the placeholder (e.g., "Folder", "Label").
    pub stage2_label: String,
    /// Generation counter to discard stale resolver results.
    pub option_load_generation: u64,
    /// Whether the resolver is currently loading options.
    pub options_loading: bool,
}

impl PaletteState {
    pub fn new() -> Self {
        Self {
            open: false,
            query: String::new(),
            results: Vec::new(),
            selected_index: 0,
            stage: PaletteStage::CommandSearch,
            option_items: Vec::new(),
            option_matches: Vec::new(),
            stage2_command_id: None,
            stage2_label: String::new(),
            option_load_generation: 0,
            options_loading: false,
        }
    }

    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Reset all palette state to closed defaults.
    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.results.clear();
        self.selected_index = 0;
        self.stage = PaletteStage::CommandSearch;
        self.option_items.clear();
        self.option_matches.clear();
        self.stage2_command_id = None;
        self.stage2_label.clear();
        self.options_loading = false;
    }

    /// Go back from stage 2 to stage 1, preserving the command search state.
    pub fn back_to_stage1(&mut self) {
        self.stage = PaletteStage::CommandSearch;
        self.query.clear();
        self.option_items.clear();
        self.option_matches.clear();
        self.stage2_command_id = None;
        self.stage2_label.clear();
        self.options_loading = false;
    }

    /// Whether the palette is in stage 2 (option picking).
    pub fn is_option_pick(&self) -> bool {
        matches!(self.stage, PaletteStage::OptionPick)
    }
}

/// Build the palette overlay widget.
///
/// Returns an `Element` that should be layered on top of the main layout
/// via `iced::widget::stack![]`. The caller provides the backdrop click
/// message externally (in `App::view()`), so this function only builds
/// the palette card itself.
pub fn palette_card<'a, M: 'a + Clone>(
    state: &'a PaletteState,
    on_query_changed: impl Fn(String) -> M + 'a,
    on_confirm: M,
    on_click_result: impl Fn(usize) -> M + 'a,
    on_click_option: impl Fn(usize) -> M + 'a,
) -> Element<'a, M> {
    match &state.stage {
        PaletteStage::CommandSearch => {
            build_command_search_card(state, on_query_changed, on_confirm, on_click_result)
        }
        PaletteStage::OptionPick => {
            build_option_pick_card(state, on_query_changed, on_confirm, on_click_option)
        }
    }
}

fn build_command_search_card<'a, M: 'a + Clone>(
    state: &PaletteState,
    on_query_changed: impl Fn(String) -> M + 'a,
    on_confirm: M,
    on_click_result: impl Fn(usize) -> M + 'a,
) -> Element<'a, M> {
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
        .id("palette-results")
        .height(Length::Fill);

    let card_content = column![input, results_scrollable]
        .spacing(SPACE_XXS);

    container(card_content)
        .width(PALETTE_WIDTH)
        .max_height(PALETTE_MAX_HEIGHT)
        .padding(SPACE_XS)
        .style(ContainerClass::PaletteCard.style())
        .into()
}

fn build_option_pick_card<'a, M: 'a + Clone>(
    state: &'a PaletteState,
    on_query_changed: impl Fn(String) -> M + 'a,
    on_confirm: M,
    on_click_option: impl Fn(usize) -> M + 'a,
) -> Element<'a, M> {
    let placeholder = if state.options_loading {
        "Loading...".to_string()
    } else {
        format!("Search {}...", state.stage2_label)
    };

    let input = text_input(&placeholder, &state.query)
        .on_input(on_query_changed)
        .on_submit(on_confirm.clone())
        .id("palette-input")
        .padding(PAD_INPUT)
        .size(TEXT_LG);

    let options_column = build_options_column(
        &state.option_matches,
        state.selected_index,
        on_click_option,
    );

    let options_scrollable = scrollable(options_column)
        .id("palette-results")
        .height(Length::Fill);

    let card_content = column![input, options_scrollable]
        .spacing(SPACE_XXS);

    container(card_content)
        .width(PALETTE_WIDTH)
        .max_height(PALETTE_MAX_HEIGHT)
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

fn build_options_column<'a, M: 'a + Clone>(
    options: &'a [OptionMatch],
    selected_index: usize,
    on_click_option: impl Fn(usize) -> M + 'a,
) -> Element<'a, M> {
    let mut col = column![].spacing(SPACE_XXXS);

    for (i, option) in options.iter().enumerate() {
        let is_selected = i == selected_index;
        let row_element = option_result_row(option, is_selected, on_click_option(i));
        col = col.push(row_element);
    }

    col.into()
}

fn option_result_row<'a, M: 'a + Clone>(
    option: &'a OptionMatch,
    is_selected: bool,
    on_click: M,
) -> Element<'a, M> {
    let label_style: fn(&iced::Theme) -> iced::widget::text::Style = if option.item.disabled {
        TextClass::Tertiary.style()
    } else {
        TextClass::Default.style()
    };

    // Option label (fills remaining space)
    let label = container(
        text(&option.item.label)
            .size(TEXT_MD)
            .style(label_style),
    )
    .width(Length::Fill)
    .align_y(iced::Alignment::Center);

    // Path breadcrumb (right-aligned, dimmed)
    let path_display = format_option_path(&option.item.path);
    let path_element: Element<'_, M> = if path_display.is_empty() {
        container("").into()
    } else {
        container(
            text(path_display)
                .size(TEXT_SM)
                .style(TextClass::Muted.style()),
        )
        .align_y(iced::Alignment::Center)
        .into()
    };

    let row_content = row![label, path_element]
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

/// Format an option's path segments as a breadcrumb string.
fn format_option_path(path: &Option<Vec<String>>) -> String {
    match path {
        Some(segments) if !segments.is_empty() => segments.join(" > "),
        _ => String::new(),
    }
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

    let label_style: fn(&iced::Theme) -> iced::widget::text::Style = if available {
        TextClass::Default.style()
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

/// Keep the selected item visible in the palette results scrollable.
///
/// TODO: The iced fork does not expose `scrollable::scroll_to()`.
/// This is a placeholder that returns `Task::none()` until
/// scroll-to-item support is available.
pub fn scroll_to_selected(_selected_index: usize, _total_items: usize) -> iced::Task<()> {
    iced::Task::none()
}

/// Build the pending chord indicator badge.
///
/// Shows the first key of a two-key sequence (e.g., "g...") as a small
/// floating badge. Returns `None` if there is no pending chord.
pub fn chord_indicator<'a, M: 'a>(
    chord_display: &str,
) -> Element<'a, M> {
    let badge = container(
        text(format!("{chord_display}..."))
            .size(TEXT_SM)
            .style(TextClass::Muted.style()),
    )
    .padding(PAD_BADGE)
    .style(ContainerClass::ChordIndicator.style());

    container(badge)
        .width(Length::Fill)
        .align_x(iced::Alignment::End)
        .padding(SPACE_SM)
        .into()
}

/// Build snooze preset options as `OptionItem`s for the DateTime picker.
///
/// These are hardcoded presets — a proper date/time picker is deferred.
pub fn snooze_preset_options() -> Vec<OptionItem> {
    use std::time::{SystemTime, UNIX_EPOCH};

    let now_secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    vec![
        snooze_option("1 hour", now_secs + 3600),
        snooze_option("2 hours", now_secs + 7200),
        snooze_option("4 hours", now_secs + 14400),
        snooze_option("Tomorrow 9am", next_morning_9am(now_secs)),
        snooze_option("Tomorrow 1pm", next_afternoon_1pm(now_secs)),
        snooze_option("Next week", next_week_morning(now_secs)),
    ]
}

fn snooze_option(label: &str, until: i64) -> OptionItem {
    OptionItem {
        id: until.to_string(),
        label: label.to_string(),
        path: None,
        keywords: None,
        disabled: false,
    }
}

/// Compute unix timestamp for tomorrow at 9:00 AM local time.
fn next_morning_9am(now_secs: i64) -> i64 {
    // Approximate: add 1 day then round to 9am.
    // This is intentionally simple — real timezone handling is deferred.
    let day_secs: i64 = 86400;
    let tomorrow_start = (now_secs / day_secs + 1) * day_secs;
    tomorrow_start + 9 * 3600
}

/// Compute unix timestamp for tomorrow at 1:00 PM local time.
fn next_afternoon_1pm(now_secs: i64) -> i64 {
    let day_secs: i64 = 86400;
    let tomorrow_start = (now_secs / day_secs + 1) * day_secs;
    tomorrow_start + 13 * 3600
}

/// Compute unix timestamp for next Monday at 9:00 AM.
fn next_week_morning(now_secs: i64) -> i64 {
    let day_secs: i64 = 86400;
    // Add 7 days then round to 9am
    let next_week_start = (now_secs / day_secs + 7) * day_secs;
    next_week_start + 9 * 3600
}
