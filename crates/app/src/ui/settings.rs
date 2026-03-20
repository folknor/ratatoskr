use iced::animation::{self, Easing};
use iced::time::{Duration, Instant};
use iced::widget::{button, column, container, mouse_area, radio, row, scrollable, slider, text, text_input, Space};
use iced::{Alignment, Element, Length, Point, Task};

use crate::db::DateDisplay;
use crate::ui::animated_toggler::animated_toggler;

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
    ScaleDragged(f32),
    ScaleReleased,
    ThemeChanged(String),
    DensityChanged(String),
    FontSizeChanged(String),
    ReadingPaneChanged(String),
    ThemeSelected(usize),
    ToggleSyncStatusBar(bool),
    ToggleBlockRemoteImages(bool),
    TogglePhishingDetection(bool),
    PhishingSensitivityChanged(String),
    DateDisplayChanged(String),
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
    // Editable list
    ListGripPress(String, usize),         // grip pressed — start potential drag
    ListDragMove(String, Point),          // cursor moved while grip held
    ListDragEnd(String),                  // grip released — end drag
    ListRowClick(String, usize),          // row clicked (not grip) — toggle
    ListRemove(String, usize),            // (list_id, item index)
    ListAdd(String),                      // (list_id)
    ListToggle(String, usize, bool),      // (list_id, item index, new value)
    ListMenu(String, usize),              // (list_id, item index)
    // Input/info rows
    FocusInput(String),
    CopyToClipboard(String),
    Noop,
    // Help tooltips
    HelpHover(String),
    HelpUnhover(String),
    ToggleHelpPin(String),
    DismissHelp,
    // Overlay
    OpenOverlay(SettingsOverlay),
    CloseOverlay,
    OverlayAnimTick(Instant),
}

/// Overlays that slide in from the right, covering the settings content.
/// One level deep — no stacking.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsOverlay {
    CreateFilter,
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
    DateDisplay,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Theme,
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
        Tab::Theme,
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
            Tab::Theme => "Theme",
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
            Tab::Theme => icon::palette(),
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
    pub scale: f32,
    pub scale_preview: Option<f32>,
    pub theme: String,
    pub density: String,
    pub font_size: String,
    pub reading_pane_position: String,
    pub selected_theme: Option<usize>,
    pub sync_status_bar: bool,
    pub block_remote_images: bool,
    pub phishing_detection: bool,
    pub phishing_sensitivity: String,
    pub date_display: DateDisplay,
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
    // Overlay
    pub overlay: Option<SettingsOverlay>,
    pub overlay_anim: animation::Animation<bool>,
    // Help tooltips
    pub hovered_help: Option<String>,
    pub pinned_help: Option<String>,
    // Editable lists
    pub drag_state: Option<DragState>,
    // Demo data for Mail Rules tab
    pub demo_labels: Vec<EditableItem>,
    pub demo_filters: Vec<EditableItem>,
}

/// State for an active drag operation.
#[derive(Debug, Clone)]
pub struct DragState {
    pub list_id: String,
    pub dragging_index: usize,
    /// Y coordinate when the grip was pressed (list-relative, set on first move).
    pub start_y: f32,
    /// Whether the mouse has moved far enough to count as a real drag.
    pub is_dragging: bool,
}

/// Minimum Y movement before a grip press becomes a drag.
const DRAG_START_THRESHOLD: f32 = 4.0;

/// An item in an editable list.
#[derive(Debug, Clone)]
pub struct EditableItem {
    pub label: String,
    pub enabled: Option<bool>,
}

impl SettingsState {
    pub fn with_scale(scale: f32) -> Self {
        Self {
            scale,
            ..Self::default()
        }
    }
}

impl Default for SettingsState {
    fn default() -> Self {
        Self {
            active_tab: Tab::General,
            open_select: None,
            scale: 1.0,
            scale_preview: None,
            theme: "Light".into(),
            density: "Default".into(),
            font_size: "Default".into(),
            reading_pane_position: "Right".into(),
            selected_theme: None,
            sync_status_bar: true,
            block_remote_images: false,
            phishing_detection: true,
            phishing_sensitivity: "Default".into(),
            date_display: DateDisplay::RelativeOffset,
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
            overlay: None,
            overlay_anim: animation::Animation::new(false)
                .easing(Easing::EaseOutCubic)
                .duration(Duration::from_millis(200)),
            hovered_help: None,
            pinned_help: None,
            drag_state: None,
            demo_labels: vec![
                EditableItem { label: "Important".into(), enabled: Some(true) },
                EditableItem { label: "Personal".into(), enabled: Some(true) },
                EditableItem { label: "Receipts".into(), enabled: Some(false) },
                EditableItem { label: "Travel".into(), enabled: None },
            ],
            demo_filters: vec![
                EditableItem { label: "Auto-archive promotions".into(), enabled: Some(true) },
                EditableItem { label: "Star from VIPs".into(), enabled: Some(true) },
            ],
        }
    }
}

