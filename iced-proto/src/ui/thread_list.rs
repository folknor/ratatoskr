use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Theme};

use crate::db::Thread;
use crate::font;
use crate::icon;
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
    let mut col = column![].spacing(0).width(Length::Fill);

    // ── Header ──────────────────────────────────────────
    let header = container(
        column![
            container(
                text("Search...").size(12).style(theme::text_tertiary),
            )
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

    col = col.push(header);

    // ── Thread cards ────────────────────────────────────
    let mut list = column![].spacing(0);

    for (i, thread) in threads.iter().enumerate() {
        let is_selected = selected_thread == Some(i);
        list = list.push(thread_card(thread, i, is_selected));
    }

    col = col.push(
        scrollable(list)
            .height(Length::Fill),
    );

    container(col)
        .width(THREAD_LIST_WIDTH)
        .height(Length::Fill)
        .style(theme::surface_container)
        .into()
}

fn thread_card(thread: &Thread, index: usize, selected: bool) -> Element<'_, Message> {
    let sender = thread
        .from_name
        .as_deref()
        .or(thread.from_address.as_deref())
        .unwrap_or("(unknown)");

    let subject = thread
        .subject
        .as_deref()
        .unwrap_or("(no subject)");

    let snippet = thread
        .snippet
        .as_deref()
        .unwrap_or("");

    let date_str = thread
        .last_message_at
        .and_then(|ts| {
            chrono::DateTime::from_timestamp(ts, 0).map(|dt| {
                let now = chrono::Utc::now();
                let diff = now.signed_duration_since(dt);
                if diff.num_hours() < 24 {
                    dt.format("%l:%M %p").to_string().trim().to_string()
                } else if diff.num_days() < 7 {
                    dt.format("%a").to_string()
                } else {
                    dt.format("%b %d").to_string()
                }
            })
        })
        .unwrap_or_default();

    let weight = if thread.is_read {
        iced::font::Weight::Normal
    } else {
        iced::font::Weight::Bold
    };
    let name_style: fn(&Theme) -> text::Style = if thread.is_read {
        text::secondary
    } else {
        text::base
    };

    let avatar = widgets::avatar_circle(sender, 28.0);

    // Right-side indicators
    let mut indicators = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    if thread.has_attachments {
        indicators = indicators.push(icon::paperclip().size(10).style(theme::text_tertiary));
    }
    if thread.is_starred {
        indicators = indicators.push(icon::star().size(11).style(text::warning));
    }
    if thread.message_count > 1 {
        indicators = indicators.push(
            container(
                text(thread.message_count.to_string())
                    .size(10)
                    .style(theme::text_tertiary),
            )
            .padding(PAD_BADGE)
            .style(theme::badge_container),
        );
    }

    let top_row = row![
        text(sender)
            .size(12)
            .style(name_style)
            .font(iced::Font { weight, ..font::TEXT }),
        Space::new().width(Length::Fill),
        text(date_str).size(10).style(theme::text_tertiary),
    ]
    .align_y(Alignment::Center);

    let subject_row = row![
        text(subject)
            .size(12)
            .style(name_style)
            .font(iced::Font { weight, ..font::TEXT })
            .wrapping(text::Wrapping::None),
    ];

    let snippet_row = row![
        text(snippet)
            .size(11)
            .style(theme::text_tertiary)
            .wrapping(text::Wrapping::None),
        Space::new().width(Length::Fill),
        indicators,
    ]
    .align_y(Alignment::Center);

    let content = row![
        avatar,
        column![top_row, subject_row, snippet_row].spacing(SPACE_XXXS).width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    button(
        container(content)
            .padding(PAD_THREAD_CARD)
            .width(Length::Fill),
    )
    .on_press(Message::SelectThread(index))
    .padding(0)
    .style(theme::thread_card_button(selected))
    .width(Length::Fill)
    .into()
}
