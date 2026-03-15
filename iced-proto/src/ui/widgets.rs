use iced::widget::{button, canvas, column, container, row, rule, text, Canvas, Space};
use iced::{mouse, Alignment, Color, Element, Length, Rectangle, Renderer, Theme};

use crate::db::{Account, Thread};
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::Message;

// ── Avatar ──────────────────────────────────────────────

pub fn avatar_circle<'a>(name: &str, size: f32) -> Element<'a, Message> {
    let color = theme::avatar_color(name);
    let letter = theme::initial(name);

    let circle = Canvas::new(CirclePainter { color, size })
        .width(size)
        .height(size);

    iced::widget::stack![
        circle,
        container(
            text(letter)
                .size(size * 0.45)
                .color(Color::WHITE),
        )
        .center(Length::Fill),
    ]
    .width(size)
    .height(size)
    .into()
}

pub fn color_dot<'a>(color: Color) -> Element<'a, Message> {
    let dot = Canvas::new(DotPainter { color })
        .width(8)
        .height(8);
    container(dot)
        .center_y(Length::Shrink)
        .into()
}

// ── Badges ──────────────────────────────────────────────

pub fn count_badge<'a>(count: i64) -> Element<'a, Message> {
    if count == 0 {
        return Space::new().width(0).height(0).into();
    }
    let label = if count > 999 {
        "999+".to_string()
    } else {
        count.to_string()
    };
    container(text(label).size(10).style(text::secondary))
        .padding(PAD_BADGE)
        .style(theme::badge_container)
        .into()
}

// ── Nav items ───────────────────────────────────────────

pub struct NavItem<'a> {
    pub label: &'a str,
    pub id: &'a str,
    pub unread: i64,
}

pub fn nav_group<'a>(
    items: &[NavItem<'a>],
    selected_label: &'a Option<String>,
) -> Element<'a, Message> {
    let mut col = column![].spacing(SPACE_XXS);
    for item in items {
        let is_active = match selected_label {
            Some(lid) => lid == item.id,
            None => item.id == "INBOX",
        };
        let on_press = if item.id == "INBOX" {
            Message::SelectLabel(None)
        } else {
            Message::SelectLabel(Some(item.id.to_string()))
        };
        col = col.push(nav_item_with_badge(item.label, item.id, is_active, item.unread, on_press));
    }
    col.into()
}

pub fn nav_item_with_badge<'a>(
    label: &'a str,
    _id: &'a str,
    active: bool,
    unread: i64,
    on_press: Message,
) -> Element<'a, Message> {
    let label_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        text::secondary
    };

    let mut content = row![text(label).size(12).style(label_style)]
        .align_y(Alignment::Center);

    if unread > 0 {
        content = content
            .push(Space::new().width(Length::Fill))
            .push(count_badge(unread));
    }

    button(content)
        .on_press(on_press)
        .padding(PAD_NAV_ITEM)
        .style(theme::nav_button(active))
        .width(Length::Fill)
        .into()
}

pub fn label_nav_item<'a>(
    name: &'a str,
    id: &'a str,
    color: Color,
    active: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let lbl_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        text::secondary
    };

    button(
        row![color_dot(color), text(name).size(12).style(lbl_style)]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_ICON_BTN)
    .style(theme::nav_button(active))
    .width(Length::Fill)
    .into()
}

// ── Dividers & section breaks ───────────────────────────

pub fn divider<'a>() -> Element<'a, Message> {
    rule::horizontal(1).style(theme::divider_rule).into()
}

pub fn section_break<'a>() -> Element<'a, Message> {
    column![
        Space::new().height(SPACE_XXS),
        divider(),
        Space::new().height(SPACE_XXS),
    ]
    .into()
}

// ── Collapsible section ─────────────────────────────────

pub fn collapsible_section<'a>(
    title: &'a str,
    expanded: bool,
    on_toggle: Message,
    children: Vec<Element<'a, Message>>,
) -> Element<'a, Message> {
    let chevron = if expanded {
        icon::chevron_down()
    } else {
        icon::chevron_right()
    };

    let header = button(
        row![
            text(title).size(10).style(theme::text_tertiary),
            Space::new().width(Length::Fill),
            chevron.size(10).style(theme::text_tertiary),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle)
    .padding(iced::Padding::from([0.0, 8.0]))
    .style(theme::bare_button)
    .width(Length::Fill);

    let mut col = column![header].spacing(SPACE_XXS);

    if expanded {
        for child in children {
            col = col.push(child);
        }
    }

    col.into()
}

// ── Dropdown / Popover ──────────────────────────────────

pub fn dropdown_trigger<'a>(
    content: Element<'a, Message>,
    on_press: Message,
) -> Element<'a, Message> {
    button(
        row![
            content,
            Space::new().width(Length::Fill),
            icon::chevron_down().size(11).style(theme::text_tertiary),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_ACCOUNT)
    .style(theme::bare_button)
    .width(Length::Fill)
    .into()
}

pub fn dropdown_menu<'a>(items: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    container(
        column(items).spacing(SPACE_XXS).width(Length::Fill),
    )
    .padding(PAD_ICON_BTN)
    .style(theme::floating_container)
    .width(Length::Fill)
    .into()
}

pub fn dropdown_item<'a>(
    content: Element<'a, Message>,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    button(content)
        .on_press(on_press)
        .padding(PAD_NAV_ITEM)
        .style(theme::nav_button(selected))
        .width(Length::Fill)
        .into()
}

// ── Scope dropdown ──────────────────────────────────────

