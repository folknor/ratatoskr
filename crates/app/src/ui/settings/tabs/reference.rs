use iced::widget::{Space, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;

pub(super) fn shortcuts_tab<'a>() -> Element<'a, SettingsMessage> {
    let sections: &[(&str, &[(&str, &str)])] = &[
        (
            "Navigation",
            &[
                ("Next thread", "j"),
                ("Previous thread", "k"),
                ("Go to Inbox", "g i"),
                ("Search", "/"),
                ("Close / dismiss", "Esc"),
            ],
        ),
        (
            "Thread",
            &[
                ("Archive", "e"),
                ("Delete", "#"),
                ("Reply", "r"),
                ("Reply All", "a"),
                ("Forward", "f"),
                ("Star / unstar", "s"),
                ("Mute thread", "m"),
                ("Mark as unread", "u"),
            ],
        ),
        ("Composing", &[("New message", "c")]),
    ];

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    for (category, items) in sections {
        let rows: Vec<RowBuilder<'_>> = items
            .iter()
            .map(|(desc, key)| {
                settings_row_container(
                    SETTINGS_ROW_HEIGHT,
                    row![
                        container(text(*desc).size(TEXT_LG).style(text::secondary))
                            .align_y(Alignment::Center)
                            .width(Length::Fill),
                        container(text(*key).size(TEXT_SM).style(text::secondary))
                            .padding(PAD_ICON_BTN)
                            .style(theme::ContainerClass::KeyBadge.style()),
                    ]
                    .align_y(Alignment::Center),
                )
            })
            .collect();

        col = col.push(section(category, rows));
    }

    col.into()
}

pub(super) fn about_tab<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Application",
        vec![
            info_row("Version", "0.1.0-dev"),
            info_row("Iced Version", "0.15.0-dev"),
            info_row("Platform", std::env::consts::OS),
            info_row("Architecture", std::env::consts::ARCH),
        ],
    ));

    col = col.push(section(
        "Updates",
        vec![action_row(
            "Software Updates",
            Some("Check for new versions"),
            None,
            ActionKind::InApp,
            SettingsMessage::CheckForUpdates,
        )],
    ));

    col = col.push(section("License", vec![static_row(
        container(
            column![
                text("Apache License 2.0").size(TEXT_LG).style(text::base),
                Space::new().height(SPACE_XS),
                text("Licensed under the Apache License, Version 2.0. You may obtain a copy of the License at:")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
                Space::new().height(SPACE_XXS),
                text("https://www.apache.org/licenses/LICENSE-2.0")
                    .size(TEXT_SM)
                    .style(text::primary),
                Space::new().height(SPACE_SM),
                text("Copyright 2024-2026 Ratatoskr contributors.")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ]
        ).padding(PAD_SETTINGS_ROW),
    )]));

    col = col.push(section(
        "Links",
        vec![action_row(
            "GitHub Repository",
            Some("folknor/ratatoskr"),
            Some(icon::globe()),
            ActionKind::Url,
            SettingsMessage::OpenGithub,
        )],
    ));

    col.into()
}
