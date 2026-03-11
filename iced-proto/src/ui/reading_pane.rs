use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Padding};

use crate::db::Thread;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(thread: Option<&'a Thread>) -> Element<'a, Message> {
    let content = match thread {
        None => empty_state(),
        Some(t) => thread_view(t),
    };

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(|_: &iced::Theme| container::Style {
            background: Some(theme::BG_BASE.into()),
            ..Default::default()
        })
        .into()
}

fn empty_state<'a>() -> Element<'a, Message> {
    container(
        column![
            text("No conversation selected").size(16).color(theme::TEXT_TERTIARY),
            text("Select a thread to read").size(12).color(theme::TEXT_TERTIARY),
        ]
        .spacing(4)
        .align_x(iced::Alignment::Center),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

fn thread_view(thread: &Thread) -> Element<'_, Message> {
    let subject = thread.subject.as_deref().unwrap_or("(no subject)");
    let sender = thread
        .from_name
        .as_deref()
        .or(thread.from_address.as_deref())
        .unwrap_or("(unknown)");

    let mut col = column![].spacing(0).width(Length::Fill);

    // ── Action bar ──────────────────────────────────────
    let actions = container(
        row![
            action_btn("Reply"),
            action_btn("Reply All"),
            action_btn("Forward"),
            Space::new().width(8),
            action_btn("Archive"),
            action_btn("Delete"),
            action_btn("Star"),
            action_btn("Snooze"),
            action_btn("Pin"),
            Space::new().width(Length::Fill),
            action_btn("Print"),
            action_btn("Pop-out"),
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([6, 16]))
    .width(Length::Fill)
    .style(|_: &iced::Theme| container::Style {
        background: Some(theme::BG_SURFACE.into()),
        border: iced::Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    });
    col = col.push(actions);

    // ── Thread header ───────────────────────────────────
    let header = container(
        column![
            text(subject).size(18).color(theme::TEXT_PRIMARY),
            text(format!("{} messages in this thread", thread.message_count))
                .size(11)
                .color(theme::TEXT_TERTIARY),
        ]
        .spacing(4),
    )
    .padding(Padding::from([16, 20]));
    col = col.push(header);

    // ── Message items (simulated) ───────────────────────
    let mut messages = column![].spacing(8).padding(Padding::from([0, 20]));

    // Show a simulated expanded message from the sender
    let avatar = widgets::avatar_circle(sender, 32.0);
    let date_str = thread
        .last_message_at
        .and_then(|ts| {
            chrono::DateTime::from_timestamp(ts, 0)
                .map(|dt| dt.format("%a, %b %d, %Y, %l:%M %p").to_string())
        })
        .unwrap_or_default();

    let msg_header = row![
        avatar,
        column![
            row![
                text(sender).size(13).color(theme::TEXT_PRIMARY),
                Space::new().width(Length::Fill),
                text(date_str).size(11).color(theme::TEXT_TERTIARY),
            ],
            text(
                thread
                    .from_address
                    .as_deref()
                    .unwrap_or(""),
            )
            .size(11)
            .color(theme::TEXT_TERTIARY),
        ]
        .spacing(2)
        .width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Start);

    // Message body placeholder
    let body_text = thread.snippet.as_deref().unwrap_or("(no preview available)");
    let msg_body = container(
        text(body_text).size(13).color(theme::TEXT_SECONDARY),
    )
    .padding(Padding::from([12, 0]));

    let message_card = container(
        column![msg_header, msg_body].spacing(8),
    )
    .padding(Padding::from([12, 16]))
    .width(Length::Fill)
    .style(|_: &iced::Theme| container::Style {
        background: Some(theme::BG_SURFACE.into()),
        border: iced::Border {
            color: theme::BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..Default::default()
    });

    messages = messages.push(message_card);
    messages = messages.push(Space::new().height(16));

    // ── Reply buttons ───────────────────────────────────
    let reply_bar = container(
        row![
            reply_btn("Reply"),
            reply_btn("Reply All"),
            reply_btn("Forward"),
        ]
        .spacing(8),
    )
    .padding(Padding::from([0, 20]));

    col = col.push(scrollable(
        column![messages, reply_bar].spacing(0),
    ).height(Length::Fill));

    col.into()
}

fn action_btn(label: &str) -> Element<'_, Message> {
    button(text(label).size(11).color(theme::TEXT_SECONDARY))
        .on_press(Message::Noop)
        .padding(Padding::from([4, 8]))
        .style(button::text)
        .into()
}

fn reply_btn(label: &str) -> Element<'_, Message> {
    button(
        text(label).size(12).color(theme::TEXT_SECONDARY),
    )
    .on_press(Message::Noop)
    .padding(Padding::from([8, 16]))
    .style(button::secondary)
    .into()
}
