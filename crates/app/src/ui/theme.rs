use std::cell::Cell;

use iced::theme::palette::Seed;
use iced::widget::{button, container, pick_list, radio, rule, slider, text, text_input, toggler};
use iced::{Color, Theme, border};
use serde::Deserialize;

use super::layout::*;
use super::settings::types::EmailBodyBackground;

// ── Email body background thread-local ──────────────────
// Style functions only receive `&Theme`, so we use a thread-local to
// communicate the user's preference to `style_email_body_container`.
thread_local! {
    static EMAIL_BODY_BG_PREF: Cell<EmailBodyBackground> = const { Cell::new(EmailBodyBackground::AlwaysWhite) };
}

/// Set the email body background preference. Call when preferences change.
pub fn set_email_body_background(pref: EmailBodyBackground) {
    EMAIL_BODY_BG_PREF.set(pref);
}

// ── Built-in theme catalog ─────────────────────────────

pub struct ThemeEntry {
    pub name: &'static str,
    pub palette: Seed,
}

pub const THEMES: &[ThemeEntry] = &[
    ThemeEntry {
        name: "Light",
        palette: Seed::LIGHT,
    },
    ThemeEntry {
        name: "Dark",
        palette: Seed::DARK,
    },
    ThemeEntry {
        name: "Dracula",
        palette: Seed::DRACULA,
    },
    ThemeEntry {
        name: "Nord",
        palette: Seed::NORD,
    },
    ThemeEntry {
        name: "Solarized Light",
        palette: Seed::SOLARIZED_LIGHT,
    },
    ThemeEntry {
        name: "Solarized Dark",
        palette: Seed::SOLARIZED_DARK,
    },
    ThemeEntry {
        name: "Gruvbox Light",
        palette: Seed::GRUVBOX_LIGHT,
    },
    ThemeEntry {
        name: "Gruvbox Dark",
        palette: Seed::GRUVBOX_DARK,
    },
    ThemeEntry {
        name: "Catppuccin Latte",
        palette: Seed::CATPPUCCIN_LATTE,
    },
    ThemeEntry {
        name: "Catppuccin Frappé",
        palette: Seed::CATPPUCCIN_FRAPPE,
    },
    ThemeEntry {
        name: "Catppuccin Macchiato",
        palette: Seed::CATPPUCCIN_MACCHIATO,
    },
    ThemeEntry {
        name: "Catppuccin Mocha",
        palette: Seed::CATPPUCCIN_MOCHA,
    },
    ThemeEntry {
        name: "Tokyo Night",
        palette: Seed::TOKYO_NIGHT,
    },
    ThemeEntry {
        name: "Tokyo Night Storm",
        palette: Seed::TOKYO_NIGHT_STORM,
    },
    ThemeEntry {
        name: "Tokyo Night Light",
        palette: Seed::TOKYO_NIGHT_LIGHT,
    },
    ThemeEntry {
        name: "Kanagawa Wave",
        palette: Seed::KANAGAWA_WAVE,
    },
    ThemeEntry {
        name: "Kanagawa Lotus",
        palette: Seed::KANAGAWA_LOTUS,
    },
    ThemeEntry {
        name: "Moonfly",
        palette: Seed::MOONFLY,
    },
    ThemeEntry {
        name: "Nightfly",
        palette: Seed::NIGHTFLY,
    },
    ThemeEntry {
        name: "Oxocarbon",
        palette: Seed::OXOCARBON,
    },
    ThemeEntry {
        name: "Ferra",
        palette: Seed::FERRA,
    },
];

pub fn theme_by_index(index: usize) -> Theme {
    let entry = &THEMES[index.min(THEMES.len() - 1)];
    Theme::custom(entry.name.to_string(), entry.palette)
}

// ── Semantic colors ────────────────────────────────────
// Colors that don't come from the theme palette.

/// Text/icon color on top of avatar circles and primary buttons.
pub const ON_AVATAR: Color = Color::WHITE;

// ── TOML loading ────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct ThemeFile {
    name: Option<String>,
    colors: ThemeColors,
}

#[derive(Debug, Deserialize)]
struct ThemeColors {
    background: String,
    text: String,
    primary: String,
    success: String,
    warning: String,
    danger: String,
}

pub fn from_toml(content: &str) -> Result<Theme, toml::de::Error> {
    let file: ThemeFile = toml::from_str(content)?;
    let palette = Seed {
        background: hex_to_color(&file.colors.background),
        text: hex_to_color(&file.colors.text),
        primary: hex_to_color(&file.colors.primary),
        success: hex_to_color(&file.colors.success),
        warning: hex_to_color(&file.colors.warning),
        danger: hex_to_color(&file.colors.danger),
    };
    Ok(Theme::custom(
        file.name.unwrap_or_else(|| "Custom".into()),
        palette,
    ))
}

