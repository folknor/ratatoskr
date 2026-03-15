use iced::widget::{button, container, pick_list, rule, text};
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
    Theme::custom("Dark", iced::theme::Palette {
        background: hex_to_color("#0F172A"),
        text: hex_to_color("#F1F5FA"),
        primary: hex_to_color("#6266F1"),
        success: hex_to_color("#059669"),
        warning: hex_to_color("#D97706"),
        danger: hex_to_color("#DC2626"),
    })
}

pub fn light() -> Theme {
    Theme::custom("Light", iced::theme::Palette {
        background: hex_to_color("#F9FAFC"),
        text: hex_to_color("#11172A"),
        primary: hex_to_color("#4F53DE"),
        success: hex_to_color("#058059"),
        warning: hex_to_color("#D97706"),
        danger: hex_to_color("#DC2626"),
    })
}

// ── Text styles ─────────────────────────────────────────
// Built-in: text::base, text::primary, text::secondary,
//           text::success, text::warning, text::danger

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
                background: Some(p.background.strong.color.into()),
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
        let bg = if selected { p.background.strong.color } else { p.background.weaker.color };
        match status {
            button::Status::Hovered => button::Style {
                background: Some(p.background.neutral.color.into()),
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
            background: Some(p.background.weak.color.into()),
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
        background: Some(p.background.weak.color.into()),
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
        color: theme.extended_palette().background.strongest.color.scale_alpha(0.15),
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
