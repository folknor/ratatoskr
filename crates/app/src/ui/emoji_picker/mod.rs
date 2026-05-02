// Emoji picker widget - a searchable, categorized emoji grid with skin tone
// support and recent-emoji persistence. The caller owns visibility state and
// positioning; this module provides the state struct and view function.
//
// NOTE: Emoji rendering depends on the system emoji font. On Linux, this
// typically requires `noto-fonts-emoji` or similar. The picker will display
// boxes/tofu without emoji fonts, but the selected emoji is inserted as
// text regardless.

#![allow(dead_code)] // Module not yet wired into compose toolbar.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};

use crate::ui::layout::*;
use crate::ui::theme;

mod table;
pub use table::EMOJI_TABLE;

// ── Emoji data model ────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EmojiCategory {
    Recent,
    Smileys,
    People,
    Nature,
    Food,
    Activities,
    Travel,
    Objects,
    Symbols,
    Flags,
}

impl EmojiCategory {
    /// Categories shown as tabs (Recent is only shown when non-empty).
    pub const ALL: &[Self] = &[
        Self::Recent,
        Self::Smileys,
        Self::People,
        Self::Nature,
        Self::Food,
        Self::Activities,
        Self::Travel,
        Self::Objects,
        Self::Symbols,
        Self::Flags,
    ];

    /// Categories excluding Recent (for iterating the static table).
    pub const STATIC: &[Self] = &[
        Self::Smileys,
        Self::People,
        Self::Nature,
        Self::Food,
        Self::Activities,
        Self::Travel,
        Self::Objects,
        Self::Symbols,
        Self::Flags,
    ];

    /// Representative emoji shown on the category tab.
    pub fn tab_emoji(self) -> &'static str {
        match self {
            Self::Recent => "\u{1F552}",         // 🕒
            Self::Smileys => "\u{1F600}",        // 😀
            Self::People => "\u{1F44B}",         // 👋
            Self::Nature => "\u{1F338}",         // 🌸
            Self::Food => "\u{1F354}",           // 🍔
            Self::Activities => "\u{26BD}",      // ⚽
            Self::Travel => "\u{2708}\u{FE0F}",  // ✈️
            Self::Objects => "\u{1F4A1}",        // 💡
            Self::Symbols => "\u{267B}\u{FE0F}", // ♻️
            Self::Flags => "\u{1F3F3}\u{FE0F}",  // 🏳️
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Recent => "Recent",
            Self::Smileys => "Smileys & Emotion",
            Self::People => "People & Body",
            Self::Nature => "Animals & Nature",
            Self::Food => "Food & Drink",
            Self::Activities => "Activities",
            Self::Travel => "Travel & Places",
            Self::Objects => "Objects",
            Self::Symbols => "Symbols",
            Self::Flags => "Flags",
        }
    }
}

pub struct EmojiEntry {
    pub emoji: &'static str,
    pub name: &'static str,
    pub keywords: &'static str,
    pub category: EmojiCategory,
    pub skin_tone_support: bool,
}

// ── Skin tone ──────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SkinTone {
    Default,
    Light,
    MediumLight,
    Medium,
    MediumDark,
    Dark,
}

impl SkinTone {
    pub const ALL: &[Self] = &[
        Self::Default,
        Self::Light,
        Self::MediumLight,
        Self::Medium,
        Self::MediumDark,
        Self::Dark,
    ];

    /// The Fitzpatrick modifier codepoint, if applicable.
    fn modifier(self) -> Option<char> {
        match self {
            Self::Default => None,
            Self::Light => Some('\u{1F3FB}'),
            Self::MediumLight => Some('\u{1F3FC}'),
            Self::Medium => Some('\u{1F3FD}'),
            Self::MediumDark => Some('\u{1F3FE}'),
            Self::Dark => Some('\u{1F3FF}'),
        }
    }

