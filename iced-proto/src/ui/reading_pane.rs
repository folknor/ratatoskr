use std::collections::HashMap;

use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Padding};

use crate::db::{DateDisplay, Thread, ThreadAttachment, ThreadMessage};
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(
    thread: Option<&'a Thread>,
    messages: &'a [ThreadMessage],
    message_expanded: &'a [bool],
    attachments: &'a [ThreadAttachment],
    attachments_collapsed: bool,
    date_display: DateDisplay,
) -> Element<'a, Message> {
    match thread {
        None => container(widgets::empty_placeholder(
            "No conversation selected",
            "Select a thread to read",
        ))
        .width(Length::Fill)
        .height(Length::Fill)
        .into(),
        Some(t) => thread_view(
            t,
            messages,
            message_expanded,
            attachments,
            attachments_collapsed,
            date_display,
        ),
    }
}

fn thread_view<'a>(
    thread: &'a Thread,
    messages: &'a [ThreadMessage],
    message_expanded: &'a [bool],
    attachments: &'a [ThreadAttachment],
    attachments_collapsed: bool,
    date_display: DateDisplay,
) -> Element<'a, Message> {
    let subject = thread.subject.as_deref().unwrap_or("(no subject)");
    let mut col = column![].spacing(0).width(Length::Fill);

    // ── Thread header ───────────────────────────────────
    let star_icon_style: fn(&iced::Theme) -> text::Style = if thread.is_starred {
        text::warning
    } else {
        text::secondary
    };
    let star_btn_style: fn(&iced::Theme, button::Status) -> button::Style = if thread.is_starred {
        theme::star_active_button
    } else {
        theme::bare_icon_button
    };

    let star_btn = button(icon::star().size(ICON_XL).style(star_icon_style))
        .on_press(Message::Noop)
        .padding(PAD_ICON_BTN)
        .style(star_btn_style);

    let toggle_label = if message_expanded.iter().all(|&e| e) {
        "Collapse all"
    } else {
        "Expand all"
    };

    let expand_collapse_btn = button(
        text(toggle_label)
            .size(TEXT_SM)
            .style(theme::text_tertiary),
    )
    .on_press(Message::ToggleAllMessages)
    .style(theme::ghost_button)
    .padding(PAD_ICON_BTN);

    col = col.push(
        container(
            column![
                row![
                    container(text(subject).size(TEXT_HEADING).style(text::base))
                        .width(Length::Fill),
                    star_btn,
                ]
                .align_y(Alignment::Center),
                row![
                    container(
                        text(format!("{} messages", messages.len()))
                            .size(TEXT_SM)
                            .style(theme::text_tertiary),
                    )
                    .align_y(Alignment::Center),
                    Space::new().width(Length::Fill),
                    expand_collapse_btn,
                ]
                .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XXS),
        )
        .padding(PAD_CONTENT),
    );

    // ── Attachment group (if any) ───────────────────────
    if !attachments.is_empty() {
        col = col.push(attachment_group(attachments, attachments_collapsed));
    }

    // ── Scrollable message list ─────────────────────────
    let messages_pad = Padding::from([0.0, SPACE_LG]);
    let first_message_date = messages.last().and_then(|m| m.date);
    let mut msg_col = column![].spacing(SPACE_XS).padding(messages_pad);

    for (i, msg) in messages.iter().enumerate() {
        let is_expanded = message_expanded.get(i).copied().unwrap_or(false);
        if is_expanded {
            msg_col = msg_col.push(widgets::expanded_message_card(
                msg,
                i,
                date_display,
                first_message_date,
            ));
        } else {
            msg_col = msg_col.push(widgets::collapsed_message_row(msg, i));
        }
    }

    msg_col = msg_col.push(Space::new().height(SPACE_MD));

    col = col.push(scrollable(msg_col).height(Length::Fill));

    col.into()
}

// ── Attachment group ────────────────────────────────────

/// Deduplicate attachments by filename, keeping the latest version (first
/// occurrence, since query orders by date DESC). Returns (deduped list,
/// version counts per filename).
fn dedup_attachments(attachments: &[ThreadAttachment]) -> Vec<(&ThreadAttachment, usize)> {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    let mut result: Vec<(&ThreadAttachment, usize)> = Vec::new();

    // First pass: count versions per filename
    for att in attachments {
        let name = att.filename.as_deref().unwrap_or("");
        *seen.entry(name).or_insert(0) += 1;
    }

    // Second pass: keep first occurrence (latest) with version count
    let mut emitted: HashMap<&str, bool> = HashMap::new();
    for att in attachments {
        let name = att.filename.as_deref().unwrap_or("");
        if !emitted.contains_key(name) {
            let count = seen.get(name).copied().unwrap_or(1);
            result.push((att, count));
            emitted.insert(name, true);
        }
    }

    result
}

fn attachment_group<'a>(
    attachments: &'a [ThreadAttachment],
    collapsed: bool,
) -> Element<'a, Message> {
    let deduped = dedup_attachments(attachments);

    let chevron = if collapsed {
        icon::chevron_right()
    } else {
        icon::chevron_down()
    };

    let header = button(
        row![
            container(chevron.size(ICON_XS).style(theme::text_tertiary))
                .align_y(Alignment::Center),
            Space::new().width(SPACE_XXS),
            container(
                text(format!("Attachments ({})", deduped.len()))
                    .size(TEXT_MD)
                    .font(font::TEXT_SEMIBOLD)
                    .style(text::base),
            )
            .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            container(
                text("Save All")
                    .size(TEXT_SM)
                    .style(theme::text_accent),
            )
            .align_y(Alignment::Center),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(Message::ToggleAttachmentsCollapsed)
    .style(theme::ghost_button)
    .width(Length::Fill);

    let mut content_col = column![header].spacing(SPACE_XS);

    if !collapsed {
        for (att, version_count) in &deduped {
            content_col = content_col.push(widgets::attachment_card(att, *version_count));
        }
    }

    container(
        container(content_col)
            .padding(PAD_CARD)
            .style(theme::elevated_container),
    )
    .padding(PAD_CONTENT)
    .width(Length::Fill)
    .into()
}