// ══════════════════════════════════════════════════════════
// ── Class enums ──────────────────────────────────────────
// Each enum variant centralizes a style that was previously
// scattered as inline closures or bare function references.
// ══════════════════════════════════════════════════════════

// ── TextClass ───────────────────────────────────────────

/// Custom text styles beyond iced's built-in `text::base`, `text::primary`, etc.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextClass {
    /// Primary accent color (e.g. unread subject).
    Accent,
    /// Faded tertiary text (timestamps, metadata).
    Tertiary,
    /// Slightly muted text (inactive nav labels, descriptions).
    Muted,
    /// Text on a primary-colored background (e.g. help tooltips).
    OnPrimary,
    /// Warning text/icon color (for status bar warnings).
    Warning,
    /// Default text color (inherits from theme, no override).
    Default,
}

impl TextClass {
    pub fn style(self) -> fn(&Theme) -> text::Style {
        match self {
            Self::Accent => style_text_accent,
            Self::Tertiary => style_text_tertiary,
            Self::Muted => style_text_muted,
            Self::OnPrimary => style_text_on_primary,
            Self::Warning => style_text_warning,
            Self::Default => style_text_default,
        }
    }
}

fn style_text_accent(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().primary.base.color),
    }
}

fn style_text_tertiary(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().background.strongest.text.scale_alpha(0.5)),
    }
}

fn style_text_muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().background.base.text.scale_alpha(0.6)),
    }
}

fn style_text_on_primary(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().primary.base.text),
    }
}

fn style_text_warning(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.palette().warning.base.color),
    }
}

fn style_text_default(_theme: &Theme) -> text::Style {
    text::Style { color: None }
}

// ── ButtonClass ─────────────────────────────────────────

/// All custom button styles used in the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ButtonClass {
    /// Rounded primary button (compose, save).
    Primary,
    /// Rounded secondary button (settings).
    Secondary,
    /// Sidebar / settings nav item with active highlight.
    Nav { active: bool },
    /// Dropdown menu item, optionally selected.
    Dropdown { selected: bool },
    /// Thread card with selection and starred state.
    ThreadCard { selected: bool, starred: bool },
    /// Invisible button - no background in any state.
    Ghost,
    /// Icon-only button on a weakest-background surface.
    BareIcon,
    /// Fully transparent button (no bg, no border).
    BareTransparent,
    /// Hoverable row (settings rows, collapsible headers).
    Action,
    /// Collapsed message row.
    CollapsedMessage,
    /// Active star toggle button.
    StarActive,
    /// Toggleable chip / pill button.
    Chip { active: bool },
    /// Pinned search card in the sidebar.
    PinnedSearch { active: bool },
    /// Protocol selection card (normal).
    ProtocolCard,
    /// Protocol selection card (selected, primary border).
    ProtocolCardSelected,
    /// Color swatch with selection ring.
    ColorSwatchSelected,
    /// Experimental numbered variant.
    Experiment { variant: usize },
    /// Experimental semantic variant (success/warning/danger).
    ExperimentSemantic { variant: usize },
}

impl ButtonClass {
    /// Returns a style closure suitable for `.style()`.
    pub fn style(self) -> impl Fn(&Theme, button::Status) -> button::Style {
        move |theme, status| self.resolve(theme, status)
    }

    fn resolve(self, theme: &Theme, status: button::Status) -> button::Style {
        match self {
            Self::Primary => style_primary_button(theme, status),
            Self::Secondary => style_secondary_button(theme, status),
            Self::Nav { active } => style_nav_button(theme, status, active),
            Self::Dropdown { selected } => style_dropdown_button(theme, status, selected),
            Self::ThreadCard { selected, starred } => {
                style_thread_card_button(theme, status, selected, starred)
            }
            Self::Ghost => style_ghost_button(theme, status),
            Self::BareIcon => style_bare_icon_button(theme, status),
            Self::BareTransparent => style_bare_transparent_button(),
            Self::Action => style_action_button(theme, status),
            Self::CollapsedMessage => style_collapsed_message_button(theme, status),
            Self::StarActive => style_star_active_button(theme, status),
            Self::Chip { active } => style_chip_button(theme, status, active),
            Self::PinnedSearch { active } => style_pinned_search_button(theme, status, active),
            Self::ProtocolCard => style_protocol_card_button(theme, status, false),
            Self::ProtocolCardSelected => style_protocol_card_button(theme, status, true),
            Self::ColorSwatchSelected => style_color_swatch_selected_button(theme, status),
            Self::Experiment { variant } => style_exp_btn(theme, status, variant),
            Self::ExperimentSemantic { variant } => style_exp_semantic_btn(theme, status, variant),
        }
    }
}

// ── ContainerClass ──────────────────────────────────────

