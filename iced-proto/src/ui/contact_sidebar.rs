use iced::widget::{column, container, row, text, Space};
use iced::{Alignment, Element, Length};

use crate::db::Thread;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(thread: Option<&'a Thread>) -> Element<'a, Message> {
    let content = match thread {
        None => Space::new().width(0).into(),
        Some(t) => contact_panel(t),
    };

    container(content)
        .width(if thread.is_some() { CONTACT_SIDEBAR_WIDTH } else { 0.0 })
        .height(Length::Fill)
        .style(theme::sidebar_container)
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
        .spacing(SPACE_XXS)
        .align_x(iced::Alignment::Center)
        .width(Length::Fill);

    // ── Avatar + name ───────────────────────────────────
    col = col.push(Space::new().height(SPACE_MD));
    col = col.push(container(avatar).center_x(Length::Fill));
    col = col.push(Space::new().height(SPACE_XS));
    col = col.push(
        container(
            text(sender).size(14).style(text::base),
        )
        .center_x(Length::Fill),
    );
    col = col.push(
        container(
            text(email).size(11).style(theme::text_tertiary),
        )
        .center_x(Length::Fill),
    );

    col = col.push(Space::new().height(SPACE_MD));

    // ── Stats section ───────────────────────────────────
    col = col.push(section_header("STATS"));
    col = col.push(stat_row("Emails", "—"));
    col = col.push(stat_row("First email", "—"));
    col = col.push(stat_row("Last email", "—"));

    col = col.push(Space::new().height(SPACE_SM));

    // ── Notes ───────────────────────────────────────────
    col = col.push(section_header("NOTES"));
    col = col.push(
        container(
            text("No notes yet").size(11).style(theme::text_tertiary),
        )
        .padding(PAD_ICON_BTN),
    );

    col = col.push(Space::new().height(SPACE_SM));

    // ── Shared files ────────────────────────────────────
    col = col.push(section_header("SHARED FILES"));
    col = col.push(
        container(
            text("No shared files").size(11).style(theme::text_tertiary),
        )
        .padding(PAD_ICON_BTN),
    );

    iced::widget::scrollable(col)
        .height(Length::Fill)
        .into()
}

fn section_header(label: &str) -> Element<'_, Message> {
    container(
        text(label).size(10).style(theme::text_tertiary),
    )
    .padding(PAD_SECTION_HEADER)
    .width(Length::Fill)
    .into()
}

fn stat_row<'a>(label: &'a str, value: &'a str) -> Element<'a, Message> {
    container(
        row![
            text(label).size(11).style(theme::text_tertiary),
            Space::new().width(Length::Fill),
            text(value).size(11).style(text::secondary),
        ]
        .align_y(Alignment::Center),
    )
    .padding(PAD_STAT_ROW)
    .width(Length::Fill)
    .into()
}
