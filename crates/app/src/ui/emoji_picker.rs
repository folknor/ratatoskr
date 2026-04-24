// Emoji picker widget - a searchable, categorized emoji grid with skin tone
// support and recent-emoji persistence. The caller owns visibility state and
// positioning; this module provides the state struct and view function.
//
// NOTE: Emoji rendering depends on the system emoji font. On Linux, this
// typically requires `noto-fonts-emoji` or similar. The picker will display
// boxes/tofu without emoji fonts, but the selected emoji is inserted as
// text regardless.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use iced::widget::{Space, button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};

use crate::ui::layout::*;
use crate::ui::theme;

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

// ── Static emoji table ──────────────────────────────────
// ~300 commonly used emoji, organized by category. Each entry includes
// name, search keywords, category, and whether skin tone modifiers apply.

pub static EMOJI_TABLE: &[EmojiEntry] = &[
    // ── Smileys ─────────────────────────────────────────
    EmojiEntry {
        emoji: "\u{1F600}",
        name: "grinning face",
        keywords: "happy,smile,grin",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F603}",
        name: "grinning face with big eyes",
        keywords: "happy,joy",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F604}",
        name: "grinning face with smiling eyes",
        keywords: "happy,joy,laugh",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F601}",
        name: "beaming face with smiling eyes",
        keywords: "happy,grin",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F606}",
        name: "grinning squinting face",
        keywords: "laugh,happy",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F605}",
        name: "grinning face with sweat",
        keywords: "hot,relief",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F923}",
        name: "rolling on the floor laughing",
        keywords: "rofl,lol,laugh",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F602}",
        name: "face with tears of joy",
        keywords: "cry,laugh,lol",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F642}",
        name: "slightly smiling face",
        keywords: "smile",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F643}",
        name: "upside down face",
        keywords: "sarcasm,silly",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F609}",
        name: "winking face",
        keywords: "wink,flirt",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F60A}",
        name: "smiling face with smiling eyes",
        keywords: "blush,happy",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F607}",
        name: "smiling face with halo",
        keywords: "angel,innocent",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F970}",
        name: "smiling face with hearts",
        keywords: "love,adore",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F60D}",
        name: "smiling face with heart eyes",
        keywords: "love,crush",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F929}",
        name: "star struck",
        keywords: "wow,amazing,stars",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F618}",
        name: "face blowing a kiss",
        keywords: "kiss,love,flirt",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F617}",
        name: "kissing face",
        keywords: "kiss",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F61A}",
        name: "kissing face with closed eyes",
        keywords: "kiss,love",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F619}",
        name: "kissing face with smiling eyes",
        keywords: "kiss",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F972}",
        name: "smiling face with tear",
        keywords: "grateful,sad,happy",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F60B}",
        name: "face savoring food",
        keywords: "yummy,delicious",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F61B}",
        name: "face with tongue",
        keywords: "playful,silly",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F61C}",
        name: "winking face with tongue",
        keywords: "playful,wink",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F92A}",
        name: "zany face",
        keywords: "crazy,wild",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F61D}",
        name: "squinting face with tongue",
        keywords: "playful",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F911}",
        name: "money mouth face",
        keywords: "rich,money",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F917}",
        name: "hugging face",
        keywords: "hug,warm",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F92D}",
        name: "face with hand over mouth",
        keywords: "oops,giggle",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F92B}",
        name: "shushing face",
        keywords: "quiet,secret,shh",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F914}",
        name: "thinking face",
        keywords: "think,hmm,wonder",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F910}",
        name: "zipper mouth face",
        keywords: "secret,quiet",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F928}",
        name: "face with raised eyebrow",
        keywords: "skeptical,doubt",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F610}",
        name: "neutral face",
        keywords: "meh,indifferent",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F611}",
        name: "expressionless face",
        keywords: "blank,meh",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F636}",
        name: "face without mouth",
        keywords: "silent,speechless",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F60F}",
        name: "smirking face",
        keywords: "smirk,suggestive",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F612}",
        name: "unamused face",
        keywords: "dissatisfied,meh",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F644}",
        name: "face with rolling eyes",
        keywords: "annoyed,whatever",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F62C}",
        name: "grimacing face",
        keywords: "awkward,nervous",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F925}",
        name: "lying face",
        keywords: "pinocchio,liar",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F60C}",
        name: "relieved face",
        keywords: "relief,calm",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F614}",
        name: "pensive face",
        keywords: "sad,thoughtful",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F62A}",
        name: "sleepy face",
        keywords: "tired,sleep",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F924}",
        name: "drooling face",
        keywords: "hungry,drool",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F634}",
        name: "sleeping face",
        keywords: "zzz,asleep",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F637}",
        name: "face with medical mask",
        keywords: "sick,mask,covid",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F912}",
        name: "face with thermometer",
        keywords: "sick,fever",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F915}",
        name: "face with head bandage",
        keywords: "hurt,injured",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F922}",
        name: "nauseated face",
        keywords: "sick,gross",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F92E}",
        name: "face vomiting",
        keywords: "sick,puke",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F927}",
        name: "sneezing face",
        keywords: "sick,achoo",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F975}",
        name: "hot face",
        keywords: "sweating,heat",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F976}",
        name: "cold face",
        keywords: "freezing,cold",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F974}",
        name: "woozy face",
        keywords: "dizzy,drunk",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F635}",
        name: "face with crossed-out eyes",
        keywords: "dead,knocked out",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F621}",
        name: "pouting face",
        keywords: "angry,rage,mad",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F620}",
        name: "angry face",
        keywords: "mad,annoyed",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F92C}",
        name: "face with symbols on mouth",
        keywords: "swearing,angry",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F608}",
        name: "smiling face with horns",
        keywords: "devil,evil",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F47F}",
        name: "angry face with horns",
        keywords: "devil,demon",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F480}",
        name: "skull",
        keywords: "dead,death,skeleton",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4A9}",
        name: "pile of poo",
        keywords: "poop,crap",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F921}",
        name: "clown face",
        keywords: "clown,silly",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F47B}",
        name: "ghost",
        keywords: "halloween,spooky",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F62D}",
        name: "loudly crying face",
        keywords: "cry,sob,sad",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F631}",
        name: "face screaming in fear",
        keywords: "scared,horror",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F633}",
        name: "flushed face",
        keywords: "embarrassed,blush",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F97A}",
        name: "pleading face",
        keywords: "puppy eyes,beg",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F622}",
        name: "crying face",
        keywords: "sad,tear",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F625}",
        name: "sad but relieved face",
        keywords: "disappointed",
        category: EmojiCategory::Smileys,
        skin_tone_support: false,
    },
    // ── People (hands & gestures) ───────────────────────
    EmojiEntry {
        emoji: "\u{1F44B}",
        name: "waving hand",
        keywords: "hello,hi,bye,wave",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F91A}",
        name: "raised back of hand",
        keywords: "hand,backhand",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{270B}",
        name: "raised hand",
        keywords: "stop,high five",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F596}",
        name: "vulcan salute",
        keywords: "spock,trek",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F44C}",
        name: "ok hand",
        keywords: "ok,perfect,fine",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{270C}\u{FE0F}",
        name: "victory hand",
        keywords: "peace,v",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F91E}",
        name: "crossed fingers",
        keywords: "luck,hope",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F91F}",
        name: "love you gesture",
        keywords: "ily,love",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F918}",
        name: "sign of the horns",
        keywords: "rock,metal",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F919}",
        name: "call me hand",
        keywords: "shaka,call",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F448}",
        name: "backhand index pointing left",
        keywords: "point,left",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F449}",
        name: "backhand index pointing right",
        keywords: "point,right",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F446}",
        name: "backhand index pointing up",
        keywords: "point,up",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F447}",
        name: "backhand index pointing down",
        keywords: "point,down",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{261D}\u{FE0F}",
        name: "index pointing up",
        keywords: "point,one",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F44D}",
        name: "thumbs up",
        keywords: "yes,good,like,approve",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F44E}",
        name: "thumbs down",
        keywords: "no,bad,dislike",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{270A}",
        name: "raised fist",
        keywords: "power,punch,fist",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F44A}",
        name: "oncoming fist",
        keywords: "punch,fist bump",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F91B}",
        name: "left facing fist",
        keywords: "fist bump",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F91C}",
        name: "right facing fist",
        keywords: "fist bump",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F44F}",
        name: "clapping hands",
        keywords: "clap,bravo,applause",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F64C}",
        name: "raising hands",
        keywords: "hooray,celebrate",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F450}",
        name: "open hands",
        keywords: "jazz hands",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F932}",
        name: "palms up together",
        keywords: "prayer,receive",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F91D}",
        name: "handshake",
        keywords: "deal,agree,shake",
        category: EmojiCategory::People,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F64F}",
        name: "folded hands",
        keywords: "pray,please,thank",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F4AA}",
        name: "flexed biceps",
        keywords: "strong,muscle,power",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F595}",
        name: "middle finger",
        keywords: "rude",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{270D}\u{FE0F}",
        name: "writing hand",
        keywords: "write,author",
        category: EmojiCategory::People,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F9E0}",
        name: "brain",
        keywords: "smart,think,mind",
        category: EmojiCategory::People,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F441}\u{FE0F}",
        name: "eye",
        keywords: "look,see,watch",
        category: EmojiCategory::People,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F440}",
        name: "eyes",
        keywords: "look,see,watch",
        category: EmojiCategory::People,
        skin_tone_support: false,
    },
    // ── Nature ──────────────────────────────────────────
    EmojiEntry {
        emoji: "\u{1F436}",
        name: "dog face",
        keywords: "puppy,pet",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F431}",
        name: "cat face",
        keywords: "kitten,pet",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F42D}",
        name: "mouse face",
        keywords: "rodent",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F439}",
        name: "hamster",
        keywords: "pet,rodent",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F430}",
        name: "rabbit face",
        keywords: "bunny,easter",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F98A}",
        name: "fox",
        keywords: "fox face",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F43B}",
        name: "bear",
        keywords: "teddy",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F43C}",
        name: "panda",
        keywords: "bear,bamboo",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F428}",
        name: "koala",
        keywords: "australia",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F42F}",
        name: "tiger face",
        keywords: "cat,wild",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F981}",
        name: "lion",
        keywords: "king,wild",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F984}",
        name: "unicorn",
        keywords: "magic,fantasy",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F41D}",
        name: "honeybee",
        keywords: "bee,insect,honey",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F98B}",
        name: "butterfly",
        keywords: "insect,pretty",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F40C}",
        name: "snail",
        keywords: "slow,slug",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F41B}",
        name: "bug",
        keywords: "insect",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F422}",
        name: "turtle",
        keywords: "slow,tortoise",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F40D}",
        name: "snake",
        keywords: "reptile",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F433}",
        name: "spouting whale",
        keywords: "ocean,sea",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F42C}",
        name: "dolphin",
        keywords: "ocean,sea",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F338}",
        name: "cherry blossom",
        keywords: "flower,spring,sakura",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F33A}",
        name: "hibiscus",
        keywords: "flower,tropical",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F33B}",
        name: "sunflower",
        keywords: "flower,sun",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F33C}",
        name: "blossom",
        keywords: "flower",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F337}",
        name: "tulip",
        keywords: "flower,spring",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F339}",
        name: "rose",
        keywords: "flower,love,romantic",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F490}",
        name: "bouquet",
        keywords: "flowers,arrangement",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F340}",
        name: "four leaf clover",
        keywords: "luck,irish",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F331}",
        name: "seedling",
        keywords: "plant,grow,sprout",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F334}",
        name: "palm tree",
        keywords: "tropical,beach",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F333}",
        name: "deciduous tree",
        keywords: "nature,forest",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F332}",
        name: "evergreen tree",
        keywords: "pine,christmas",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F335}",
        name: "cactus",
        keywords: "desert,plant",
        category: EmojiCategory::Nature,
        skin_tone_support: false,
    },
    // ── Food ────────────────────────────────────────────
    EmojiEntry {
        emoji: "\u{1F34E}",
        name: "red apple",
        keywords: "fruit,healthy",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F34A}",
        name: "tangerine",
        keywords: "orange,fruit,citrus",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F34B}",
        name: "lemon",
        keywords: "citrus,sour",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F34C}",
        name: "banana",
        keywords: "fruit",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F349}",
        name: "watermelon",
        keywords: "fruit,summer",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F347}",
        name: "grapes",
        keywords: "fruit,wine",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F353}",
        name: "strawberry",
        keywords: "fruit,berry",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F352}",
        name: "cherries",
        keywords: "fruit",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F351}",
        name: "peach",
        keywords: "fruit",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F34D}",
        name: "pineapple",
        keywords: "fruit,tropical",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F96D}",
        name: "mango",
        keywords: "fruit,tropical",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F951}",
        name: "avocado",
        keywords: "fruit,guacamole",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F955}",
        name: "carrot",
        keywords: "vegetable",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F33D}",
        name: "ear of corn",
        keywords: "vegetable,maize",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F336}\u{FE0F}",
        name: "hot pepper",
        keywords: "spicy,chili",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F354}",
        name: "hamburger",
        keywords: "burger,fast food",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F355}",
        name: "pizza",
        keywords: "food,italian",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F32E}",
        name: "taco",
        keywords: "mexican,food",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F32F}",
        name: "burrito",
        keywords: "mexican,wrap",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F363}",
        name: "sushi",
        keywords: "japanese,food,fish",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F35C}",
        name: "steaming bowl",
        keywords: "noodles,ramen,soup",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F370}",
        name: "shortcake",
        keywords: "dessert,cake",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F382}",
        name: "birthday cake",
        keywords: "party,celebration",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F36B}",
        name: "chocolate bar",
        keywords: "candy,sweet",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F369}",
        name: "doughnut",
        keywords: "donut,sweet",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F36A}",
        name: "cookie",
        keywords: "biscuit,sweet",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2615}",
        name: "hot beverage",
        keywords: "coffee,tea",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F37A}",
        name: "beer mug",
        keywords: "drink,alcohol",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F377}",
        name: "wine glass",
        keywords: "drink,alcohol",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F379}",
        name: "tropical drink",
        keywords: "cocktail,vacation",
        category: EmojiCategory::Food,
        skin_tone_support: false,
    },
    // ── Activities ──────────────────────────────────────
    EmojiEntry {
        emoji: "\u{26BD}",
        name: "soccer ball",
        keywords: "football,sport",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3C0}",
        name: "basketball",
        keywords: "sport,ball",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3C8}",
        name: "american football",
        keywords: "sport,nfl",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{26BE}",
        name: "baseball",
        keywords: "sport,ball",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3BE}",
        name: "tennis",
        keywords: "sport,racket",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3D0}",
        name: "volleyball",
        keywords: "sport,ball",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3B3}",
        name: "bowling",
        keywords: "sport,pins",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3CF}",
        name: "cricket game",
        keywords: "sport,bat",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{26F3}",
        name: "flag in hole",
        keywords: "golf,sport",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3AF}",
        name: "bullseye",
        keywords: "target,dart",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3AE}",
        name: "video game",
        keywords: "gaming,controller",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3B2}",
        name: "game die",
        keywords: "dice,random",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3B5}",
        name: "musical note",
        keywords: "music,song",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3B6}",
        name: "musical notes",
        keywords: "music,song",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3A4}",
        name: "microphone",
        keywords: "sing,karaoke",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3AC}",
        name: "clapper board",
        keywords: "movie,film",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3A8}",
        name: "artist palette",
        keywords: "art,paint",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3AD}",
        name: "performing arts",
        keywords: "theater,drama",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3C6}",
        name: "trophy",
        keywords: "winner,award",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3C5}",
        name: "sports medal",
        keywords: "award,winner",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F947}",
        name: "first place medal",
        keywords: "gold,winner",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F948}",
        name: "second place medal",
        keywords: "silver",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F949}",
        name: "third place medal",
        keywords: "bronze",
        category: EmojiCategory::Activities,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3C3}",
        name: "person running",
        keywords: "run,exercise,jog",
        category: EmojiCategory::Activities,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F6B4}",
        name: "person biking",
        keywords: "bicycle,cycling",
        category: EmojiCategory::Activities,
        skin_tone_support: true,
    },
    EmojiEntry {
        emoji: "\u{1F3CA}",
        name: "person swimming",
        keywords: "swim,pool",
        category: EmojiCategory::Activities,
        skin_tone_support: true,
    },
    // ── Travel ──────────────────────────────────────────
    EmojiEntry {
        emoji: "\u{2708}\u{FE0F}",
        name: "airplane",
        keywords: "flight,travel,plane",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F680}",
        name: "rocket",
        keywords: "space,launch",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F697}",
        name: "automobile",
        keywords: "car,drive",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F695}",
        name: "taxi",
        keywords: "cab,ride",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F68C}",
        name: "bus",
        keywords: "transit,public",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F682}",
        name: "locomotive",
        keywords: "train,railway",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F6A2}",
        name: "ship",
        keywords: "boat,cruise",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F6F3}\u{FE0F}",
        name: "passenger ship",
        keywords: "boat,cruise,ferry",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3D6}\u{FE0F}",
        name: "beach with umbrella",
        keywords: "vacation,coast",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3D4}\u{FE0F}",
        name: "snow capped mountain",
        keywords: "alps,skiing",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F30D}",
        name: "globe europe africa",
        keywords: "world,earth",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F30E}",
        name: "globe americas",
        keywords: "world,earth",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F30F}",
        name: "globe asia australia",
        keywords: "world,earth",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F5FA}\u{FE0F}",
        name: "world map",
        keywords: "geography,atlas",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3E0}",
        name: "house",
        keywords: "home,building",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3E2}",
        name: "office building",
        keywords: "work,corporate",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3EB}",
        name: "school",
        keywords: "education,building",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3E5}",
        name: "hospital",
        keywords: "medical,building",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{26F0}\u{FE0F}",
        name: "mountain",
        keywords: "peak,hiking",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F30B}",
        name: "volcano",
        keywords: "lava,eruption",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F307}",
        name: "sunset",
        keywords: "evening,dusk",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F305}",
        name: "sunrise",
        keywords: "morning,dawn",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F319}",
        name: "crescent moon",
        keywords: "night,sleep",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2B50}",
        name: "star",
        keywords: "night,gold",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F31F}",
        name: "glowing star",
        keywords: "sparkle,bright",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2600}\u{FE0F}",
        name: "sun",
        keywords: "sunny,bright,weather",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F326}\u{FE0F}",
        name: "sun behind rain cloud",
        keywords: "weather",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F308}",
        name: "rainbow",
        keywords: "colorful,pride",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2744}\u{FE0F}",
        name: "snowflake",
        keywords: "winter,cold,snow",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F525}",
        name: "fire",
        keywords: "hot,flame,lit",
        category: EmojiCategory::Travel,
        skin_tone_support: false,
    },
    // ── Objects ─────────────────────────────────────────
    EmojiEntry {
        emoji: "\u{1F4A1}",
        name: "light bulb",
        keywords: "idea,electricity",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4BB}",
        name: "laptop",
        keywords: "computer,tech",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4F1}",
        name: "mobile phone",
        keywords: "cell,smartphone",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4E7}",
        name: "email",
        keywords: "mail,message,letter",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4E8}",
        name: "incoming envelope",
        keywords: "mail,received",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4E9}",
        name: "envelope with arrow",
        keywords: "mail,outgoing",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4EC}",
        name: "open mailbox with raised flag",
        keywords: "mail,inbox",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4DD}",
        name: "memo",
        keywords: "note,write,pencil",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4D6}",
        name: "open book",
        keywords: "read,literature",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4DA}",
        name: "books",
        keywords: "library,read",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4CE}",
        name: "paperclip",
        keywords: "attach,office",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4CB}",
        name: "clipboard",
        keywords: "paste,copy",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4C5}",
        name: "calendar",
        keywords: "date,schedule",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4C6}",
        name: "tear off calendar",
        keywords: "date,schedule",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4C8}",
        name: "chart increasing",
        keywords: "graph,growth,trend",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4C9}",
        name: "chart decreasing",
        keywords: "graph,decline",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4CA}",
        name: "bar chart",
        keywords: "graph,stats",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F512}",
        name: "locked",
        keywords: "security,padlock",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F513}",
        name: "unlocked",
        keywords: "open,security",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F511}",
        name: "key",
        keywords: "password,lock",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F50D}",
        name: "magnifying glass left",
        keywords: "search,find,zoom",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F50E}",
        name: "magnifying glass right",
        keywords: "search,find,zoom",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2699}\u{FE0F}",
        name: "gear",
        keywords: "settings,cog",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F527}",
        name: "wrench",
        keywords: "tool,fix,repair",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F528}",
        name: "hammer",
        keywords: "tool,build",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4B0}",
        name: "money bag",
        keywords: "dollar,rich,cash",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4B5}",
        name: "dollar banknote",
        keywords: "money,cash",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F381}",
        name: "wrapped gift",
        keywords: "present,birthday",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F389}",
        name: "party popper",
        keywords: "celebrate,tada",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F38A}",
        name: "confetti ball",
        keywords: "celebrate,party",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4F7}",
        name: "camera",
        keywords: "photo,picture",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4FA}",
        name: "television",
        keywords: "tv,watch,screen",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{23F0}",
        name: "alarm clock",
        keywords: "time,wake up",
        category: EmojiCategory::Objects,
        skin_tone_support: false,
    },
    // ── Symbols ─────────────────────────────────────────
    EmojiEntry {
        emoji: "\u{2764}\u{FE0F}",
        name: "red heart",
        keywords: "love,like",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F9E1}",
        name: "orange heart",
        keywords: "love",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F49B}",
        name: "yellow heart",
        keywords: "love,friendship",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F49A}",
        name: "green heart",
        keywords: "love,nature",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F499}",
        name: "blue heart",
        keywords: "love,trust",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F49C}",
        name: "purple heart",
        keywords: "love",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F5A4}",
        name: "black heart",
        keywords: "love,dark",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F90D}",
        name: "white heart",
        keywords: "love,pure",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F90E}",
        name: "brown heart",
        keywords: "love",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F494}",
        name: "broken heart",
        keywords: "sad,breakup",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F495}",
        name: "two hearts",
        keywords: "love,couple",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F496}",
        name: "sparkling heart",
        keywords: "love,sparkle",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4AF}",
        name: "hundred points",
        keywords: "100,perfect,score",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4A5}",
        name: "collision",
        keywords: "boom,crash,bang",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4AB}",
        name: "dizzy",
        keywords: "star,sparkle",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2728}",
        name: "sparkles",
        keywords: "glitter,magic,clean",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4A2}",
        name: "anger symbol",
        keywords: "angry,vein",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2757}",
        name: "red exclamation mark",
        keywords: "important,alert",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2753}",
        name: "red question mark",
        keywords: "confused,what",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2705}",
        name: "check mark button",
        keywords: "done,complete,yes",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{274C}",
        name: "cross mark",
        keywords: "no,wrong,delete",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{274E}",
        name: "cross mark button",
        keywords: "no,wrong",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2795}",
        name: "plus",
        keywords: "add,more",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2796}",
        name: "minus",
        keywords: "subtract,less",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{27A1}\u{FE0F}",
        name: "right arrow",
        keywords: "next,forward",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2B05}\u{FE0F}",
        name: "left arrow",
        keywords: "back,previous",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2B06}\u{FE0F}",
        name: "up arrow",
        keywords: "top",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2B07}\u{FE0F}",
        name: "down arrow",
        keywords: "bottom",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{267B}\u{FE0F}",
        name: "recycling symbol",
        keywords: "recycle,green",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F504}",
        name: "counterclockwise arrows",
        keywords: "refresh,sync",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F51D}",
        name: "top arrow",
        keywords: "top",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F6AB}",
        name: "prohibited",
        keywords: "forbidden,no",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{26A0}\u{FE0F}",
        name: "warning",
        keywords: "caution,alert",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{2139}\u{FE0F}",
        name: "information",
        keywords: "info,help",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F198}",
        name: "SOS button",
        keywords: "help,emergency",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F4F4}",
        name: "mobile phone off",
        keywords: "mute,silent",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{269B}\u{FE0F}",
        name: "atom symbol",
        keywords: "science,physics",
        category: EmojiCategory::Symbols,
        skin_tone_support: false,
    },
    // ── Flags ───────────────────────────────────────────
    EmojiEntry {
        emoji: "\u{1F3F3}\u{FE0F}",
        name: "white flag",
        keywords: "surrender,peace",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3F4}",
        name: "black flag",
        keywords: "pirate",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3C1}",
        name: "chequered flag",
        keywords: "finish,race,racing",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F6A9}",
        name: "triangular flag",
        keywords: "pennant,post",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F3F3}\u{FE0F}\u{200D}\u{1F308}",
        name: "rainbow flag",
        keywords: "pride,lgbt,lgbtq",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    // Country flags (regional indicator pairs)
    EmojiEntry {
        emoji: "\u{1F1FA}\u{1F1F8}",
        name: "flag United States",
        keywords: "us,usa,america",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EC}\u{1F1E7}",
        name: "flag United Kingdom",
        keywords: "uk,britain,gb",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E8}\u{1F1E6}",
        name: "flag Canada",
        keywords: "canada,ca",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E6}\u{1F1FA}",
        name: "flag Australia",
        keywords: "australia,au",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E9}\u{1F1EA}",
        name: "flag Germany",
        keywords: "germany,de,deutsch",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EB}\u{1F1F7}",
        name: "flag France",
        keywords: "france,fr",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EA}\u{1F1F8}",
        name: "flag Spain",
        keywords: "spain,es",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EE}\u{1F1F9}",
        name: "flag Italy",
        keywords: "italy,it",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EF}\u{1F1F5}",
        name: "flag Japan",
        keywords: "japan,jp",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F0}\u{1F1F7}",
        name: "flag South Korea",
        keywords: "korea,kr",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E8}\u{1F1F3}",
        name: "flag China",
        keywords: "china,cn",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EE}\u{1F1F3}",
        name: "flag India",
        keywords: "india,in",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E7}\u{1F1F7}",
        name: "flag Brazil",
        keywords: "brazil,br",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F7}\u{1F1FA}",
        name: "flag Russia",
        keywords: "russia,ru",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F2}\u{1F1FD}",
        name: "flag Mexico",
        keywords: "mexico,mx",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F8}\u{1F1EA}",
        name: "flag Sweden",
        keywords: "sweden,se",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F3}\u{1F1F4}",
        name: "flag Norway",
        keywords: "norway,no",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E9}\u{1F1F0}",
        name: "flag Denmark",
        keywords: "denmark,dk",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EB}\u{1F1EE}",
        name: "flag Finland",
        keywords: "finland,fi",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F3}\u{1F1F1}",
        name: "flag Netherlands",
        keywords: "netherlands,nl,dutch",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E7}\u{1F1EA}",
        name: "flag Belgium",
        keywords: "belgium,be",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E8}\u{1F1ED}",
        name: "flag Switzerland",
        keywords: "switzerland,ch",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E6}\u{1F1F9}",
        name: "flag Austria",
        keywords: "austria,at",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F5}\u{1F1F1}",
        name: "flag Poland",
        keywords: "poland,pl",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F5}\u{1F1F9}",
        name: "flag Portugal",
        keywords: "portugal,pt",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EC}\u{1F1F7}",
        name: "flag Greece",
        keywords: "greece,gr",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F9}\u{1F1F7}",
        name: "flag Turkey",
        keywords: "turkey,tr",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EE}\u{1F1F1}",
        name: "flag Israel",
        keywords: "israel,il",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F8}\u{1F1E6}",
        name: "flag Saudi Arabia",
        keywords: "saudi,sa",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E6}\u{1F1EA}",
        name: "flag United Arab Emirates",
        keywords: "uae,emirates",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1FF}\u{1F1E6}",
        name: "flag South Africa",
        keywords: "south africa,za",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F3}\u{1F1EC}",
        name: "flag Nigeria",
        keywords: "nigeria,ng",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EA}\u{1F1EC}",
        name: "flag Egypt",
        keywords: "egypt,eg",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E6}\u{1F1F7}",
        name: "flag Argentina",
        keywords: "argentina,ar",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1E8}\u{1F1F4}",
        name: "flag Colombia",
        keywords: "colombia,co",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F9}\u{1F1ED}",
        name: "flag Thailand",
        keywords: "thailand,th",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F5}\u{1F1ED}",
        name: "flag Philippines",
        keywords: "philippines,ph",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EE}\u{1F1E9}",
        name: "flag Indonesia",
        keywords: "indonesia,id",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F2}\u{1F1FE}",
        name: "flag Malaysia",
        keywords: "malaysia,my",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F8}\u{1F1EC}",
        name: "flag Singapore",
        keywords: "singapore,sg",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1F3}\u{1F1FF}",
        name: "flag New Zealand",
        keywords: "new zealand,nz",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1EE}\u{1F1EA}",
        name: "flag Ireland",
        keywords: "ireland,ie",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
    EmojiEntry {
        emoji: "\u{1F1FA}\u{1F1E6}",
        name: "flag Ukraine",
        keywords: "ukraine,ua",
        category: EmojiCategory::Flags,
        skin_tone_support: false,
    },
];
