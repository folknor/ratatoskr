use iced::widget::{button, column, container, row, scrollable, text, text_input, toggler, Space};
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
    // Composing
    ToggleSendAndArchive(bool),
    UndoDelayChanged(String),
    DefaultReplyChanged(String),
    MarkAsReadChanged(String),
    // Notifications
    ToggleNotifications(bool),
    ToggleSmartNotifications(bool),
    ToggleNotifyCategory(String),
    VipEmailChanged(String),
    AddVipSender,
    RemoveVipSender(String),
    // AI
    AiProviderChanged(String),
    AiModelChanged(String),
    ToggleAiEnabled(bool),
    ToggleAiAutoCategorize(bool),
    ToggleAiAutoSummarize(bool),
    ToggleAiAutoDraft(bool),
    ToggleAiWritingStyle(bool),
    ToggleAiAutoArchiveUpdates(bool),
    ToggleAiAutoArchivePromotions(bool),
    ToggleAiAutoArchiveSocial(bool),
    ToggleAiAutoArchiveNewsletters(bool),
    AiApiKeyChanged(String),
    OllamaUrlChanged(String),
    OllamaModelChanged(String),
    SaveAiSettings,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectField {
    Theme,
    ReadingPane,
    Density,
    FontSize,
    UndoDelay,
    DefaultReply,
    MarkAsRead,
    AiProvider,
    AiModel,
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
    // Composing
    pub undo_delay: String,
    pub send_and_archive: bool,
    pub default_reply_mode: String,
    pub mark_as_read: String,
    // Notifications
    pub notifications_enabled: bool,
    pub smart_notifications: bool,
    pub notify_categories: Vec<String>,
    pub vip_email_input: String,
    pub vip_senders: Vec<String>,
    // AI
    pub ai_provider: String,
    pub ai_api_key: String,
    pub ai_model: String,
    pub ai_ollama_url: String,
    pub ai_ollama_model: String,
    pub ai_enabled: bool,
    pub ai_auto_categorize: bool,
    pub ai_auto_summarize: bool,
    pub ai_auto_draft: bool,
    pub ai_writing_style: bool,
    pub ai_auto_archive_updates: bool,
    pub ai_auto_archive_promotions: bool,
    pub ai_auto_archive_social: bool,
    pub ai_auto_archive_newsletters: bool,
    pub ai_key_saved: bool,
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
            undo_delay: "5 seconds".into(),
            send_and_archive: false,
            default_reply_mode: "Reply".into(),
            mark_as_read: "After 2 Seconds".into(),
            notifications_enabled: true,
            smart_notifications: true,
            notify_categories: vec!["Primary".into()],
            vip_email_input: String::new(),
            vip_senders: Vec::new(),
            ai_provider: "Claude".into(),
            ai_api_key: String::new(),
            ai_model: "claude-sonnet-4-6".into(),
            ai_ollama_url: "http://localhost:11434".into(),
            ai_ollama_model: "llama3.2".into(),
            ai_enabled: true,
            ai_auto_categorize: true,
            ai_auto_summarize: true,
            ai_auto_draft: true,
            ai_writing_style: true,
            ai_auto_archive_updates: false,
            ai_auto_archive_promotions: false,
            ai_auto_archive_social: false,
            ai_auto_archive_newsletters: false,
            ai_key_saved: false,
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
            // General
            SettingsMessage::ThemeChanged(v) => { self.theme = v; self.open_select = None; }
            SettingsMessage::DensityChanged(v) => { self.density = v; self.open_select = None; }
            SettingsMessage::FontSizeChanged(v) => { self.font_size = v; self.open_select = None; }
            SettingsMessage::ReadingPaneChanged(v) => { self.reading_pane_position = v; self.open_select = None; }
            SettingsMessage::AccentColorSelected(i) => self.accent_color_index = i,
            SettingsMessage::ToggleSyncStatusBar(v) => self.sync_status_bar = v,
            SettingsMessage::ToggleBlockRemoteImages(v) => self.block_remote_images = v,
            SettingsMessage::TogglePhishingDetection(v) => self.phishing_detection = v,
            SettingsMessage::PhishingSensitivityChanged(v) => self.phishing_sensitivity = v,
            // About
            SettingsMessage::CheckForUpdates | SettingsMessage::OpenGithub => {}
            // Composing
            SettingsMessage::ToggleSendAndArchive(v) => self.send_and_archive = v,
            SettingsMessage::UndoDelayChanged(v) => { self.undo_delay = v; self.open_select = None; }
            SettingsMessage::DefaultReplyChanged(v) => { self.default_reply_mode = v; self.open_select = None; }
            SettingsMessage::MarkAsReadChanged(v) => { self.mark_as_read = v; self.open_select = None; }
            // Notifications
            SettingsMessage::ToggleNotifications(v) => self.notifications_enabled = v,
            SettingsMessage::ToggleSmartNotifications(v) => self.smart_notifications = v,
            SettingsMessage::ToggleNotifyCategory(cat) => {
                if let Some(pos) = self.notify_categories.iter().position(|c| c == &cat) {
                    self.notify_categories.remove(pos);
                } else {
                    self.notify_categories.push(cat);
                }
            }
            SettingsMessage::VipEmailChanged(v) => self.vip_email_input = v,
            SettingsMessage::AddVipSender => {
                let email = self.vip_email_input.trim().to_string();
                if !email.is_empty() && !self.vip_senders.contains(&email) {
                    self.vip_senders.push(email);
                    self.vip_email_input.clear();
                }
            }
            SettingsMessage::RemoveVipSender(email) => {
                self.vip_senders.retain(|e| e != &email);
            }
            // AI
            SettingsMessage::AiProviderChanged(v) => { self.ai_provider = v; self.open_select = None; }
            SettingsMessage::AiModelChanged(v) => { self.ai_model = v; self.open_select = None; }
            SettingsMessage::ToggleAiEnabled(v) => self.ai_enabled = v,
            SettingsMessage::ToggleAiAutoCategorize(v) => self.ai_auto_categorize = v,
            SettingsMessage::ToggleAiAutoSummarize(v) => self.ai_auto_summarize = v,
            SettingsMessage::ToggleAiAutoDraft(v) => self.ai_auto_draft = v,
            SettingsMessage::ToggleAiWritingStyle(v) => self.ai_writing_style = v,
            SettingsMessage::ToggleAiAutoArchiveUpdates(v) => self.ai_auto_archive_updates = v,
            SettingsMessage::ToggleAiAutoArchivePromotions(v) => self.ai_auto_archive_promotions = v,
            SettingsMessage::ToggleAiAutoArchiveSocial(v) => self.ai_auto_archive_social = v,
            SettingsMessage::ToggleAiAutoArchiveNewsletters(v) => self.ai_auto_archive_newsletters = v,
            SettingsMessage::AiApiKeyChanged(v) => self.ai_api_key = v,
            SettingsMessage::OllamaUrlChanged(v) => self.ai_ollama_url = v,
            SettingsMessage::OllamaModelChanged(v) => self.ai_ollama_model = v,
            SettingsMessage::SaveAiSettings => self.ai_key_saved = true,
        }
    }
}

