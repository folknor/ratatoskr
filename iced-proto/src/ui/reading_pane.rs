use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length};

use crate::db::Thread;
use crate::icon;
use crate::ui::layout::*;
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
        .into()
}

fn empty_state<'a>() -> Element<'a, Message> {
    container(
        column![
            text("No conversation selected").size(16).style(theme::text_tertiary),
            text("Select a thread to read").size(12).style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXS)
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
            icon_btn(icon::reply(), "Reply"),
            icon_btn(icon::reply_all(), "Reply All"),
            icon_btn(icon::forward(), "Forward"),
            Space::new().width(SPACE_XS),
            icon_btn(icon::archive(), "Archive"),
            icon_btn(icon::trash(), "Delete"),
            icon_btn(icon::star(), "Star"),
            icon_btn(icon::clock(), "Snooze"),
            icon_btn(icon::pin(), "Pin"),
            Space::new().width(Length::Fill),
            icon_btn(icon::printer(), "Print"),
            icon_btn(icon::external_link(), "Pop-out"),
        ]
        .spacing(SPACE_XXXS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_TOOLBAR)
    .width(Length::Fill)
    .style(theme::action_bar_container);
    col = col.push(actions);

    // ── Thread header ───────────────────────────────────
    let header = container(
        column![
            text(subject).size(18).style(text::base),
            text(format!("{} messages in this thread", thread.message_count))
                .size(11)
                .style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_CONTENT);
    col = col.push(header);

    // ── Message items (simulated) ───────────────────────
    let mut messages = column![].spacing(SPACE_XS).padding(iced::Padding::from([0.0, SPACE_LG]));

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
                text(sender).size(13).style(text::base),
                Space::new().width(Length::Fill),
                text(date_str).size(11).style(theme::text_tertiary),
            ],
            text(
                thread
                    .from_address
                    .as_deref()
                    .unwrap_or(""),
            )
            .size(11)
            .style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    // Message body placeholder
    let body_text = thread.snippet.as_deref().unwrap_or("(no preview available)");
    let msg_body = container(
        text(body_text).size(13).style(text::secondary),
    )
    .padding(iced::Padding::from([SPACE_SM, 0.0]));

    let message_card = container(
        column![msg_header, msg_body].spacing(SPACE_XS),
    )
    .padding(PAD_CARD)
    .width(Length::Fill)
    .style(theme::message_card_container);

    messages = messages.push(message_card);
    messages = messages.push(Space::new().height(SPACE_MD));

    // ── Reply buttons ───────────────────────────────────
    let reply_bar = container(
        row![
            reply_btn(icon::reply(), "Reply"),
            reply_btn(icon::reply_all(), "Reply All"),
            reply_btn(icon::forward(), "Forward"),
        ]
        .spacing(SPACE_XS),
    )
    .padding(iced::Padding::from([0.0, SPACE_LG]));

    col = col.push(scrollable(
        column![messages, reply_bar].spacing(0),
    ).height(Length::Fill));

    col.into()
}

fn icon_btn<'a>(ico: iced::widget::Text<'a>, label: &'a str) -> Element<'a, Message> {
    button(
        row![
            ico.size(12).style(text::secondary),
            text(label).size(11).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::Noop)
    .padding(PAD_ICON_BTN)
    .style(theme::action_button)
    .into()
}

fn reply_btn<'a>(ico: iced::widget::Text<'a>, label: &'a str) -> Element<'a, Message> {
    button(
        row![
            ico.size(14).style(text::secondary),
            text(label).size(12).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::Noop)
    .padding(PAD_BUTTON)
    .style(button::secondary)
    .into()
}