impl SettingsState {
    pub fn update(&mut self, message: SettingsMessage) -> Task<SettingsMessage> {
        match message {
            SettingsMessage::FocusInput(id) => {
                return iced::widget::operation::focus(id);
            }
            SettingsMessage::CopyToClipboard(contents) => {
                return iced::clipboard::write(contents);
            }
            SettingsMessage::Noop => {}
            SettingsMessage::HelpHover(id) => {
                self.hovered_help = Some(id);
            }
            SettingsMessage::HelpUnhover(id) => {
                if self.hovered_help.as_ref() == Some(&id) {
                    self.hovered_help = None;
                }
            }
            SettingsMessage::ToggleHelpPin(id) => {
                if self.pinned_help.as_ref() == Some(&id) {
                    self.pinned_help = None;
                } else {
                    self.pinned_help = Some(id);
                }
            }
            SettingsMessage::DismissHelp => {
                self.pinned_help = None;
                self.hovered_help = None;
            }
            SettingsMessage::Close => {}
            SettingsMessage::SelectTab(tab) => {
                self.active_tab = tab;
                self.pinned_help = None;
            }
            SettingsMessage::ToggleSelect(field) => {
                self.open_select = if self.open_select == Some(field) {
                    None
                } else {
                    Some(field)
                };
            }
            // General
            SettingsMessage::ScaleDragged(v) => self.scale_preview = Some(v),
            SettingsMessage::ScaleReleased => {
                if let Some(v) = self.scale_preview.take() {
                    self.scale = v;
                }
            }
            SettingsMessage::ThemeChanged(v) => { self.theme = v; self.open_select = None; }
            SettingsMessage::DensityChanged(v) => { self.density = v; self.open_select = None; }
            SettingsMessage::FontSizeChanged(v) => { self.font_size = v; self.open_select = None; }
            SettingsMessage::ReadingPaneChanged(v) => { self.reading_pane_position = v; self.open_select = None; }
            SettingsMessage::ThemeSelected(i) => {
                self.selected_theme = Some(i);
                self.theme = "Theme".into();
            }
            SettingsMessage::ToggleSyncStatusBar(v) => self.sync_status_bar = v,
            SettingsMessage::ToggleBlockRemoteImages(v) => self.block_remote_images = v,
            SettingsMessage::TogglePhishingDetection(v) => self.phishing_detection = v,
            SettingsMessage::PhishingSensitivityChanged(v) => self.phishing_sensitivity = v,
            SettingsMessage::DateDisplayChanged(v) => {
                self.date_display = match v.as_str() {
                    "Absolute" => DateDisplay::Absolute,
                    _ => DateDisplay::RelativeOffset,
                };
                self.open_select = None;
            }
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
            // Editable list
            SettingsMessage::ListGripPress(list_id, index) => {
                self.drag_state = Some(DragState {
                    list_id,
                    dragging_index: index,
                    start_y: -1.0, // Set on first move
                    is_dragging: false,
                });
            }
            SettingsMessage::ListDragMove(list_id, point) => {
                // Only act if drag was initiated from the grip.
                let has_drag = self.drag_state.as_ref()
                    .is_some_and(|d| d.list_id == list_id);
                if !has_drag { return Task::none(); }

                // Record start_y on first move event.
                if let Some(ref mut drag) = self.drag_state
                    && drag.start_y < 0.0
                {
                    drag.start_y = point.y;
                    return Task::none();
                }

                let Some(drag_ref) = self.drag_state.as_ref() else {
                    return Task::none();
                };
                let (from, start_y) = (drag_ref.dragging_index, drag_ref.start_y);

                // Check if we've moved enough to start dragging.
                if !drag_ref.is_dragging {
                    if (point.y - start_y).abs() < DRAG_START_THRESHOLD {
                        return Task::none();
                    }
                    if let Some(ref mut drag) = self.drag_state {
                        drag.is_dragging = true;
                    }
                }

                // Compute target index from cursor Y relative to list top.
                // Each row is SETTINGS_ROW_HEIGHT, plus 1px divider between rows.
                let row_step = SETTINGS_ROW_HEIGHT + 1.0;
                let count = self.list_items_mut(&list_id).len();
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let target = ((point.y / row_step).max(0.0) as usize).min(count.saturating_sub(1));

                if target != from {
                    self.list_items_mut(&list_id).swap(from, target);
                    if let Some(ref mut drag) = self.drag_state {
                        drag.dragging_index = target;
                    }
                }
            }
            SettingsMessage::ListDragEnd(_) => {
                self.drag_state = None;
            }
            SettingsMessage::ListRowClick(list_id, index) => {
                // Only toggle if no drag is active.
                if self.drag_state.is_some() { return Task::none(); }
                let items = self.list_items_mut(&list_id);
                if let Some(item) = items.get_mut(index)
                    && let Some(ref mut enabled) = item.enabled
                {
                    *enabled = !*enabled;
                }
            }
            SettingsMessage::ListRemove(list_id, index) => {
                let items = self.list_items_mut(&list_id);
                if index < items.len() {
                    items.remove(index);
                }
            }
            SettingsMessage::ListAdd(list_id) => {
                let items = self.list_items_mut(&list_id);
                items.push(EditableItem {
                    label: format!("New item {}", items.len() + 1),
                    enabled: None,
                });
            }
            SettingsMessage::ListToggle(list_id, index, value) => {
                let items = self.list_items_mut(&list_id);
                if let Some(item) = items.get_mut(index) {
                    item.enabled = Some(value);
                }
            }
            SettingsMessage::ListMenu(_, _) => {
                // TODO: open context menu
            }
            // Overlay
            SettingsMessage::OpenOverlay(overlay) => {
                self.overlay = Some(overlay);
                self.overlay_anim.go_mut(true, Instant::now());
            }
            SettingsMessage::CloseOverlay => {
                self.overlay = None;
                self.overlay_anim.go_mut(false, Instant::now());
            }
            SettingsMessage::OverlayAnimTick(_) => {
                // Just triggers a redraw — the animation reads Instant::now() in view.
            }
        }
        Task::none()
    }