// ── View ────────────────────────────────────────────────

pub fn view(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let nav = tab_nav(state.active_tab);
    let content = match state.active_tab {
        Tab::General => general_tab(state),
        Tab::Notifications => notifications_tab(state),
        Tab::Composing => composing_tab(state),
        Tab::MailRules => mail_rules_tab(),
        Tab::People => people_tab(),
        Tab::Shortcuts => shortcuts_tab(),
        Tab::Ai => ai_tab(state),
        Tab::About => about_tab(),
    };

    row![
        nav,
        iced::widget::rule::vertical(1).style(theme::sidebar_divider_rule),
        container(scrollable(content).height(Length::Fill))
            .width(Length::Fill)
            .height(Length::Fill)
            .padding(PAD_SETTINGS_CONTENT)
            .style(theme::content_container),
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
        ), SettingsMessage::ToggleSelect(SelectField::Theme)),
        setting_row("Reading Pane", widgets::select(
            &["Right", "Bottom", "Hidden"], &state.reading_pane_position,
            state.open_select == Some(SelectField::ReadingPane),
            SettingsMessage::ToggleSelect(SelectField::ReadingPane),
            SettingsMessage::ReadingPaneChanged,
        ), SettingsMessage::ToggleSelect(SelectField::ReadingPane)),
        setting_row("Email Density", widgets::select(
            &["Compact", "Default", "Spacious"], &state.density,
            state.open_select == Some(SelectField::Density),
            SettingsMessage::ToggleSelect(SelectField::Density),
            SettingsMessage::DensityChanged,
        ), SettingsMessage::ToggleSelect(SelectField::Density)),
        setting_row("Font Size", widgets::select(
            &["Small", "Default", "Large", "XLarge"], &state.font_size,
            state.open_select == Some(SelectField::FontSize),
            SettingsMessage::ToggleSelect(SelectField::FontSize),
            SettingsMessage::FontSizeChanged,
        ), SettingsMessage::ToggleSelect(SelectField::FontSize)),
        accent_color_row(state.accent_color_index),
        toggle_row("Show Sync Status Bar", "Display sync progress in the status bar", state.sync_status_bar, SettingsMessage::ToggleSyncStatusBar),
    ]));

    col = col.push(section("Privacy & Security", vec![
        toggle_row("Block Remote Images", "Don't load remote images in email bodies", state.block_remote_images, SettingsMessage::ToggleBlockRemoteImages),
        toggle_row("Phishing Detection", "Warn about suspicious emails", state.phishing_detection, SettingsMessage::TogglePhishingDetection),
    ]));

    col.into()
}