    /// Display swatch for the skin tone selector.
    pub fn swatch(self) -> &'static str {
        match self {
            Self::Default => "\u{270B}", // ✋ (default yellow)
            Self::Light => "\u{270B}\u{1F3FB}",
            Self::MediumLight => "\u{270B}\u{1F3FC}",
            Self::Medium => "\u{270B}\u{1F3FD}",
            Self::MediumDark => "\u{270B}\u{1F3FE}",
            Self::Dark => "\u{270B}\u{1F3FF}",
        }
    }
}

/// Apply skin tone modifier to an emoji string. For emoji that support skin
/// tones, the modifier is inserted after the first codepoint (the base).
fn apply_skin_tone(base: &str, tone: SkinTone) -> String {
    let Some(modifier) = tone.modifier() else {
        return base.to_string();
    };
    let mut chars = base.chars();
    let mut result = String::with_capacity(base.len() + 4);
    if let Some(first) = chars.next() {
        result.push(first);
        result.push(modifier);
        for ch in chars {
            result.push(ch);
        }
    }
    result
}

// ── Picker state ──────────────────────────────────────

/// Maximum number of recent emoji to track.
const MAX_RECENT: usize = 28;

/// Picker state owned by the caller. Create with `EmojiPickerState::new()`,
/// pass to `emoji_picker_view()` for rendering.
#[derive(Debug, Clone)]
pub struct EmojiPickerState {
    pub search_text: String,
    pub active_category: EmojiCategory,
    pub skin_tone: SkinTone,
    pub recent: VecDeque<String>,
    data_dir: PathBuf,
}

impl EmojiPickerState {
    pub fn new(data_dir: &Path) -> Self {
        let recent = load_recent(data_dir);
        Self {
            search_text: String::new(),
            active_category: if recent.is_empty() {
                EmojiCategory::Smileys
            } else {
                EmojiCategory::Recent
            },
            skin_tone: SkinTone::Default,
            recent,
            data_dir: data_dir.to_path_buf(),
        }
    }

    /// Record an emoji as recently used and persist to disk.
    pub fn record_recent(&mut self, emoji: &str) {
        // Remove if already present so it moves to front.
        self.recent.retain(|e| e != emoji);
        self.recent.push_front(emoji.to_string());
        self.recent.truncate(MAX_RECENT);
        save_recent(&self.data_dir, &self.recent);
    }

    /// Reset search and category when the picker is reopened.
    pub fn reset_for_open(&mut self) {
        self.search_text.clear();
        self.active_category = if self.recent.is_empty() {
            EmojiCategory::Smileys
        } else {
            EmojiCategory::Recent
        };
    }
}

// ── Messages ──────────────────────────────────────────

/// Messages emitted by the emoji picker. The caller maps these in its own
/// message enum.
#[derive(Debug, Clone)]
pub enum EmojiPickerMessage {
    SearchChanged(String),
    CategorySelected(EmojiCategory),
    SkinToneSelected(SkinTone),
    EmojiSelected(String),
}

// ── Update ──────────────────────────────────────────

/// Process an emoji picker message. Returns `Some(emoji_string)` when the
/// user selects an emoji, so the caller can insert it.
pub fn update(state: &mut EmojiPickerState, msg: EmojiPickerMessage) -> Option<String> {
    match msg {
        EmojiPickerMessage::SearchChanged(query) => {
            state.search_text = query;
            None
        }
        EmojiPickerMessage::CategorySelected(cat) => {
            state.active_category = cat;
            state.search_text.clear();
            None
        }
        EmojiPickerMessage::SkinToneSelected(tone) => {
            state.skin_tone = tone;
            None
        }
        EmojiPickerMessage::EmojiSelected(emoji) => {
            state.record_recent(&emoji);
            Some(emoji)
        }
    }
}

// ── View ──────────────────────────────────────────────