    fn list_items_mut(&mut self, list_id: &str) -> &mut Vec<EditableItem> {
        match list_id {
            "labels" => &mut self.demo_labels,
            "filters" => &mut self.demo_filters,
            _ => &mut self.demo_labels,
        }
    }
}

// ── View ────────────────────────────────────────────────

pub fn view(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let nav = tab_nav(state.active_tab);
    let content = match state.active_tab {
        Tab::General => general_tab(state),
        Tab::Theme => theme_tab(state),
        Tab::Notifications => notifications_tab(state),
        Tab::Composing => composing_tab(state),
        Tab::MailRules => mail_rules_tab(state),
        Tab::People => people_tab(),
        Tab::Shortcuts => shortcuts_tab(),
        Tab::Ai => ai_tab(state),
        Tab::About => about_tab(),
    };

    let now = Instant::now();
    let overlay_t: f32 = state.overlay_anim.interpolate(0.0, 1.0, now);
    let show_overlay = state.overlay.is_some() || overlay_t > 0.001;

    let content_area = container(
        scrollable(
            container(content)
                .padding(PAD_SETTINGS_CONTENT)
                .align_x(Alignment::Center)
        ).spacing(SCROLLBAR_SPACING).height(Length::Fill)
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::content_container);

    let main_content: Element<'_, SettingsMessage> = if show_overlay {
        let overlay_content = match state.overlay {
            Some(SettingsOverlay::CreateFilter) => create_filter_overlay(),
            None => column![].into(), // closing animation
        };

        // Overlay panel: back button header + scrollable content
        let overlay_panel = container(
            column![
                container(
                    button(
                        row![
                            container(icon::arrow_left().size(ICON_XL).style(text::base))
                                .align_y(Alignment::Center),
                            text("Back").size(TEXT_LG).style(text::base)
                                .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
                        ]
                        .spacing(SPACE_XS)
                        .align_y(Alignment::Center),
                    )
                    .on_press(SettingsMessage::CloseOverlay)
                    .padding(PAD_NAV_ITEM)
                    .style(theme::bare_icon_button),
                )
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
                scrollable(
                    container(overlay_content)
                        .padding(PAD_SETTINGS_CONTENT)
                        .align_x(Alignment::Center)
                ).spacing(SCROLLBAR_SPACING).height(Length::Fill),
            ]
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::content_container);

        // Slide from right: use a large fixed offset (2000px) scaled by (1-t).
        // The stack clips to bounds so overshooting doesn't matter.
        let offset = ((1.0 - overlay_t) * 2000.0).round();

        // Event blocker: opaque mouse_area between content and overlay
        // prevents clicks/hovers from reaching the content underneath.
        let blocker = mouse_area(
            container(Space::new().width(Length::Fill).height(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(SettingsMessage::CloseOverlay)
        .interaction(iced::mouse::Interaction::default());

        iced::widget::stack![
            content_area,
            blocker,
            container(overlay_panel)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 0.0, left: offset }),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    } else {
        content_area.into()
    };

    row![
        nav,
        iced::widget::rule::vertical(1).style(theme::sidebar_divider_rule),
        main_content,
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

    container(scrollable(col).spacing(SCROLLBAR_SPACING).height(Length::Fill))
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
            &["System", "Light", "Dark", "Theme"], &state.theme,
            state.open_select == Some(SelectField::Theme),
            SettingsMessage::ToggleSelect(SelectField::Theme),
            SettingsMessage::ThemeChanged,
        ), SettingsMessage::ToggleSelect(SelectField::Theme)),
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
        slider_row("Scale", None, 1.0..=4.0, state.scale_preview.unwrap_or(state.scale), 1.0, 0.125, SettingsMessage::ScaleDragged, Some(SettingsMessage::ScaleReleased)),
        setting_row("Message Dates", widgets::select(
            &["Relative Offset", "Absolute"],
            match state.date_display {
                DateDisplay::RelativeOffset => "Relative Offset",
                DateDisplay::Absolute => "Absolute",
            },
            state.open_select == Some(SelectField::DateDisplay),
            SettingsMessage::ToggleSelect(SelectField::DateDisplay),
            SettingsMessage::DateDisplayChanged,
        ), SettingsMessage::ToggleSelect(SelectField::DateDisplay)),
        toggle_row("Show Sync Status Bar", "Display sync progress in the status bar", state.sync_status_bar, SettingsMessage::ToggleSyncStatusBar),
    ]));

    col = col.push(section("Reading Pane", radio_group(
        &[("Right", "Right"), ("Bottom", "Bottom"), ("Hidden", "Hidden")],
        Some(state.reading_pane_position.as_str()),
        |v| SettingsMessage::ReadingPaneChanged(v.to_string()),
    )));

    let privacy_help_id = "privacy-security";
    let privacy_help_visible = state.hovered_help.as_deref() == Some(privacy_help_id)
        || state.pinned_help.as_deref() == Some(privacy_help_id);
    col = col.push(section_with_help("Privacy & Security", SectionHelp {
        id: privacy_help_id,
        content: column![
            text("Remote images can be used to track when you open an email. Blocking them prevents this but some emails may not display correctly.")
                .size(TEXT_SM)
                .style(theme::text_on_primary),
            Space::new().height(SPACE_XS),
            text("Phishing detection analyzes incoming emails for suspicious links, sender spoofing, and social engineering patterns.")
                .size(TEXT_SM)
                .style(theme::text_on_primary),
        ]
        .into(),
        visible: privacy_help_visible,
        pinned: state.pinned_help.as_deref() == Some(privacy_help_id),
    }, vec![
        toggle_row("Block Remote Images", "Don't load remote images in email bodies", state.block_remote_images, SettingsMessage::ToggleBlockRemoteImages),
        toggle_row("Phishing Detection", "Warn about suspicious emails", state.phishing_detection, SettingsMessage::TogglePhishingDetection),
    ]));

    col.into()
}