// ── Composing tab ────────────────────────────────────────

fn composing_tab(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Sending", vec![
        setting_row("Undo Send Delay", widgets::select(
            &["None", "5 seconds", "10 seconds", "30 seconds"], &state.undo_delay,
            state.open_select == Some(SelectField::UndoDelay),
            SettingsMessage::ToggleSelect(SelectField::UndoDelay),
            SettingsMessage::UndoDelayChanged,
        ), SettingsMessage::ToggleSelect(SelectField::UndoDelay)),
        toggle_row("Send & Archive", "Archive a thread immediately after sending a reply",
            state.send_and_archive, SettingsMessage::ToggleSendAndArchive),
    ]));

    col = col.push(section("Behavior", vec![
        setting_row("Default Reply Action", widgets::select(
            &["Reply", "Reply All"], &state.default_reply_mode,
            state.open_select == Some(SelectField::DefaultReply),
            SettingsMessage::ToggleSelect(SelectField::DefaultReply),
            SettingsMessage::DefaultReplyChanged,
        ), SettingsMessage::ToggleSelect(SelectField::DefaultReply)),
        setting_row("Mark as Read", widgets::select(
            &["Instantly", "After 2 Seconds", "Manually"], &state.mark_as_read,
            state.open_select == Some(SelectField::MarkAsRead),
            SettingsMessage::ToggleSelect(SelectField::MarkAsRead),
            SettingsMessage::MarkAsReadChanged,
        ), SettingsMessage::ToggleSelect(SelectField::MarkAsRead)),
    ]));

    col = col.push(section("Signatures", vec![
        coming_soon_row("Signature management"),
    ]));

    col = col.push(section("Templates", vec![
        coming_soon_row("Template management"),
    ]));

    col.into()
}

// ── Notifications tab ────────────────────────────────────

