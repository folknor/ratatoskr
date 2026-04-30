use std::collections::{HashMap, HashSet};

use iced::widget::{Space, column, container, image, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::component::Component;
use crate::ui::layout::*;
use crate::ui::theme;

use chrono::TimeZone;
use rtsk::chat::ChatMessage;

// ── Messages & Events ──────────────────────────────────

#[derive(Debug, Clone)]
pub enum ChatTimelineMessage {
    /// User clicked "show full message" on a bubble.
    ToggleExpand(String),
    /// User wants to load older messages.
    LoadOlder,
}

#[derive(Debug, Clone)]
pub enum ChatTimelineEvent {
    /// Request to load older messages.
    LoadOlderRequested,
}

// ── State ──────────────────────────────────────────────

/// Page size for chat timeline loads. Anything smaller than this in a
/// returned page means we've hit the start of history.
pub const CHAT_TIMELINE_PAGE: usize = 50;

/// Stable widget id for the chat timeline scrollable. The handler issues
/// `snap_to_end` against this id after the initial load so the latest
/// message is visible without the user having to scroll.
pub const CHAT_SCROLLABLE_ID: &str = "chat-timeline-scroll";

pub struct ChatTimeline {
    pub messages: Vec<ChatMessage>,
    pub loading: bool,
    pub contact_email: String,
    /// Whether more (older) messages may exist on the server. Set to true
    /// pessimistically until a load returns fewer than `CHAT_TIMELINE_PAGE`
    /// rows, at which point we know we've reached the start.
    pub has_more: bool,
    /// Precomputed image handles keyed by (message_id, inline_image_index).
    ///
    /// `image::Handle::from_bytes` calls `Id::unique()` internally, so
    /// constructing handles in `view()` thrashes iced's GPU cache: every
    /// frame the renderer sees a new handle id, re-decodes the PNG, and
    /// re-uploads to the GPU. Doing it once when messages arrive and
    /// reusing the same `Handle` (cheap to clone - shares underlying bytes
    /// via `Arc`) keeps the cache stable.
    pub image_handles: HashMap<(String, usize), image::Handle>,
    expanded: HashSet<String>,
}

impl ChatTimeline {
    pub fn new(contact_email: String) -> Self {
        Self {
            messages: Vec::new(),
            loading: true,
            contact_email,
            has_more: true,
            image_handles: HashMap::new(),
            expanded: HashSet::new(),
        }
    }

    /// Precompute image handles for any messages in `messages` whose images
    /// don't yet have cached handles. Idempotent: messages already in the
    /// cache are left alone.
    pub fn refresh_image_handles(&mut self) {
        for msg in &self.messages {
            for (idx, img) in msg.inline_images.iter().enumerate() {
                let key = (msg.message_id.clone(), idx);
                self.image_handles
                    .entry(key)
                    .or_insert_with(|| image::Handle::from_bytes(img.bytes.clone()));
            }
        }
    }
}

impl Component for ChatTimeline {
    type Message = ChatTimelineMessage;
    type Event = ChatTimelineEvent;

    fn update(
        &mut self,
        message: ChatTimelineMessage,
    ) -> (iced::Task<ChatTimelineMessage>, Option<ChatTimelineEvent>) {
        match message {
            ChatTimelineMessage::ToggleExpand(id) => {
                if self.expanded.contains(&id) {
                    self.expanded.remove(&id);
                } else {
                    self.expanded.insert(id);
                }
                (iced::Task::none(), None)
            }
            ChatTimelineMessage::LoadOlder => (
                iced::Task::none(),
                Some(ChatTimelineEvent::LoadOlderRequested),
            ),
        }
    }

    fn view(&self) -> Element<'_, ChatTimelineMessage> {
        if self.loading && self.messages.is_empty() {
            return container(
                text("Loading...")
                    .size(TEXT_MD)
                    .style(theme::TextClass::Muted.style()),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        }

        let mut col = column![].spacing(CHAT_BUBBLE_SPACING).padding(SPACE_MD);

        // "Load older" button at top - only when there's actually more
        // history to fetch.
        if !self.messages.is_empty() && self.has_more {
            let load_btn = iced::widget::button(
                text("Load older messages")
                    .size(TEXT_MD)
                    .style(theme::TextClass::Accent.style()),
            )
            .on_press(ChatTimelineMessage::LoadOlder)
            .padding(SPACE_XS)
            .style(theme::ButtonClass::Ghost.style());
            col = col.push(container(load_btn).center_x(Length::Fill).padding(SPACE_SM));
        }

        let mut prev: Option<&ChatMessage> = None;

        for msg in &self.messages {
            // Date separator
            if let Some(p) = prev {
                if needs_date_separator(p, msg) {
                    col = col.push(Space::new().height(CHAT_DATE_SEPARATOR_SPACING));
                    col = col.push(date_separator(msg.date));
                    col = col.push(Space::new().height(CHAT_DATE_SEPARATOR_SPACING));
                }

                // Subject change indicator
                if needs_subject_indicator(p, msg)
                    && let Some(ref subj) = msg.subject
                {
                    col = col.push(
                        text(subj)
                            .size(TEXT_MD)
                            .style(theme::TextClass::Muted.style()),
                    );
                }

                // Extra spacing on sender change
                if p.is_from_user != msg.is_from_user {
                    col = col.push(Space::new().height(CHAT_GROUP_SPACING - CHAT_BUBBLE_SPACING));
                }
            } else {
                // First message - add date separator
                col = col.push(date_separator(msg.date));
                col = col.push(Space::new().height(CHAT_DATE_SEPARATOR_SPACING));
            }

            col = col.push(chat_bubble(
                msg,
                &self.image_handles,
                self.expanded.contains(&msg.message_id),
            ));
            prev = Some(msg);
        }

        scrollable(col)
            .id(CHAT_SCROLLABLE_ID.to_string())
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    }
}

// ── Bubble rendering ───────────────────────────────────

fn chat_bubble<'a>(
    msg: &'a ChatMessage,
    image_handles: &'a HashMap<(String, usize), image::Handle>,
    _expanded: bool,
) -> Element<'a, ChatTimelineMessage> {
    let body = msg
        .body_text
        .as_deref()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .or_else(|| msg.subject.clone())
        .unwrap_or_else(|| "(no content)".to_string());

    let content = column![
        text(body).size(TEXT_MD),
        text(format_time(msg.date))
            .size(TEXT_SM)
            .style(theme::TextClass::Muted.style()),
    ]
    .spacing(SPACE_XXXS);

    let style = if msg.is_from_user {
        theme::ContainerClass::ChatBubbleSent.style()
    } else {
        theme::ContainerClass::ChatBubbleReceived.style()
    };

    let bubble = container(content)
        .padding(PAD_CHAT_BUBBLE)
        .max_width(CHAT_BUBBLE_MAX_WIDTH)
        .style(style);

    // Inline images render as separate "image bubbles" above the text bubble -
    // same alignment as the sender, no extra padding/background. Keeps the
    // chat-app feel ("photos appear inline") while sidestepping the complexity
    // of injecting <img> nodes into a rendered HTML body.
    let mut col = column![].spacing(SPACE_XXS).align_x(if msg.is_from_user {
        Alignment::End
    } else {
        Alignment::Start
    });
    for (i, _img) in msg.inline_images.iter().enumerate() {
        if let Some(handle) = image_handles.get(&(msg.message_id.clone(), i)) {
            col = col.push(inline_image_widget(handle));
        }
    }
    col = col.push(bubble);

    if msg.is_from_user {
        // Right-aligned: spacer + content column.
        row![Space::new().width(Length::Fill), col]
            .align_y(Alignment::End)
            .width(Length::Fill)
            .into()
    } else {
        // Left-aligned: content column + spacer.
        row![col, Space::new().width(Length::Fill)]
            .align_y(Alignment::Start)
            .width(Length::Fill)
            .into()
    }
}