// ── Theme tab ───────────────────────────────────────────

fn theme_tab(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    // Build a 3-column grid of theme previews
    let mut grid = column![].spacing(SPACE_XS);
    let mut current_row = row![].spacing(SPACE_XS);
    let mut col_count = 0;

    for (i, entry) in theme::THEMES.iter().enumerate() {
        let selected = state.selected_theme == Some(i)
            || (state.selected_theme.is_none() && state.theme == entry.name);

        let card = column![
            widgets::theme_preview(&entry.palette, selected, crate::Message::Noop)
                .map(move |_| SettingsMessage::ThemeSelected(i)),
            container(
                text(entry.name).size(TEXT_SM).style(if selected { text::base } else { text::secondary }),
            )
            .width(Length::Fill)
            .align_x(Alignment::Center),
        ]
        .spacing(SPACE_XXS)
        .align_x(Alignment::Center);

        current_row = current_row.push(container(card).width(Length::FillPortion(1)));
        col_count += 1;

        if col_count == 3 {
            grid = grid.push(current_row);
            current_row = row![].spacing(SPACE_XS);
            col_count = 0;
        }
    }

    // Push remaining items with empty spacers to fill the row
    if col_count > 0 {
        while col_count < 3 {
            current_row = current_row.push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1)));
            col_count += 1;
        }
        grid = grid.push(current_row);
    }

    col = col.push(section("Themes", vec![
        container(grid).padding(PAD_SETTINGS_ROW).width(Length::Fill).into(),
    ]));

    // Button experiment grid: each candidate next to a Primary for comparison
    let experiments: Vec<(&str, usize)> = vec![
        ("pri border", 8),
        ("text border", 9),
        ("pri+fill", 10),
        ("muted border", 11),
        ("mix 15%", 20),
        ("text 10%", 19),
    ];

    let mut grid = column![].spacing(SPACE_XS);
    let mut current_row = row![].spacing(SPACE_XS);
    let mut col_count = 0;

    for (label, idx) in &experiments {
        let btn_width = Length::Fixed(120.0);
        let pair = row![
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::exp_btn(*idx)).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::primary_button).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS);

        current_row = current_row.push(container(pair).width(Length::FillPortion(1)));
        col_count += 1;

        if col_count == 2 {
            grid = grid.push(current_row);
            current_row = row![].spacing(SPACE_XS);
            col_count = 0;
        }
    }
    if col_count > 0 {
        while col_count < 2 {
            current_row = current_row.push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1)));
            col_count += 1;
        }
        grid = grid.push(current_row);
    }

    col = col.push(section("Button Experiments (section bg)", vec![
        container(grid).padding(PAD_SETTINGS_ROW).width(Length::Fill).into(),
    ]));

    // Same grid on content/main area background
    let mut grid2 = column![].spacing(SPACE_XS);
    let mut current_row2 = row![].spacing(SPACE_XS);
    let mut col_count2 = 0;
    for (label, idx) in &experiments {
        let btn_width = Length::Fixed(120.0);
        let pair = row![
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::exp_btn(*idx)).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::primary_button).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS);
        current_row2 = current_row2.push(container(pair).width(Length::FillPortion(1)));
        col_count2 += 1;
        if col_count2 == 2 {
            grid2 = grid2.push(current_row2);
            current_row2 = row![].spacing(SPACE_XS);
            col_count2 = 0;
        }
    }
    if col_count2 > 0 {
        while col_count2 < 2 { current_row2 = current_row2.push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1))); col_count2 += 1; }
        grid2 = grid2.push(current_row2);
    }

    let content_bg_box = container(
        column![
            text("Content / main area background").size(TEXT_SM).style(theme::text_tertiary),
            grid2,
        ].spacing(SPACE_SM),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .style(theme::content_container);

    col = col.push(content_bg_box);

    // Same grid on sidebar background
    let mut grid3 = column![].spacing(SPACE_XS);
    let mut current_row3 = row![].spacing(SPACE_XS);
    let mut col_count3 = 0;
    for (label, idx) in &experiments {
        let btn_width = Length::Fixed(120.0);
        let pair = row![
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::exp_btn(*idx)).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::primary_button).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS);
        current_row3 = current_row3.push(container(pair).width(Length::FillPortion(1)));
        col_count3 += 1;
        if col_count3 == 2 {
            grid3 = grid3.push(current_row3);
            current_row3 = row![].spacing(SPACE_XS);
            col_count3 = 0;
        }
    }
    if col_count3 > 0 {
        while col_count3 < 2 { current_row3 = current_row3.push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1))); col_count3 += 1; }
        grid3 = grid3.push(current_row3);
    }

    let sidebar_bg_box = container(
        column![
            text("Sidebar background").size(TEXT_SM).style(theme::text_tertiary),
            grid3,
        ].spacing(SPACE_SM),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .style(theme::sidebar_container);

    col = col.push(sidebar_bg_box);

    // Semantic color pairs
    let btn_width = Length::Fixed(120.0);
    let semantic_grid = column![
        row![
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::primary_button).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::primary_button).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS),
        row![
            button(container(text("Success").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::exp_semantic_btn(0)).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::primary_button).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS),
        row![
            button(container(text("Warning").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::exp_semantic_btn(1)).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::primary_button).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS),
        row![
            button(container(text("Danger").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::exp_semantic_btn(2)).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::primary_button).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS),
    ].spacing(SPACE_XS);

    col = col.push(section("Semantic Color Pairs", vec![
        container(semantic_grid).padding(PAD_SETTINGS_ROW).width(Length::Fill).into(),
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
                            .style(theme::action_button),
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

fn mail_rules_tab(state: &SettingsState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Labels", vec![
        editable_list("labels", &state.demo_labels, "Add Label", &state.drag_state),
    ]));
    col = col.push(section("Filters", vec![
        action_row("Create Filter", Some("Add a new mail filter rule"), Some(icon::filter()), ActionKind::InApp, SettingsMessage::OpenOverlay(SettingsOverlay::CreateFilter)),
    ]));
    if !state.demo_filters.is_empty() {
        col = col.push(section_untitled(vec![
            editable_list("filters", &state.demo_filters, "Add Filter", &state.drag_state),
        ]));
    }
    col = col.push(section("Smart Labels", vec![coming_soon_row("Smart label management")]));
    col = col.push(section("Smart Folders", vec![coming_soon_row("Smart folder management")]));
    col = col.push(section("Quick Steps", vec![coming_soon_row("Quick step management")]));

    col.into()
}