fn notifications_tab(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Notifications", vec![
        toggle_row("Enable Notifications", "Receive desktop notifications for new email",
            state.notifications_enabled, SettingsMessage::ToggleNotifications),
        toggle_row("Smart Notifications", "Only notify about important emails",
            state.smart_notifications, SettingsMessage::ToggleSmartNotifications),
    ]));

    if state.smart_notifications {
        let chips: Vec<Element<'_, SettingsMessage>> =
            ["Primary", "Updates", "Promotions", "Social", "Newsletters"]
                .iter()
                .map(|cat| {
                    let active = state.notify_categories.contains(&(*cat).to_string());
                    button(text(*cat).size(TEXT_SM))
                        .on_press(SettingsMessage::ToggleNotifyCategory((*cat).to_string()))
                        .padding(PAD_ICON_BTN)
                        .style(theme::chip_button(active))
                        .into()
                })
                .collect();
        let chips_row = iced::widget::row(chips).spacing(SPACE_XS).align_y(Alignment::Center);
        col = col.push(section("Notify for Categories", vec![
            settings_row_container(SETTINGS_ROW_HEIGHT, chips_row),
        ]));

        let mut vip_col = column![].spacing(SPACE_XXXS).width(Length::Fill);

        vip_col = vip_col.push(
            container(
                text("Always notify when email arrives from a VIP sender.")
                    .size(TEXT_SM)
                    .style(theme::text_tertiary),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        );

        for email in &state.vip_senders {
            vip_col = vip_col.push(
                container(
                    row![
                        container(text(email.clone()).size(TEXT_LG).style(text::base))
                            .align_y(Alignment::Center)
                            .width(Length::Fill),
                        button(text("Remove").size(TEXT_SM).style(text::danger))
                            .on_press(SettingsMessage::RemoveVipSender(email.clone()))
                            .padding(PAD_ICON_BTN)
                            .style(theme::bare_button),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
            );
        }

        vip_col = vip_col.push(
            container(
                row![
                    text_input("email@example.com", &state.vip_email_input)
                        .on_input(SettingsMessage::VipEmailChanged)
                        .on_submit(SettingsMessage::AddVipSender)
                        .size(TEXT_LG)
                        .padding(PAD_INPUT)
                        .style(theme::settings_text_input)
                        .width(Length::Fill),
                    Space::new().width(SPACE_XS),
                    button(text("Add").size(TEXT_LG))
                        .on_press(SettingsMessage::AddVipSender)
                        .padding(PAD_ICON_BTN)
                        .style(theme::secondary_button),
                ]
                .align_y(Alignment::Center),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        );

        col = col.push(section("VIP Senders", vec![vip_col.into()]));
    }

    col.into()
}

// ── Mail Rules tab ───────────────────────────────────────

fn mail_rules_tab<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Labels", vec![coming_soon_row("Label management")]));
    col = col.push(section("Filters", vec![coming_soon_row("Filter management")]));
    col = col.push(section("Smart Labels", vec![coming_soon_row("Smart label management")]));
    col = col.push(section("Smart Folders", vec![coming_soon_row("Smart folder management")]));
    col = col.push(section("Quick Steps", vec![coming_soon_row("Quick step management")]));

    col.into()
}

// ── People tab ───────────────────────────────────────────

fn people_tab<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Contacts", vec![coming_soon_row("Contact management")]));
    col = col.push(section("Groups", vec![coming_soon_row("Group management")]));
    col = col.push(section("Subscriptions", vec![coming_soon_row("Subscription management")]));

    col.into()
}

// ── Shortcuts tab ────────────────────────────────────────

fn shortcuts_tab<'a>() -> Element<'a, SettingsMessage> {
    let sections: &[(&str, &[(&str, &str)])] = &[
        ("Navigation", &[
            ("Next thread", "j"),
            ("Previous thread", "k"),
            ("Go to Inbox", "g i"),
            ("Search", "/"),
            ("Close / dismiss", "Esc"),
        ]),
        ("Thread", &[
            ("Archive", "e"),
            ("Delete", "#"),
            ("Reply", "r"),
            ("Reply All", "a"),
            ("Forward", "f"),
            ("Star / unstar", "s"),
            ("Mute thread", "m"),
            ("Mark as unread", "u"),
        ]),
        ("Composing", &[
            ("New message", "c"),
        ]),
    ];

    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    for (category, items) in sections {
        let rows: Vec<Element<'_, SettingsMessage>> = items
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
                            .style(theme::key_badge_container),
                    ]
                    .align_y(Alignment::Center),
                )
            })
            .collect();

        col = col.push(section(category, rows));
    }

    col.into()
}

