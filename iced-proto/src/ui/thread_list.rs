use iced::widget::{column, container, row, scrollable, text, Space};
use iced::{Color, Element, Length};

use crate::db::Thread;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(
    threads: &'a [Thread],
    selected_thread: Option<usize>,
    folder_name: &'a str,
    scope_name: &'a str,
) -> Element<'a, Message> {
    let header = container(
        column![
            // Search placeholder (existing)
            container(text("Search...").size(TEXT_MD).style(theme::text_tertiary))
                .padding(PAD_INPUT)
                .width(Length::Fill)
                .style(theme::elevated_container),
            // Context line (new)
            row![
                text(folder_name).size(TEXT_SM).style(theme::text_tertiary),
                Space::new().width(Length::Fill),
                text(scope_name).size(TEXT_SM).style(theme::text_tertiary),
            ]
            .align_y(iced::Alignment::Center),
        ]
        .spacing(SPACE_XXS),
    )
    .padding(PAD_PANEL_HEADER);

    let body: Element<'a, Message> = if threads.is_empty() {
        widgets::empty_placeholder("No conversations", "This folder is empty")
    } else {
        let mut list = column![].spacing(0);
        for (i, thread) in threads.iter().enumerate() {
            // Empty label colors for now — backend integration later
            let label_colors: &[(Color,)] = &[];
            list = list.push(widgets::thread_card(thread, i, selected_thread == Some(i), label_colors));
        }
        scrollable(list).height(Length::Fill).into()
    };

    container(
        column![header, body]
            .spacing(0)
            .width(Length::Fill),
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::base_container)
    .into()
}