/// All custom container styles used in the app.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ContainerClass {
    /// Darkest background (thread list).
    Base,
    /// Panel divider (normal).
    Divider,
    /// Panel divider (hovered/dragging).
    DividerHover,
    /// Content area (reading pane, settings content).
    Content,
    /// Sidebar background.
    Sidebar,
    /// Surface with subtle border (no radius).
    Surface,
    /// Elevated card with border and rounded corners.
    Elevated,
    /// Pill-shaped badge background.
    Badge,
    /// Message card with rounded border.
    MessageCard,
    /// Email body inset - always white for rendering fidelity.
    EmailBody,
    /// Action bar (no radius).
    ActionBar,
    /// Floating tooltip/popover on primary background.
    Floating,
    /// Settings section card with shadow.
    SettingsSection,
    /// Select dropdown menu with shadow.
    SelectMenu,
    /// Selected theme preview ring.
    ThemeSelectedRing,
    /// Shortcut key badge.
    KeyBadge,
    /// Active drag-reorder row highlight.
    DraggingRow,
    /// Status bar background.
    StatusBar,
    /// Palette card (elevated container with shadow).
    PaletteCard,
    /// Palette selected result row.
    PaletteSelectedRow,
    /// Semi-transparent dark overlay behind modals.
    ModalBackdrop,
    /// Calendar day cell with subtle border.
    CalendarCell,
    /// Today's calendar cell (accent border/tint).
    CalendarCellToday,
    /// Non-current-month calendar cell (muted background).
    CalendarCellMuted,
    /// Mini-month selected date highlight.
    MiniMonthSelected,
    /// Time grid hour label cell (left column).
    TimeGridHourLabel,
    /// Time grid cell border (very subtle).
    TimeGridCell,
    /// Today column header highlight in time grid.
    TimeGridTodayHeader,
    /// Current-time indicator line (red/accent).
    TimeGridNowLine,
    /// Floating chord indicator badge (bottom-right).
    ChordIndicator,
    /// Chat bubble - sent by user (accent background).
    ChatBubbleSent,
    /// Chat bubble - received from contact (surface background).
    ChatBubbleReceived,
}

impl ContainerClass {
    pub fn style(self) -> fn(&Theme) -> container::Style {
        match self {
            Self::Base => style_base_container,
            Self::Divider => style_divider_container,
            Self::DividerHover => style_divider_hover_container,
            Self::Content => style_content_container,
            Self::Sidebar => style_sidebar_container,
            Self::Surface => style_surface_container,
            Self::Elevated => style_elevated_container,
            Self::Badge => style_badge_container,
            Self::MessageCard => style_message_card_container,
            Self::EmailBody => style_email_body_container,
            Self::ActionBar => style_action_bar_container,
            Self::Floating => style_floating_container,
            Self::SettingsSection => style_settings_section_container,
            Self::SelectMenu => style_select_menu_container,
            Self::ThemeSelectedRing => style_theme_selected_ring,
            Self::KeyBadge => style_key_badge_container,
            Self::DraggingRow => style_dragging_row_container,
            Self::StatusBar => style_status_bar_container,
            Self::PaletteCard => style_palette_card_container,
            Self::PaletteSelectedRow => style_palette_selected_row_container,
            Self::ModalBackdrop => style_modal_backdrop_container,
            Self::CalendarCell => style_calendar_cell_container,
            Self::CalendarCellToday => style_calendar_cell_today_container,
            Self::CalendarCellMuted => style_calendar_cell_muted_container,
            Self::MiniMonthSelected => style_mini_month_selected_container,
            Self::TimeGridHourLabel => style_time_grid_hour_label_container,
            Self::TimeGridCell => style_time_grid_cell_container,
            Self::TimeGridTodayHeader => style_time_grid_today_header_container,
            Self::TimeGridNowLine => style_time_grid_now_line_container,
            Self::ChordIndicator => style_chord_indicator_container,
            Self::ChatBubbleSent => style_chat_bubble_sent,
            Self::ChatBubbleReceived => style_chat_bubble_received,
        }
    }
}

// ── RuleClass ───────────────────────────────────────────

/// Custom horizontal/vertical rule styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleClass {
    /// Standard divider (15% alpha).
    Divider,
    /// Sidebar vertical divider (solid weak).
    SidebarDivider,
    /// Subtle divider within sections (25% alpha).
    Subtle,
}

impl RuleClass {
    pub fn style(self) -> fn(&Theme) -> rule::Style {
        match self {
            Self::Divider => style_divider_rule,
            Self::SidebarDivider => style_sidebar_divider_rule,
            Self::Subtle => style_subtle_divider_rule,
        }
    }
}

// ── TextInputClass ──────────────────────────────────────

/// Custom text input styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextInputClass {
    /// Invisible input that looks like plain text.
    Inline,
    /// Settings field with border and background.
    Settings,
}