// ── AI tab ───────────────────────────────────────────────

fn ai_tab(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Provider", vec![
        setting_row("AI Provider", widgets::select(
            &["Claude", "OpenAI", "Gemini", "Ollama", "Copilot"],
            &state.ai_provider,
            state.open_select == Some(SelectField::AiProvider),
            SettingsMessage::ToggleSelect(SelectField::AiProvider),
            SettingsMessage::AiProviderChanged,
        ), SettingsMessage::ToggleSelect(SelectField::AiProvider)),
    ]));

    if state.ai_provider == "Ollama" {
        col = col.push(section("Local Server", vec![
            container(
                column![
                    text("Server URL").size(TEXT_LG).style(text::base),
                    Space::new().height(SPACE_XXS),
                    text_input("http://localhost:11434", &state.ai_ollama_url)
                        .on_input(SettingsMessage::OllamaUrlChanged)
                        .size(TEXT_LG)
                        .padding(PAD_INPUT)
                        .style(theme::settings_text_input)
                        .width(Length::Fill),
                ]
                .spacing(SPACE_XXXS),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .into(),
            container(
                column![
                    text("Model Name").size(TEXT_LG).style(text::base),
                    Space::new().height(SPACE_XXS),
                    text_input("e.g. llama3.2", &state.ai_ollama_model)
                        .on_input(SettingsMessage::OllamaModelChanged)
                        .size(TEXT_LG)
                        .padding(PAD_INPUT)
                        .style(theme::settings_text_input)
                        .width(Length::Fill),
                ]
                .spacing(SPACE_XXXS),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .into(),
        ]));
    } else {
        let key_label = match state.ai_provider.as_str() {
            "OpenAI" => "OpenAI API Key",
            "Gemini" => "Google AI API Key",
            "Copilot" => "GitHub Personal Access Token",
            _ => "Anthropic API Key",
        };

        let model_options: &[&str] = match state.ai_provider.as_str() {
            "OpenAI" => &["gpt-4o", "gpt-4o-mini", "o4-mini"],
            "Gemini" => &["gemini-2.0-flash", "gemini-2.5-flash-preview-05-20", "gemini-2.5-pro"],
            "Copilot" => &["openai/gpt-4o", "openai/gpt-4o-mini"],
            _ => &["claude-haiku-4-5-20251001", "claude-sonnet-4-5", "claude-sonnet-4-6", "claude-opus-4-6"],
        };

        col = col.push(section("API Key", vec![
            container(
                column![
                    text(key_label).size(TEXT_LG).style(text::base),
                    Space::new().height(SPACE_XXS),
                    row![
                        text_input("", &state.ai_api_key)
                            .on_input(SettingsMessage::AiApiKeyChanged)
                            .secure(true)
                            .size(TEXT_LG)
                            .padding(PAD_INPUT)
                            .style(theme::settings_text_input)
                            .width(Length::Fill),
                        Space::new().width(SPACE_XS),
                        button(
                            text(if state.ai_key_saved { "Saved" } else { "Save" }).size(TEXT_LG),
                        )
                        .on_press(SettingsMessage::SaveAiSettings)
                        .padding(PAD_ICON_BTN)
                        .style(theme::secondary_button),
                    ]
                    .align_y(Alignment::Center),
                ]
                .spacing(SPACE_XXXS),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .into(),
            setting_row("Model", widgets::select(
                model_options, &state.ai_model,
                state.open_select == Some(SelectField::AiModel),
                SettingsMessage::ToggleSelect(SelectField::AiModel),
                SettingsMessage::AiModelChanged,
            ), SettingsMessage::ToggleSelect(SelectField::AiModel)),
        ]));
    }

    col = col.push(section("Features", vec![
        toggle_row("Enable AI Features", "Use AI-powered features across the app",
            state.ai_enabled, SettingsMessage::ToggleAiEnabled),
        toggle_row("Auto-Categorize", "Automatically categorize incoming emails",
            state.ai_auto_categorize, SettingsMessage::ToggleAiAutoCategorize),
        toggle_row("Auto-Summarize", "Generate summaries for long email threads",
            state.ai_auto_summarize, SettingsMessage::ToggleAiAutoSummarize),
    ]));

    col = col.push(section("Auto-Draft Replies", vec![
        toggle_row("Auto-Draft", "Automatically draft replies based on email content",
            state.ai_auto_draft, SettingsMessage::ToggleAiAutoDraft),
        toggle_row("Learn Writing Style", "Analyze your sent emails to match your writing style",
            state.ai_writing_style, SettingsMessage::ToggleAiWritingStyle),
    ]));

    col = col.push(section("Auto-Archive Categories", vec![
        toggle_row("Updates", "Automatically archive update emails",
            state.ai_auto_archive_updates, SettingsMessage::ToggleAiAutoArchiveUpdates),
        toggle_row("Promotions", "Automatically archive promotional emails",
            state.ai_auto_archive_promotions, SettingsMessage::ToggleAiAutoArchivePromotions),
        toggle_row("Social", "Automatically archive social notification emails",
            state.ai_auto_archive_social, SettingsMessage::ToggleAiAutoArchiveSocial),
        toggle_row("Newsletters", "Automatically archive newsletters",
            state.ai_auto_archive_newsletters, SettingsMessage::ToggleAiAutoArchiveNewsletters),
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
        text(title)
            .size(TEXT_XL)
            .style(text::base)
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::TEXT }),
        container(col)
            .width(Length::Fill)
            .style(theme::settings_section_container),
    ]
    .spacing(SPACE_XS)
    .into()
}

fn settings_row_container<'a>(
    height: f32,
    content: impl Into<iced::Element<'a, SettingsMessage>>,
) -> Element<'a, SettingsMessage> {
    container(content)
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .height(height)
        .align_y(Alignment::Center)
        .into()
}

