use iced::widget::{column, container, scrollable, text};
use iced::Element;
use iced::Length;

use crate::db::Thread;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(
    threads: &'a [Thread],
    selected_thread: Option<usize>,
) -> Element<'a, Message> {
    let header = container(
        container(text("Search...").size(TEXT_MD).style(theme::text_tertiary))
            .padding(PAD_INPUT)
            .width(Length::Fill)
            .style(theme::elevated_container),
    )
    .padding(PAD_PANEL_HEADER);

    let mut list = column![].spacing(0);
    for (i, thread) in threads.iter().enumerate() {
        list = list.push(widgets::thread_card(thread, i, selected_thread == Some(i)));
    }

    container(
        column![header, scrollable(list).height(Length::Fill)]
            .spacing(0)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::base_container)
    .into()
}