/// Render an inline image bubble using a handle whose `Id` was assigned once
/// (when the message was loaded) so iced's GPU cache stays stable across
/// view cycles. Cloning the handle is cheap - shares the underlying bytes
/// via `Arc`.
fn inline_image_widget<'a>(handle: &'a image::Handle) -> Element<'a, ChatTimelineMessage> {
    container(
        image(handle.clone())
            .width(Length::Shrink)
            .height(Length::Shrink)
            .content_fit(iced::ContentFit::Contain),
    )
    .max_width(CHAT_BUBBLE_MAX_WIDTH)
    .into()
}

fn date_separator<'a>(timestamp: i64) -> Element<'a, ChatTimelineMessage> {
    let label = format_date_label(timestamp);
    container(
        text(label)
            .size(TEXT_MD)
            .style(theme::TextClass::Muted.style()),
    )
    .center_x(Length::Fill)
    .width(Length::Fill)
    .into()
}

// ── Helpers ────────────────────────────────────────────

fn needs_date_separator(prev: &ChatMessage, curr: &ChatMessage) -> bool {
    local_date(prev.date) != local_date(curr.date)
}

fn needs_subject_indicator(prev: &ChatMessage, curr: &ChatMessage) -> bool {
    prev.thread_id != curr.thread_id
        && normalize_subject(curr.subject.as_deref().unwrap_or_default())
            != normalize_subject(prev.subject.as_deref().unwrap_or_default())
}

/// Strip leading Re:/Fwd:/Fw: prefixes (case-insensitive, repeated) so that
/// "Re: hello" and "Re: Re: hello" compare as equal.
fn normalize_subject(s: &str) -> String {
    let mut s = s.trim().to_lowercase();
    while let Some(rest) = s
        .strip_prefix("re:")
        .or_else(|| s.strip_prefix("fwd:"))
        .or_else(|| s.strip_prefix("fw:"))
    {
        s = rest.trim_start().to_string();
    }
    s
}

fn local_date(timestamp: i64) -> chrono::NaiveDate {
    chrono::Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| dt.date_naive())
        .unwrap_or_default()
}

fn format_date_label(timestamp: i64) -> String {
    let today = chrono::Local::now().date_naive();
    let msg_date = local_date(timestamp);

    if msg_date == today {
        "Today".to_string()
    } else if msg_date == today.pred_opt().unwrap_or(today) {
        "Yesterday".to_string()
    } else {
        msg_date.format("%B %e").to_string()
    }
}

fn format_time(timestamp: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|d| d.format("%H:%M").to_string())
        .unwrap_or_default()
}