impl TextInputClass {
    pub fn style(self) -> fn(&Theme, text_input::Status) -> text_input::Style {
        match self {
            Self::Inline => style_inline_text_input,
            Self::Settings => style_settings_text_input,
        }
    }
}

// ── SliderClass ─────────────────────────────────────────

/// Custom slider styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliderClass {
    Settings,
}

impl SliderClass {
    pub fn style(self) -> fn(&Theme, slider::Status) -> slider::Style {
        match self {
            Self::Settings => style_settings_slider,
        }
    }
}

// ── RadioClass ──────────────────────────────────────────

/// Custom radio button styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RadioClass {
    Settings,
}

impl RadioClass {
    pub fn style(self) -> fn(&Theme, radio::Status) -> radio::Style {
        match self {
            Self::Settings => style_settings_radio,
        }
    }
}

// ── TogglerClass ────────────────────────────────────────

/// Custom toggler styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TogglerClass {
    Settings,
}

impl TogglerClass {
    pub fn style(self) -> fn(&Theme, toggler::Status) -> toggler::Style {
        match self {
            Self::Settings => style_settings_toggler,
        }
    }
}

// ── PickListClass ───────────────────────────────────────

/// Custom pick list styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickListClass {
    Ghost,
}

impl PickListClass {
    pub fn style(self) -> fn(&Theme, pick_list::Status) -> pick_list::Style {
        match self {
            Self::Ghost => style_ghost_pick_list,
        }
    }
}

// ══════════════════════════════════════════════════════════
// ── Style implementations ────────────────────────────────
// Private functions that hold the actual style logic.
// ══════════════════════════════════════════════════════════

// ── Button style implementations ────────────────────────

fn style_primary_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.border = border::rounded(RADIUS_LG);
    style
}

fn style_secondary_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::secondary(theme, status);
    style.border = border::rounded(RADIUS_LG);
    style
}

fn style_dropdown_button(theme: &Theme, status: button::Status, selected: bool) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weakest.color.into()),
            text_color: p.background.base.text,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            background: None,
            text_color: if selected {
                p.background.base.text
            } else {
                p.secondary.base.color
            },
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
    }
}

fn style_nav_button(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let p = theme.palette();
    let inactive_text = p.background.base.text.scale_alpha(0.6);
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weak.color.into()),
            text_color: if active {
                p.primary.base.color
            } else {
                p.background.base.text
            },
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            background: if active {
                Some(p.background.strong.color.into())
            } else {
                None
            },
            text_color: if active {
                p.primary.base.color
            } else {
                inactive_text
            },
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
    }
}

fn style_thread_card_button(
    theme: &Theme,
    status: button::Status,
    selected: bool,
    starred: bool,
) -> button::Style {
    let p = theme.palette();
    let base_bg = if starred {
        mix(
            p.background.base.color,
            p.warning.base.color,
            STARRED_BG_ALPHA,
        )
    } else if selected {
        p.background.weakest.color
    } else {
        p.background.base.color
    };
    match status {
        button::Status::Hovered => button::Style {
            background: Some(if starred {
                mix(
                    p.background.weakest.color,
                    p.warning.base.color,
                    STARRED_BG_ALPHA,
                )
                .into()
            } else {
                p.background.weakest.color.into()
            }),
            text_color: p.background.base.text,
            ..Default::default()
        },
        _ => button::Style {
            background: Some(base_bg.into()),
            text_color: p.background.base.text,
            ..Default::default()
        },
    }
}

fn style_ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weakest.color.into()),
            text_color: p.background.base.text,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            text_color: p.background.base.text,
            ..Default::default()
        },
    }
}

fn style_bare_icon_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weaker.color.into()),
            text_color: p.background.base.text,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            text_color: p.secondary.base.color,
            ..Default::default()
        },
    }
}

fn style_bare_transparent_button() -> button::Style {
    button::Style {
        background: None,
        ..Default::default()
    }
}

fn style_action_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weak.color.into()),
            text_color: p.background.base.text,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            text_color: p.secondary.base.color,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
    }
}

fn style_collapsed_message_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weakest.color.into()),
            text_color: p.background.base.text,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            background: None,
            text_color: p.background.base.text,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
    }
}

fn style_star_active_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.warning.base.color.scale_alpha(0.2).into()),
            text_color: p.warning.base.color,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
        _ => button::Style {
            background: Some(p.warning.base.color.scale_alpha(0.1).into()),
            text_color: p.warning.base.color,
            border: border::rounded(RADIUS_SM),
            ..Default::default()
        },
    }
}

fn style_chip_button(theme: &Theme, status: button::Status, active: bool) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered => style_chip_hovered(p, active),
        _ => style_chip_idle(p, active),
    }
}

