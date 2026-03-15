use iced::widget::{column, container, row, scrollable, Space};
use iced::{Alignment, Element, Length};

use crate::db::Thread;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(thread: Option<&'a Thread>) -> Element<'a, Message> {
    let content = match thread {
        None => widgets::empty_placeholder("No conversation selected", "Select a thread to read"),
        Some(t) => thread_view(t),
    };

    container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

fn thread_view(thread: &Thread) -> Element<'_, Message> {
    let subject = thread.subject.as_deref().unwrap_or("(no subject)");
    let mut col = column![].spacing(0).width(Length::Fill);

    // ── Action bar ──────────────────────────────────────
    col = col.push(
        container(
            row![
                widgets::action_icon_button(icon::reply(), "Reply"),
                widgets::action_icon_button(icon::reply_all(), "Reply All"),
                widgets::action_icon_button(icon::forward(), "Forward"),
                Space::new().width(SPACE_XS),
                widgets::action_icon_button(icon::archive(), "Archive"),
                widgets::action_icon_button(icon::trash(), "Delete"),
                widgets::action_icon_button(icon::star(), "Star"),
                widgets::action_icon_button(icon::clock(), "Snooze"),
                widgets::action_icon_button(icon::pin(), "Pin"),
                Space::new().width(Length::Fill),
                widgets::action_icon_button(icon::printer(), "Print"),
                widgets::action_icon_button(icon::external_link(), "Pop-out"),
            ]
            .spacing(SPACE_XXXS)
            .align_y(Alignment::Center),
        )
        .padding(PAD_TOOLBAR)
        .width(Length::Fill)
        .style(theme::action_bar_container),
    );

    // ── Thread header ───────────────────────────────────
    col = col.push(
        container(
            column![
                iced::widget::text(subject).size(18).style(iced::widget::text::base),
                iced::widget::text(format!("{} messages in this thread", thread.message_count))
                    .size(11)
                    .style(theme::text_tertiary),
            ]
            .spacing(SPACE_XXS),
        )
        .padding(PAD_CONTENT),
    );

    // ── Messages ────────────────────────────────────────
    let mut messages = column![].spacing(SPACE_XS).padding(iced::Padding::from([0.0, SPACE_LG]));
    messages = messages.push(widgets::message_card(thread));
    messages = messages.push(Space::new().height(SPACE_MD));

    // ── Reply bar ───────────────────────────────────────
    let reply_bar = container(
        row![
            widgets::reply_button(icon::reply(), "Reply"),
            widgets::reply_button(icon::reply_all(), "Reply All"),
            widgets::reply_button(icon::forward(), "Forward"),
        ]
        .spacing(SPACE_XS),
    )
    .padding(iced::Padding::from([0.0, SPACE_LG]));

    col = col.push(
        scrollable(column![messages, reply_bar].spacing(0)).height(Length::Fill),
    );

    col.into()
}