pub fn scope_dropdown<'a>(
    accounts: &'a [Account],
    selected_account: Option<usize>,
    dropdown_open: bool,
) -> Element<'a, Message> {
    let trigger_content: Element<'a, Message> = if let Some(idx) = selected_account {
        if let Some(acc) = accounts.get(idx) {
            let name = acc.display_name.as_deref().unwrap_or(&acc.email);
            row![
                avatar_circle(name, 24.0),
                text(name).size(12).style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center)
            .into()
        } else {
            text("All Accounts").size(12).style(text::base).into()
        }
    } else {
        text("All Accounts").size(12).style(text::base).into()
    };

    let trigger = dropdown_trigger(trigger_content, Message::ToggleScopeDropdown);

    if !dropdown_open {
        return trigger;
    }

    let mut items: Vec<Element<'a, Message>> = Vec::new();

    items.push(dropdown_item(
        row![
            icon::inbox().size(12).style(text::secondary),
            text("All Accounts").size(12).style(text::base),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center)
        .into(),
        selected_account.is_none(),
        Message::ToggleScopeDropdown,
    ));

    for (idx, acc) in accounts.iter().enumerate() {
        let name = acc.display_name.as_deref().unwrap_or(&acc.email);
        let is_selected = selected_account == Some(idx);
        items.push(dropdown_item(
            row![
                avatar_circle(name, 20.0),
                text(name).size(12).style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center)
            .into(),
            is_selected,
            Message::SelectAccount(idx),
        ));
    }

    let menu = dropdown_menu(items);

    column![trigger, menu].spacing(SPACE_XXS).into()
}

// ── Compose button ──────────────────────────────────────

pub fn compose_button<'a>() -> Element<'a, Message> {
    button(
        container(
            row![
                icon::pencil().size(13).color(Color::WHITE),
                text("Compose").size(13).color(Color::WHITE),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill),
    )
    .on_press(Message::Compose)
    .padding(PAD_BUTTON)
    .style(button::primary)
    .width(Length::Fill)
    .into()
}

// ── Settings button ─────────────────────────────────────

pub fn settings_button<'a>() -> Element<'a, Message> {
    button(
        row![
            icon::settings().size(12).style(text::secondary),
            text("Settings").size(12).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::Noop)
    .style(theme::bare_button)
    .padding(PAD_NAV_ITEM)
    .width(Length::Fill)
    .into()
}

// ── Canvas painters ─────────────────────────────────────

struct CirclePainter {
    color: Color,
    size: f32,
}

impl canvas::Program<Message> for CirclePainter {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let radius = self.size / 2.0;
        let circle = canvas::path::Path::circle(
            iced::Point::new(radius, radius),
            radius,
        );
        frame.fill(&circle, self.color);
        vec![frame.into_geometry()]
    }
}

// ── Thread card ─────────────────────────────────────────

pub fn thread_card(thread: &Thread, index: usize, selected: bool) -> Element<'_, Message> {
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

    let avatar = avatar_circle(sender, 28.0);

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

// ── Action / reply buttons ──────────────────────────────

pub fn action_icon_button<'a>(ico: iced::widget::Text<'a>, label: &'a str) -> Element<'a, Message> {
    button(
        row![
            ico.size(12).style(text::secondary),
            text(label).size(11).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::Noop)
    .padding(PAD_ICON_BTN)
    .style(theme::action_button)
    .into()
}

pub fn reply_button<'a>(ico: iced::widget::Text<'a>, label: &'a str) -> Element<'a, Message> {
    button(
        row![
            ico.size(14).style(text::secondary),
            text(label).size(12).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::Noop)
    .padding(PAD_BUTTON)
    .style(button::secondary)
    .into()
}

// ── Message card ────────────────────────────────────────

pub fn message_card(thread: &Thread) -> Element<'_, Message> {
    let sender = thread
        .from_name
        .as_deref()
        .or(thread.from_address.as_deref())
        .unwrap_or("(unknown)");

    let avatar = avatar_circle(sender, 32.0);
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
                text(sender).size(13).style(text::base),
                Space::new().width(Length::Fill),
                text(date_str).size(11).style(theme::text_tertiary),
            ],
            text(thread.from_address.as_deref().unwrap_or(""))
                .size(11)
                .style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let body_text = thread.snippet.as_deref().unwrap_or("(no preview available)");
    let body = container(text(body_text).size(13).style(text::secondary))
        .padding(iced::Padding::from([SPACE_SM, 0.0]));

    container(column![header, body].spacing(SPACE_XS))
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::message_card_container)
        .into()
}

// ── Empty state placeholder ─────────────────────────────

pub fn empty_placeholder<'a>(title: &'a str, subtitle: &'a str) -> Element<'a, Message> {
    container(
        column![
            text(title).size(16).style(theme::text_tertiary),
            text(subtitle).size(12).style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXS)
        .align_x(Alignment::Center),
    )
    .center_x(Length::Fill)
    .center_y(Length::Fill)
    .width(Length::Fill)
    .height(Length::Fill)
    .into()
}

// ── Section header / stat row ───────────────────────────

pub fn section_header<'a>(label: &'a str) -> Element<'a, Message> {
    container(text(label).size(10).style(theme::text_tertiary))
        .padding(PAD_SECTION_HEADER)
        .width(Length::Fill)
        .into()
}

pub fn stat_row<'a>(label: &'a str, value: &'a str) -> Element<'a, Message> {
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

// ── Canvas painters ─────────────────────────────────────

struct DotPainter {
    color: Color,
}

impl canvas::Program<Message> for DotPainter {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let circle = canvas::path::Path::circle(
            iced::Point::new(4.0, 4.0),
            4.0,
        );
        frame.fill(&circle, self.color);
        vec![frame.into_geometry()]
    }
}