// ── Overlays ─────────────────────────────────────────────

fn create_filter_overlay<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text("Create Filter")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
    );

    col = col.push(section("Conditions", vec![
        coming_soon_row("Match conditions"),
    ]));

    col = col.push(section("Actions", vec![
        coming_soon_row("Filter actions"),
    ]));

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
            input_row("ollama-url", "Server URL", "http://localhost:11434", &state.ai_ollama_url, SettingsMessage::OllamaUrlChanged),
            input_row("ollama-model", "Model Name", "e.g. llama3.2", &state.ai_ollama_model, SettingsMessage::OllamaModelChanged),
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

    col = col.push(section_with_subtitle("Features", "AI-powered tools to help manage your inbox", vec![
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
        action_row("Software Updates", Some("Check for new versions"), None, ActionKind::InApp, SettingsMessage::CheckForUpdates),
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
        action_row("GitHub Repository", Some("folknor/ratatoskr"), Some(icon::globe()), ActionKind::Url, SettingsMessage::OpenGithub),
    ]));

    col.into()
}

// ── Shared setting widgets ──────────────────────────────

fn section<'a>(
    title: &'a str,
    items: Vec<Element<'a, SettingsMessage>>,
) -> Element<'a, SettingsMessage> {
    section_inner(Some(title), None, None, items)
}

