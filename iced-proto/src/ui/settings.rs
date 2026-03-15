use iced::widget::{button, column, container, row, scrollable, text, toggler, Space};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

// ── Messages ────────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum SettingsMessage {
    Close,
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
    ToggleSelect(SelectField),
    // About
    CheckForUpdates,
    OpenGithub,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectField {
    Theme,
    ReadingPane,
    Density,
    FontSize,
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
    pub open_select: Option<SelectField>,
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
            open_select: None,
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
            SettingsMessage::Close => {}
            SettingsMessage::SelectTab(tab) => self.active_tab = tab,
            SettingsMessage::ToggleSelect(field) => {
                self.open_select = if self.open_select == Some(field) {
                    None
                } else {
                    Some(field)
                };
            }
            SettingsMessage::ThemeChanged(v) => { self.theme = v; self.open_select = None; }
            SettingsMessage::DensityChanged(v) => { self.density = v; self.open_select = None; }
            SettingsMessage::FontSizeChanged(v) => { self.font_size = v; self.open_select = None; }
            SettingsMessage::ReadingPaneChanged(v) => { self.reading_pane_position = v; self.open_select = None; }
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
    let mut col = column![].spacing(SPACE_XXS).width(SETTINGS_NAV_WIDTH);

    col = col.push(widgets::nav_button(
        Some(icon::arrow_left()),
        "Settings",
        false,
        widgets::NavSize::Regular,
        None,
        SettingsMessage::Close,
    ));
    col = col.push(Space::new().height(SPACE_XS));

    for tab in Tab::ALL {
        let is_active = *tab == active;
        col = col.push(widgets::nav_button(
            Some(tab.icon()),
            tab.label(),
            is_active,
            widgets::NavSize::Regular,
            None,
            SettingsMessage::SelectTab(*tab),
        ));
    }

    container(scrollable(col).height(Length::Fill))
        .padding(PAD_SIDEBAR)
        .height(Length::Fill)
        .style(theme::sidebar_container)
        .into()
}

// ── General tab ─────────────────────────────────────────

fn general_tab(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Appearance", vec![
        setting_row("Theme", widgets::select(
            &["System", "Light", "Dark"], &state.theme,
            state.open_select == Some(SelectField::Theme),
            SettingsMessage::ToggleSelect(SelectField::Theme),
            SettingsMessage::ThemeChanged,
        )),
        setting_row("Reading Pane", widgets::select(
            &["Right", "Bottom", "Hidden"], &state.reading_pane_position,
            state.open_select == Some(SelectField::ReadingPane),
            SettingsMessage::ToggleSelect(SelectField::ReadingPane),
            SettingsMessage::ReadingPaneChanged,
        )),
        setting_row("Email Density", widgets::select(
            &["Compact", "Default", "Spacious"], &state.density,
            state.open_select == Some(SelectField::Density),
            SettingsMessage::ToggleSelect(SelectField::Density),
            SettingsMessage::DensityChanged,
        )),
        setting_row("Font Size", widgets::select(
            &["Small", "Default", "Large", "XLarge"], &state.font_size,
            state.open_select == Some(SelectField::FontSize),
            SettingsMessage::ToggleSelect(SelectField::FontSize),
            SettingsMessage::FontSizeChanged,
        )),
        accent_color_row(state.accent_color_index),
        toggle_row("Show Sync Status Bar", "Display sync progress in the status bar", state.sync_status_bar, SettingsMessage::ToggleSyncStatusBar),
    ]));

    col = col.push(section("Privacy & Security", vec![
        toggle_row("Block Remote Images", "Don't load remote images in email bodies", state.block_remote_images, SettingsMessage::ToggleBlockRemoteImages),
        toggle_row("Phishing Detection", "Warn about suspicious emails", state.phishing_detection, SettingsMessage::TogglePhishingDetection),
    ]));

    col.into()
}

// ── About tab ───────────────────────────────────────────

