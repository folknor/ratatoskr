use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length, Padding};

use crate::Message;
use crate::db::{self, ThreadMessage};
use crate::font;
use crate::icon;
use crate::pop_out::session::MessageViewSessionEntry;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;

use super::PopOutMessage;

pub use db::MessageViewAttachment;

// ── Rendering mode ─────────────────────────────────────

/// Rendering mode for the message body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RenderingMode {
    PlainText,
    #[default]
    SimpleHtml,
    OriginalHtml,
    Source,
}

// ── Messages ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum MessageViewMessage {
    /// Body content loaded from the body store.
    BodyLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
        Result<(Option<String>, Option<String>), String>,
    ),
    /// Attachments loaded for this message.
    AttachmentsLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
        Result<Vec<MessageViewAttachment>, String>,
    ),
    /// Raw source loaded (for Source rendering mode).
    RawSourceLoaded(Result<String, String>),
    /// User changed the rendering mode toggle.
    SetRenderingMode(RenderingMode),
    /// Reply/Reply All/Forward button pressed.
    Reply,
    ReplyAll,
    Forward,
    /// Overflow menu actions.
    Archive,
    Delete,
    Print,
    SaveAs,
    /// Overflow menu toggle.
    ToggleOverflowMenu,
    /// Load remote content in Original HTML mode.
    LoadRemoteContent,
    /// No-op (placeholder for unimplemented actions).
    Noop,
}

// ── State ──────────────────────────────────────────────

/// Per-window state for a message view pop-out.
#[derive(Debug, Clone)]
pub struct MessageViewState {
    // ── Identity ──
    pub message_id: String,
    pub thread_id: String,
    pub account_id: String,

    // ── Header data ──
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub cc_addresses: Option<String>,
    pub subject: Option<String>,
    pub date: Option<i64>,

    // ── Body ──
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    /// Snippet fallback used before async body load completes.
    pub snippet: Option<String>,
    /// Raw email source (headers + MIME), loaded lazily on Source mode.
    pub raw_source: Option<String>,

    // ── Attachments ──
    pub attachments: Vec<MessageViewAttachment>,

    // ── Window-local state ──
    pub rendering_mode: RenderingMode,
    pub overflow_menu_open: bool,
    pub remote_content_loaded: bool,
    pub error_banner: Option<String>,

    // ── Action context ──
    /// The sidebar selection when the pop-out was opened. Used for action
    /// resolution (spam toggle direction, trash undo source folder) so the
    /// pop-out resolves against its original context, not whatever the main
    /// window happens to show later.
    pub source_selection: Option<types::SidebarSelection>,

    // ── Window geometry ──
    pub width: f32,
    pub height: f32,
    pub x: Option<f32>,
    pub y: Option<f32>,

    // ── Generation tracking ──
    /// Per-window generation token to guard against stale data loads.
    pub generation: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
}

impl MessageViewState {
    pub fn from_thread_message(
        msg: &ThreadMessage,
        generation: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
        source_selection: Option<types::SidebarSelection>,
        default_rendering_mode: RenderingMode,
    ) -> Self {
        Self {
            message_id: msg.id.clone(),
            thread_id: msg.thread_id.clone(),
            account_id: msg.account_id.clone(),
            from_name: msg.from_name.clone(),
            from_address: msg.from_address.clone(),
            to_addresses: msg.to_addresses.clone(),
            cc_addresses: msg.cc_addresses.clone(),
            subject: msg.subject.clone(),
            date: msg.date,
            body_text: None,
            body_html: None,
            snippet: msg.snippet.clone(),
            raw_source: None,
            attachments: Vec::new(),
            rendering_mode: default_rendering_mode,

            overflow_menu_open: false,
            remote_content_loaded: false,
            error_banner: None,
            source_selection,
            width: MESSAGE_VIEW_DEFAULT_WIDTH,
            height: MESSAGE_VIEW_DEFAULT_HEIGHT,
            x: None,
            y: None,
            generation,
        }
    }

