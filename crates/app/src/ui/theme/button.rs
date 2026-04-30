use iced::widget::{button, container};
use iced::{Color, Theme, border};

use super::color::mix;
use crate::ui::layout::{RADIUS_LG, RADIUS_MD, RADIUS_SM, STARRED_BG_ALPHA};

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

/// Position of a row within a settings section, controlling which corners
/// match the section's outer `RADIUS_LG` rounding vs. the inner `RADIUS_SM`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowPosition {
    Top,
    Middle,
    Bottom,
    Only,
}

impl RowPosition {
    pub fn radii(self) -> border::Radius {
        let outer = RADIUS_LG;
        let inner = RADIUS_SM;
        match self {
            Self::Top => border::Radius::default()
                .top_left(outer)
                .top_right(outer)
                .bottom_left(inner)
                .bottom_right(inner),
            Self::Bottom => border::Radius::default()
                .top_left(inner)
                .top_right(inner)
                .bottom_left(outer)
                .bottom_right(outer),
            Self::Only => border::Radius::new(outer),
            Self::Middle => border::Radius::new(inner),
        }
    }
}

/// Container style for a search/filter input wrapper. The outer container
/// owns the bg + border so the search icon, text_input (Inline style), and
/// clear button all read as one unified field. When `focused` is true the
/// border switches to the primary color to indicate the field has focus.
pub fn style_filter_container(theme: &Theme, focused: bool) -> container::Style {
    let p = theme.palette();
    let border_color = if focused {
        p.primary.base.color
    } else {
        p.background.strongest.color.scale_alpha(0.35)
    };
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: border_color,
            width: 1.0,
            radius: RADIUS_SM.into(),
        },
        ..Default::default()
    }
}

/// Container style for a recessed scrollable list panel inside a settings
/// section (e.g. the contacts / groups list in the People tab). Uses a
/// slightly inset background so it reads as a different surface from the
/// section it sits in.
pub fn style_recessed_list_panel(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.35),
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

/// "Pill" button style for free-floating contact / group cards inside a
/// recessed panel. Visible border at rest; brighter background on hover.
pub fn style_pill_card_button(theme: &Theme, status: button::Status) -> button::Style {
    let p = theme.palette();
    let (bg, border_alpha) = match status {
        button::Status::Hovered => (p.background.weakest.color, 0.25),
        _ => (p.background.base.color, 0.15),
    };
    button::Style {
        background: Some(bg.into()),
        text_color: p.background.base.text,
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(border_alpha),
            width: 1.0,
            radius: RADIUS_MD.into(),
        },
        ..Default::default()
    }
}

/// Position-aware variant of `style_action_button` for settings rows. Outer
/// corners use `RADIUS_LG` (matching the section container); inner corners
/// stay `RADIUS_SM` so adjacent rows share a sharper seam.
pub fn style_settings_row_button(
    theme: &Theme,
    status: button::Status,
    position: RowPosition,
) -> button::Style {
    let p = theme.palette();
    let radius = position.radii();
    match status {
        button::Status::Hovered => button::Style {
            background: Some(p.background.weakest.color.into()),
            text_color: p.background.base.text,
            border: iced::Border {
                radius,
                ..Default::default()
            },
            ..Default::default()
        },
        _ => button::Style {
            text_color: p.secondary.base.color,
            border: iced::Border {
                radius,
                ..Default::default()
            },
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
