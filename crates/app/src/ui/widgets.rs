use iced::widget::{button, canvas, column, container, row, rule, text, Canvas, Space};
use iced::{mouse, Alignment, Color, Element, Length, Rectangle, Renderer, Theme};

use crate::db::{DateDisplay, Thread, ThreadAttachment, ThreadMessage};
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
                .color(theme::ON_AVATAR)
                .font(iced::Font { weight: iced::font::Weight::Bold, ..font::TEXT }),
        )
        .center(Length::Fill),
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

pub fn count_badge<'a, M: 'a>(count: i64) -> Element<'a, M> {
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavSize {
    /// Sidebar folder list — compact padding
    Compact,
    /// Settings tabs — more spacious padding
    Regular,
}

/// Generic navigation button used in both the sidebar and settings.
/// Accepts data only — builds its own two-slot (icon + label) structure.
/// Generic over message type so settings can use it with SettingsMessage.
pub fn nav_button<'a, M: Clone + 'a>(
    ico: Option<iced::widget::Text<'a>>,
    label: &'a str,
    active: bool,
    size: NavSize,
    badge: Option<i64>,
    on_press: M,
) -> Element<'a, M> {
    let label_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        theme::text_muted
    };
    let icon_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        theme::text_muted
    };
    let pad = match size {
        NavSize::Compact => PAD_NAV_ITEM,
        NavSize::Regular => PAD_SETTINGS_ROW,
    };
    let icon_size = match size {
        NavSize::Compact => ICON_MD,
        NavSize::Regular => ICON_XL,
    };
    let text_size = match size {
        NavSize::Compact => TEXT_MD,
        NavSize::Regular => TEXT_LG,
    };

    let mut content = row![].spacing(SPACE_XS).align_y(Alignment::Center);

    if let Some(ico) = ico {
        content = content.push(
            container(ico.size(icon_size).style(icon_style))
                .align_y(Alignment::Center),
        );
    }

    content = content.push(
        container(text(label).size(text_size).style(label_style))
            .align_y(Alignment::Center),
    );

    if let Some(count) = badge
        && count > 0
    {
        content = content
            .push(Space::new().width(Length::Fill))
            .push(count_badge(count));
    }

    button(content)
        .on_press(on_press)
        .padding(pad)
        .style(theme::nav_button(active))
        .width(Length::Fill)
        .into()
}

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
        col = col.push(nav_button(
            None,
            item.label,
            is_active,
            NavSize::Compact,
            Some(item.unread),
            on_press,
        ));
    }
    col.into()
}

pub fn label_nav_item<'a>(
    name: &'a str,
    _id: &'a str,
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
        row![
            color_dot(color),
            container(text(name).size(TEXT_MD).style(lbl_style))
                .align_y(Alignment::Center),
        ]
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
            container(text(title).size(TEXT_XS).style(theme::text_tertiary))
                .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            container(chevron.size(ICON_XS).style(theme::text_tertiary))
                .align_y(Alignment::Center),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle)
    .padding(PAD_COLLAPSIBLE_HEADER)
    .style(theme::action_button)
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

/// Icon type for dropdown items. The dropdown builds the
/// Element internally — callers never pass pre-built UI.
pub enum DropdownIcon<'a> {
    /// Renders an avatar circle from a name string.
    Avatar(&'a str),
    /// Renders an icon glyph from a codepoint char.
    Icon(char),
}

impl DropdownIcon<'_> {
    fn into_element<'a>(self, size: f32) -> Element<'a, Message> {
        match self {
            DropdownIcon::Avatar(name) => avatar_circle(name, size),
            DropdownIcon::Icon(codepoint) => icon::to_icon(codepoint)
                .size(ICON_XL)
                .style(text::secondary)
                .into(),
        }
    }
}

/// One entry in a dropdown menu.
pub struct DropdownEntry<'a> {
    pub icon: DropdownIcon<'a>,
    pub label: &'a str,
    pub selected: bool,
    pub on_press: Message,
}

