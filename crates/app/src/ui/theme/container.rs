use iced::widget::container;
use iced::{Color, Theme, border};

use super::{EMAIL_BODY_BG_PREF, ON_AVATAR};
use crate::ui::layout::{
    CHAT_BUBBLE_RADIUS, RADIO_CIRCLE_SIZE, RADIUS_LG, RADIUS_MD, RADIUS_SM,
};
use crate::ui::settings::types::EmailBodyBackground;

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
    /// Radio circle outer ring - selected state (primary color).
    RadioCircleSelected,
    /// Radio circle outer ring - unselected state (muted border).
    RadioCircleUnselected,
    /// Radio circle inner filled disk (rendered only when selected).
    RadioCircleInner,
    /// Modal `Modal` surface card. Window-like opaque background, generous
    /// rounding, and a soft drop shadow so the dialog reads as a discrete
    /// surface above the dimmed backdrop. Used by the `alert_dialog` /
    /// `form_dialog` primitives in `ui/dialog.rs`.
    DialogCard,
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
            Self::DialogCard => style_dialog_card_container,
            Self::RadioCircleSelected => style_radio_circle_selected,
            Self::RadioCircleUnselected => style_radio_circle_unselected,
            Self::RadioCircleInner => style_radio_circle_inner,
        }
    }
}

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

fn style_dialog_card_container(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.2),
            width: 1.0,
            radius: RADIUS_LG.into(),
        },
        shadow: iced::Shadow {
            color: Color::BLACK.scale_alpha(0.35),
            offset: iced::Vector::new(0.0, 4.0),
            blur_radius: 24.0,
        },
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
        border: border::rounded(CHAT_BUBBLE_RADIUS),
        ..Default::default()
    }
}

fn style_chat_bubble_received(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: border::rounded(CHAT_BUBBLE_RADIUS),
        ..Default::default()
    }
}

fn style_radio_circle_selected(theme: &Theme) -> container::Style {
    container::Style {
        background: Some(theme.palette().primary.base.color.into()),
        border: border::rounded(RADIO_CIRCLE_SIZE / 2.0),
        ..Default::default()
    }
}

fn style_radio_circle_unselected(theme: &Theme) -> container::Style {
    let p = theme.palette();
    container::Style {
        background: Some(p.background.base.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.5),
            width: 1.5,
            radius: (RADIO_CIRCLE_SIZE / 2.0).into(),
        },
        ..Default::default()
    }
}

fn style_radio_circle_inner(_theme: &Theme) -> container::Style {
    let inner_size = RADIO_CIRCLE_SIZE * 0.3;
    container::Style {
        background: Some(ON_AVATAR.into()),
        border: border::rounded(inner_size / 2.0),
        ..Default::default()
    }
}
