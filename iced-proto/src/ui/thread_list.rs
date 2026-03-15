use iced::widget::{column, container, scrollable, text, row, Space};
use iced::{Alignment, Element, Length};

use crate::db::Thread;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(
    threads: &'a [Thread],
    selected_thread: Option<usize>,
    status: &'a str,
    label_name: &'a str,
) -> Element<'a, Message> {
    let header = container(
        column![
            container(text("Search...").size(12).style(theme::text_tertiary))
                .padding(PAD_INPUT)
                .width(Length::Fill)
                .style(theme::elevated_container),
            Space::new().height(SPACE_XS),
            row![
                text(label_name).size(14).style(text::base),
                Space::new().width(SPACE_XXS),
                text(status).size(11).style(theme::text_tertiary),
                Space::new().width(Length::Fill),
                text("All").size(11).style(text::secondary),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(0),
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
    .width(THREAD_LIST_WIDTH)
    .height(Length::Fill)
    .style(theme::surface_container)
    .into()
}