fn section_with_subtitle<'a>(
    title: &'a str,
    subtitle: &'a str,
    items: Vec<Element<'a, SettingsMessage>>,
) -> Element<'a, SettingsMessage> {
    section_inner(Some(title), Some(subtitle), None, items)
}

fn section_with_help<'a>(
    title: &'a str,
    help: SectionHelp<'a>,
    items: Vec<Element<'a, SettingsMessage>>,
) -> Element<'a, SettingsMessage> {
    section_inner(Some(title), None, Some(help), items)
}

fn section_untitled<'a>(
    items: Vec<Element<'a, SettingsMessage>>,
) -> Element<'a, SettingsMessage> {
    section_inner(None, None, None, items)
}

/// Help tooltip configuration for a section header.
struct SectionHelp<'a> {
    id: &'a str,
    content: Element<'a, SettingsMessage>,
    visible: bool,
    pinned: bool,
}

fn section_inner<'a>(
    title: Option<&'a str>,
    subtitle: Option<&'a str>,
    help: Option<SectionHelp<'a>>,
    items: Vec<Element<'a, SettingsMessage>>,
) -> Element<'a, SettingsMessage> {
    let mut col = column![].width(Length::Fill).padding(1);
    for (i, item) in items.into_iter().enumerate() {
        if i > 0 {
            col = col.push(iced::widget::rule::horizontal(1).style(theme::subtle_divider_rule));
        }
        col = col.push(item);
    }
    let section_box = container(col)
        .width(Length::Fill)
        .style(theme::settings_section_container);

    if let Some(title) = title {
        let title_text: Element<'a, SettingsMessage> = text(title)
            .size(TEXT_XL)
            .style(text::base)
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() })
            .into();

        let header_row: Element<'a, SettingsMessage> = if let Some(help_cfg) = help {
            let help_id = help_cfg.id.to_string();
            let help_id_hover = help_id.clone();
            let help_id_unhover = help_id.clone();
            let icon_style: fn(&iced::Theme) -> text::Style = if help_cfg.pinned {
                text::primary
            } else {
                theme::text_muted
            };

            let help_icon = mouse_area(
                button(
                    container(icon::help_circle().size(ICON_XL).style(icon_style))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center),
                )
                .on_press(SettingsMessage::ToggleHelpPin(help_id.clone()))
                .padding(PAD_ICON_BTN)
                .style(theme::bare_icon_button),
            )
            .on_enter(SettingsMessage::HelpHover(help_id_hover))
            .on_exit(SettingsMessage::HelpUnhover(help_id_unhover));

            let mut pop = crate::ui::popover::popover(help_icon)
                .position(crate::ui::popover::Position::BelowRight)
                .popup_width(HELP_TOOLTIP_WIDTH);

            if help_cfg.visible {
                pop = pop
                    .popup(
                        container(help_cfg.content)
                            .padding(PAD_SETTINGS_ROW)
                            .width(Length::Fill)
                            .style(theme::floating_container),
                    )
                    .on_dismiss(SettingsMessage::DismissHelp);
            }

            row![
                title_text,
                Space::new().width(Length::Fill),
                pop,
            ]
            .align_y(Alignment::Center)
            .into()
        } else {
            title_text
        };

        let mut header = column![header_row].spacing(SPACE_XXXS);

        if let Some(subtitle) = subtitle {
            header = header.push(
                text(subtitle)
                    .size(TEXT_SM)
                    .style(theme::text_tertiary),
            );
        }

        column![header, section_box].spacing(SPACE_XS)
    } else {
        column![section_box]
    }
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
    .style(theme::action_button)
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
                animated_toggler(value).size(TEXT_HEADING).on_toggle(on_toggle).style(theme::settings_toggler),
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
    .style(theme::action_button)
    .width(Length::Fill)
    .into()
}

fn info_row(
    label: &str,
    value: &str,
) -> Element<'static, SettingsMessage> {
    let label_owned = label.to_string();
    let value_owned = value.to_string();
    let value_for_clipboard = value_owned.clone();
    container(
        row![
            column![
                text(label_owned).size(TEXT_SM).style(theme::text_tertiary),
                text_input("", &value_owned)
                    .on_input(|_| SettingsMessage::Noop)
                    .size(TEXT_LG)
                    .padding(0)
                    .style(theme::inline_text_input),
            ]
            .spacing(SPACE_XXXS)
            .width(Length::Fill),
            button(
                container(icon::copy().size(ICON_MD).style(text::base))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::CopyToClipboard(value_for_clipboard))
            .padding(PAD_ICON_BTN)
            .style(theme::bare_icon_button),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn input_row(
    id: &str,
    label: &str,
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> SettingsMessage + 'static,
) -> Element<'static, SettingsMessage> {
    let id_owned = id.to_string();
    let label_owned = label.to_string();
    let placeholder_owned = placeholder.to_string();
    let value_owned = value.to_string();
    mouse_area(
        button(
            container(
                row![
                    column![
                        text(label_owned).size(TEXT_SM).style(theme::text_tertiary),
                        text_input(&placeholder_owned, &value_owned)
                            .id(id_owned.clone())
                            .on_input(on_input)
                            .size(TEXT_LG)
                            .padding(0)
                            .style(theme::inline_text_input),
                    ]
                    .spacing(SPACE_XXXS)
                    .width(Length::Fill),
                    container(icon::pencil().size(ICON_MD).style(text::base))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center),
                ]
                .spacing(SPACE_SM)
                .align_y(Alignment::Center),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        )
        .on_press(SettingsMessage::FocusInput(id_owned.clone()))
        .padding(0)
        .style(theme::action_button)
        .width(Length::Fill),
    )
    .interaction(iced::mouse::Interaction::Text)
    .into()
}

fn coming_soon_row<'a>(feature: &'a str) -> Element<'a, SettingsMessage> {
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(format!("{feature} coming soon."))
            .size(TEXT_LG)
            .style(theme::text_tertiary),
    )
}