fn style_chip_hovered(p: &iced::theme::palette::Palette, active: bool) -> button::Style {
    button::Style {
        background: Some(if active {
            p.primary.base.color.scale_alpha(0.25).into()
        } else {
            p.background.weakest.color.into()
        }),
        text_color: if active {
            p.primary.base.color
        } else {
            p.background.base.text
        },
        border: iced::Border {
            color: if active {
                p.primary.base.color.scale_alpha(0.4)
            } else {
                p.background.strongest.color.scale_alpha(0.2)
            },
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

fn style_chip_idle(p: &iced::theme::palette::Palette, active: bool) -> button::Style {
    button::Style {
        background: Some(if active {
            p.primary.base.color.scale_alpha(0.15).into()
        } else {
            p.background.base.color.into()
        }),
        text_color: if active {
            p.primary.base.color
        } else {
            p.background.base.text.scale_alpha(0.7)
        },
        border: iced::Border {
            color: if active {
                p.primary.base.color.scale_alpha(0.3)
            } else {
                p.background.strongest.color.scale_alpha(0.15)
            },
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

fn style_pinned_search_button(
    theme: &Theme,
    status: button::Status,
    active: bool,
) -> button::Style {
    let p = theme.palette();
    match status {
        button::Status::Hovered if active => button::Style {
            background: Some(p.background.stronger.color.into()),
            text_color: p.background.base.text,
            border: iced::Border {
                color: p.background.stronger.color.scale_alpha(0.1),
                width: 1.0,
                radius: RADIUS_MD.into(),
            },
            ..Default::default()
        },
        button::Status::Hovered => button::Style {
            background: Some(p.background.weak.color.into()),
            text_color: p.background.base.text,
            border: iced::Border {
                color: p.background.strongest.color.scale_alpha(0.1),
                width: 1.0,
                radius: RADIUS_MD.into(),
            },
            ..Default::default()
        },
        _ => button::Style {
            background: Some(if active {
                p.background.strong.color.into()
            } else {
                p.background.weakest.color.into()
            }),
            text_color: if active {
                p.primary.base.color
            } else {
                p.background.base.text
            },
            border: iced::Border {
                color: p.background.strongest.color.scale_alpha(0.08),
                width: 1.0,
                radius: RADIUS_MD.into(),
            },
            ..Default::default()
        },
    }
}

// ── Protocol card + color swatch button styles ──────────

fn style_protocol_card_button(
    theme: &Theme,
    status: button::Status,
    selected: bool,
) -> button::Style {
    let p = theme.palette();
    let bg_base = p.background.base.color;
    let pri = p.primary.base.color;
    let is_hovered = matches!(status, button::Status::Hovered);

    let border_color = if selected {
        pri
    } else if is_hovered {
        mix(bg_base, p.background.base.text, 0.2)
    } else {
        mix(bg_base, p.background.base.text, 0.1)
    };

    let background = if selected {
        Some(mix(bg_base, pri, 0.08).into())
    } else if is_hovered {
        Some(mix(bg_base, p.background.base.text, 0.04).into())
    } else {
        Some(bg_base.into())
    };

    button::Style {
        background,
        text_color: p.background.base.text,
        border: iced::Border {
            color: border_color,
            width: if selected { 2.0 } else { 1.0 },
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}

fn style_color_swatch_selected_button(theme: &Theme, _status: button::Status) -> button::Style {
    let p = theme.palette();
    let pri = p.primary.base.color;

    button::Style {
        background: None,
        text_color: p.background.base.text,
        border: iced::Border {
            color: pri,
            width: 2.0,
            radius: 6.0.into(),
        },
        ..Default::default()
    }
}

// ── Experimental button implementations ─────────────────

fn style_exp_btn(theme: &Theme, status: button::Status, variant: usize) -> button::Style {
    let p = theme.palette();
    let bg_base = p.background.base.color;
    let pri = p.primary.base.color;
    let txt = p.background.base.text;
    let is_hovered = matches!(status, button::Status::Hovered);

    let (background, text_color, border_color) =
        exp_btn_colors(p, bg_base, pri, txt, is_hovered, variant);

    button::Style {
        background,
        text_color,
        border: match border_color {
            Some(c) => iced::Border {
                color: c,
                width: 1.0,
                radius: RADIUS_LG.into(),
            },
            None => border::rounded(RADIUS_LG),
        },
        ..Default::default()
    }
}

#[allow(clippy::type_complexity)]
fn exp_btn_colors(
    p: &iced::theme::palette::Palette,
    bg_base: Color,
    pri: Color,
    txt: Color,
    hovered: bool,
    variant: usize,
) -> (Option<iced::Background>, Color, Option<Color>) {
    match variant {
        8 => exp_btn_outlined_primary(bg_base, pri, hovered),
        9 => exp_btn_outlined_text(bg_base, txt, hovered),
        10 => exp_btn_filled_primary(bg_base, pri, hovered),
        11 => exp_btn_muted_border(bg_base, txt, hovered),
        12 => exp_btn_derived(p, hovered),
        16..=20 => exp_btn_mixed(bg_base, pri, txt, hovered, variant),
        _ => (None, txt, None),
    }
}

fn exp_btn_outlined_primary(
    bg: Color,
    pri: Color,
    hovered: bool,
) -> (Option<iced::Background>, Color, Option<Color>) {
    if hovered {
        (Some(mix(bg, pri, 0.12).into()), pri, Some(pri))
    } else {
        (None, pri, Some(pri))
    }
}

fn exp_btn_outlined_text(
    bg: Color,
    txt: Color,
    hovered: bool,
) -> (Option<iced::Background>, Color, Option<Color>) {
    if hovered {
        (
            Some(mix(bg, txt, 0.08).into()),
            txt,
            Some(txt.scale_alpha(0.4)),
        )
    } else {
        (None, txt, Some(txt.scale_alpha(0.3)))
    }
}

fn exp_btn_filled_primary(
    bg: Color,
    pri: Color,
    hovered: bool,
) -> (Option<iced::Background>, Color, Option<Color>) {
    if hovered {
        (
            Some(mix(bg, pri, 0.18).into()),
            pri,
            Some(pri.scale_alpha(0.6)),
        )
    } else {
        (
            Some(mix(bg, pri, 0.08).into()),
            pri,
            Some(pri.scale_alpha(0.4)),
        )
    }
}

fn exp_btn_muted_border(
    bg: Color,
    txt: Color,
    hovered: bool,
) -> (Option<iced::Background>, Color, Option<Color>) {
    if hovered {
        (
            Some(mix(bg, txt, 0.08).into()),
            txt.scale_alpha(0.85),
            Some(txt.scale_alpha(0.25)),
        )
    } else {
        (None, txt.scale_alpha(0.7), Some(txt.scale_alpha(0.15)))
    }
}

fn exp_btn_derived(
    p: &iced::theme::palette::Palette,
    hovered: bool,
) -> (Option<iced::Background>, Color, Option<Color>) {
    if hovered {
        (Some(p.primary.base.color.into()), p.primary.base.text, None)
    } else {
        (Some(p.primary.weak.color.into()), p.primary.weak.text, None)
    }
}

fn exp_btn_mixed(
    bg: Color,
    pri: Color,
    txt: Color,
    hovered: bool,
    variant: usize,
) -> (Option<iced::Background>, Color, Option<Color>) {
    let (base_t, hover_t, blend_color) = match variant {
        16 => (0.10, 0.18, pri),
        17 => (0.20, 0.28, pri),
        18 => (0.30, 0.38, pri),
        19 => (0.10, 0.18, txt),
        _ /* 20 */ => (0.15, 0.23, pri),
    };
    let t = if hovered { hover_t } else { base_t };
    (Some(mix(bg, blend_color, t).into()), txt, None)
}

fn style_exp_semantic_btn(theme: &Theme, status: button::Status, variant: usize) -> button::Style {
    let p = theme.palette();
    let is_hovered = matches!(status, button::Status::Hovered);

    let (base_color, base_text) = match variant {
        0 => (p.success.base.color, p.success.base.text),
        1 => (p.warning.base.color, p.warning.base.text),
        2 => (p.danger.base.color, p.danger.base.text),
        _ => (p.primary.base.color, p.primary.base.text),
    };

    let bg = if is_hovered {
        mix(base_color, p.background.base.color, 0.15)
    } else {
        base_color
    };

    button::Style {
        background: Some(bg.into()),
        text_color: base_text,
        border: border::rounded(RADIUS_LG),
        ..Default::default()
    }
}

// ── Container style implementations ─────────────────────

fn style_base_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.base.color.into()),
        ..Default::default()
    }
}

fn style_divider_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.strong.color.into()),
        ..Default::default()
    }
}

fn style_divider_hover_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.strongest.color.into()),
        ..Default::default()
    }
}