fn about_tab<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Application", vec![
        info_row("Version", "0.1.0-dev"),
        info_row("Iced Version", "0.15.0-dev"),
        info_row("Platform", std::env::consts::OS),
        info_row("Architecture", std::env::consts::ARCH),
    ]));

    col = col.push(section("Updates", vec![
        container(
            row![
                column![
                    text("Software Updates").size(TEXT_LG).style(text::base),
                    text("Check for new versions").size(TEXT_SM).style(theme::text_tertiary),
                ].spacing(SPACE_XXXS),
                Space::new().width(Length::Fill),
                button(text("Check for Updates").size(TEXT_MD))
                    .on_press(SettingsMessage::CheckForUpdates)
                    .padding(PAD_ICON_BTN)
                    .style(theme::secondary_button),
            ].align_y(Alignment::Center),
        ).padding(PAD_SETTINGS_ROW).into(),
    ]));

    col = col.push(section("License", vec![
        container(
            column![
                text("Apache License 2.0").size(TEXT_LG).style(text::base),
                Space::new().height(SPACE_XS),
                text("Licensed under the Apache License, Version 2.0. You may obtain a copy of the License at:")
                    .size(TEXT_SM)
                    .style(theme::text_tertiary),
                Space::new().height(SPACE_XXS),
                text("https://www.apache.org/licenses/LICENSE-2.0")
                    .size(TEXT_SM)
                    .style(text::primary),
                Space::new().height(SPACE_SM),
                text("Copyright 2024-2026 Ratatoskr contributors.")
                    .size(TEXT_SM)
                    .style(theme::text_tertiary),
            ]
        ).padding(PAD_SETTINGS_ROW).into(),
    ]));

    col = col.push(section("Links", vec![
        button(
            row![
                container(icon::globe().size(ICON_XL).style(text::secondary))
                    .align_y(Alignment::Center),
                column![
                    text("GitHub Repository").size(TEXT_LG).style(text::base),
                    text("folknor/ratatoskr").size(TEXT_SM).style(theme::text_tertiary),
                ].spacing(SPACE_XXXS),
                Space::new().width(Length::Fill),
                container(icon::external_link().size(ICON_MD).style(theme::text_tertiary))
                    .align_y(Alignment::Center),
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .on_press(SettingsMessage::OpenGithub)
        .padding(PAD_SETTINGS_ROW)
        .style(theme::bare_button)
        .width(Length::Fill)
        .into(),
    ]));

    col.into()
}

// ── Placeholder tab ─────────────────────────────────────

fn placeholder_tab(tab: Tab) -> Element<'static, SettingsMessage> {
    container(
        column![
            text(tab.label()).size(TEXT_TITLE).style(theme::text_tertiary),
            text("Not yet implemented").size(TEXT_MD).style(theme::text_tertiary),
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
    items: Vec<Element<'a, SettingsMessage>>,
) -> Element<'a, SettingsMessage> {
    let mut col = column![].width(Length::Fill);
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            col = col.push(iced::widget::rule::horizontal(1).style(theme::subtle_divider_rule));
        }
        col = col.push(item);
    }
    column![
        text(title).size(TEXT_XL).style(text::base),
        Space::new().height(SPACE_XS),
        container(col)
            .width(Length::Fill)
            .style(theme::settings_section_container),
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
            container(text(label).size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            control,
        ]
        .align_y(Alignment::Center),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn toggle_row<'a>(
    label: &'a str,
    description: &'a str,
    value: bool,
    on_toggle: fn(bool) -> SettingsMessage,
) -> Element<'a, SettingsMessage> {
    container(
        row![
            column![
                text(label).size(TEXT_LG).style(text::base),
                text(description).size(TEXT_SM).style(theme::text_tertiary),
            ]
            .spacing(SPACE_XXXS),
            Space::new().width(Length::Fill),
            toggler(value).size(TEXT_HEADING).on_toggle(on_toggle),
        ]
        .align_y(Alignment::Center),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn info_row<'a>(label: &'a str, value: &'a str) -> Element<'a, SettingsMessage> {
    container(
        row![
            container(text(label).size(TEXT_LG).style(theme::text_tertiary))
                .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            container(text(value).size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center),
        ]
        .align_y(Alignment::Center),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}


fn accent_color_row(selected: usize) -> Element<'static, SettingsMessage> {
    let mut swatches = row![].spacing(SPACE_XS).align_y(Alignment::Center);
    for (i, &color) in theme::ACCENT_COLORS.iter().enumerate() {
        let is_selected = i == selected;
        let swatch = button(
            container(
                if is_selected {
                    Element::from(icon::check().size(ICON_MD).color(theme::ON_AVATAR))
                } else {
                    Element::from(Space::new().width(0).height(0))
                },
            )
            .center(SWATCH_SIZE),
        )
        .on_press(SettingsMessage::AccentColorSelected(i))
        .padding(0)
        .style(theme::swatch_button(color));
        swatches = swatches.push(swatch);
    }

    setting_row("Accent Color", swatches.into())
}