/// A row with a label on the left (50%) and an optional icon + slider on the right (50%).
/// No hover effect — only direct slider interaction. The slider has a strong snap toward `default`.
fn slider_row<'a>(
    label: &'a str,
    icon: Option<iced::widget::Text<'a>>,
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    default: f32,
    step: f32,
    on_change: impl Fn(f32) -> SettingsMessage + 'a,
    on_release: Option<SettingsMessage>,
) -> Element<'a, SettingsMessage> {
    let mut slider_widget = slider(range, value, on_change)
        .default(default)
        .step(step)
        .style(theme::settings_slider)
        .width(Length::Fill);
    if let Some(msg) = on_release {
        slider_widget = slider_widget.on_release(msg);
    }

    let right_content: Element<'a, SettingsMessage> = if let Some(ic) = icon {
        row![
            container(ic.size(ICON_XL).style(text::secondary))
                .align_y(Alignment::Center),
            slider_widget,
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center)
        .width(Length::Fill)
        .into()
    } else {
        slider_widget.into()
    };

    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            container(text(label).size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center)
                .width(Length::FillPortion(1)),
            container(right_content)
                .align_y(Alignment::Center)
                .width(Length::FillPortion(1)),
        ]
        .align_y(Alignment::Center),
    )
}

/// A group of mutually exclusive radio options, rendered as rows with hover effects.
/// Each row has a radio circle on the left, label text a fixed distance away.
/// Radio groups must always have their own `section()` — don't mix with other row types.
fn radio_group<'a, V>(
    options: &'a [(&'a str, V)],
    selected: Option<V>,
    on_select: impl Fn(V) -> SettingsMessage + 'a + Copy,
) -> Vec<Element<'a, SettingsMessage>>
where
    V: Copy + Eq + 'a,
{
    options
        .iter()
        .map(|(label, value)| {
            let msg = on_select(*value);
            button(
                container(
                    row![
                        radio("", *value, selected, on_select)
                            .size(RADIO_SIZE)
                            .spacing(0)
                            .style(theme::settings_radio),
                        container(text(*label).size(TEXT_LG).style(text::base))
                            .align_y(Alignment::Center),
                    ]
                    .spacing(RADIO_LABEL_SPACING)
                    .align_y(Alignment::Center),
                )
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill)
                .height(SETTINGS_ROW_HEIGHT)
                .align_y(Alignment::Center),
            )
            .on_press(msg)
            .padding(0)
            .style(theme::action_button)
            .width(Length::Fill)
            .into()
        })
        .collect()
}

