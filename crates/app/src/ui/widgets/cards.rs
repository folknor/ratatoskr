#![allow(dead_code)]

use std::path::Path;

use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Color, Element, Length, Theme};

use crate::db::{DateDisplay, Thread, ThreadMessage};
use crate::font;
use crate::icon;
use crate::ui::layout::{
    AVATAR_MESSAGE_CARD, AVATAR_THREAD_CARD, ICON_SM, ICON_XS, PAD_BADGE, PAD_BODY, PAD_CARD,
    PAD_ICON_BTN, PAD_THREAD_CARD, SPACE_SM, SPACE_XS, SPACE_XXS, SPACE_XXXS, TEXT_LG, TEXT_MD,
    TEXT_SM, TEXT_XS, THREAD_CARD_HEIGHT,
};
use crate::ui::theme;

use super::avatars::{avatar_circle, label_dot, sender_avatar};
use super::buttons::reply_button;
use super::highlighted::highlighted_text_body;

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn thread_card<'a, M: Clone + 'a>(
    thread: &'a Thread,
    index: usize,
    selected: bool,
    label_colors: &[(Color,)],
    bimi_logo: Option<&Path>,
    on_select: impl Fn(usize) -> M,
) -> Element<'a, M> {
    let sender = thread
        .from_name
        .as_deref()
        .or(thread.from_address.as_deref())
        .unwrap_or("(unknown)");

    let subject = thread.subject.as_deref().unwrap_or("(no subject)");
    let snippet = thread.snippet.as_deref().unwrap_or("");

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

    let sender_font = if thread.is_read {
        font::text()
    } else {
        font::text_semibold()
    };

    let subject_style: fn(&Theme) -> text::Style = if thread.is_read {
        theme::TextClass::Muted.style()
    } else {
        theme::TextClass::Accent.style()
    };

    let mut indicators = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    if thread.is_local_draft {
        indicators = indicators.push(
            container(
                text("Draft")
                    .size(TEXT_XS)
                    .style(theme::TextClass::Accent.style()),
            )
            .padding(PAD_BADGE)
            .style(theme::ContainerClass::KeyBadge.style()),
        );
    }
    for &(color,) in label_colors {
        indicators = indicators.push(label_dot(color));
    }
    if thread.has_attachments {
        indicators = indicators.push(
            icon::paperclip()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style()),
        );
    }

    let top_row = row![
        container(
            text(sender)
                .size(TEXT_MD)
                .style(text::base)
                .font(sender_font),
        )
        .width(Length::Fill),
        container(
            text(date_str)
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style())
        ),
    ]
    .align_y(Alignment::Center);

    let subject_row = row![
        container(
            text(subject)
                .size(TEXT_MD)
                .style(subject_style)
                .font(font::text())
                .wrapping(text::Wrapping::None),
        )
        .width(Length::Fill),
    ];

    let snippet_row = row![
        container(
            text(snippet)
                .size(TEXT_SM)
                .style(text::secondary)
                .wrapping(text::Wrapping::None),
        )
        .width(Length::Fill),
        indicators,
    ]
    .align_y(Alignment::Center);

    let text_content = column![top_row, subject_row, snippet_row]
        .spacing(SPACE_XXXS)
        .width(Length::Fill);

    let avatar = sender_avatar(sender, bimi_logo, AVATAR_THREAD_CARD);

    let content = row![avatar, text_content]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center);

    button(
        container(content)
            .padding(PAD_THREAD_CARD)
            .height(THREAD_CARD_HEIGHT)
            .width(Length::Fill),
    )
    .on_press(on_select(index))
    .padding(0)
    .style(
        theme::ButtonClass::ThreadCard {
            selected,
            starred: thread.is_starred,
        }
        .style(),
    )
    .width(Length::Fill)
    .into()
}

