use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;

// ── Messages ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    SelectTab(Tab),
    // General
    ThemeChanged(String),
    DensityChanged(String),
    FontSizeChanged(String),
    ReadingPaneChanged(String),
    AccentColorSelected(usize),
    ToggleSyncStatusBar(bool),
    ToggleBlockRemoteImages(bool),
    TogglePhishingDetection(bool),
    PhishingSensitivityChanged(String),
    // About
    CheckForUpdates,
    OpenGithub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Notifications,
    Composing,
    MailRules,
    People,
    Shortcuts,
    Ai,
    About,
}

impl Tab {
    const ALL: &[Tab] = &[
        Tab::General,
        Tab::Notifications,
        Tab::Composing,
        Tab::MailRules,
        Tab::People,
        Tab::Shortcuts,
        Tab::Ai,
        Tab::About,
    ];

    fn label(self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Notifications => "Notifications",
            Tab::Composing => "Composing",
            Tab::MailRules => "Mail Rules",
            Tab::People => "People",
            Tab::Shortcuts => "Shortcuts",
            Tab::Ai => "AI",
            Tab::About => "About",
        }
    }

    fn icon(self) -> iced::widget::Text<'static> {
        match self {
            Tab::General => icon::settings(),
            Tab::Notifications => icon::bell(),
            Tab::Composing => icon::pencil(),
            Tab::MailRules => icon::filter(),
            Tab::People => icon::users(),
            Tab::Shortcuts => icon::zap(),
            Tab::Ai => icon::globe(),
            Tab::About => icon::info(),
        }
    }
}

// ── State ───────────────────────────────────────────────

pub struct SettingsState {
    pub active_tab: Tab,
    // General
    pub theme: String,
    pub density: String,
    pub font_size: String,
    pub reading_pane_position: String,
    pub accent_color_index: usize,
    pub sync_status_bar: bool,
    pub block_remote_images: bool,
    pub phishing_detection: bool,
    pub phishing_sensitivity: String,
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            active_tab: Tab::General,
            theme: "System".into(),
            density: "Default".into(),
            font_size: "Default".into(),
            reading_pane_position: "Right".into(),
            accent_color_index: 0,
            sync_status_bar: true,
            block_remote_images: false,
            phishing_detection: true,
            phishing_sensitivity: "Default".into(),
        }
    }
}

impl SettingsState {
    pub fn update(&mut self, message: SettingsMessage) {
        match message {
            SettingsMessage::SelectTab(tab) => self.active_tab = tab,
            SettingsMessage::ThemeChanged(v) => self.theme = v,
            SettingsMessage::DensityChanged(v) => self.density = v,
            SettingsMessage::FontSizeChanged(v) => self.font_size = v,
            SettingsMessage::ReadingPaneChanged(v) => self.reading_pane_position = v,
            SettingsMessage::AccentColorSelected(i) => self.accent_color_index = i,
            SettingsMessage::ToggleSyncStatusBar(v) => self.sync_status_bar = v,
            SettingsMessage::ToggleBlockRemoteImages(v) => self.block_remote_images = v,
            SettingsMessage::TogglePhishingDetection(v) => self.phishing_detection = v,
            SettingsMessage::PhishingSensitivityChanged(v) => self.phishing_sensitivity = v,
            SettingsMessage::CheckForUpdates | SettingsMessage::OpenGithub => {}
        }
    }
}

// ── View ────────────────────────────────────────────────

