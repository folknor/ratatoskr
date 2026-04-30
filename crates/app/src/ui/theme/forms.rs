use iced::widget::{pick_list, radio, rule, slider, text, text_input, toggler};
use iced::{Color, Theme, border};

use super::ON_AVATAR;
use crate::ui::layout::{RADIUS_SM, SLIDER_HANDLE_RADIUS, SLIDER_RAIL_WIDTH};

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