/// An editable, reorderable list with drag handles, optional toggles/menus/remove buttons,
/// and an "Add" button at the bottom. This is the full-featured private implementation;
/// public wrappers will expose only the slots each use case needs.
///
/// Drag reordering uses a single `mouse_area` wrapping the entire list so that
/// `on_move` continues to fire as the cursor leaves individual row bounds.
/// The grip `on_press` initiates the drag; the list-level `on_move` computes
/// the target index from the cursor's Y position relative to the list top.
fn editable_list<'a>(
    list_id: &'a str,
    items: &'a [EditableItem],
    add_label: &'a str,
    drag_state: &'a Option<DragState>,
) -> Element<'a, SettingsMessage> {
    let id = list_id.to_string();

    let mut col = column![].width(Length::Fill);

    for (i, item) in items.iter().enumerate() {
        if i > 0 {
            col = col.push(iced::widget::rule::horizontal(1).style(theme::subtle_divider_rule));
        }

        let is_drag_item = drag_state
            .as_ref()
            .is_some_and(|d| d.list_id == list_id && d.dragging_index == i && d.is_dragging);

        // ── Left half: grip + label ──
        let lid_grip = id.clone();
        let grip_slot = mouse_area(
            container(
                icon::grip_vertical().size(ICON_MD).style(theme::text_tertiary),
            )
            .width(GRIP_SLOT_WIDTH)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
        )
        .on_press(SettingsMessage::ListGripPress(lid_grip, i))
        .interaction(iced::mouse::Interaction::Grab);

        let label_slot = container(
            text(&item.label).size(TEXT_LG).style(text::base),
        )
        .align_y(Alignment::Center)
        .width(Length::Fill);

        let left_half = row![grip_slot, label_slot]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center)
            .width(Length::FillPortion(1));

        // ── Right half: optional toggle, menu, remove — all float right ──
        let mut right_items: Vec<Element<'a, SettingsMessage>> = Vec::new();
        right_items.push(Space::new().width(Length::Fill).into());

        if let Some(enabled) = item.enabled {
            let idx = i;
            let lid = id.clone();
            right_items.push(
                animated_toggler(enabled)
                    .size(TEXT_HEADING)
                    .on_toggle(move |v| SettingsMessage::ListToggle(lid.clone(), idx, v))
                    .style(theme::settings_toggler)
                    .into(),
            );
        }

        // Menu button (⋯)
        right_items.push(
            button(
                container(icon::ellipsis().size(ICON_MD).style(text::secondary))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::ListMenu(id.clone(), i))
            .padding(PAD_ICON_BTN)
            .style(theme::bare_icon_button)
            .into(),
        );

        // Remove button (✕)
        right_items.push(
            button(
                container(icon::x().size(ICON_MD).style(text::secondary))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::ListRemove(id.clone(), i))
            .padding(PAD_ICON_BTN)
            .style(theme::bare_icon_button)
            .into(),
        );

        let right_half = iced::widget::row(right_items)
            .spacing(SPACE_XS)
            .align_y(Alignment::Center)
            .width(Length::FillPortion(1));

        let item_row = row![left_half, right_half]
            .align_y(Alignment::Center);

        // Button for hover effect + row click (toggle).
        let lid_click = id.clone();

        let mut inner_container = container(item_row)
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(SETTINGS_ROW_HEIGHT)
            .align_y(Alignment::Center);

        if is_drag_item {
            inner_container = inner_container.style(theme::dragging_row_container);
        }

        let item_btn = button(inner_container)
            .on_press(SettingsMessage::ListRowClick(lid_click, i))
            .padding(0)
            .style(theme::action_button)
            .width(Length::Fill);

        col = col.push(item_btn);
    }

    // Divider before Add button (if there are items)
    if !items.is_empty() {
        col = col.push(iced::widget::rule::horizontal(1).style(theme::subtle_divider_rule));
    }

    // Add button — label centered
    let add_id = id.clone();
    let add_btn = button(
        container(
            row![
                icon::plus().size(ICON_MD).style(text::base),
                text(add_label).size(TEXT_LG).style(text::base)
                    .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::ListAdd(add_id))
    .padding(PAD_SETTINGS_ROW)
    .style(theme::action_button)
    .width(Length::Fill)
    .height(SETTINGS_ROW_HEIGHT);

    col = col.push(add_btn);

    // Wrap entire list in a single mouse_area for drag tracking.
    // on_move gives us Y relative to the list top, so we can compute
    // the target index directly: target = (y / row_height).
    let lid_move = id.clone();
    let lid_release = id;
    let lid_exit = lid_release.clone();
    let list_area = mouse_area(col)
        .on_release(SettingsMessage::ListDragEnd(lid_release))
        .on_exit(SettingsMessage::ListDragEnd(lid_exit))
        .on_move(move |point| SettingsMessage::ListDragMove(lid_move.clone(), point));

    list_area.into()
}

/// The action type determines the trailing icon.
#[derive(Debug, Clone, Copy)]
enum ActionKind {
    /// Opens an external URL — shows external_link icon.
    Url,
    /// In-app action or slide-in overlay — shows arrow_right icon.
    InApp,
}

/// A full-row button with optional leading icon, label + optional description,
/// and a trailing icon indicating the action type. The entire row is the click
/// target — no nested buttons. Follows the rule that section rows never contain buttons.
fn action_row<'a>(
    label: &'a str,
    description: Option<&'a str>,
    icon: Option<iced::widget::Text<'a>>,
    kind: ActionKind,
    on_press: SettingsMessage,
) -> Element<'a, SettingsMessage> {
    let mut content = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if let Some(ico) = icon {
        content = content.push(
            container(ico.size(ICON_XL).style(text::secondary))
                .align_y(Alignment::Center),
        );
    }

    let label_col: Element<'a, SettingsMessage> = if let Some(desc) = description {
        column![
            text(label).size(TEXT_LG).style(text::base),
            text(desc).size(TEXT_SM).style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXXS)
        .into()
    } else {
        text(label).size(TEXT_LG).style(text::base).into()
    };

    content = content.push(label_col);
    content = content.push(Space::new().width(Length::Fill));

    let trailing = match kind {
        ActionKind::Url => icon::external_link(),
        ActionKind::InApp => icon::arrow_right(),
    };
    content = content.push(
        container(trailing.size(ICON_XL).style(text::base))
            .align_y(Alignment::Center),
    );

    button(content)
        .on_press(on_press)
        .padding(PAD_SETTINGS_ROW)
        .style(theme::action_button)
        .width(Length::Fill)
        .into()
}