/// Build the emoji picker element. The caller wraps this in a popover or
/// overlay container and controls visibility.
pub fn emoji_picker_view<'a, M: Clone + 'a>(
    state: &'a EmojiPickerState,
    map_msg: impl Fn(EmojiPickerMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let search_bar = build_search_bar(state, map_msg.clone());
    let skin_tone_row = build_skin_tone_selector(state, map_msg.clone());
    let category_tabs = build_category_tabs(state, map_msg.clone());
    let grid = build_emoji_grid(state, map_msg);

    let content = column![search_bar, skin_tone_row, category_tabs, grid].spacing(SPACE_XXS);

    container(content)
        .width(EMOJI_PICKER_WIDTH)
        .max_height(EMOJI_PICKER_MAX_HEIGHT)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .into()
}

fn build_search_bar<'a, M: Clone + 'a>(
    state: &'a EmojiPickerState,
    map_msg: impl Fn(EmojiPickerMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let input = text_input("Search emoji...", &state.search_text)
        .on_input(move |s| map_msg.clone()(EmojiPickerMessage::SearchChanged(s)))
        .size(TEXT_SM)
        .padding(PAD_INPUT);

    container(input).width(Length::Fill).into()
}

fn build_skin_tone_selector<'a, M: Clone + 'a>(
    state: &'a EmojiPickerState,
    map_msg: impl Fn(EmojiPickerMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let tones: Vec<Element<'a, M>> = SkinTone::ALL
        .iter()
        .map(|&tone| {
            let map_msg = map_msg.clone();
            let is_active = state.skin_tone == tone;

            let label = text(tone.swatch()).size(TEXT_LG);

            let btn_class = if is_active {
                theme::ButtonClass::Chip { active: true }
            } else {
                theme::ButtonClass::Ghost
            };

            button(
                container(label)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            )
            .on_press(map_msg(EmojiPickerMessage::SkinToneSelected(tone)))
            .padding(PAD_BADGE)
            .style(btn_class.style())
            .into()
        })
        .collect();

    let tone_row = iced::widget::Row::with_children(tones).spacing(SPACE_XXXS);

    container(
        row![
            text("Skin tone:")
                .size(TEXT_XS)
                .style(theme::TextClass::Muted.style()),
            tone_row,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .width(Length::Fill)
    .into()
}

fn build_category_tabs<'a, M: Clone + 'a>(
    state: &'a EmojiPickerState,
    map_msg: impl Fn(EmojiPickerMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let tabs: Vec<Element<'a, M>> = EmojiCategory::ALL
        .iter()
        .filter(|&&cat| cat != EmojiCategory::Recent || !state.recent.is_empty())
        .map(|&cat| {
            let map_msg = map_msg.clone();
            let is_active = state.active_category == cat;

            let label = text(cat.tab_emoji()).size(TEXT_LG);

            let btn_class = if is_active {
                theme::ButtonClass::Chip { active: true }
            } else {
                theme::ButtonClass::Ghost
            };

            button(
                container(label)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            )
            .on_press(map_msg(EmojiPickerMessage::CategorySelected(cat)))
            .padding(PAD_BADGE)
            .style(btn_class.style())
            .into()
        })
        .collect();

    let tab_row = iced::widget::Row::with_children(tabs).spacing(SPACE_XXXS);

    container(tab_row).width(Length::Fill).into()
}

fn build_emoji_grid<'a, M: Clone + 'a>(
    state: &'a EmojiPickerState,
    map_msg: impl Fn(EmojiPickerMessage) -> M + Clone + 'a,
) -> Element<'a, M> {
    let query = state.search_text.trim().to_lowercase();
    let is_searching = !query.is_empty();

    let mut grid_rows: Vec<Element<'a, M>> = Vec::new();

    if is_searching {
        // Search mode: show matching emoji from all categories.
        let matching = collect_search_results(&query, state.skin_tone);
        grid_rows.extend(build_grid_rows(&matching, &map_msg));
    } else if state.active_category == EmojiCategory::Recent {
        // Recent mode: show recently used emoji.
        let recents: Vec<(String, &str)> = state.recent.iter().map(|e| (e.clone(), "")).collect();
        grid_rows.extend(build_grid_rows(&recents, &map_msg));
    } else {
        // Category mode: show emoji for the active category.
        let category_emoji = collect_category(state.active_category, state.skin_tone);
        grid_rows.extend(build_grid_rows(&category_emoji, &map_msg));
    }

    if grid_rows.is_empty() {
        let empty = container(
            text("No emoji found")
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        )
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .padding(SPACE_LG);

        return empty.into();
    }

    let grid_col = iced::widget::Column::with_children(grid_rows);

    scrollable(container(grid_col).width(Length::Fill))
        .height(Length::Fill)
        .into()
}

/// Build rows of emoji buttons from a list of (emoji_string, name) pairs.
fn build_grid_rows<'a, M: Clone + 'a>(
    items: &[(String, &'a str)],
    map_msg: &(impl Fn(EmojiPickerMessage) -> M + Clone + 'a),
) -> Vec<Element<'a, M>> {
    items
        .chunks(EMOJI_GRID_COLUMNS)
        .map(|chunk| {
            let btns: Vec<Element<'a, M>> = chunk
                .iter()
                .map(|(emoji, name)| {
                    let map_msg = map_msg.clone();
                    let emoji_owned = emoji.clone();
                    let emoji_display = emoji.clone();

                    let label = text(emoji_display)
                        .size(EMOJI_FONT_SIZE)
                        .line_height(iced::widget::text::LineHeight::Relative(1.0));

                    let btn = button(
                        container(label)
                            .width(EMOJI_BUTTON_SIZE)
                            .height(EMOJI_BUTTON_SIZE)
                            .align_x(Alignment::Center)
                            .align_y(Alignment::Center),
                    )
                    .on_press(map_msg(EmojiPickerMessage::EmojiSelected(emoji_owned)))
                    .padding(0.0)
                    .style(theme::ButtonClass::Ghost.style());

                    let el: Element<'a, M> = if name.is_empty() {
                        btn.into()
                    } else {
                        iced::widget::tooltip(
                            btn,
                            text(*name).size(TEXT_XS),
                            iced::widget::tooltip::Position::Bottom,
                        )
                        .gap(SPACE_XXXS)
                        .style(theme::ContainerClass::Floating.style())
                        .into()
                    };

                    el
                })
                .collect();

            // Pad with spacers if the last row is incomplete.
            let mut row_children = btns;
            while row_children.len() < EMOJI_GRID_COLUMNS {
                row_children.push(
                    Space::new()
                        .width(EMOJI_BUTTON_SIZE)
                        .height(EMOJI_BUTTON_SIZE)
                        .into(),
                );
            }

            iced::widget::Row::with_children(row_children)
                .spacing(SPACE_XXXS)
                .into()
        })
        .collect()
}

/// Collect emoji matching a search query, with skin tone applied.
fn collect_search_results(query: &str, tone: SkinTone) -> Vec<(String, &'static str)> {
    EMOJI_TABLE
        .iter()
        .filter(|entry| {
            entry.name.contains(query)
                || entry
                    .keywords
                    .split(',')
                    .any(|kw| kw.trim().contains(query))
        })
        .map(|entry| {
            let rendered = if entry.skin_tone_support {
                apply_skin_tone(entry.emoji, tone)
            } else {
                entry.emoji.to_string()
            };
            (rendered, entry.name)
        })
        .collect()
}

/// Collect emoji for a specific category, with skin tone applied.
fn collect_category(category: EmojiCategory, tone: SkinTone) -> Vec<(String, &'static str)> {
    EMOJI_TABLE
        .iter()
        .filter(|entry| entry.category == category)
        .map(|entry| {
            let rendered = if entry.skin_tone_support {
                apply_skin_tone(entry.emoji, tone)
            } else {
                entry.emoji.to_string()
            };
            (rendered, entry.name)
        })
        .collect()
}

// ── Recent emoji persistence ─────────────────────────

fn recent_file_path(data_dir: &Path) -> PathBuf {
    data_dir.join("emoji_recent.json")
}

fn load_recent(data_dir: &Path) -> VecDeque<String> {
    let path = recent_file_path(data_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return VecDeque::new();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn save_recent(data_dir: &Path, recent: &VecDeque<String>) {
    let path = recent_file_path(data_dir);
    if let Ok(json) = serde_json::to_string(recent) {
        let _ = std::fs::write(&path, json);
    }
}