/// A complete dropdown: closed trigger + optional open menu.
/// Both trigger and items share the same two-slot layout.
pub fn dropdown<'a>(
    trigger_icon: DropdownIcon<'a>,
    trigger_label: &'a str,
    open: bool,
    on_toggle: Message,
    items: Vec<DropdownEntry<'a>>,
) -> Element<'a, Message> {
    // trigger_button
    let trigger = button(
        row![
            // icon_slot: fixed size, content centered
            container(trigger_icon.into_element(AVATAR_DROPDOWN_TRIGGER))
                .width(SLOT_DROPDOWN)
                .height(SLOT_DROPDOWN)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            // label_slot: fills remaining width, vertically centered
            container(text(trigger_label).size(TEXT_MD).style(text::base))
                .width(Length::Fill)
                .align_y(Alignment::Center),
            // chevron_slot
            container(icon::chevron_down().size(ICON_SM).style(theme::text_tertiary))
                .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle.clone())
    .padding(PAD_DROPDOWN)
    .style(theme::action_button)
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
                    container(entry.icon.into_element(AVATAR_DROPDOWN_ITEM))
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
    .padding(PAD_DROPDOWN)
    .style(theme::floating_container)
    .width(Length::Fill);

    crate::ui::popover::popover(trigger)
        .popup(menu)
        .on_dismiss(on_toggle)
        .into()
}

// ── Select (settings-style dropdown) ────────────────────
// Ghost trigger (no background) with right-aligned label
// and chevron. Opens a floating menu of text options.

