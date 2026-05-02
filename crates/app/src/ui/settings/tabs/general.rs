use iced::widget::{Space, column, text};
use iced::Length;

use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::widgets;

pub(super) fn general_tab(state: &Settings) -> iced::Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Appearance",
        vec![
            setting_row(
                "Theme",
                widgets::select(
                    &["System", "Light", "Dark", "Theme"],
                    &state.theme,
                    state.open_select == Some(SelectField::Theme),
                    SettingsMessage::ToggleSelect(SelectField::Theme),
                    SettingsMessage::ThemeChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::Theme),
            ),
            setting_row(
                "Email Density",
                widgets::select(
                    &["Compact", "Default", "Spacious"],
                    &state.density,
                    state.open_select == Some(SelectField::Density),
                    SettingsMessage::ToggleSelect(SelectField::Density),
                    SettingsMessage::DensityChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::Density),
            ),
            setting_row(
                "Font Size",
                widgets::select(
                    &["Small", "Default", "Large", "XLarge"],
                    &state.font_size,
                    state.open_select == Some(SelectField::FontSize),
                    SettingsMessage::ToggleSelect(SelectField::FontSize),
                    SettingsMessage::FontSizeChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::FontSize),
            ),
            setting_row(
                "Email Body Background",
                widgets::select(
                    &["Always White", "Match Theme", "Auto"],
                    state.email_body_background.label(),
                    state.open_select == Some(SelectField::EmailBodyBg),
                    SettingsMessage::ToggleSelect(SelectField::EmailBodyBg),
                    SettingsMessage::EmailBodyBgChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::EmailBodyBg),
            ),
            slider_row(
                "Scale",
                None,
                1.0..=4.0,
                state.scale_preview.unwrap_or(state.scale),
                1.0,
                0.125,
                SettingsMessage::ScaleDragged,
                Some(SettingsMessage::ScaleReleased),
            ),
            setting_row_with_description(
                "Message Dates",
                Some("Relative offset shows \"2h ago\"; Absolute shows the calendar date."),
                widgets::select(
                    &["Relative Offset", "Absolute"],
                    match state.date_display {
                        crate::db::DateDisplay::RelativeOffset => "Relative Offset",
                        crate::db::DateDisplay::Absolute => "Absolute",
                    },
                    state.open_select == Some(SelectField::DateDisplay),
                    SettingsMessage::ToggleSelect(SelectField::DateDisplay),
                    SettingsMessage::DateDisplayChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::DateDisplay),
            ),
            toggle_row(
                "Show status bar",
                "Show sync progress, action confirmations, warnings, and the Out-of-Office indicator at the bottom of the window.",
                state.sync_status_bar,
                SettingsMessage::ToggleSyncStatusBar,
            ),
        ],
    ));

    col = col.push(section(
        "Reading Pane",
        radio_group(
            &[
                ("Right", "Right"),
                ("Bottom", "Bottom"),
                ("Hidden", "Hidden"),
            ],
            Some(state.reading_pane_position.as_str()),
            |v| SettingsMessage::ReadingPaneChanged(v.to_string()),
        ),
    ));

    let privacy_help_id = "privacy-security";
    let privacy_help_visible = state.hovered_help.as_deref() == Some(privacy_help_id);
    col = col.push(section_with_help("Privacy & Security", SectionHelp {
        id: privacy_help_id,
        content: column![
            text("Remote images can be used to track when you open an email. Blocking them prevents this but some emails may not display correctly.")
                .size(TEXT_SM)
                .style(theme::TextClass::OnPrimary.style()),
            Space::new().height(SPACE_XS),
            text("Phishing detection analyzes incoming emails for suspicious links, sender spoofing, and social engineering patterns.")
                .size(TEXT_SM)
                .style(theme::TextClass::OnPrimary.style()),
        ]
        .into(),
        visible: privacy_help_visible,
    }, vec![
        toggle_row("Block Remote Images", "Don't load remote images in email bodies", state.block_remote_images, SettingsMessage::ToggleBlockRemoteImages),
        toggle_row("Phishing Detection", "Warn about suspicious emails", state.phishing_detection, SettingsMessage::TogglePhishingDetection),
    ]));

    col.into()
}
