use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Padding};

use crate::db::Thread;
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
            // Search bar placeholder
            container(
                text("Search...").size(12).color(theme::TEXT_TERTIARY),
            )
            .padding(Padding::from([6, 10]))
            .width(Length::Fill)
            .style(|_: &iced::Theme| container::Style {
                background: Some(theme::BG_ELEVATED.into()),
                border: iced::Border {
                    color: theme::BORDER,
                    width: 1.0,
                    radius: 6.0.into(),
                },
                ..Default::default()
            }),
            Space::new().height(8),
            row![
                text(label_name).size(14).color(theme::TEXT_PRIMARY),
                Space::new().width(6),
                text(status).size(11).color(theme::TEXT_TERTIARY),
                Space::new().width(Length::Fill),
                text("All").size(11).color(theme::TEXT_SECONDARY),
            ]
            .align_y(Alignment::Center),
        ]
        .spacing(0),
    )
    .padding(Padding::from([10, 12]));

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
        .width(280)
        .height(Length::Fill)
        .style(|_: &iced::Theme| container::Style {
            background: Some(theme::BG_SURFACE.into()),
            border: iced::Border {
                color: theme::BORDER,
                width: 1.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
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
    let name_color = if thread.is_read {
        theme::TEXT_SECONDARY
    } else {
        theme::TEXT_PRIMARY
    };

    let avatar = widgets::avatar_circle(sender, 28.0);

    // Right-side indicators
    let mut indicators = row![].spacing(4).align_y(Alignment::Center);
    if thread.has_attachments {
        indicators = indicators.push(text("@").size(10).color(theme::TEXT_TERTIARY));
    }
    if thread.is_starred {
        indicators = indicators.push(text("*").size(12).color(theme::WARNING));
    }
    if thread.message_count > 1 {
        indicators = indicators.push(
            container(
                text(thread.message_count.to_string())
                    .size(10)
                    .color(theme::TEXT_TERTIARY),
            )
            .padding(Padding::from([1, 4]))
            .style(|_: &iced::Theme| container::Style {
                background: Some(theme::BG_ELEVATED.into()),
                border: iced::Border {
                    radius: 8.0.into(),
                    ..Default::default()
                },
                ..Default::default()
            }),
        );
    }

    let top_row = row![
        text(sender)
            .size(12)
            .color(name_color)
            .font(iced::Font { weight, ..iced::Font::DEFAULT }),
        Space::new().width(Length::Fill),
        text(date_str).size(10).color(theme::TEXT_TERTIARY),
    ]
    .align_y(Alignment::Center);

    let subject_row = row![
        text(subject)
            .size(12)
            .color(name_color)
            .font(iced::Font { weight, ..iced::Font::DEFAULT })
            .wrapping(text::Wrapping::None),
    ];

    let snippet_row = row![
        text(snippet)
            .size(11)
            .color(theme::TEXT_TERTIARY)
            .wrapping(text::Wrapping::None),
        Space::new().width(Length::Fill),
        indicators,
    ]
    .align_y(Alignment::Center);

    let content = row![
        avatar,
        column![top_row, subject_row, snippet_row].spacing(2).width(Length::Fill),
    ]
    .spacing(10)
    .align_y(Alignment::Start);

    let bg = if selected {
        theme::BG_SELECTED
    } else {
        theme::BG_SURFACE
    };

    button(
        container(content)
            .padding(Padding::from([8, 12]))
            .width(Length::Fill),
    )
    .on_press(Message::SelectThread(index))
    .padding(0)
    .style(move |_: &iced::Theme, _| button::Style {
        background: Some(bg.into()),
        border: iced::Border {
            color: theme::BORDER_SUBTLE,
            width: 0.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    })
    .width(Length::Fill)
    .into()
}