pub fn message_card<'a, M: 'a>(thread: &'a Thread) -> Element<'a, M> {
    let sender = thread
        .from_name
        .as_deref()
        .or(thread.from_address.as_deref())
        .unwrap_or("(unknown)");

    let avatar = avatar_circle(sender, AVATAR_MESSAGE_CARD);
    let date_str = thread
        .last_message_at
        .and_then(|ts| {
            chrono::DateTime::from_timestamp(ts, 0)
                .map(|dt| dt.format("%a, %b %d, %Y, %l:%M %p").to_string())
        })
        .unwrap_or_default();

    let header = row![
        avatar,
        column![
            row![
                text(sender).size(TEXT_LG).style(text::base),
                Space::new().width(Length::Fill),
                text(date_str)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ],
            text(thread.from_address.as_deref().unwrap_or(""))
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let body_text = thread
        .snippet
        .as_deref()
        .unwrap_or("(no preview available)");
    let body = container(text(body_text).size(TEXT_LG).style(text::secondary)).padding(PAD_BODY);

    container(column![header, body].spacing(SPACE_XS))
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::ContainerClass::MessageCard.style())
        .into()
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
#[allow(clippy::too_many_arguments)]
pub fn expanded_message_card<'a, M: Clone + 'a>(
    msg: &'a ThreadMessage,
    index: usize,
    date_display: DateDisplay,
    first_message_date: Option<i64>,
    search_highlight_terms: &'a [String],
    cached_html: Option<&'a crate::ui::html_render::CachedHtmlBody>,
    inline_images: &'a std::collections::HashMap<String, iced::widget::image::Handle>,
    on_toggle: impl Fn(usize) -> M,
    on_pop_out: impl Fn(usize) -> M,
    on_reply: impl Fn(usize) -> M,
    on_reply_all: impl Fn(usize) -> M,
    on_forward: impl Fn(usize) -> M,
    on_edit_contact: impl Fn(String) -> M + 'a,
    on_create_event: impl Fn(usize) -> M,
    on_link_click: impl Fn(String) -> M + 'a,
) -> Element<'a, M> {
    let sender_name = msg
        .from_name
        .as_deref()
        .or(msg.from_address.as_deref())
        .unwrap_or("(unknown)");
    let sender_email_str = msg.from_address.as_deref().unwrap_or("");

    let avatar = avatar_circle(sender_name, AVATAR_MESSAGE_CARD);
    let date_str = format_message_date(msg.date, first_message_date, date_display);

    let recipients = msg.to_addresses.as_deref().unwrap_or("");

    let pop_out_btn = button(icon::external_link().size(ICON_SM).style(text::secondary))
        .on_press(on_pop_out(index))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let sender_email_owned = msg.from_address.clone().unwrap_or_default();
    let sender_element: Element<'a, M> = button(
        text(sender_name)
            .size(TEXT_LG)
            .font(font::text_semibold())
            .style(text::base),
    )
    .on_press(on_edit_contact(sender_email_owned))
    .padding(0)
    .style(theme::ButtonClass::BareTransparent.style())
    .into();

    let collapse_btn = button(icon::chevron_up().size(ICON_SM).style(text::secondary))
        .on_press(on_toggle(index))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let avatar_rows = row![
        avatar,
        column![
            row![
                container(sender_element).align_y(Alignment::Center),
                Space::new().width(Length::Fill),
                container(
                    text(date_str)
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .align_y(Alignment::Center),
                collapse_btn,
                pop_out_btn,
            ]
            .align_y(Alignment::Center)
            .spacing(SPACE_XS),
            text(sender_email_str)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let to_row = row![
        container(
            text("To")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .width(AVATAR_MESSAGE_CARD + SPACE_SM)
        .padding(iced::Padding {
            top: 0.0,
            right: SPACE_XS,
            bottom: 0.0,
            left: 0.0
        })
        .align_x(Alignment::End),
        text(recipients).size(TEXT_SM).style(text::base),
    ]
    .spacing(0)
    .align_y(Alignment::Start);

    let header = column![avatar_rows, to_row].spacing(SPACE_XXS);

    let on_link = std::rc::Rc::new(on_link_click);
    let body_inner: Element<'_, M> = if msg.body_html.is_some() {
        let link_cb = std::rc::Rc::clone(&on_link);
        let rendered = if let Some(cached) = cached_html {
            crate::ui::html_render::render_cached_html(
                cached,
                msg.body_text.as_deref(),
                move |url| link_cb(url),
                inline_images,
            )
        } else if let Some(html) = msg.body_html.as_deref() {
            let link_cb2 = std::rc::Rc::clone(&on_link);
            crate::ui::html_render::render_html(
                html,
                msg.body_text.as_deref(),
                move |url| link_cb2(url),
                inline_images,
            )
        } else {
            unreachable!()
        };
        container(rendered).padding(PAD_BODY).into()
    } else {
        let display = msg
            .body_text
            .as_deref()
            .or(msg.snippet.as_deref())
            .unwrap_or("(no preview available)");
        if search_highlight_terms.is_empty() {
            container(text(display).size(TEXT_LG))
                .padding(PAD_BODY)
                .into()
        } else {
            container(highlighted_text_body::<M>(display, search_highlight_terms))
                .padding(PAD_BODY)
                .into()
        }
    };

    let body: Element<'_, M> = container(body_inner)
        .width(Length::Fill)
        .style(theme::ContainerClass::EmailBody.style())
        .into();

    let cal_btn = button(
        row![
            icon::calendar().size(ICON_SM).style(text::secondary),
            text("Event").size(TEXT_SM).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(on_create_event(index))
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Ghost.style());

    let actions = row![
        reply_button(icon::reply(), "Reply", on_reply(index)),
        reply_button(icon::reply_all(), "Reply All", on_reply_all(index)),
        reply_button(icon::forward(), "Forward", on_forward(index)),
        cal_btn,
    ]
    .spacing(SPACE_XS);

    let card_content = column![header, body, actions].spacing(SPACE_XS);

    container(card_content)
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::ContainerClass::MessageCard.style())
        .into()
}

#[cfg_attr(feature = "hotpath", hotpath::measure)]
pub fn collapsed_message_row<'a, M: Clone + 'a>(
    msg: &'a ThreadMessage,
    index: usize,
    on_toggle: impl Fn(usize) -> M,
) -> Element<'a, M> {
    let sender = msg
        .from_name
        .as_deref()
        .or(msg.from_address.as_deref())
        .unwrap_or("(unknown)");

    let short_date = msg
        .date
        .and_then(|ts| {
            chrono::DateTime::from_timestamp(ts, 0).map(|dt| dt.format("%b %d").to_string())
        })
        .unwrap_or_default();

    let snippet = truncate_snippet(msg.snippet.as_deref(), 60);

    let content = row![
        container(
            icon::chevron_right()
                .size(ICON_XS)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            text(sender)
                .size(TEXT_SM)
                .font(font::text_semibold())
                .style(text::base),
        )
        .align_y(Alignment::Center),
        container(
            text("\u{00B7}")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            text(short_date)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            text("\u{00B7}")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            text(snippet)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
                .wrapping(text::Wrapping::None),
        )
        .width(Length::Fill)
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    button(container(content).width(Length::Fill))
        .on_press(on_toggle(index))
        .padding(0)
        .style(theme::ButtonClass::CollapsedMessage.style())
        .width(Length::Fill)
        .into()
}

fn format_message_date(
    timestamp: Option<i64>,
    first_message_timestamp: Option<i64>,
    display: DateDisplay,
) -> String {
    let Some(ts) = timestamp else {
        return String::new();
    };
    let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) else {
        return String::new();
    };

    match display {
        DateDisplay::RelativeOffset => {
            let abs = dt.format("%b %d, %Y, %l:%M %p").to_string();
            match first_message_timestamp.and_then(|fts| chrono::DateTime::from_timestamp(fts, 0)) {
                Some(first_dt) => {
                    let days = (dt - first_dt).num_days();
                    if days == 0 {
                        abs.trim().to_string()
                    } else {
                        format!("{} (+{}d)", abs.trim(), days)
                    }
                }
                None => abs.trim().to_string(),
            }
        }
        DateDisplay::Absolute => dt
            .format("%b %d, %Y, %l:%M %p")
            .to_string()
            .trim()
            .to_string(),
    }
}

fn truncate_snippet(snippet: Option<&str>, max_chars: usize) -> String {
    let s = snippet.unwrap_or("");
    if s.len() <= max_chars {
        s.to_string()
    } else {
        format!("{}...", &s[..s.floor_char_boundary(max_chars)])
    }
}
