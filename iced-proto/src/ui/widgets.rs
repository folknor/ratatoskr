use iced::widget::{button, canvas, column, container, row, rule, text, Canvas, Space};
use iced::{mouse, Alignment, Color, Element, Length, Rectangle, Renderer, Theme};

use crate::db::Thread;
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::Message;

// ── Leading slot ───────────────────────────────────────
// Wraps any content (icon, avatar, dot) in a fixed-size
// centered container so all list items align their labels.

pub fn leading_slot<'a>(
    content: impl Into<Element<'a, Message>>,
    size: f32,
) -> Element<'a, Message> {
    container(content)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .center(Length::Shrink)
        .into()
}

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
                .color(theme::ON_AVATAR),
        )
        .center(Length::Shrink),
    ]
    .width(size)
    .height(size)
    .into()
}

pub fn color_dot<'a>(color: Color) -> Element<'a, Message> {
    let dot = Canvas::new(DotPainter { color })
        .width(DOT_SIZE)
        .height(DOT_SIZE);
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
    container(text(label).size(TEXT_XS).style(text::secondary))
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

    let mut content = row![text(label).size(TEXT_MD).style(label_style)]
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
        row![color_dot(color), text(name).size(TEXT_MD).style(lbl_style)]
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
            text(title).size(TEXT_XS).style(theme::text_tertiary),
            Space::new().width(Length::Fill),
            chevron.size(ICON_XS).style(theme::text_tertiary),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle)
    .padding(PAD_COLLAPSIBLE_HEADER)
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

// ── Dropdown ────────────────────────────────────────────
// Fully opaque dropdown widget. Callers provide data only,
// never layout elements. The dropdown builds its own
// two-slot (icon + label) structure for both the trigger
// and every menu item.

/// One entry in a dropdown menu.
pub struct DropdownEntry<'a> {
    pub icon: Element<'a, Message>,
    pub label: &'a str,
    pub selected: bool,
    pub on_press: Message,
}

/// A complete dropdown: closed trigger + optional open menu.
/// Both trigger and items share the same two-slot layout.
pub fn dropdown<'a>(
    trigger_icon: Element<'a, Message>,
    trigger_label: &'a str,
    open: bool,
    on_toggle: Message,
    items: Vec<DropdownEntry<'a>>,
) -> Element<'a, Message> {
    // trigger_button
    let trigger = button(
        row![
            // icon_slot: fixed size, content centered
            container(trigger_icon)
                .width(SLOT_DROPDOWN)
                .height(SLOT_DROPDOWN)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            // label_slot: fills remaining width, vertically centered
            container(text(trigger_label).size(TEXT_MD).style(text::base))
                .width(Length::Fill)
                .align_y(Alignment::Center),
            // chevron_slot
            icon::chevron_down().size(ICON_SM).style(theme::text_tertiary),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle)
    .padding(PAD_ACCOUNT)
    .style(theme::bare_button)
    .width(Length::Fill);

    if !open {
        return trigger.into();
    }

    let menu_items: Vec<Element<'a, Message>> = items
        .into_iter()
        .map(|entry| {
            // item_button
            button(
                row![
                    // icon_slot: fixed size, content centered
                    container(entry.icon)
                        .width(SLOT_DROPDOWN)
                        .height(SLOT_DROPDOWN)
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center),
                    // label_slot: fills remaining width, vertically centered
                    container(text(entry.label).size(TEXT_MD).style(text::base))
                        .width(Length::Fill)
                        .align_y(Alignment::Center),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
            )
            .on_press(entry.on_press)
            .padding(PAD_NAV_ITEM)
            .height(DROPDOWN_ITEM_HEIGHT)
            .style(theme::dropdown_button(entry.selected))
            .width(Length::Fill)
            .into()
        })
        .collect();

    let menu = container(
        column(menu_items).spacing(SPACE_XXS).width(Length::Fill),
    )
    .padding(PAD_ICON_BTN)
    .style(theme::floating_container)
    .width(Length::Fill);

    crate::ui::popover::popover(trigger)
        .popup(menu)
        .into()
}

// ── Compose button ──────────────────────────────────────

pub fn compose_button<'a>() -> Element<'a, Message> {
    button(
        container(
            row![
                icon::pencil().size(ICON_LG).color(theme::ON_AVATAR),
                text("Compose").size(TEXT_LG).color(theme::ON_AVATAR),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill),
    )
    .on_press(Message::Compose)
    .padding(PAD_BUTTON)
    .style(theme::primary_button)
    .width(Length::Fill)
    .into()
}

// ── Settings button ─────────────────────────────────────

pub fn settings_button<'a>() -> Element<'a, Message> {
    button(
        row![
            icon::settings().size(ICON_MD).style(text::secondary),
            text("Settings").size(TEXT_MD).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(Message::ToggleSettings)
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

    let avatar = avatar_circle(sender, AVATAR_THREAD_CARD);

    let mut indicators = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    if thread.has_attachments {
        indicators = indicators.push(icon::paperclip().size(ICON_XS).style(theme::text_tertiary));
    }
    if thread.is_starred {
        indicators = indicators.push(icon::star().size(ICON_SM).style(text::warning));
    }
    if thread.message_count > 1 {
        indicators = indicators.push(
            container(
                text(thread.message_count.to_string())
                    .size(TEXT_XS)
                    .style(theme::text_tertiary),
            )
            .padding(PAD_BADGE)
            .style(theme::badge_container),
        );
    }

    let top_row = row![
        text(sender)
            .size(TEXT_MD)
            .style(name_style)
            .font(iced::Font { weight, ..font::TEXT }),
        Space::new().width(Length::Fill),
        text(date_str).size(TEXT_XS).style(theme::text_tertiary),
    ]
    .align_y(Alignment::Center);

    let subject_row = row![
        text(subject)
            .size(TEXT_MD)
            .style(name_style)
            .font(iced::Font { weight, ..font::TEXT })
            .wrapping(text::Wrapping::None),
    ];

    let snippet_row = row![
        text(snippet)
            .size(TEXT_SM)
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
            ico.size(ICON_MD).style(text::secondary),
            text(label).size(TEXT_SM).style(text::secondary),
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
            ico.size(ICON_XL).style(text::secondary),
            text(label).size(TEXT_MD).style(text::secondary),
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
                text(date_str).size(TEXT_SM).style(theme::text_tertiary),
            ],
            text(thread.from_address.as_deref().unwrap_or(""))
                .size(TEXT_SM)
                .style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let body_text = thread.snippet.as_deref().unwrap_or("(no preview available)");
    let body = container(text(body_text).size(TEXT_LG).style(text::secondary))
        .padding(PAD_BODY);

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
            text(title).size(TEXT_TITLE).style(theme::text_tertiary),
            text(subtitle).size(TEXT_MD).style(theme::text_tertiary),
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
    container(text(label).size(TEXT_XS).style(theme::text_tertiary))
        .padding(PAD_SECTION_HEADER)
        .width(Length::Fill)
        .into()
}

pub fn stat_row<'a>(label: &'a str, value: &'a str) -> Element<'a, Message> {
    container(
        row![
            text(label).size(TEXT_SM).style(theme::text_tertiary),
            Space::new().width(Length::Fill),
            text(value).size(TEXT_SM).style(text::secondary),
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
        let radius = DOT_SIZE / 2.0;
        let circle = canvas::path::Path::circle(
            iced::Point::new(radius, radius),
            radius,
        );
        frame.fill(&circle, self.color);
        vec![frame.into_geometry()]
    }
}
