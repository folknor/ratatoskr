use iced::widget::{column, container, text, Space};
use iced::{Element, Length};

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
        .width(CONTACT_SIDEBAR_WIDTH)
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

    let avatar = widgets::avatar_circle(sender, AVATAR_CONTACT_HERO);

    let mut col = column![]
        .spacing(SPACE_XXS)
        .align_x(iced::Alignment::Center)
        .width(Length::Fill);

    // ── Avatar + name ───────────────────────────────────
    col = col.push(Space::new().height(SPACE_MD));
    col = col.push(container(avatar).center_x(Length::Fill));
    col = col.push(Space::new().height(SPACE_XS));
    col = col.push(container(text(sender).size(TEXT_XL).style(text::base)).center_x(Length::Fill));
    col = col.push(container(text(email).size(TEXT_SM).style(theme::text_tertiary)).center_x(Length::Fill));
    col = col.push(Space::new().height(SPACE_MD));

    // ── Stats ───────────────────────────────────────────
    col = col.push(widgets::section_header("STATS"));
    col = col.push(widgets::stat_row("Emails", "—"));
    col = col.push(widgets::stat_row("First email", "—"));
    col = col.push(widgets::stat_row("Last email", "—"));
    col = col.push(Space::new().height(SPACE_SM));

    // ── Notes ───────────────────────────────────────────
    col = col.push(widgets::section_header("NOTES"));
    col = col.push(
        container(text("No notes yet").size(TEXT_SM).style(theme::text_tertiary)).padding(PAD_ICON_BTN),
    );
    col = col.push(Space::new().height(SPACE_SM));

    // ── Shared files ────────────────────────────────────
    col = col.push(widgets::section_header("SHARED FILES"));
    col = col.push(
        container(text("No shared files").size(TEXT_SM).style(theme::text_tertiary)).padding(PAD_ICON_BTN),
    );

    iced::widget::scrollable(col)
        .height(Length::Fill)
        .into()
}
