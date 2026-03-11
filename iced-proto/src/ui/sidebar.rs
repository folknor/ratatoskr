use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Padding};

use crate::db::{Account, Label};
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

pub fn view<'a>(
    accounts: &'a [Account],
    selected_account: Option<usize>,
    labels: &'a [Label],
    selected_label: &'a Option<String>,
) -> Element<'a, Message> {
    let mut col = column![].spacing(2).width(180);

    // ── Account switcher ────────────────────────────────
    if let Some(idx) = selected_account {
        if let Some(acc) = accounts.get(idx) {
            let name = acc.display_name.as_deref().unwrap_or(&acc.email);
            let avatar = widgets::avatar_circle(name, 32.0);
            let account_info = column![
                text(name).size(13).color(theme::TEXT_PRIMARY),
                text(&acc.email).size(11).color(theme::TEXT_TERTIARY),
            ]
            .spacing(1);
            let switcher = button(
                row![avatar, account_info]
                    .spacing(8)
                    .align_y(Alignment::Center),
            )
            .on_press(Message::CycleAccount)
            .padding(Padding::new(6.0))
            .style(button::text)
            .width(Length::Fill);
            col = col.push(switcher);
        }
    }

    col = col.push(Space::new().height(4));

    // ── Compose button ──────────────────────────────────
    let compose_btn = button(
        container(text("Compose").size(13).color(iced::Color::WHITE))
            .center_x(Length::Fill)
            .center_y(Length::Fill),
    )
    .on_press(Message::Compose)
    .padding(Padding::from([8, 16]))
    .style(button::primary)
    .width(Length::Fill);
    col = col.push(compose_btn);

    col = col.push(Space::new().height(8));

    // ── Navigation items ────────────────────────────────
    let nav_items = [
        ("Inbox", "INBOX"),
        ("Starred", "__starred"),
        ("Snoozed", "__snoozed"),
        ("Sent", "__sent"),
        ("Drafts", "__drafts"),
        ("Trash", "__trash"),
        ("Spam", "__spam"),
        ("All Mail", "__all"),
    ];

    for (display_name, id) in nav_items {
        let is_active = match selected_label {
            Some(lid) => lid == id,
            None => id == "INBOX",
        };
        let nav_btn = nav_item(display_name, id, is_active);
        col = col.push(nav_btn);
    }

    col = col.push(Space::new().height(12));

    // ── Labels section ──────────────────────────────────
    col = col.push(
        row![
            text("LABELS").size(10).color(theme::TEXT_TERTIARY),
        ]
        .padding(Padding::from([0, 8])),
    );
    col = col.push(Space::new().height(4));

    let user_labels: Vec<&Label> = labels
        .iter()
        .filter(|l| {
            !matches!(
                l.name.as_str(),
                "INBOX" | "Sent" | "Drafts" | "Trash" | "Spam" | "Archive"
                    | "Junk" | "Templates" | "Calendar" | "Contacts"
                    | "Journal" | "Notes" | "Outbox" | "Deleted Items"
                    | "Sent Items" | "Conversation History"
            )
        })
        .collect();

    for label in user_labels.iter().take(8) {
        let is_active = selected_label.as_deref() == Some(&label.id);
        let color_dot = widgets::color_dot(theme::avatar_color(&label.name));
        let btn = button(
            row![color_dot, text(&label.name).size(12).color(if is_active {
                theme::ACCENT
            } else {
                theme::TEXT_SECONDARY
            })]
            .spacing(8)
            .align_y(Alignment::Center),
        )
        .on_press(Message::SelectLabel(Some(label.id.clone())))
        .padding(Padding::from([4, 8]))
        .style(if is_active {
            button::secondary
        } else {
            button::text
        })
        .width(Length::Fill);
        col = col.push(btn);
    }

    if user_labels.len() > 8 {
        col = col.push(
            text(format!("{} more", user_labels.len() - 8))
                .size(11)
                .color(theme::TEXT_TERTIARY),
        );
    }

    // ── Bottom spacer + settings ────────────────────────
    col = col.push(Space::new().height(Length::Fill));
    col = col.push(
        button(text("Settings").size(12).color(theme::TEXT_SECONDARY))
            .on_press(Message::Noop)
            .style(button::text)
            .padding(Padding::from([6, 8]))
            .width(Length::Fill),
    );

    container(scrollable(col).height(Length::Fill))
        .padding(Padding::from([8, 4]))
        .width(180)
        .style(|_: &iced::Theme| container::Style {
            background: Some(theme::BG_SIDEBAR.into()),
            border: iced::Border {
                color: theme::BORDER,
                width: 0.0,
                radius: 0.0.into(),
            },
            ..Default::default()
        })
        .into()
}

fn nav_item<'a>(label: &'a str, id: &'a str, active: bool) -> Element<'a, Message> {
    let txt_color = if active {
        theme::ACCENT
    } else {
        theme::TEXT_SECONDARY
    };

    button(text(label).size(12).color(txt_color))
        .on_press(Message::SelectLabel(if id == "INBOX" {
            None
        } else {
            Some(id.to_string())
        }))
        .padding(Padding::from([5, 8]))
        .style(if active {
            button::secondary
        } else {
            button::text
        })
        .width(Length::Fill)
        .into()
}