    /// Construct state from a session restore entry (minimal data, body loaded async).
    pub fn from_session_entry(
        entry: &MessageViewSessionEntry,
        generation: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
        default_rendering_mode: RenderingMode,
    ) -> Self {
        Self {
            message_id: entry.message_id.clone(),
            thread_id: entry.thread_id.clone(),
            account_id: entry.account_id.clone(),
            from_name: None,
            from_address: None,
            to_addresses: None,
            cc_addresses: None,
            subject: None,
            date: None,
            body_text: None,
            body_html: None,
            snippet: None,
            raw_source: None,
            attachments: Vec::new(),
            rendering_mode: default_rendering_mode,

            overflow_menu_open: false,
            remote_content_loaded: false,
            error_banner: None,
            source_selection: None, // not available on session restore
            width: entry.width,
            height: entry.height,
            x: entry.x,
            y: entry.y,
            generation,
        }
    }

    /// Check if a loaded result matches this window's current generation.
    pub fn is_current_generation(
        &self,
        token: rtsk::generation::GenerationToken<rtsk::generation::PopOut>,
    ) -> bool {
        self.generation == token
    }
}

// ── View ───────────────────────────────────────────────

/// Render the message view window content.
pub fn view_message_window<'a>(
    window_id: iced::window::Id,
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let header = message_view_header(window_id, state);
    let mode_toggle = rendering_mode_toggle(state.rendering_mode, window_id);
    let body = message_view_body(window_id, state);

    let mut content = column![header, widgets::divider(), mode_toggle].spacing(SPACE_0);

    // Error banner (shown above body for failed loads / deleted messages)
    if let Some(ref error) = state.error_banner {
        content = content.push(error_banner_view(error));
    }

    // Remote content banner for Original HTML mode
    if state.rendering_mode == RenderingMode::OriginalHtml && !state.remote_content_loaded {
        content = content.push(remote_content_banner(window_id));
    }

    content = content.push(body);

    if !state.attachments.is_empty() {
        content = content.push(widgets::divider());
        content = content.push(message_view_attachments(&state.attachments));
    }

    let scrollable_content = scrollable(content)
        .spacing(SCROLLBAR_SPACING)
        .height(Length::Fill);

    container(scrollable_content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style())
        .into()
}

// ── Header ─────────────────────────────────────────────

fn message_view_header<'a>(
    window_id: iced::window::Id,
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let sender_name = state
        .from_name
        .as_deref()
        .or(state.from_address.as_deref())
        .unwrap_or("(unknown)");
    let sender_email = state.from_address.as_deref().unwrap_or("");

    // Action buttons + overflow menu (right-aligned on sender name row)
    let actions = action_buttons_with_overflow(window_id, state);

    // From row: name + email + actions
    let from_row = row![
        column![
            text(sender_name)
                .size(TEXT_LG)
                .font(font::text_semibold())
                .style(text::base),
            text(sender_email)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
        actions,
    ]
    .align_y(Alignment::Start);

    // To row
    let to_text = state.to_addresses.as_deref().unwrap_or("");
    let mut header_fields = column![from_row].spacing(SPACE_XS);

    if !to_text.is_empty() {
        header_fields = header_fields.push(
            row![
                text("To: ")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
                text(to_text).size(TEXT_SM).style(text::secondary),
            ]
            .spacing(SPACE_XXS),
        );
    }

    // Cc row (if present)
    if let Some(ref cc) = state.cc_addresses {
        if !cc.is_empty() {
            header_fields = header_fields.push(
                row![
                    text("Cc: ")
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                    text(cc.as_str()).size(TEXT_SM).style(text::secondary),
                ]
                .spacing(SPACE_XXS),
            );
        }
    }

    // Subject + date row
    let date_str = format_date(state.date).trim().to_string();
    let subject = state.subject.as_deref().unwrap_or("(no subject)");
    let subject_row = row![
        text(subject)
            .size(TEXT_HEADING)
            .style(text::base)
            .width(Length::Fill),
        text(date_str)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    ]
    .align_y(Alignment::End);

    header_fields = header_fields.push(subject_row);

    container(header_fields)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

// ── Action buttons with overflow menu ──────────────────

fn action_buttons_with_overflow<'a>(
    window_id: iced::window::Id,
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let reply_btn = widgets::action_icon_button(
        icon::reply(),
        "Reply",
        Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::Reply),
        ),
    );
    let reply_all_btn = widgets::action_icon_button(
        icon::reply_all(),
        "Reply All",
        Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::ReplyAll),
        ),
    );
    let forward_btn = widgets::action_icon_button(
        icon::forward(),
        "Forward",
        Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::Forward),
        ),
    );
    let overflow = overflow_menu(state.overflow_menu_open, window_id);

    row![reply_btn, reply_all_btn, forward_btn, overflow]
        .spacing(SPACE_XXS)
        .into()
}

