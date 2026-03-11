use iced::widget::{column, container, row, text, Space};
use iced::{Alignment, Element, Length, Padding};

use crate::db::Thread;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(thread: Option<&'a Thread>) -> Element<'a, Message> {
    let content = match thread {
        None => Space::new().width(0).into(),
        Some(t) => contact_panel(t),
    };

    container(content)
        .width(if thread.is_some() { 240 } else { 0 })
        .height(Length::Fill)
        .style(|_: &iced::Theme| container::Style {
            background: Some(theme::BG_SIDEBAR.into()),
            border: iced::Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn contact_panel(thread: &Thread) -> Element<'_, Message> {
    let sender = thread
        .from_name
        .as_deref()
        .or(thread.from_address.as_deref())
        .unwrap_or("(unknown)");
    let email = thread.from_address.as_deref().unwrap_or("");

    let avatar = widgets::avatar_circle(sender, 56.0);

    let mut col = column![]
        .spacing(4)
        .align_x(iced::Alignment::Center)
        .width(Length::Fill);

    // ── Avatar + name ───────────────────────────────────
    col = col.push(Space::new().height(16));
    col = col.push(container(avatar).center_x(Length::Fill));
    col = col.push(Space::new().height(8));
    col = col.push(
        container(
            text(sender).size(14).color(theme::TEXT_PRIMARY),
        )
        .center_x(Length::Fill),
    );
    col = col.push(
        container(
            text(email).size(11).color(theme::TEXT_TERTIARY),
        )
        .center_x(Length::Fill),
    );

    col = col.push(Space::new().height(16));

    // ── Stats section ───────────────────────────────────
    col = col.push(section_header("STATS"));
    col = col.push(stat_row("Emails", "—"));
    col = col.push(stat_row("First email", "—"));
    col = col.push(stat_row("Last email", "—"));

    col = col.push(Space::new().height(12));

    // ── Notes ───────────────────────────────────────────
    col = col.push(section_header("NOTES"));
    col = col.push(
        container(
            text("No notes yet").size(11).color(theme::TEXT_TERTIARY),
        )
        .padding(Padding::from([4, 12])),
    );

    col = col.push(Space::new().height(12));

    // ── Shared files ────────────────────────────────────
    col = col.push(section_header("SHARED FILES"));
    col = col.push(
        container(
            text("No shared files").size(11).color(theme::TEXT_TERTIARY),
        )
        .padding(Padding::from([4, 12])),
    );

    iced::widget::scrollable(col)
        .height(Length::Fill)
        .into()
}

fn section_header(label: &str) -> Element<'_, Message> {
    container(
        text(label).size(10).color(theme::TEXT_TERTIARY),
    )
    .padding(Padding::from([8, 12]))
    .width(Length::Fill)
    .into()
}

fn stat_row<'a>(label: &'a str, value: &'a str) -> Element<'a, Message> {
    container(
        row![
            text(label).size(11).color(theme::TEXT_TERTIARY),
            Space::new().width(Length::Fill),
            text(value).size(11).color(theme::TEXT_SECONDARY),
        ]
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([2, 12]))
    .width(Length::Fill)
    .into()
}