/// A select widget for choosing from a list of text options.
/// Trigger is transparent with right-aligned label + chevron.
/// Generic over message type.
pub fn select<'a, M: Clone + 'a>(
    options: &[&'a str],
    selected: &'a str,
    open: bool,
    on_toggle: M,
    on_select: impl Fn(String) -> M + 'a,
) -> Element<'a, M> {
    // The trigger has a fixed minimum width (SELECT_MIN_WIDTH) because the
    // popover overlay sizes its menu to the trigger's width. Without this,
    // the shrink-to-fit trigger would be too narrow for the menu items.
    // The fill spacer pushes label + chevron to the right edge.
    let trigger = button(
        row![
            // right-align spacer
            Space::new().width(Length::Fill),
            // label_slot
            container(text(selected).size(TEXT_MD).style(text::base))
                .align_y(Alignment::Center),
            // chevron_slot
            container(icon::chevron_down().size(ICON_MD).style(text::secondary))
                .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle.clone())
    .padding(PAD_SELECT_TRIGGER)
    .style(theme::ghost_button)
    .width(SELECT_MIN_WIDTH);

    if !open {
        return trigger.into();
    }

    let menu_items: Vec<Element<'a, M>> = options
        .iter()
        .map(|&option| {
            let is_selected = option == selected;
            let mut label_row = row![
                container(text(option).size(TEXT_MD).style(text::base))
                    .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center);

            if is_selected {
                label_row = label_row.push(
                    container(icon::check().size(ICON_MD).style(text::base))
                        .align_y(Alignment::Center),
                );
            }

            button(
                container(label_row)
                    .width(Length::Fill)
                    .align_y(Alignment::Center),
            )
            .on_press(on_select(option.to_string()))
            .padding(PAD_NAV_ITEM)
            .height(DROPDOWN_ITEM_HEIGHT)
            .style(theme::dropdown_button(is_selected))
            .width(Length::Fill)
            .into()
        })
        .collect();

    let menu = container(
        column(menu_items).spacing(SPACE_XXS).width(Length::Fill),
    )
    .padding(PAD_DROPDOWN)
    .style(theme::select_menu_container);

    crate::ui::popover::popover(trigger)
        .popup(menu)
        .on_dismiss(on_toggle.clone())
        .into()
}

// ── Compose button ──────────────────────────────────────

pub fn compose_button<'a>() -> Element<'a, Message> {
    button(
        container(
            row![
                container(icon::pencil().size(ICON_LG).color(theme::ON_AVATAR))
                    .align_y(Alignment::Center),
                container(text("Compose").size(TEXT_LG).color(theme::ON_AVATAR))
                    .align_y(Alignment::Center),
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
        container(
            row![
                container(icon::settings().size(ICON_LG))
                    .align_y(Alignment::Center),
                container(text("Settings").size(TEXT_LG))
                    .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill),
    )
    .on_press(Message::ToggleSettings)
    .style(theme::secondary_button)
    .padding(PAD_BUTTON)
    .width(Length::Fill)
    .into()
}

// ── Canvas painters ─────────────────────────────────────

/// Paints 6 vertical color stripes (bg, text, primary, success, warning, danger).
struct ThemePreviewPainter {
    colors: [Color; 6],
    radius: f32,
}

impl canvas::Program<Message> for ThemePreviewPainter {
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
        let stripe_width = bounds.width / 6.0;
        let r = self.radius;

        // Draw full rounded rect with first color, then paint stripes on top
        // This avoids gaps between stripes from rounding
        let bg_rect = canvas::path::Path::new(|b| {
            rounded_rect(b, bounds.width, bounds.height, r);
        });
        frame.fill(&bg_rect, self.colors[0]);

        // Middle stripes (no rounding needed)
        for i in 1..5 {
            let x = stripe_width * i as f32;
            let rect = canvas::path::Path::rectangle(
                iced::Point::new(x, 0.0),
                iced::Size::new(stripe_width, bounds.height),
            );
            frame.fill(&rect, self.colors[i]);
        }

        // Last stripe with right-side rounding
        let last = canvas::path::Path::new(|b| {
            let x = stripe_width * 5.0;
            b.move_to(iced::Point::new(x, 0.0));
            b.line_to(iced::Point::new(bounds.width - r, 0.0));
            b.arc_to(
                iced::Point::new(bounds.width, 0.0),
                iced::Point::new(bounds.width, r),
                r,
            );
            b.line_to(iced::Point::new(bounds.width, bounds.height - r));
            b.arc_to(
                iced::Point::new(bounds.width, bounds.height),
                iced::Point::new(bounds.width - r, bounds.height),
                r,
            );
            b.line_to(iced::Point::new(x, bounds.height));
            b.close();
        });
        frame.fill(&last, self.colors[5]);

        vec![frame.into_geometry()]
    }
}

fn rounded_rect(builder: &mut canvas::path::Builder, w: f32, h: f32, r: f32) {
    builder.move_to(iced::Point::new(r, 0.0));
    builder.line_to(iced::Point::new(w - r, 0.0));
    builder.arc_to(
        iced::Point::new(w, 0.0),
        iced::Point::new(w, r),
        r,
    );
    builder.line_to(iced::Point::new(w, h - r));
    builder.arc_to(
        iced::Point::new(w, h),
        iced::Point::new(w - r, h),
        r,
    );
    builder.line_to(iced::Point::new(r, h));
    builder.arc_to(
        iced::Point::new(0.0, h),
        iced::Point::new(0.0, h - r),
        r,
    );
    builder.line_to(iced::Point::new(0.0, r));
    builder.arc_to(
        iced::Point::new(0.0, 0.0),
        iced::Point::new(r, 0.0),
        r,
    );
    builder.close();
}

/// Theme preview: 6 vertical stripes in a rounded 16:9 rectangle.
pub fn theme_preview<'a>(
    palette: &iced::theme::palette::Seed,
    selected: bool,
    on_press: Message,
) -> Element<'a, Message> {
    let colors = [
        palette.background,
        palette.text,
        palette.primary,
        palette.success,
        palette.warning,
        palette.danger,

    ];

    let preview_width: f32 = 120.0;
    let preview_height: f32 = preview_width * 9.0 / 16.0;

    let preview_canvas = Canvas::new(ThemePreviewPainter {
        colors,
        radius: RADIUS_MD,
    })
    .width(preview_width)
    .height(preview_height);

    // 2px gap + 2px border for selected state
    let gap = 2.0;
    let border_width = 2.0;
    let total_inset = gap + border_width;

    if selected {
        container(container(preview_canvas).padding(total_inset))
            .style(theme::theme_selected_ring)
            .into()
    } else {
        button(container(preview_canvas).padding(total_inset))
            .on_press(on_press)
            .padding(0)
            .style(theme::bare_transparent_button)
            .into()
    }
}

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

// ── Label dot (thread card indicators) ──────────────────

pub fn label_dot<'a>(color: Color) -> Element<'a, Message> {
    let dot = Canvas::new(DotPainter { color })
        .width(LABEL_DOT_SIZE)
        .height(LABEL_DOT_SIZE);
    container(dot)
        .center_y(Length::Shrink)
        .into()
}

// ── Thread card ─────────────────────────────────────────

pub fn thread_card<'a>(
    thread: &'a Thread,
    index: usize,
    selected: bool,
    label_colors: &[(Color,)],
) -> Element<'a, Message> {
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

    // Sender: semibold if unread, normal if read
    let sender_font = if thread.is_read {
        font::TEXT
    } else {
        font::TEXT_SEMIBOLD
    };

    // Subject: accent if unread, muted if read; always normal weight
    let subject_style: fn(&Theme) -> text::Style = if thread.is_read {
        theme::text_muted
    } else {
        theme::text_accent
    };

    // Line 3 indicators: label dots + attachment icon
    let mut indicators = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
    for &(color,) in label_colors {
        indicators = indicators.push(label_dot(color));
    }
    if thread.has_attachments {
        indicators = indicators.push(icon::paperclip().size(ICON_XS).style(theme::text_tertiary));
    }

    // Line 1: sender + date
    let top_row = row![
        container(
            text(sender)
                .size(TEXT_MD)
                .style(text::base)
                .font(sender_font),
        )
        .width(Length::Fill),
        container(text(date_str).size(TEXT_XS).style(theme::text_tertiary)),
    ]
    .align_y(Alignment::Center);

    // Line 2: subject
    let subject_row = row![
        container(
            text(subject)
                .size(TEXT_MD)
                .style(subject_style)
                .font(font::TEXT)
                .wrapping(text::Wrapping::None),
        )
        .width(Length::Fill),
    ];

    // Line 3: snippet + indicators
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

    let content = column![top_row, subject_row, snippet_row]
        .spacing(SPACE_XXXS)
        .width(Length::Fill);

    button(
        container(content)
            .padding(PAD_THREAD_CARD)
            .height(THREAD_CARD_HEIGHT)
            .width(Length::Fill),
    )
    .on_press(Message::SelectThread(index))
    .padding(0)
    .style(theme::thread_card_button(selected, thread.is_starred))
    .width(Length::Fill)
    .into()
}

