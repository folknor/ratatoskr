use iced::widget::{button, canvas, column, container, row, rule, text, Canvas, Space};
use iced::{mouse, Alignment, Element, Length, Rectangle, Renderer, Theme};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::Message;

/// Colored circle with initial letter, used for avatars.
pub fn avatar_circle<'a>(name: &str, size: f32) -> Element<'a, Message> {
    let color = theme::avatar_color(name);
    let letter = theme::initial(name);

    let circle = Canvas::new(CirclePainter { color, size })
        .width(size)
        .height(size);

    // Overlay the letter on the circle
    iced::widget::stack![
        circle,
        container(
            text(letter)
                .size(size * 0.45)
                .color(iced::Color::WHITE),
        )
        .center(Length::Fill),
    ]
    .width(size)
    .height(size)
    .into()
}

struct CirclePainter {
    color: iced::Color,
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

/// Small colored dot for labels in sidebar.
pub fn color_dot<'a>(color: iced::Color) -> Element<'a, Message> {
    let dot = Canvas::new(DotPainter { color })
        .width(8)
        .height(8);
    container(dot)
        .center_y(Length::Shrink)
        .into()
}

// ── Unread count badge ──────────────────────────────────

pub fn count_badge<'a>(count: i64) -> Element<'a, Message> {
    if count == 0 {
        return Space::new().width(0).height(0).into();
    }
    let label = if count > 999 {
        "999+".to_string()
    } else {
        count.to_string()
    };
    container(
        text(label)
            .size(10)
            .style(text::secondary),
    )
    .padding(PAD_BADGE)
    .style(theme::badge_container)
    .into()
}

/// A sidebar nav item with an optional unread count badge on the right.
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

// ── Horizontal divider ──────────────────────────────────

pub fn divider<'a>() -> Element<'a, Message> {
    rule::horizontal(1).style(theme::divider_rule).into()
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

// ── Canvas painters ─────────────────────────────────────

struct DotPainter {
    color: iced::Color,
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