fn style_content_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.weakest.color.into()),
        ..Default::default()
    }
}

fn style_sidebar_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.weaker.color.into()),
        ..Default::default()
    }
}

fn style_surface_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weaker.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_elevated_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

fn style_badge_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().background.weak.color.into()),
        border: iced::Border {
            radius: RADIUS_LG.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

fn style_message_card_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weaker.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        ..Default::default()
    }
}

fn style_email_body_container(theme: &Theme) -> container::Style {
    let pref = EMAIL_BODY_BG_PREF.get();
    let theme_bg = theme.palette().background.base.color;
    let use_white = match pref {
        EmailBodyBackground::AlwaysWhite => true,
        EmailBodyBackground::MatchTheme => false,
        EmailBodyBackground::Auto => {
            // Luminance check: if the theme background is light, use white.
            let lum = 0.299 * theme_bg.r + 0.587 * theme_bg.g + 0.114 * theme_bg.b;
            lum > 0.5
        }
    };
    let (bg, border_color) = if use_white {
        (Color::WHITE, Color::BLACK.scale_alpha(0.08))
    } else {
        (
            theme_bg,
            theme.palette().background.strongest.color.scale_alpha(0.15),
        )
    };
    container::Style {
        background: Some(bg.into()),
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

fn style_action_bar_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weaker.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.15),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_floating_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.85).into()),
        border: iced::Border {
            color: p.primary.strong.color,
            width: 2.0,
            radius: RADIUS_LG.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.25),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: RADIUS_LG,
        },
        ..Default::default()
    }
}

