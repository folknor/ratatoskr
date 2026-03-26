use std::collections::HashSet;

use iced::widget::{column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

use crate::ui::layout::*;
use crate::ui::theme;

use chrono::TimeZone;
use ratatoskr_core::chat::ChatMessage;

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

pub struct ChatTimeline {
    pub messages: Vec<ChatMessage>,
    pub loading: bool,
    pub contact_email: String,
    pub scroll_id: String,
    expanded: HashSet<String>,
}

impl ChatTimeline {
    pub fn new(contact_email: String) -> Self {
        Self {
            messages: Vec::new(),
            loading: true,
            scroll_id: format!("chat-timeline-{contact_email}"),
            contact_email,
            expanded: HashSet::new(),
        }
    }

    pub fn update(
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
            ChatTimelineMessage::LoadOlder => {
                (iced::Task::none(), Some(ChatTimelineEvent::LoadOlderRequested))
            }
        }
    }

    pub fn view(&self) -> Element<'_, ChatTimelineMessage> {
        if self.loading && self.messages.is_empty() {
            return container(
                text("Loading...").size(TEXT_SM).style(theme::TextClass::Muted.style()),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .width(Length::Fill)
            .height(Length::Fill)
            .into();
        }

        let mut col = column![].spacing(CHAT_BUBBLE_SPACING).padding(SPACE_MD);

        // "Load older" button at top
        if !self.messages.is_empty() {
            let load_btn = iced::widget::button(
                text("Load older messages")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Accent.style()),
            )
            .on_press(ChatTimelineMessage::LoadOlder)
            .padding(SPACE_XS)
            .style(theme::ButtonClass::Ghost.style());
            col = col.push(
                container(load_btn).center_x(Length::Fill).padding(SPACE_SM),
            );
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
                if needs_subject_indicator(p, msg) {
                    if let Some(ref subj) = msg.subject {
                        col = col.push(
                            text(subj)
                                .size(TEXT_SM)
                                .style(theme::TextClass::Muted.style()),
                        );
                    }
                }

                // Extra spacing on sender change
                if p.is_from_user != msg.is_from_user {
                    col = col.push(Space::new().height(
                        CHAT_GROUP_SPACING - CHAT_BUBBLE_SPACING,
                    ));
                }
            } else {
                // First message — add date separator
                col = col.push(date_separator(msg.date));
                col = col.push(Space::new().height(CHAT_DATE_SEPARATOR_SPACING));
            }

            col = col.push(chat_bubble(msg, self.expanded.contains(&msg.message_id)));
            prev = Some(msg);
        }

        scrollable(col)
            .height(Length::Fill)
            .width(Length::Fill)
            .into()
    }
}

// ── Bubble rendering ───────────────────────────────────

fn chat_bubble<'a>(
    msg: &'a ChatMessage,
    _expanded: bool,
) -> Element<'a, ChatTimelineMessage> {
    // TODO: load body text from body store. For now use subject as placeholder.
    let body = msg.subject.as_deref().unwrap_or("(no content)");

    let content = column![
        text(body).size(TEXT_SM),
        text(format_time(msg.date))
            .size(TEXT_XS)
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

    if msg.is_from_user {
        // Right-aligned: spacer + bubble
        row![
            Space::new().width(Length::Fill),
            bubble,
        ]
        .align_y(Alignment::End)
        .width(Length::Fill)
        .into()
    } else {
        // Left-aligned: bubble + spacer
        row![
            bubble,
            Space::new().width(Length::Fill),
        ]
        .align_y(Alignment::Start)
        .width(Length::Fill)
        .into()
    }
}

fn date_separator<'a>(timestamp: i64) -> Element<'a, ChatTimelineMessage> {
    let label = format_date_label(timestamp);
    container(
        text(label)
            .size(TEXT_SM)
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
        && curr.subject != prev.subject
}

fn local_date(timestamp: i64) -> chrono::NaiveDate {
    chrono::Local
        .timestamp_opt(timestamp, 0)
        .single()
        .map(|dt| dt.date_naive())
        .unwrap_or_default()
}

fn format_date_label(timestamp: i64) -> String {
    use chrono::TimeZone;
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
