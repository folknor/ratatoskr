use std::sync::Arc;

use iced::widget::{column, container, mouse_area, row, scrollable, text, text_input};
use iced::{Element, Length, Task};
use ratatoskr_command_palette::{
    CommandArgs, CommandContext, CommandId, CommandInputResolver, CommandMatch,
    CommandRegistry, InputMode, OptionItem, OptionMatch,
};

use super::layout::*;
use super::theme::{ContainerClass, TextClass};
use crate::command_resolver::AppInputResolver;
use crate::component::Component;
use crate::handlers::commands::build_command_args;

// ── Messages & Events ──────────────────────────────────

/// Internal messages for the palette component.
#[derive(Debug, Clone)]
pub enum PaletteMessage {
    /// Open the palette. Carries the current command context for registry queries.
    Open(CommandContext),
    /// Close the palette (Escape or backdrop click).
    /// Carries context for re-querying when backing from stage 2 to stage 1.
    Close(CommandContext),
    /// Text input changed. Carries context for registry queries in stage 1.
    QueryChanged(String, CommandContext),
    /// Arrow down: select next result.
    SelectNext,
    /// Arrow up: select previous result.
    SelectPrev,
    /// Enter pressed: execute the currently selected command.
    /// Carries context for loading parameterized options.
    Confirm(CommandContext),
    /// Mouse click on a result row. Carries context.
    ClickResult(usize, CommandContext),
    /// Mouse click on a stage 2 option row.
    ClickOption(usize),
    /// Stage 2: option list loaded from resolver.
    /// The `u64` is the generation counter to discard stale results.
    OptionsLoaded(u64, CommandId, Result<Vec<OptionItem>, String>),
}

/// Events the palette emits upward to the App.
#[derive(Debug, Clone)]
pub enum PaletteEvent {
    /// Execute a direct (non-parameterized) command.
    ExecuteCommand(CommandId),
    /// Execute a parameterized command with resolved arguments.
    ExecuteParameterized(CommandId, CommandArgs),
    /// The palette was dismissed (closed without executing).
    Dismissed,
    /// An error occurred (e.g., from the options resolver).
    Error(String),
}

// ── Stage ──────────────────────────────────────────────

/// Which stage the palette is in.
#[derive(Debug, Clone, Default)]
enum PaletteStage {
    /// Stage 1: searching commands via `CommandRegistry::query()`.
    #[default]
    CommandSearch,
    /// Stage 2: picking an option for a parameterized command.
    OptionPick,
}

// ── Component ──────────────────────────────────────────

/// Self-contained command palette component.
///
/// Owns the `CommandRegistry` and `AppInputResolver` references needed
/// to query commands and load stage-2 options. Emits `PaletteEvent`
/// variants for the App to handle (execute command, dismiss, etc.).
pub struct Palette {
    registry: CommandRegistry,
    resolver: Arc<AppInputResolver>,
    open: bool,
    query: String,
    results: Vec<CommandMatch>,
    selected_index: usize,
    stage: PaletteStage,
    /// Raw option items from the resolver (unfiltered).
    option_items: Vec<OptionItem>,
    /// Filtered option matches for the current query.
    option_matches: Vec<OptionMatch>,
    /// The command ID that entered stage 2.
    stage2_command_id: Option<CommandId>,
    /// The param label to display in the placeholder (e.g., "Folder", "Label").
    stage2_label: String,
    /// Generation counter to discard stale resolver results.
    option_load_generation: u64,
    /// Whether the resolver is currently loading options.
    options_loading: bool,
}

