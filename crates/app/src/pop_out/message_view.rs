use iced::widget::{column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::db::{self, ThreadMessage};
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

use super::PopOutMessage;

pub use db::MessageViewAttachment;

// ── Messages ───────────────────────────────────────────

#[derive(Debug, Clone)]
pub enum MessageViewMessage {
    /// Body content loaded from the body store.
    BodyLoaded(Result<(Option<String>, Option<String>), String>),
    /// Attachments loaded for this message.
    AttachmentsLoaded(Result<Vec<MessageViewAttachment>, String>),
    /// Reply/Reply All/Forward button pressed (stubs).
    Reply,
    ReplyAll,
    Forward,
    /// No-op (placeholder for unimplemented actions).
    Noop,
}

// ── State ──────────────────────────────────────────────

/// Per-window state for a message view pop-out.
#[derive(Debug, Clone)]
pub struct MessageViewState {
    // Identity
    pub message_id: String,
    pub thread_id: String,
    pub account_id: String,

    // Header data
    pub from_name: Option<String>,
    pub from_address: Option<String>,
    pub to_addresses: Option<String>,
    pub subject: Option<String>,
    pub date: Option<i64>,

    // Body
    pub body_text: Option<String>,
    pub body_html: Option<String>,
    /// Snippet fallback used before async body load completes.
    pub snippet: Option<String>,

    // Attachments
    pub attachments: Vec<MessageViewAttachment>,

    // Window geometry
    pub width: f32,
    pub height: f32,
}

impl MessageViewState {
    pub fn from_thread_message(msg: &ThreadMessage) -> Self {
        Self {
            message_id: msg.id.clone(),
            thread_id: msg.thread_id.clone(),
            account_id: msg.account_id.clone(),
            from_name: msg.from_name.clone(),
            from_address: msg.from_address.clone(),
            to_addresses: msg.to_addresses.clone(),
            subject: msg.subject.clone(),
            date: msg.date,
            body_text: None,
            body_html: None,
            snippet: msg.snippet.clone(),
            attachments: Vec::new(),
            width: MESSAGE_VIEW_DEFAULT_WIDTH,
            height: MESSAGE_VIEW_DEFAULT_HEIGHT,
        }
    }
}

// ── View ───────────────────────────────────────────────

/// Render the message view window content.
pub fn view_message_window<'a>(
    window_id: iced::window::Id,
    state: &'a MessageViewState,
) -> Element<'a, Message> {
    let header = message_view_header(window_id, state);
    let body = message_view_body(state);

    let mut content = column![header, widgets::divider(), body].spacing(SPACE_0);

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

    // Action buttons (right-aligned on sender name row)
    let actions = action_buttons(window_id);

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

fn action_buttons(window_id: iced::window::Id) -> Element<'static, Message> {
    row![
        widgets::action_icon_button(
            icon::reply(),
            "Reply",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Reply),
            ),
        ),
        widgets::action_icon_button(
            icon::reply_all(),
            "Reply All",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::ReplyAll),
            ),
        ),
        widgets::action_icon_button(
            icon::forward(),
            "Forward",
            Message::PopOut(
                window_id,
                PopOutMessage::MessageView(MessageViewMessage::Forward),
            ),
        ),
    ]
    .spacing(SPACE_XXS)
    .into()
}

// ── Body ───────────────────────────────────────────────

fn message_view_body<'a>(state: &'a MessageViewState) -> Element<'a, Message> {
    let txt = state
        .body_text
        .as_deref()
        .or(state.snippet.as_deref())
        .unwrap_or("(no content)");

    let body_content: Element<'_, Message> =
        text(txt).size(TEXT_LG).style(text::secondary).into();

    container(body_content)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

// ── Attachments ────────────────────────────────────────

fn message_view_attachments<'a>(
    attachments: &'a [MessageViewAttachment],
) -> Element<'a, Message> {
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
        t if t.contains("spreadsheet") || t.contains("excel") => {
            icon::file_spreadsheet()
        }
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
        Some(b) if b < 1024 * 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        Some(b) => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
}
