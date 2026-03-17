use iced::widget::{button, container, pick_list, radio, rule, slider, text, text_input, toggler};
use iced::{border, Color, Theme};
use serde::Deserialize;

use super::layout::*;

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
    let palette = iced::theme::Palette {
        background: hex_to_color(&file.colors.background),
        text: hex_to_color(&file.colors.text),
        primary: hex_to_color(&file.colors.primary),
        success: hex_to_color(&file.colors.success),
        warning: hex_to_color(&file.colors.warning),
        danger: hex_to_color(&file.colors.danger),
    };
    Ok(Theme::custom(file.name.unwrap_or_else(|| "Custom".into()), palette))
}

// ── Built-in dark/light seeds ───────────────────────────

pub fn dark() -> Theme {
    dark_with_accent(hex_to_color("#6266F1"))
}

pub fn dark_with_accent(accent: Color) -> Theme {
    Theme::custom("Dark", iced::theme::Palette {
        background: hex_to_color("#0F172A"),
        text: hex_to_color("#F1F5FA"),
        primary: accent,
        success: hex_to_color("#059669"),
        warning: hex_to_color("#D97706"),
        danger: hex_to_color("#DC2626"),
    })
}

pub fn light() -> Theme {
    light_with_accent(hex_to_color("#4F53DE"))
}

pub fn light_with_accent(accent: Color) -> Theme {
    Theme::custom("Light", iced::theme::Palette {
        background: hex_to_color("#F9FAFC"),
        text: hex_to_color("#11172A"),
        primary: accent,
        success: hex_to_color("#058059"),
        warning: hex_to_color("#D97706"),
        danger: hex_to_color("#DC2626"),
    })
}

// ── Text styles ─────────────────────────────────────────
// Built-in: text::base, text::primary, text::secondary,
//           text::success, text::warning, text::danger

pub fn text_accent(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().primary.base.color),
    }
}

pub fn text_tertiary(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().background.strongest.text.scale_alpha(0.5)),
    }
}

pub fn text_muted(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().background.base.text.scale_alpha(0.6)),
    }
}

/// Text on a primary-colored background (e.g. help tooltips).
pub fn text_on_primary(theme: &Theme) -> text::Style {
    text::Style {
        color: Some(theme.extended_palette().primary.base.text),
    }
}

// ── Button styles ───────────────────────────────────────
// Built-in: button::primary, button::secondary, button::text,
//           button::danger, button::subtle

pub fn primary_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::primary(theme, status);
    style.border = border::rounded(RADIUS_LG);
    style
}

pub fn secondary_button(theme: &Theme, status: button::Status) -> button::Style {
    let mut style = button::secondary(theme, status);
    style.border = border::rounded(RADIUS_LG);
    style
}

pub fn dropdown_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.extended_palette();
        match status {
            button::Status::Hovered => button::Style {
                background: Some(p.background.weakest.color.into()),
                text_color: p.background.base.text,
                border: border::rounded(RADIUS_SM),
                ..Default::default()
            },
            _ => button::Style {
                background: None,
                text_color: if selected { p.background.base.text } else { p.secondary.base.color },
                border: border::rounded(RADIUS_SM),
                ..Default::default()
            },
        }
    }
}

pub fn nav_button(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.extended_palette();
        let inactive_text = p.background.base.text.scale_alpha(0.6);
        match status {
            button::Status::Hovered => button::Style {
                background: Some(p.background.weak.color.into()),
                text_color: if active { p.primary.base.color } else { p.background.base.text },
                border: border::rounded(RADIUS_SM),
                ..Default::default()
            },
            _ => button::Style {
                background: if active { Some(p.background.strong.color.into()) } else { None },
                text_color: if active { p.primary.base.color } else { inactive_text },
                border: border::rounded(RADIUS_SM),
                ..Default::default()
            },
        }
    }
}

pub fn thread_card_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.extended_palette();
        let bg = if selected { p.background.weakest.color } else { p.background.base.color };
        match status {
            button::Status::Hovered => button::Style {
                background: Some(p.background.weakest.color.into()),
                text_color: p.background.base.text,
                ..Default::default()
            },
            _ => button::Style {
                background: Some(bg.into()),
                text_color: p.background.base.text,
                ..Default::default()
            },
        }
    }
}

pub fn ghost_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
    match status {
        button::Status::Hovered => button::Style {
            text_color: p.background.base.text,
            border: iced::Border {
                color: p.background.strongest.color.scale_alpha(0.15),
                width: 1.0,
                radius: RADIUS_SM.into(),
            },
            ..Default::default()
        },
        _ => button::Style {
            text_color: p.background.base.text,
            ..Default::default()
        },
    }
}

pub fn bare_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weakest.color.into()),
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

/// Icon-only button that sits inside a hovered row or on a `weakest` background.
/// Hover is `weaker` (one step above `weakest`) so it's visible on both
/// `base` (unhovered row) and `weakest` (hovered row / content area).
pub fn bare_icon_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
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

pub fn action_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.extended_palette();
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

// ── Container styles ────────────────────────────────────
// Built-in: container::transparent, container::bordered_box,
//           container::dark, container::rounded_box

pub fn base_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.base.color.into()),
        ..Default::default()
    }
}

pub fn divider_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.strong.color.into()),
        ..Default::default()
    }
}

pub fn divider_hover_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.strongest.color.into()),
        ..Default::default()
    }
}

pub fn content_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.weakest.color.into()),
        ..Default::default()
    }
}

pub fn sidebar_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.weaker.color.into()),
        ..Default::default()
    }
}