fn style_settings_section_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.1),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.15),
            offset: iced::Vector::ZERO,
            blur_radius: RADIUS_LG,
        },
        ..Default::default()
    }
}

fn style_select_menu_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.2),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.25),
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: RADIUS_LG,
        },
        ..Default::default()
    }
}

fn style_theme_selected_ring(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        border: iced::Border {
            color: p.primary.base.color,
            width: 2.0,
            radius: (RADIUS_MD + 4.0).into(),
        },
        ..Default::default()
    }
}

fn style_key_badge_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.2),
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

fn style_dragging_row_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.1).into()),
        border: iced::Border {
            color: p.primary.base.color.scale_alpha(0.3),
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

fn style_status_bar_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weaker.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.1),
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

// ── Palette style implementations ────────────────────────

fn style_palette_card_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        border: iced::Border {
            radius: RADIUS_LG.into(),
            width: 1.0,
            color: p.background.strong.color,
        },
        shadow: iced::Shadow {
            color: Color {
                a: 0.3,
                ..Color::BLACK
            },
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 16.0,
        },
        ..Default::default()
    }
}

fn style_palette_selected_row_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.weak.color.into()),
        border: border::rounded(RADIUS_SM),
        ..Default::default()
    }
}

fn style_modal_backdrop_container(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(
            Color {
                r: 0.0,
                g: 0.0,
                b: 0.0,
                a: 0.5,
            }
            .into(),
        ),
        ..Default::default()
    }
}

// ── Calendar container style implementations ────────────

fn style_calendar_cell_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.1),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_calendar_cell_today_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.06).into()),
        border: iced::Border {
            color: p.primary.base.color.scale_alpha(0.4),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_calendar_cell_muted_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.08),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_mini_month_selected_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.15).into()),
        border: iced::Border {
            radius: RADIUS_SM.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ── Time grid container style implementations ────────

fn style_time_grid_hour_label_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.08),
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_time_grid_cell_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.08),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_time_grid_today_header_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.base.color.scale_alpha(0.08).into()),
        border: iced::Border {
            color: p.primary.base.color.scale_alpha(0.3),
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn style_time_grid_now_line_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.danger.base.color.into()),
        ..Default::default()
    }
}

fn style_chord_indicator_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weak.color.into()),
        border: border::rounded(RADIUS_SM)
            .color(p.background.strongest.color.scale_alpha(0.3))
            .width(1.0),
        shadow: iced::Shadow {
            color: Color {
                a: 0.15,
                ..Color::BLACK
            },
            offset: iced::Vector::new(0.0, 2.0),
            blur_radius: 4.0,
        },
        ..Default::default()
    }
}

fn style_chat_bubble_sent(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.primary.weak.color.into()),
        border: border::rounded(super::layout::CHAT_BUBBLE_RADIUS),
        ..Default::default()
    }
}

fn style_chat_bubble_received(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: border::rounded(super::layout::CHAT_BUBBLE_RADIUS),
        ..Default::default()
    }
}

// ── Rule style implementations ──────────────────────────