// ── Overflow menu ──────────────────────────────────────

fn overflow_menu<'a>(open: bool, window_id: iced::window::Id) -> Element<'a, Message> {
    let trigger = button(
        icon::ellipsis_vertical()
            .size(ICON_MD)
            .style(text::secondary),
    )
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::MessageView(MessageViewMessage::ToggleOverflowMenu),
    ))
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::BareIcon.style());

    if !open {
        return trigger.into();
    }

    let menu_items = column![
        overflow_menu_item(
            icon::archive(),
            "Archive",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Archive),
            ),
        ),
        overflow_menu_item(
            icon::trash(),
            "Delete",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Delete),
            ),
        ),
        overflow_menu_item(
            icon::printer(),
            "Print",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Print),
            ),
        ),
        overflow_menu_item(
            icon::download(),
            "Save As",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::SaveAs),
            ),
        ),
    ]
    .spacing(SPACE_XXS);

    let menu = container(menu_items)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style());

    crate::ui::popover::popover(trigger)
        .popup(menu)
        .position(crate::ui::popover::Position::BelowRight)
        .on_dismiss(Message::PopOut(
            window_id,
            PopOutMessage::MessageView(MessageViewMessage::ToggleOverflowMenu),
        ))
        .into()
}

fn overflow_menu_item<'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: Message,
) -> Element<'a, Message> {
    button(
        row![
            container(ico.size(ICON_MD).style(text::secondary)).align_y(Alignment::Center),
            container(text(label).size(TEXT_MD).style(text::base)).align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_NAV_ITEM)
    .height(DROPDOWN_ITEM_HEIGHT)
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .into()
}

// ── Rendering mode toggle ──────────────────────────────

fn rendering_mode_toggle<'a>(
    current: RenderingMode,
    window_id: iced::window::Id,
) -> Element<'a, Message> {
    let modes = [
        (RenderingMode::PlainText, "Plain Text"),
        (RenderingMode::SimpleHtml, "Simple HTML"),
        (RenderingMode::OriginalHtml, "Original HTML"),
        (RenderingMode::Source, "Source"),
    ];

    let mut toggle_row = row![].spacing(SPACE_XS);
    for (mode, label) in modes {
        let is_active = current == mode;
        toggle_row = toggle_row.push(
            button(text(label).size(TEXT_SM).style(if is_active {
                text::primary
            } else {
                text::secondary
            }))
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::SetRenderingMode(mode)),
            ))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::Chip { active: is_active }.style()),
        );
    }

    container(toggle_row)
        .padding(Padding {
            top: 0.0,
            right: SPACE_LG,
            bottom: SPACE_SM,
            left: SPACE_LG,
        })
        .width(Length::Fill)
        .into()
}

// ── Body ───────────────────────────────────────────────