pub fn view(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let nav = tab_nav(state.active_tab);
    let content = match state.active_tab {
        Tab::General => general_tab(state),
        Tab::About => about_tab(),
        _ => placeholder_tab(state.active_tab),
    };

    row![
        nav,
        container(scrollable(content).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(PAD_CONTENT),
    ]
    .into()
}

// ── Tab navigation ──────────────────────────────────────

fn tab_nav(active: Tab) -> Element<'static, SettingsMessage> {
    let mut col = column![].spacing(SPACE_XXS).width(200);

    col = col.push(
        container(text("Settings").size(16).style(text::base))
            .padding(iced::Padding::from([SPACE_MD, SPACE_SM])),
    );
    col = col.push(Space::new().height(SPACE_XS));

    for tab in Tab::ALL {
        let is_active = *tab == active;
        col = col.push(
            button(
                row![
                    tab.icon().size(14).style(if is_active { text::primary } else { text::secondary }),
                    text(tab.label()).size(13).style(if is_active { text::base } else { text::secondary }),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::SelectTab(*tab))
            .padding(iced::Padding::from([SPACE_XS, SPACE_SM]))
            .style(theme::nav_button(is_active))
            .width(Length::Fill),
        );
    }

    container(scrollable(col).height(Length::Fill))
        .padding(PAD_SIDEBAR)
        .height(Length::Fill)
        .style(theme::sidebar_container)
        .into()
}

// ── General tab ─────────────────────────────────────────

fn general_tab(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(600);

    // Appearance section
    col = col.push(section("Appearance", column![
        setting_row("Theme", dropdown_display(&state.theme)),
        setting_row("Reading Pane", dropdown_display(&state.reading_pane_position)),
        setting_row("Email Density", dropdown_display(&state.density)),
        setting_row("Font Size", dropdown_display(&state.font_size)),
        accent_color_row(state.accent_color_index),
        toggle_row("Show Sync Status Bar", "Display sync progress in the status bar", state.sync_status_bar),
    ].spacing(SPACE_0)));

    // Privacy & Security section
    col = col.push(section("Privacy & Security", column![
        toggle_row("Block Remote Images", "Don't load remote images in email bodies", state.block_remote_images),
        toggle_row("Phishing Detection", "Warn about suspicious emails", state.phishing_detection),
    ].spacing(SPACE_0)));

    col.into()
}

// ── About tab ───────────────────────────────────────────

fn about_tab<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(600);

    // App info section
    col = col.push(section("Application", column![
        info_row("Version", "0.1.0-dev"),
        info_row("Iced Version", "0.15.0-dev"),
        info_row("Platform", std::env::consts::OS),
        info_row("Architecture", std::env::consts::ARCH),
    ].spacing(SPACE_0)));

    // Updates section
    col = col.push(section("Updates", column![
        container(
            row![
                column![
                    text("Software Updates").size(13).style(text::base),
                    text("Check for new versions").size(11).style(theme::text_tertiary),
                ].spacing(SPACE_XXXS),
                Space::new().width(Length::Fill),
                button(text("Check for Updates").size(12))
                    .on_press(SettingsMessage::CheckForUpdates)
                    .padding(PAD_ICON_BTN)
                    .style(button::secondary),
            ].align_y(Alignment::Center),
        ).padding(iced::Padding::from([SPACE_SM, SPACE_MD])),
    ].spacing(SPACE_0)));

    // License section
    col = col.push(section("License", column![
        container(
            column![
                text("Apache License 2.0").size(13).style(text::base),
                Space::new().height(SPACE_XS),
                text("Licensed under the Apache License, Version 2.0. You may obtain a copy of the License at:")
                    .size(11)
                    .style(theme::text_tertiary),
                Space::new().height(SPACE_XXS),
                text("https://www.apache.org/licenses/LICENSE-2.0")
                    .size(11)
                    .style(text::primary),
                Space::new().height(SPACE_SM),
                text("Copyright 2024-2026 Ratatoskr contributors.")
                    .size(11)
                    .style(theme::text_tertiary),
            ]
        ).padding(iced::Padding::from([SPACE_SM, SPACE_MD])),
    ].spacing(SPACE_0)));

    // Links section
    col = col.push(section("Links", column![
        container(
            button(
                row![
                    icon::globe().size(14).style(text::secondary),
                    column![
                        text("GitHub Repository").size(13).style(text::base),
                        text("folknor/ratatoskr").size(11).style(theme::text_tertiary),
                    ].spacing(SPACE_XXXS),
                    Space::new().width(Length::Fill),
                    icon::external_link().size(12).style(theme::text_tertiary),
                ]
                .spacing(SPACE_SM)
                .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::OpenGithub)
            .padding(iced::Padding::from([SPACE_SM, SPACE_MD]))
            .style(theme::bare_button)
            .width(Length::Fill),
        ),
    ].spacing(SPACE_0)));

    col.into()
}

// ── Placeholder tab ─────────────────────────────────────