impl Palette {
    pub fn new(registry: CommandRegistry, resolver: Arc<AppInputResolver>) -> Self {
        Self {
            registry,
            resolver,
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

    /// Go back from stage 2 to stage 1.
    fn back_to_stage1(&mut self) {
        self.stage = PaletteStage::CommandSearch;
        self.query.clear();
        self.option_items.clear();
        self.option_matches.clear();
        self.stage2_command_id = None;
        self.stage2_label.clear();
        self.options_loading = false;
    }

    /// Whether the palette is in stage 2 (option picking).
    fn is_option_pick(&self) -> bool {
        matches!(self.stage, PaletteStage::OptionPick)
    }

    fn confirm(
        &mut self,
        ctx: &CommandContext,
    ) -> (Task<PaletteMessage>, Option<PaletteEvent>) {
        let Some(result) = self.results.get(self.selected_index) else {
            return (Task::none(), None);
        };
        if !result.available {
            return (Task::none(), None);
        }
        let id = result.id;
        let input_mode = result.input_mode;

        match input_mode {
            InputMode::Direct => {
                self.close();
                (Task::none(), Some(PaletteEvent::ExecuteCommand(id)))
            }
            InputMode::Parameterized { schema } => {
                let param_label = schema
                    .param_at(0)
                    .map(|p| match p {
                        ratatoskr_command_palette::ParamDef::ListPicker { label } => label,
                        ratatoskr_command_palette::ParamDef::DateTime { label } => label,
                        ratatoskr_command_palette::ParamDef::Enum { label, .. } => label,
                        ratatoskr_command_palette::ParamDef::Text { label, .. } => label,
                    })
                    .unwrap_or("option");

                // DateTime commands use preset options instead of a full picker
                if matches!(
                    schema.param_at(0),
                    Some(ratatoskr_command_palette::ParamDef::DateTime { .. })
                ) {
                    let task = self.enter_snooze_stage2(id, param_label);
                    return (task, None);
                }

                self.enter_option_stage2(id, param_label, ctx)
            }
        }
    }

    fn enter_option_stage2(
        &mut self,
        id: CommandId,
        param_label: &str,
        ctx: &CommandContext,
    ) -> (Task<PaletteMessage>, Option<PaletteEvent>) {
        self.stage = PaletteStage::OptionPick;
        self.query.clear();
        self.selected_index = 0;
        self.stage2_command_id = Some(id);
        self.stage2_label = param_label.to_string();
        self.option_items.clear();
        self.option_matches.clear();
        self.options_loading = true;
        self.option_load_generation += 1;
        let generation = self.option_load_generation;

        let resolver = Arc::clone(&self.resolver);
        let ctx = ctx.clone();
        let focus_task = iced::widget::operation::focus::<PaletteMessage>(
            "palette-input".to_string(),
        );
        let load_task = Task::perform(
            async move {
                tokio::task::spawn_blocking(move || {
                    resolver.get_options(id, 0, &[], &ctx)
                })
                .await
                .unwrap_or_else(|e| Err(format!("spawn_blocking: {e}")))
            },
            move |result| PaletteMessage::OptionsLoaded(generation, id, result),
        );
        (Task::batch([focus_task, load_task]), None)
    }

    fn enter_snooze_stage2(
        &mut self,
        id: CommandId,
        param_label: &str,
    ) -> Task<PaletteMessage> {
        self.stage = PaletteStage::OptionPick;
        self.query.clear();
        self.selected_index = 0;
        self.stage2_command_id = Some(id);
        self.stage2_label = param_label.to_string();
        self.options_loading = false;

        let items = snooze_preset_options();
        self.option_matches = ratatoskr_command_palette::search_options(&items, "");
        self.option_items = items;

        iced::widget::operation::focus::<PaletteMessage>(
            "palette-input".to_string(),
        )
    }

    fn confirm_option(&mut self) -> (Task<PaletteMessage>, Option<PaletteEvent>) {
        let Some(option_match) = self.option_matches.get(self.selected_index) else {
            return (Task::none(), None);
        };
        if option_match.item.disabled {
            return (Task::none(), None);
        }

        let Some(command_id) = self.stage2_command_id else {
            return (Task::none(), None);
        };

        let Some(args) = build_command_args(command_id, &option_match.item) else {
            return (Task::none(), None);
        };

        self.close();
        (
            Task::none(),
            Some(PaletteEvent::ExecuteParameterized(command_id, args)),
        )
    }

    fn handle_options_loaded(
        &mut self,
        generation: u64,
        command_id: CommandId,
        result: Result<Vec<OptionItem>, String>,
    ) -> (Task<PaletteMessage>, Option<PaletteEvent>) {
        // Discard stale results
        if generation < self.option_load_generation {
            return (Task::none(), None);
        }
        // Verify we're still in the right stage for this command
        if self.stage2_command_id != Some(command_id) {
            return (Task::none(), None);
        }

        self.options_loading = false;

        match result {
            Ok(items) => {
                self.option_matches =
                    ratatoskr_command_palette::search_options(&items, &self.query);
                self.option_items = items;
                self.selected_index = 0;
                (Task::none(), None)
            }
            Err(msg) => {
                self.option_items.clear();
                self.option_matches.clear();
                (Task::none(), Some(PaletteEvent::Error(msg)))
            }
        }
    }
}

impl Component for Palette {
    type Message = PaletteMessage;
    type Event = PaletteEvent;

    fn update(
        &mut self,
        message: PaletteMessage,
    ) -> (Task<PaletteMessage>, Option<PaletteEvent>) {
        match message {
            PaletteMessage::Open(ctx) => {
                let results = self.registry.query(&ctx, "");
                self.open = true;
                self.query.clear();
                self.results = results;
                self.selected_index = 0;
                self.stage = PaletteStage::CommandSearch;
                let task = iced::widget::operation::focus::<PaletteMessage>(
                    "palette-input".to_string(),
                );
                (task, None)
            }
            PaletteMessage::Close(ctx) => {
                // In stage 2, Escape goes back to stage 1 instead of closing.
                if self.is_option_pick() {
                    self.back_to_stage1();
                    self.results = self.registry.query(&ctx, "");
                    let task = iced::widget::operation::focus::<PaletteMessage>(
                        "palette-input".to_string(),
                    );
                    return (task, None);
                }
                self.close();
                (Task::none(), Some(PaletteEvent::Dismissed))
            }
            PaletteMessage::QueryChanged(query, ctx) => {
                if self.is_option_pick() {
                    // Stage 2: filter options with fuzzy search
                    self.option_matches = ratatoskr_command_palette::search_options(
                        &self.option_items,
                        &query,
                    );
                    self.query = query;
                    self.selected_index = 0;
                } else {
                    // Stage 1: query the registry
                    self.results = self.registry.query(&ctx, &query);
                    self.query = query;
                    self.selected_index = 0;
                }
                (Task::none(), None)
            }
            PaletteMessage::SelectNext => {
                let len = if self.is_option_pick() {
                    self.option_matches.len()
                } else {
                    self.results.len()
                };
                if len > 0 {
                    self.selected_index = (self.selected_index + 1).min(len - 1);
                }
                let task = scroll_to_selected(self.selected_index, len).discard();
                (task, None)
            }
            PaletteMessage::SelectPrev => {
                self.selected_index = self.selected_index.saturating_sub(1);
                let len = if self.is_option_pick() {
                    self.option_matches.len()
                } else {
                    self.results.len()
                };
                let task = scroll_to_selected(self.selected_index, len).discard();
                (task, None)
            }
            PaletteMessage::Confirm(ctx) => {
                if self.is_option_pick() {
                    self.confirm_option()
                } else {
                    self.confirm(&ctx)
                }
            }
            PaletteMessage::ClickResult(idx, ctx) => {
                if idx < self.results.len() {
                    self.selected_index = idx;
                    self.confirm(&ctx)
                } else {
                    (Task::none(), None)
                }
            }
            PaletteMessage::ClickOption(idx) => {
                if idx < self.option_matches.len() {
                    self.selected_index = idx;
                    self.confirm_option()
                } else {
                    (Task::none(), None)
                }
            }
            PaletteMessage::OptionsLoaded(generation, command_id, result) => {
                self.handle_options_loaded(generation, command_id, result)
            }
        }
    }

    fn view(&self) -> Element<'_, PaletteMessage> {
        match &self.stage {
            PaletteStage::CommandSearch => build_command_search_card(self),
            PaletteStage::OptionPick => build_option_pick_card(self),
        }
    }
}

// ── View helpers ───────────────────────────────────────

fn build_command_search_card(state: &Palette) -> Element<'_, PaletteMessage> {
    // We need a placeholder context for closures. The real context is
    // carried by the message variant, but for the closure type we need
    // a default. We use a dummy that will be replaced by the App's
    // context wrapper when the message is mapped.
    let input = text_input("Type a command...", &state.query)
        .on_input(|q| PaletteMessage::QueryChanged(q, CommandContext::default()))
        .on_submit(PaletteMessage::Confirm(CommandContext::default()))
        .id("palette-input")
        .padding(PAD_INPUT)
        .size(TEXT_LG);