pub fn surface_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

pub fn elevated_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

pub fn badge_container(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.extended_palette().background.weak.color.into()),
        border: iced::Border {
            radius: RADIUS_LG.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

pub fn message_card_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

pub fn action_bar_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

pub fn floating_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

pub fn settings_section_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

pub fn select_menu_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

// ── Rule styles ─────────────────────────────────────────

pub fn divider_rule(theme: &Theme) -> rule::Style {
    rule::Style {
        color: theme.extended_palette().background.strongest.color.scale_alpha(0.15),
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

pub fn sidebar_divider_rule(theme: &Theme) -> rule::Style {
    rule::Style {
        color: theme.extended_palette().background.weak.color,
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

pub fn subtle_divider_rule(theme: &Theme) -> rule::Style {
    rule::Style {
        color: theme.extended_palette().background.strongest.color.scale_alpha(0.25),
        radius: 0.0.into(),
        fill_mode: rule::FillMode::Full,
        snap: true,
    }
}

// ── Pick list style ─────────────────────────────────────

pub fn ghost_pick_list(theme: &Theme, status: pick_list::Status) -> pick_list::Style {
    let p = theme.extended_palette();
    pick_list::Style {
        text_color: p.background.base.text,
        placeholder_color: p.secondary.weak.color,
        handle_color: p.background.base.text,
        background: iced::Background::Color(Color::TRANSPARENT),
        border: match status {
            pick_list::Status::Hovered => iced::Border {
                color: p.background.strongest.color.scale_alpha(0.15),
                width: 1.0,
                radius: RADIUS_SM.into(),
            },
            _ => iced::Border::default(),
        },
    }
}

// ── Text input style ────────────────────────────────────

/// Text input that looks like plain text. No background or border in any state.
pub fn inline_text_input(theme: &Theme, _status: text_input::Status) -> text_input::Style {
    let p = theme.extended_palette();
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

pub fn settings_text_input(theme: &Theme, status: text_input::Status) -> text_input::Style {
    let p = theme.extended_palette();
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

// ── Chip button style ────────────────────────────────────
// Used for toggleable category/tag selector pills.

pub fn chip_button(active: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let p = theme.extended_palette();
        match status {
            button::Status::Hovered => button::Style {
                background: Some(if active {
                    p.primary.base.color.scale_alpha(0.25).into()
                } else {
                    p.background.weakest.color.into()
                }),
                text_color: if active { p.primary.base.color } else { p.background.base.text },
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
            },
            _ => button::Style {
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
            },
        }
    }
}

// ── Key badge container ──────────────────────────────────
// Used for shortcut key display in the Shortcuts tab.

pub fn key_badge_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

// ── Dragging row style ───────────────────────────────────

pub fn dragging_row_container(theme: &Theme) -> container::Style {
    let p = theme.extended_palette();
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

// ── Slider style ─────────────────────────────────────────

pub fn settings_slider(theme: &Theme, status: slider::Status) -> slider::Style {
    let p = theme.extended_palette();
    let color = match status {
        slider::Status::Active => p.primary.base.color,
        slider::Status::Hovered => p.primary.strong.color,
        slider::Status::Dragged => p.primary.base.color,
    };
    slider::Style {
        rail: slider::Rail {
            backgrounds: (color.into(), p.background.strong.color.into()),
            width: SLIDER_RAIL_WIDTH,
            border: border::rounded(SLIDER_RAIL_WIDTH / 2.0),
        },
        handle: slider::Handle {
            shape: slider::HandleShape::Circle { radius: SLIDER_HANDLE_RADIUS },
            background: color.into(),
            border_width: 0.0,
            border_color: Color::TRANSPARENT,
        },
    }
}

// ── Radio style ──────────────────────────────────────────

pub fn settings_radio(theme: &Theme, status: radio::Status) -> radio::Style {
    let p = theme.extended_palette();
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
        text_color: None, // We handle text ourselves
    }
}

// ── Toggler style ───────────────────────────────────────

/// Toggler with a pill that always matches the section row background (`base`),
/// instead of the default which uses `primary.base.text` when ON — that color
/// changes with the accent color and clashes with the section background.
pub fn settings_toggler(theme: &Theme, status: toggler::Status) -> toggler::Style {
    let p = theme.extended_palette();
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

// ── Swatch button style ─────────────────────────────────

pub fn swatch_button(color: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, _status| button::Style {
        background: Some(color.into()),
        border: iced::Border {
            radius: RADIUS_ROUND.into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

// ── Accent colors ───────────────────────────────────────

pub const ACCENT_COLORS: &[Color] = &[
    Color::from_rgb(0.384, 0.400, 0.945), // Indigo
    Color::from_rgb(0.059, 0.522, 0.780), // Blue
    Color::from_rgb(0.020, 0.588, 0.412), // Green
    Color::from_rgb(0.608, 0.318, 0.878), // Purple
    Color::from_rgb(0.878, 0.318, 0.518), // Pink
    Color::from_rgb(0.851, 0.467, 0.024), // Orange
];

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

fn hex_to_color(hex: &str) -> Color {
    let hex = hex.trim_start_matches('#');
    let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(0);
    Color::from_rgb8(r, g, b)
}

fn hsl_to_color(h: f32, s: f32, l: f32) -> Color {
    let c = (1.0 - (2.0 * l - 1.0).abs()) * s;
    let h_prime = h / 60.0;
    let x = c * (1.0 - (h_prime % 2.0 - 1.0).abs());
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