fn style_divider_rule(theme: &Theme) -> rule::Style {
    rule::Style {
        color: theme.palette().background.strongest.color.scale_alpha(0.15),
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

fn style_sidebar_divider_rule(theme: &Theme) -> rule::Style {
    rule::Style {
        color: theme.palette().background.weak.color,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

fn style_subtle_divider_rule(theme: &Theme) -> rule::Style {
    rule::Style {
        color: theme.palette().background.strongest.color.scale_alpha(0.25),
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

// ── Pick list style implementation ──────────────────────

fn style_ghost_pick_list(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let p = theme.palette();
    let background = match status {
        pick_list::Status::Hovered | pick_list::Status::Opened { .. } => {
            iced::Background::Color(p.background.weakest.color)
        }
        _ => iced::Background::Color(Color::TRANSPARENT),
    };
    pick_list::Style {
        text_color: p.background.base.text,
        placeholder_color: p.secondary.weak.color,
        handle_color: p.background.base.text,
        background,
        border: match status {
            pick_list::Status::Hovered | pick_list::Status::Opened { .. } => iced::Border {
                color: p.background.strongest.color.scale_alpha(0.15),
                width: 1.0,
                radius: RADIUS_SM.into(),
            },
            _ => iced::Border::default(),
        },
    }
}

// ── Text input style implementations ────────────────────

fn style_inline_text_input(theme: &Theme, _status: text_input::Status) -> text_input::Style {
    let p = theme.palette();
    text_input::Style {
        background: Color::TRANSPARENT.into(),
        border: iced::Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 0.0.into(),
        },
        icon: p.background.base.text.scale_alpha(0.5),
        placeholder: p.background.base.text.scale_alpha(0.4),
        value: p.background.base.text,
        selection: p.primary.base.color.scale_alpha(0.3),
    }
}

fn style_settings_text_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let p = theme.palette();
    let border_color = match status {
        text_input::Status::Focused { .. } => p.primary.base.color,
        text_input::Status::Hovered => p.background.strongest.color.scale_alpha(0.3),
        _ => p.background.strongest.color.scale_alpha(0.15),
    };
    text_input::Style {
        background: p.background.weak.color.into(),
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        icon: p.background.base.text.scale_alpha(0.5),
        placeholder: p.background.base.text.scale_alpha(0.4),
        value: p.background.base.text,
        selection: p.primary.base.color.scale_alpha(0.3),
    }
}

// ── Slider style implementation ─────────────────────────

fn style_settings_slider(theme: &Theme, status: slider::Status) -> slider::Style {
    let p = theme.palette();
    let color = match status {
        slider::Status::Active | slider::Status::Dragged => p.primary.base.color,
        slider::Status::Hovered => p.primary.strong.color,
    };
    slider::Style {
        rail: slider::Rail {
            backgrounds: (color.into(), p.background.strong.color.into()),
            width: SLIDER_RAIL_WIDTH,
            border: border::rounded(SLIDER_RAIL_WIDTH / 2.0),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle {
                radius: SLIDER_HANDLE_RADIUS,
            },
            background: color.into(),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

// ── Radio style implementation ──────────────────────────

fn style_settings_radio(theme: &Theme, status: radio::Status) -> radio::Style {
    let p = theme.palette();
    let is_selected = matches!(
        status,
        radio::Status::Active { is_selected: true } | radio::Status::Hovered { is_selected: true }
    );
    radio::Style {
        background: if is_selected {
            p.primary.base.color.into()
        } else {
            p.background.base.color.into()
        },
        dot_color: ON_AVATAR,
        border_width: if is_selected { 0.0 } else { 1.5 },
        border_color: if is_selected {
            Color::TRANSPARENT
        } else {
            p.background.strongest.color.scale_alpha(0.5)
        },
        text_color: None,
    }
}

// ── Toggler style implementation ────────────────────────

fn style_settings_toggler(theme: &Theme, status: toggler::Status) -> toggler::Style {
    let p = theme.palette();
    let background = match status {
        toggler::Status::Active { is_toggled } | toggler::Status::Hovered { is_toggled } => {
            if is_toggled {
                p.primary.base.color
            } else {
                p.background.strong.color
            }
        }
        toggler::Status::Disabled { .. } => p.background.weak.color,
    };
    toggler::Style {
        background: background.into(),
        foreground: p.background.base.color.into(),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        text_color: None,
        border_radius: None,
        padding_ratio: 0.1,
    }
}

// ── Avatar colors ───────────────────────────────────────

const AVATAR_HUES: &[f32] = &[
    260.0, // indigo
    160.0, // green
    25.0,  // red-orange
    45.0,  // amber
    290.0, // purple
    195.0, // cyan
    340.0, // pink
    130.0, // emerald
];

pub fn avatar_color(name: &str) -> Color {
    let hash: usize = name.bytes().map(|b| b as usize).sum();
    let hue = AVATAR_HUES[hash % AVATAR_HUES.len()];
    hsl_to_color(hue, 0.65, 0.55)
}

pub fn initial(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "?".to_string())
}

// ── Color utilities ─────────────────────────────────────

/// Mix two colors by ratio (0.0 = a, 1.0 = b).
pub fn mix(a: Color, b: Color, t: f32) -> Color {
    Color::from_rgba(
        a.r + (b.r - a.r) * t,
        a.g + (b.g - a.g) * t,
        a.b + (b.b - a.b) * t,
        a.a + (b.a - a.a) * t,
    )
}

pub fn hex_to_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    // Guard against short/malformed hex strings - fall back to mid-gray
    // rather than panicking on slice bounds.
    if hex.len() < 6 {
        return Color::from_rgb8(128, 128, 128);
    }
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(128);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(128);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(128);
    Color::from_rgb8(r, g, b)
}

fn hsl_to_color(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
    // h_prime = h / 60.0 where h is in [0, 360), so h_prime is in [0, 6).
    // Truncation to u32 yields 0..=5, which is the intended bucket index.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let (r1, g1, b1) = match h_prime as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = l - c / 2.0;
    Color::from_rgb(r1 + m, g1 + m, b1 + m)
}