    let results_column = build_results_column(
        &state.results,
        state.selected_index,
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

fn build_option_pick_card(state: &Palette) -> Element<'_, PaletteMessage> {
    let placeholder = if state.options_loading {
        "Loading...".to_string()
    } else {
        format!("Search {}...", state.stage2_label)
    };

    let input = text_input(&placeholder, &state.query)
        .on_input(|q| PaletteMessage::QueryChanged(q, CommandContext::default()))
        .on_submit(PaletteMessage::Confirm(CommandContext::default()))
        .id("palette-input")
        .padding(PAD_INPUT)
        .size(TEXT_LG);

    let options_column = build_options_column(
        &state.option_matches,
        state.selected_index,
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

fn build_results_column(
    results: &[CommandMatch],
    selected_index: usize,
) -> Element<'_, PaletteMessage> {
    let mut col = column![].spacing(SPACE_XXXS);

    for (i, result) in results.iter().enumerate() {
        let is_selected = i == selected_index;
        let row_element = palette_result_row(
            result.category,
            result.palette_label,
            result.keybinding.clone(),
            result.available,
            result.input_mode,
            is_selected,
            PaletteMessage::ClickResult(i, CommandContext::default()),
        );
        col = col.push(row_element);
    }

    col.into()
}

fn build_options_column<'a>(
    options: &'a [OptionMatch],
    selected_index: usize,
) -> Element<'a, PaletteMessage> {
    let mut col = column![].spacing(SPACE_XXXS);

    for (i, option) in options.iter().enumerate() {
        let is_selected = i == selected_index;
        let row_element = option_result_row(option, is_selected, PaletteMessage::ClickOption(i));
        col = col.push(row_element);
    }

    col.into()
}

fn option_result_row<'a>(
    option: &'a OptionMatch,
    is_selected: bool,
    on_click: PaletteMessage,
) -> Element<'a, PaletteMessage> {
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
    let path_element: Element<'_, PaletteMessage> = if path_display.is_empty() {
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

fn palette_result_row<'a>(
    category_str: &'a str,
    label_str: &'a str,
    keybinding_str: Option<String>,
    available: bool,
    input_mode: InputMode,
    is_selected: bool,
    on_click: PaletteMessage,
) -> Element<'a, PaletteMessage> {
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
    let keybinding: Element<'_, PaletteMessage> = match keybinding_str {
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
fn scroll_to_selected(_selected_index: usize, _total_items: usize) -> Task<()> {
    Task::none()
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
fn snooze_preset_options() -> Vec<OptionItem> {
    use std::time::{SystemTime, UNIX_EPOCH};

    #[allow(clippy::cast_possible_wrap)]
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