fn placeholder_tab(tab: Tab) -> Element<'static, SettingsMessage> {
    container(
        column![
            text(tab.label()).size(16).style(theme::text_tertiary),
            text("Not yet implemented").size(12).style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXS)
        .align_x(Alignment::Center),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ── Shared setting widgets ──────────────────────────────

fn section<'a>(
    title: &'a str,
    content: impl Into<Element<'a, SettingsMessage>>,
) -> Element<'a, SettingsMessage> {
    let content = content.into();
    column![
        text(title).size(14).style(text::base),
        Space::new().height(SPACE_XS),
        container(content)
            .width(Length::Fill)
            .style(settings_section_container),
    ]
    .spacing(SPACE_0)
    .into()
}

fn setting_row<'a>(
    label: &'a str,
    control: Element<'a, SettingsMessage>,
) -> Element<'a, SettingsMessage> {
    container(
        row![
            text(label).size(13).style(text::base),
            Space::new().width(Length::Fill),
            control,
        ]
        .align_y(Alignment::Center),
    )
    .padding(iced::Padding::from([SPACE_SM, SPACE_MD]))
    .width(Length::Fill)
    .into()
}

fn toggle_row<'a>(
    label: &'a str,
    description: &'a str,
    _value: bool,
) -> Element<'a, SettingsMessage> {
    container(
        row![
            column![
                text(label).size(13).style(text::base),
                text(description).size(11).style(theme::text_tertiary),
            ]
            .spacing(SPACE_XXXS),
            Space::new().width(Length::Fill),
            // Placeholder toggle — iced has checkbox but no toggle switch.
            // Using a text indicator for now.
            text(if _value { "ON" } else { "OFF" })
                .size(11)
                .style(if _value { text::primary } else { text::secondary }),
        ]
        .align_y(Alignment::Center),
    )
    .padding(iced::Padding::from([SPACE_SM, SPACE_MD]))
    .width(Length::Fill)
    .into()
}

fn info_row<'a>(label: &'a str, value: &'a str) -> Element<'a, SettingsMessage> {
    container(
        row![
            text(label).size(13).style(theme::text_tertiary),
            Space::new().width(Length::Fill),
            text(value).size(13).style(text::base),
        ]
        .align_y(Alignment::Center),
    )
    .padding(iced::Padding::from([SPACE_SM, SPACE_MD]))
    .width(Length::Fill)
    .into()
}

fn dropdown_display<'a>(current: &'a str) -> Element<'a, SettingsMessage> {
    // Placeholder — iced has pick_list for real dropdowns.
    // Scaffolding as a styled button showing the current value.
    button(
        row![
            text(current).size(12).style(text::base),
            icon::chevron_down().size(10).style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_ICON_BTN)
    .style(button::secondary)
    .into()
}

const ACCENT_COLORS: &[iced::Color] = &[
    iced::Color::from_rgb(0.384, 0.400, 0.945), // Indigo
    iced::Color::from_rgb(0.059, 0.522, 0.780), // Blue
    iced::Color::from_rgb(0.020, 0.588, 0.412), // Green
    iced::Color::from_rgb(0.608, 0.318, 0.878), // Purple
    iced::Color::from_rgb(0.878, 0.318, 0.518), // Pink
    iced::Color::from_rgb(0.851, 0.467, 0.024), // Orange
];

fn accent_color_row(selected: usize) -> Element<'static, SettingsMessage> {
    let mut swatches = row![].spacing(SPACE_XS).align_y(Alignment::Center);
    for (i, &color) in ACCENT_COLORS.iter().enumerate() {
        let is_selected = i == selected;
        let swatch = button(
            container(
                if is_selected {
                    Element::from(icon::check().size(12).color(iced::Color::WHITE))
                } else {
                    Element::from(Space::new().width(0).height(0))
                },
            )
            .center(24),
        )
        .on_press(SettingsMessage::AccentColorSelected(i))
        .padding(0)
        .style(move |_theme: &iced::Theme, _status| button::Style {
            background: Some(color.into()),
            border: iced::Border {
                radius: 12.0.into(),
                ..Default::default()
            },
            ..Default::default()
        });
        swatches = swatches.push(swatch);
    }

    setting_row("Accent Color", swatches.into())
}

// ── Container style for settings sections ───────────────

fn settings_section_container(theme: &iced::Theme) -> container::Style {
    let p = theme.extended_palette();
    container::Style {
        background: Some(p.background.weakest.color.into()),
        border: iced::Border {
            color: p.background.strongest.color.scale_alpha(0.1),
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    }
}