fn message_view_body<'a>(
    _window_id: iced::window::Id,
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let body_content: Element<'_, Message> = match state.rendering_mode {
        RenderingMode::PlainText => {
            let txt = state
                .body_text
                .as_deref()
                .or(state.snippet.as_deref())
                .unwrap_or("(no content)");
            text(txt).size(TEXT_LG).style(text::secondary).into()
        }
        RenderingMode::SimpleHtml | RenderingMode::OriginalHtml => {
            // Phase 3 placeholder: fall back to plain text until HTML pipeline exists
            let txt = state
                .body_text
                .as_deref()
                .or(state.snippet.as_deref())
                .unwrap_or("(no content)");
            text(txt).size(TEXT_LG).style(text::secondary).into()
        }
        RenderingMode::Source => {
            let src = state.raw_source.as_deref().unwrap_or("Loading source...");
            text(src)
                .size(TEXT_SM)
                .font(font::monospace())
                .style(text::secondary)
                .into()
        }
    };

    container(body_content)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

// ── Error banner ───────────────────────────────────────

fn error_banner_view<'a>(error: &'a str) -> Element<'a, Message> {
    container(
        row![
            icon::alert_triangle().size(ICON_MD).style(text::secondary),
            text(error)
                .size(TEXT_MD)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CARD)
    .style(theme::ContainerClass::Elevated.style())
    .width(Length::Fill)
    .into()
}

// ── Remote content banner ──────────────────────────────

fn remote_content_banner<'a>(window_id: iced::window::Id) -> Element<'a, Message> {
    container(
        row![
            text("Remote content is blocked.")
                .size(TEXT_SM)
                .style(text::secondary),
            button(
                text("Load for this message")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Accent.style()),
            )
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::LoadRemoteContent,),
            ))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::Ghost.style()),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CARD)
    .style(theme::ContainerClass::Elevated.style())
    .width(Length::Fill)
    .into()
}

// ── Attachments ────────────────────────────────────────

fn message_view_attachments<'a>(attachments: &'a [MessageViewAttachment]) -> Element<'a, Message> {
    let header = text(format!("Attachments ({})", attachments.len()))
        .size(TEXT_MD)
        .font(font::text_semibold())
        .style(text::base);

    let mut col = column![header].spacing(SPACE_XS);

    for att in attachments {
        let filename = att.filename.as_deref().unwrap_or("(unnamed)");
        let file_ico = file_type_icon(att.mime_type.as_deref());
        let size_str = format_file_size(att.size);
        let type_label = mime_to_type_label(att.mime_type.as_deref());
        let meta = format!("{type_label} \u{00B7} {size_str}");

        let card = container(
            column![
                row![
                    container(file_ico.size(ICON_MD).style(text::secondary))
                        .align_y(Alignment::Center),
                    container(text(filename).size(TEXT_MD).style(text::base))
                        .align_y(Alignment::Center),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
                text(meta)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ]
            .spacing(SPACE_XXXS),
        )
        .padding(PAD_NAV_ITEM)
        .style(theme::ContainerClass::Elevated.style())
        .width(Length::Fill);

        col = col.push(card);
    }

    container(col)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

// ── Helpers ────────────────────────────────────────────

fn format_date(timestamp: Option<i64>) -> String {
    timestamp
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%a, %b %d, %Y, %l:%M %p").to_string())
        .unwrap_or_default()
}

fn file_type_icon<'a>(mime_type: Option<&str>) -> iced::widget::Text<'a> {
    match mime_type.unwrap_or("") {
        t if t.starts_with("image/") => icon::image(),
        t if t.contains("pdf") => icon::file_text(),
        t if t.contains("spreadsheet") || t.contains("excel") => icon::file_spreadsheet(),
        _ => icon::file(),
    }
}

fn mime_to_type_label(mime: Option<&str>) -> &'static str {
    match mime.unwrap_or("") {
        t if t.starts_with("image/") => "Image",
        t if t.contains("pdf") => "PDF",
        t if t.contains("spreadsheet") || t.contains("excel") => "Excel",
        t if t.contains("word") || t.contains("document") => "Word",
        t if t.contains("zip") || t.contains("archive") => "Archive",
        _ => "File",
    }
}

fn format_file_size(size: Option<i64>) -> String {
    match size {
        None => "\u{2014}".to_string(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => {
            format!("{:.0} KB", b as f64 / 1024.0)
        }
        Some(b) => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}