fn setting_row<'a>(
    label: &'a str,
    control: Element<'a, SettingsMessage>,
    on_press: SettingsMessage,
) -> Element<'a, SettingsMessage> {
    button(
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
        .height(SETTINGS_ROW_HEIGHT)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(0)
    .style(theme::bare_button)
    .width(Length::Fill)
    .into()
}

fn toggle_row<'a>(
    label: &'a str,
    description: &'a str,
    value: bool,
    on_toggle: impl Fn(bool) -> SettingsMessage + 'a,
) -> Element<'a, SettingsMessage> {
    // Compute the button's press message before on_toggle is moved into the toggler.
    // The toggler captures its own click events, so the button only fires when the
    // user clicks outside the knob (e.g. on the label). No double-firing.
    let on_press_msg = on_toggle(!value);
    button(
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
        .height(SETTINGS_TOGGLE_ROW_HEIGHT)
        .align_y(Alignment::Center),
    )
    .on_press(on_press_msg)
    .padding(0)
    .style(theme::bare_button)
    .width(Length::Fill)
    .into()
}

fn info_row<'a>(label: &'a str, value: &'a str) -> Element<'a, SettingsMessage> {
    settings_row_container(SETTINGS_ROW_HEIGHT,
        row![
            container(text(label).size(TEXT_LG).style(theme::text_tertiary))
                .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            container(text(value).size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center),
        ]
        .align_y(Alignment::Center),
    )
}

fn coming_soon_row<'a>(feature: &'a str) -> Element<'a, SettingsMessage> {
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(format!("{feature} coming soon."))
            .size(TEXT_LG)
            .style(theme::text_tertiary),
    )
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

    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            container(text("Accent Color").size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            swatches,
        ]
        .align_y(Alignment::Center),
    )
}