// ── Action / reply buttons ──────────────────────────────

pub fn action_icon_button<'a>(ico: iced::widget::Text<'a>, label: &'a str) -> Element<'a, Message> {
    button(
        row![
            container(ico.size(ICON_MD).style(text::secondary))
                .align_y(Alignment::Center),
            container(text(label).size(TEXT_SM).style(text::secondary))
                .align_y(Alignment::Center),
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
            container(ico.size(ICON_XL).style(text::secondary))
                .align_y(Alignment::Center),
            container(text(label).size(TEXT_MD).style(text::secondary))
                .align_y(Alignment::Center),
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

// ── Expanded message card ───────────────────────────────

pub fn expanded_message_card<'a>(
    msg: &'a ThreadMessage,
    index: usize,
    date_display: DateDisplay,
    first_message_date: Option<i64>,
) -> Element<'a, Message> {
    let sender = msg
        .from_name
        .as_deref()
        .or(msg.from_address.as_deref())
        .unwrap_or("(unknown)");

    let avatar = avatar_circle(sender, AVATAR_MESSAGE_CARD);
    let date_str = format_message_date(msg.date, first_message_date, date_display);

    let recipients = msg.to_addresses.as_deref().unwrap_or("");

    let header = row![
        avatar,
        column![
            row![
                container(
                    text(sender)
                        .size(TEXT_LG)
                        .font(font::TEXT_SEMIBOLD)
                        .style(text::base),
                )
                .align_y(Alignment::Center),
                Space::new().width(Length::Fill),
                container(
                    text(date_str)
                        .size(TEXT_SM)
                        .style(theme::text_tertiary),
                )
                .align_y(Alignment::Center),
            ]
            .align_y(Alignment::Center),
            text(recipients)
                .size(TEXT_SM)
                .style(theme::text_tertiary),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let body_text = msg.snippet.as_deref().unwrap_or("(no preview available)");
    let body = container(text(body_text).size(TEXT_LG).style(text::secondary))
        .padding(PAD_BODY);

    let actions = row![
        reply_button(icon::reply(), "Reply"),
        reply_button(icon::reply_all(), "Reply All"),
        reply_button(icon::forward(), "Forward"),
    ]
    .spacing(SPACE_XS);

    let card_content = column![header, body, actions].spacing(SPACE_XS);

    button(
        container(card_content)
            .padding(PAD_CARD)
            .width(Length::Fill)
            .style(theme::message_card_container),
    )
    .on_press(Message::ToggleMessageExpanded(index))
    .padding(0)
    .style(theme::bare_transparent_button)
    .width(Length::Fill)
    .into()
}

// ── Collapsed message row ───────────────────────────────

pub fn collapsed_message_row<'a>(
    msg: &'a ThreadMessage,
    index: usize,
) -> Element<'a, Message> {
    let sender = msg
        .from_name
        .as_deref()
        .or(msg.from_address.as_deref())
        .unwrap_or("(unknown)");

    let short_date = msg
        .date
        .and_then(|ts| {
            chrono::DateTime::from_timestamp(ts, 0).map(|dt| {
                dt.format("%b %d").to_string()
            })
        })
        .unwrap_or_default();

    let snippet = truncate_snippet(msg.snippet.as_deref(), 60);

    let content = row![
        container(text("\u{2014}").size(TEXT_SM).style(theme::text_tertiary))
            .align_y(Alignment::Center),
        container(
            text(sender)
                .size(TEXT_SM)
                .font(font::TEXT_SEMIBOLD)
                .style(text::base),
        )
        .align_y(Alignment::Center),
        container(text("\u{00B7}").size(TEXT_SM).style(theme::text_tertiary))
            .align_y(Alignment::Center),
        container(text(short_date).size(TEXT_SM).style(theme::text_tertiary))
            .align_y(Alignment::Center),
        container(text("\u{00B7}").size(TEXT_SM).style(theme::text_tertiary))
            .align_y(Alignment::Center),
        container(
            text(snippet)
                .size(TEXT_SM)
                .style(theme::text_tertiary)
                .wrapping(text::Wrapping::None),
        )
        .width(Length::Fill)
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let pad = iced::Padding {
        top: SPACE_XXS,
        right: SPACE_SM,
        bottom: SPACE_XXS,
        left: SPACE_SM,
    };

    button(
        container(content).padding(pad).width(Length::Fill),
    )
    .on_press(Message::ToggleMessageExpanded(index))
    .padding(0)
    .style(theme::collapsed_message_button)
    .width(Length::Fill)
    .into()
}

// ── Attachment card ─────────────────────────────────────

pub fn attachment_card<'a>(att: &'a ThreadAttachment, version_count: usize) -> Element<'a, Message> {
    let filename = att.filename.as_deref().unwrap_or("(unnamed)");
    let file_icon = file_type_icon(att.mime_type.as_deref());
    let meta = format_attachment_meta(att);

    let mut line1 = row![
        container(file_icon.size(ICON_MD).style(text::secondary))
            .align_y(Alignment::Center),
        container(
            text(filename)
                .size(TEXT_MD)
                .style(text::base)
                .wrapping(text::Wrapping::None),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    // Show version count badge for deduplicated attachments
    if version_count > 1 {
        line1 = line1.push(Space::new().width(SPACE_XS));
        line1 = line1.push(
            container(
                text(format!("{version_count} versions"))
                    .size(TEXT_XS)
                    .style(theme::text_tertiary),
            )
            .align_y(Alignment::Center),
        );
    }

    let line2 = text(meta).size(TEXT_SM).style(theme::text_tertiary);

    container(
        column![line1, line2].spacing(SPACE_XXXS),
    )
    .padding(PAD_NAV_ITEM)
    .style(theme::elevated_container)
    .width(Length::Fill)
    .into()
}

// ── Helpers ─────────────────────────────────────────────

fn file_type_icon<'a>(mime_type: Option<&str>) -> iced::widget::Text<'a> {
    match mime_type.unwrap_or("") {
        t if t.starts_with("image/") => icon::image(),
        t if t.contains("pdf") => icon::file_text(),
        t if t.contains("spreadsheet") || t.contains("excel") => icon::file_spreadsheet(),
        _ => icon::file(),
    }
}

fn format_message_date(
    timestamp: Option<i64>,
    first_message_timestamp: Option<i64>,
    display: DateDisplay,
) -> String {
    let Some(ts) = timestamp else { return String::new() };
    let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) else { return String::new() };

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
        DateDisplay::Absolute => {
            dt.format("%b %d, %Y, %l:%M %p").to_string().trim().to_string()
        }
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

fn format_attachment_meta(att: &ThreadAttachment) -> String {
    let type_label = mime_to_type_label(att.mime_type.as_deref());
    let size = format_file_size(att.size);
    let date = att.date
        .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
        .map(|dt| dt.format("%b %d").to_string())
        .unwrap_or_default();
    let sender = att.from_name.as_deref().unwrap_or("unknown");
    format!("{type_label} \u{00B7} {size} \u{00B7} {date} from {sender}")
}

fn mime_to_type_label(mime: Option<&str>) -> &'static str {
    match mime.unwrap_or("") {
        t if t.starts_with("image/") => "Image",
        t if t.contains("pdf") => "PDF",
        t if t.contains("spreadsheet") || t.contains("excel") => "Excel",
        t if t.contains("word") || t.contains("document") => "Word",
        t if t.contains("zip") || t.contains("archive") => "Archive",
        _ => "File",
    }
}

fn format_file_size(size: Option<i64>) -> String {
    match size {
        None => "\u{2014}".to_string(),
        Some(b) if b < 1024 => format!("{b} B"),
        Some(b) if b < 1024 * 1024 => format!("{:.0} KB", b as f64 / 1024.0),
        Some(b) => format!("{:.1} MB", b as f64 / (1024.0 * 1024.0)),
    }
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
