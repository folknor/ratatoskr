use iced::widget::{
    button, canvas, column, container, row, rule, scrollable, text, text_input, tooltip, Canvas,
    Space,
};
use iced::{mouse, Alignment, Color, Element, Length, Rectangle, Renderer, Theme};

use ratatoskr_command_palette::{BindingTable, CommandContext, CommandId, CommandRegistry};

use crate::db::{DateDisplay, Thread, ThreadAttachment, ThreadMessage};
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::Message;

// ── Command button ──────────────────────────────────────
// Builds a button from a CommandId, pulling label, availability,
// and keybinding hint from the registry. Disabled buttons are
// greyed out but visible. Emits ExecuteCommand on click.

/// Build a toolbar button for a registered command.
///
/// Pulls label (including toggle resolution like Star/Unstar),
/// availability, and keybinding hint from the registry and
/// binding table. Unavailable commands render as disabled buttons
/// with muted text. Keybinding hints appear as tooltips.
pub fn command_button<'a>(
    id: CommandId,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let desc = registry.get(id);
    let (label, available) = desc.map_or(("???", false), |d| {
        (d.resolved_label(ctx), (d.is_available)(ctx))
    });
    let keybinding = binding_table.display_binding(id);

    let label_style: fn(&Theme) -> text::Style = if available {
        text::secondary
    } else {
        theme::TextClass::Tertiary.style()
    };

    let label_text = text(label).size(TEXT_SM).style(label_style);
    let mut btn = button(
        container(label_text).align_y(Alignment::Center),
    )
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Action.style());

    if available {
        btn = btn.on_press(Message::ExecuteCommand(id));
    }

    if let Some(kb) = keybinding {
        tooltip(btn, text(kb).size(TEXT_XS), tooltip::Position::Bottom)
            .gap(SPACE_XXS)
            .style(theme::ContainerClass::Floating.style())
            .into()
    } else {
        btn.into()
    }
}

/// Build a toolbar button for a registered command, with an icon.
///
/// Same as [`command_button`] but prepends an icon glyph before the label.
pub fn command_icon_button<'a>(
    id: CommandId,
    ico: iced::widget::Text<'a>,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let desc = registry.get(id);
    let (label, available) = desc.map_or(("???", false), |d| {
        (d.resolved_label(ctx), (d.is_available)(ctx))
    });
    let keybinding = binding_table.display_binding(id);

    let label_style: fn(&Theme) -> text::Style = if available {
        text::secondary
    } else {
        theme::TextClass::Tertiary.style()
    };
    let icon_style: fn(&Theme) -> text::Style = label_style;

    let content = row![
        container(ico.size(ICON_MD).style(icon_style))
            .align_y(Alignment::Center),
        container(text(label).size(TEXT_SM).style(label_style))
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    let mut btn = button(content)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Action.style());

    if available {
        btn = btn.on_press(Message::ExecuteCommand(id));
    }

    if let Some(kb) = keybinding {
        tooltip(btn, text(kb).size(TEXT_XS), tooltip::Position::Bottom)
            .gap(SPACE_XXS)
            .style(theme::ContainerClass::Floating.style())
            .into()
    } else {
        btn.into()
    }
}

// ── Leading slot ───────────────────────────────────────
// Wraps any content (icon, avatar, dot) in a fixed-size
// centered container so all list items align their labels.

pub fn leading_slot<'a, M: 'a>(
    content: impl Into<Element<'a, M>>,
    size: f32,
) -> Element<'a, M> {
    container(content)
        .width(Length::Fixed(size))
        .height(Length::Fixed(size))
        .center(Length::Shrink)
        .into()
}

// ── Avatar ──────────────────────────────────────────────

pub fn avatar_circle<'a, M: 'a>(name: &str, size: f32) -> Element<'a, M> {
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
                .font(iced::Font { weight: iced::font::Weight::Bold, ..font::text() }),
        )
        .center(Length::Fill),
    ]
    .width(size)
    .height(size)
    .into()
}

pub fn color_dot<'a, M: 'a>(color: Color) -> Element<'a, M> {
    let dot = Canvas::new(DotPainter { color })
        .width(DOT_SIZE)
        .height(DOT_SIZE);
    container(dot)
        .center_y(Length::Shrink)
        .into()
}

/// A color dot at a custom size.
pub fn color_dot_sized<'a, M: 'a>(color: Color, size: f32) -> Element<'a, M> {
    let dot = Canvas::new(DotPainter { color })
        .width(size)
        .height(size);
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
        .style(theme::ContainerClass::Badge.style())
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
        theme::TextClass::Muted.style()
    };
    let icon_style: fn(&Theme) -> text::Style = if active {
        text::primary
    } else {
        theme::TextClass::Muted.style()
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
        .style(theme::ButtonClass::Nav { active }.style())
        .width(Length::Fill)
        .into()
}

pub struct NavItem<'a> {
    pub label: &'a str,
    pub id: &'a str,
    pub unread: i64,
}

pub fn nav_group<'a, M: Clone + 'a>(
    items: &[NavItem<'a>],
    selected_label: &'a Option<String>,
    on_select: impl Fn(Option<String>) -> M,
) -> Element<'a, M> {
    let mut col = column![].spacing(SPACE_XXS);
    for item in items {
        let is_active = match selected_label {
            Some(lid) => lid == item.id,
            None => item.id == "INBOX",
        };
        let on_press = if item.id == "INBOX" {
            on_select(None)
        } else {
            on_select(Some(item.id.to_string()))
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

pub fn label_nav_item<'a, M: Clone + 'a>(
    name: &'a str,
    _id: &'a str,
    color: Color,
    active: bool,
    on_press: M,
) -> Element<'a, M> {
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
    .style(theme::ButtonClass::Nav { active }.style())
    .width(Length::Fill)
    .into()
}

// ── Dividers & section breaks ───────────────────────────

pub fn divider<'a, M: 'a>() -> Element<'a, M> {
    rule::horizontal(1).style(theme::RuleClass::Divider.style()).into()
}

pub fn section_break<'a, M: 'a>() -> Element<'a, M> {
    column![
        Space::new().height(SPACE_XXS),
        divider(),
        Space::new().height(SPACE_XXS),
    ]
    .into()
}

// ── Collapsible section ─────────────────────────────────

pub fn collapsible_section<'a, M: Clone + 'a>(
    title: &'a str,
    expanded: bool,
    on_toggle: M,
    children: Vec<Element<'a, M>>,
) -> Element<'a, M> {
    let chevron = if expanded {
        icon::chevron_down()
    } else {
        icon::chevron_right()
    };

    let header = button(
        row![
            container(text(title).size(TEXT_XS).style(theme::TextClass::Tertiary.style()))
                .align_y(Alignment::Center),
            Space::new().width(Length::Fill),
            container(chevron.size(ICON_XS).style(theme::TextClass::Tertiary.style()))
                .align_y(Alignment::Center),
        ]
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle)
    .padding(PAD_COLLAPSIBLE_HEADER)
    .style(theme::ButtonClass::Action.style())
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
    /// Renders a filled color dot.
    ColorDot(Color),
}

impl DropdownIcon<'_> {
    fn into_element<'a, M: 'a>(self, size: f32) -> Element<'a, M> {
        match self {
            DropdownIcon::Avatar(name) => avatar_circle(name, size),
            DropdownIcon::Icon(codepoint) => icon::to_icon(codepoint)
                .size(ICON_XL)
                .style(text::secondary)
                .into(),
            DropdownIcon::ColorDot(color) => color_dot_sized(color, size),
        }
    }
}

/// One entry in a dropdown menu.
pub struct DropdownEntry<'a, M> {
    pub icon: DropdownIcon<'a>,
    pub label: &'a str,
    pub selected: bool,
    pub on_press: M,
}

/// A complete dropdown: closed trigger + optional open menu.
/// Both trigger and items share the same two-slot layout.
pub fn dropdown<'a, M: Clone + 'a>(
    trigger_icon: DropdownIcon<'a>,
    trigger_label: &'a str,
    open: bool,
    on_toggle: M,
    items: Vec<DropdownEntry<'a, M>>,
) -> Element<'a, M> {
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
            container(icon::chevron_down().size(ICON_SM).style(theme::TextClass::Tertiary.style()))
                .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .on_press(on_toggle.clone())
    .padding(PAD_DROPDOWN)
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill);

    if !open {
        return trigger.into();
    }

    let menu_items: Vec<Element<'a, M>> = items
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
            .style(theme::ButtonClass::Dropdown { selected: entry.selected }.style())
            .width(Length::Fill)
            .into()
        })
        .collect();

    let menu = container(
        column(menu_items).spacing(SPACE_XXS).width(Length::Fill),
    )
    .padding(PAD_DROPDOWN)
    .style(theme::ContainerClass::Floating.style())
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
    .style(theme::ButtonClass::Ghost.style())
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
            .style(theme::ButtonClass::Dropdown { selected: is_selected }.style())
            .width(Length::Fill)
            .into()
        })
        .collect();

    let menu = container(
        column(menu_items).spacing(SPACE_XXS).width(Length::Fill),
    )
    .padding(PAD_DROPDOWN)
    .style(theme::ContainerClass::SelectMenu.style());

    crate::ui::popover::popover(trigger)
        .popup(menu)
        .on_dismiss(on_toggle.clone())
        .into()
}

// ── Compose button ──────────────────────────────────────

pub fn compose_button<'a, M: Clone + 'a>(on_press: M) -> Element<'a, M> {
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
    .on_press(on_press)
    .padding(PAD_BUTTON)
    .style(theme::ButtonClass::Primary.style())
    .width(Length::Fill)
    .into()
}

// ── Settings button ─────────────────────────────────────

pub fn settings_button<'a, M: Clone + 'a>(on_press: M) -> Element<'a, M> {
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
    .on_press(on_press)
    .style(theme::ButtonClass::Secondary.style())
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

impl<M> canvas::Program<M> for ThemePreviewPainter {
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
pub fn theme_preview<'a, M: Clone + 'a>(
    palette: &iced::theme::palette::Seed,
    selected: bool,
    on_press: M,
) -> Element<'a, M> {
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
            .style(theme::ContainerClass::ThemeSelectedRing.style())
            .into()
    } else {
        button(container(preview_canvas).padding(total_inset))
            .on_press(on_press)
            .padding(0)
            .style(theme::ButtonClass::BareTransparent.style())
            .into()
    }
}

struct CirclePainter {
    color: Color,
    size: f32,
}

impl<M> canvas::Program<M> for CirclePainter {
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

pub fn label_dot<'a, M: 'a>(color: Color) -> Element<'a, M> {
    let dot = Canvas::new(DotPainter { color })
        .width(LABEL_DOT_SIZE)
        .height(LABEL_DOT_SIZE);
    container(dot)
        .center_y(Length::Shrink)
        .into()
}

// ── Thread card ─────────────────────────────────────────

pub fn thread_card<'a, M: Clone + 'a>(
    thread: &'a Thread,
    index: usize,
    selected: bool,
    label_colors: &[(Color,)],
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

    // Sender: semibold if unread, normal if read
    let sender_font = if thread.is_read {
        font::text()
    } else {
        font::text_semibold()
    };

    // Subject: accent if unread, muted if read; always normal weight
    let subject_style: fn(&Theme) -> text::Style = if thread.is_read {
        theme::TextClass::Muted.style()
    } else {
        theme::TextClass::Accent.style()
    };

    // Line 3 indicators: draft badge + label dots + attachment icon
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
        indicators = indicators.push(icon::paperclip().size(ICON_XS).style(theme::TextClass::Tertiary.style()));
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
        container(text(date_str).size(TEXT_XS).style(theme::TextClass::Tertiary.style())),
    ]
    .align_y(Alignment::Center);

    // Line 2: subject
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
    .on_press(on_select(index))
    .padding(0)
    .style(theme::ButtonClass::ThreadCard { selected, starred: thread.is_starred }.style())
    .width(Length::Fill)
    .into()
}

// ── Action / reply buttons ──────────────────────────────

pub fn action_icon_button<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: M,
) -> Element<'a, M> {
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
    .on_press(on_press)
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Action.style())
    .into()
}

pub fn reply_button<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: M,
) -> Element<'a, M> {
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
    .on_press(on_press)
    .padding(PAD_BUTTON)
    .style(button::secondary)
    .into()
}

// ── Message card ────────────────────────────────────────

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
                text(date_str).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
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

    let body_text = thread.snippet.as_deref().unwrap_or("(no preview available)");
    let body = container(text(body_text).size(TEXT_LG).style(text::secondary))
        .padding(PAD_BODY);

    container(column![header, body].spacing(SPACE_XS))
        .padding(PAD_CARD)
        .width(Length::Fill)
        .style(theme::ContainerClass::MessageCard.style())
        .into()
}

// ── Expanded message card ───────────────────────────────

#[allow(clippy::too_many_arguments)]
pub fn expanded_message_card<'a, M: Clone + 'a>(
    msg: &'a ThreadMessage,
    index: usize,
    date_display: DateDisplay,
    first_message_date: Option<i64>,
    on_toggle: impl Fn(usize) -> M,
    on_pop_out: impl Fn(usize) -> M,
    on_reply: impl Fn(usize) -> M,
    on_reply_all: impl Fn(usize) -> M,
    on_forward: impl Fn(usize) -> M,
    on_edit_contact: impl Fn(String) -> M + 'a,
    on_create_event: impl Fn(usize) -> M,
) -> Element<'a, M> {
    let sender = msg
        .from_name
        .as_deref()
        .or(msg.from_address.as_deref())
        .unwrap_or("(unknown)");

    let avatar = avatar_circle(sender, AVATAR_MESSAGE_CARD);
    let date_str = format_message_date(msg.date, first_message_date, date_display);

    let recipients = msg.to_addresses.as_deref().unwrap_or("");

    // Pop-out icon button
    let pop_out_btn = button(
        icon::external_link().size(ICON_MD).style(text::secondary),
    )
    .on_press(on_pop_out(index))
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::BareIcon.style());

    // Sender name — clickable to open contact editing
    let sender_email = msg
        .from_address
        .clone()
        .unwrap_or_default();
    let sender_element: Element<'a, M> = button(
        text(sender)
            .size(TEXT_LG)
            .font(font::text_semibold())
            .style(text::base),
    )
    .on_press(on_edit_contact(sender_email))
    .padding(0)
    .style(theme::ButtonClass::BareTransparent.style())
    .into();

    let header = row![
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
                pop_out_btn,
            ]
            .align_y(Alignment::Center)
            .spacing(SPACE_XS),
            text(recipients)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Start);

    let body: Element<'_, M> = if let Some(html) = msg.body_html.as_deref() {
        container(super::html_render::render_html::<M>(
            html,
            msg.body_text.as_deref(),
        ))
        .padding(PAD_BODY)
        .into()
    } else {
        let display = msg
            .body_text
            .as_deref()
            .or(msg.snippet.as_deref())
            .unwrap_or("(no preview available)");
        container(text(display).size(TEXT_LG).style(text::secondary))
            .padding(PAD_BODY)
            .into()
    };

    let cal_btn = button(
        row![
            icon::calendar().size(ICON_SM).style(text::secondary),
            text("Event").size(TEXT_SM).style(text::secondary),
        ].spacing(SPACE_XXS).align_y(Alignment::Center),
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

    button(
        container(card_content)
            .padding(PAD_CARD)
            .width(Length::Fill)
            .style(theme::ContainerClass::MessageCard.style()),
    )
    .on_press(on_toggle(index))
    .padding(0)
    .style(theme::ButtonClass::BareTransparent.style())
    .width(Length::Fill)
    .into()
}

// ── Collapsed message row ───────────────────────────────

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
            chrono::DateTime::from_timestamp(ts, 0).map(|dt| {
                dt.format("%b %d").to_string()
            })
        })
        .unwrap_or_default();

    let snippet = truncate_snippet(msg.snippet.as_deref(), 60);

    let content = row![
        container(text("\u{2014}").size(TEXT_SM).style(theme::TextClass::Tertiary.style()))
            .align_y(Alignment::Center),
        container(
            text(sender)
                .size(TEXT_SM)
                .font(font::text_semibold())
                .style(text::base),
        )
        .align_y(Alignment::Center),
        container(text("\u{00B7}").size(TEXT_SM).style(theme::TextClass::Tertiary.style()))
            .align_y(Alignment::Center),
        container(text(short_date).size(TEXT_SM).style(theme::TextClass::Tertiary.style()))
            .align_y(Alignment::Center),
        container(text("\u{00B7}").size(TEXT_SM).style(theme::TextClass::Tertiary.style()))
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

    let pad = iced::Padding {
        top: SPACE_XXS,
        right: SPACE_SM,
        bottom: SPACE_XXS,
        left: SPACE_SM,
    };

    button(
        container(content).padding(pad).width(Length::Fill),
    )
    .on_press(on_toggle(index))
    .padding(0)
    .style(theme::ButtonClass::CollapsedMessage.style())
    .width(Length::Fill)
    .into()
}

// ── Attachment card ─────────────────────────────────────

pub fn attachment_card<'a, M: 'a>(att: &'a ThreadAttachment, version_count: usize) -> Element<'a, M> {
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
                    .style(theme::TextClass::Tertiary.style()),
            )
            .align_y(Alignment::Center),
        );
    }

    let line2 = text(meta).size(TEXT_SM).style(theme::TextClass::Tertiary.style());

    container(
        column![line1, line2].spacing(SPACE_XXXS),
    )
    .padding(PAD_NAV_ITEM)
    .style(theme::ContainerClass::Elevated.style())
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

pub fn empty_placeholder<'a, M: 'a>(title: &'a str, subtitle: &'a str) -> Element<'a, M> {
    container(
        column![
            text(title).size(TEXT_TITLE).style(theme::TextClass::Tertiary.style()),
            text(subtitle).size(TEXT_MD).style(theme::TextClass::Tertiary.style()),
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

pub fn section_header<'a, M: 'a>(label: &'a str) -> Element<'a, M> {
    container(text(label).size(TEXT_XS).style(theme::TextClass::Tertiary.style()))
        .padding(PAD_SECTION_HEADER)
        .width(Length::Fill)
        .into()
}

pub fn stat_row<'a, M: 'a>(label: &'a str, value: &'a str) -> Element<'a, M> {
    container(
        row![
            text(label).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
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

impl<M> canvas::Program<M> for DotPainter {
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

// ── Emoji picker ────────────────────────────────────────

use super::emoji_picker::{EmojiCategory, EmojiEntry, EMOJI_TABLE};

/// Builds the emoji picker widget. The caller owns visibility state and positioning.
///
/// - `search_query`: current text in the search field
/// - `selected_category`: which category tab is active
/// - `on_select`: called with the emoji string when a user clicks one
/// - `on_category_changed`: called when the user clicks a category tab
/// - `on_search_changed`: called when the search input text changes
pub fn emoji_picker<'a, M: 'a + Clone>(
    search_query: &str,
    selected_category: EmojiCategory,
    on_select: impl Fn(&'static str) -> M + 'a,
    on_category_changed: impl Fn(EmojiCategory) -> M + 'a,
    on_search_changed: impl Fn(String) -> M + 'a,
) -> Element<'a, M> {
    // Filter emoji by search query or selected category.
    let filtered: Vec<&EmojiEntry> = if search_query.is_empty() {
        EMOJI_TABLE
            .iter()
            .filter(|e| e.category == selected_category)
            .collect()
    } else {
        let query = search_query.to_lowercase();
        EMOJI_TABLE
            .iter()
            .filter(|e| e.name.contains(&query))
            .collect()
    };

    // Search bar
    let search = text_input("Search emoji...", search_query)
        .on_input(on_search_changed)
        .padding(PAD_INPUT)
        .size(TEXT_MD)
        .style(theme::TextInputClass::Settings.style());

    // Category tabs
    let mut tab_row = row![].spacing(SPACE_XXXS).align_y(Alignment::Center);
    for &cat in EmojiCategory::ALL {
        let is_active = cat == selected_category;
        let tab = button(
            container(text(cat.tab_emoji()).size(TEXT_TITLE))
                .width(EMOJI_BUTTON_SIZE)
                .height(EMOJI_BUTTON_SIZE)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(on_category_changed(cat))
        .style(theme::ButtonClass::Chip { active: is_active }.style());
        tab_row = tab_row.push(tab);
    }

    // Emoji grid — build rows of EMOJI_GRID_COLUMNS items.
    let mut grid_col = column![].spacing(SPACE_XXXS);
    let mut current_row = row![].spacing(SPACE_XXXS);
    let mut col_idx = 0;

    for entry in &filtered {
        let emoji_btn = button(
            container(text(entry.emoji).size(TEXT_TITLE))
                .width(EMOJI_BUTTON_SIZE)
                .height(EMOJI_BUTTON_SIZE)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(on_select(entry.emoji))
        .style(theme::ButtonClass::BareIcon.style());

        current_row = current_row.push(emoji_btn);
        col_idx += 1;

        if col_idx >= EMOJI_GRID_COLUMNS {
            grid_col = grid_col.push(current_row);
            current_row = row![].spacing(SPACE_XXXS);
            col_idx = 0;
        }
    }

    // Push trailing partial row if any.
    if col_idx > 0 {
        for _ in col_idx..EMOJI_GRID_COLUMNS {
            current_row = current_row
                .push(Space::new().width(EMOJI_BUTTON_SIZE).height(EMOJI_BUTTON_SIZE));
        }
        grid_col = grid_col.push(current_row);
    }

    let grid_scrollable = scrollable(
        container(grid_col).padding([SPACE_XXS, 0.0]),
    )
    .spacing(SCROLLBAR_SPACING)
    .height(Length::Fill);

    // Assemble
    container(
        column![
            search,
            tab_row,
            grid_scrollable,
        ]
        .spacing(SPACE_XS),
    )
    .padding(SPACE_XS)
    .width(EMOJI_PICKER_WIDTH)
    .height(EMOJI_PICKER_MAX_HEIGHT)
    .style(theme::ContainerClass::SelectMenu.style())
    .into()
}

// ── Color palette grid ──────────────────────────────────
//
// Reusable grid of color swatches from the label-color presets.
// The `on_select` callback receives the preset index when a swatch
// is clicked.

/// Swatch canvas painter for the color palette grid.
struct SwatchPainter {
    color: Color,
    selected: bool,
    used: bool,
}

impl<M> canvas::Program<M> for SwatchPainter {
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
        let radius = bounds.width.min(bounds.height) / 2.0;
        let center =
            iced::Point::new(bounds.width / 2.0, bounds.height / 2.0);

        let circle = canvas::path::Path::circle(center, radius);

        let draw_color = if self.used && !self.selected {
            Color {
                a: COLOR_SWATCH_DIMMED_ALPHA,
                ..self.color
            }
        } else {
            self.color
        };

        frame.fill(&circle, draw_color);

        if self.used {
            swatch_check_mark(&mut frame, bounds, radius);
        }

        vec![frame.into_geometry()]
    }
}

/// Draw a small check-mark inside a swatch circle.
fn swatch_check_mark(
    frame: &mut canvas::Frame<Renderer>,
    bounds: Rectangle,
    radius: f32,
) {
    let check_color = Color::WHITE;
    let check = canvas::path::Path::new(|b| {
        let cx = bounds.width / 2.0;
        let cy = bounds.height / 2.0;
        let s = radius * COLOR_SWATCH_CHECK_SCALE;
        b.move_to(iced::Point::new(cx - s * 0.5, cy));
        b.line_to(iced::Point::new(cx - s * 0.1, cy + s * 0.4));
        b.line_to(iced::Point::new(cx + s * 0.5, cy - s * 0.3));
    });
    frame.stroke(
        &check,
        canvas::Stroke::default()
            .with_color(check_color)
            .with_width(2.0),
    );
}

/// Build a reusable color palette grid.
///
/// `selected` is the currently selected preset index (if any).
/// `used_colors` are background hex strings of already-assigned colors
/// (shown dimmed with a check mark).
/// `on_select` maps a preset index to the caller's message type.
pub fn color_palette_grid<'a, M: Clone + 'a>(
    selected: Option<usize>,
    used_colors: &[String],
    on_select: impl Fn(usize) -> M + 'a,
) -> Element<'a, M> {
    let presets = ratatoskr_label_colors::category_colors::all_presets();
    let mut grid = column![].spacing(SPACE_XS);
    let mut current_row = row![].spacing(SPACE_XS);

    for (i, &(_name, bg_hex, _fg_hex)) in presets.iter().enumerate() {
        let is_selected = selected == Some(i);
        let is_used = used_colors.iter().any(|c| c == bg_hex);
        let color = theme::hex_to_color(bg_hex);

        let swatch = Canvas::new(SwatchPainter {
            color,
            selected: is_selected,
            used: is_used,
        })
        .width(COLOR_SWATCH_SIZE)
        .height(COLOR_SWATCH_SIZE);

        let style = if is_selected {
            theme::ButtonClass::ColorSwatchSelected
        } else {
            theme::ButtonClass::BareTransparent
        };

        let swatch_btn = button(swatch)
            .on_press(on_select(i))
            .padding(PAD_COLOR_SWATCH)
            .style(style.style());

        current_row = current_row.push(swatch_btn);

        if (i + 1) % COLOR_PALETTE_COLUMNS == 0 {
            grid = grid.push(current_row);
            current_row = row![].spacing(SPACE_XS);
        }
    }

    if presets.len() % COLOR_PALETTE_COLUMNS != 0 {
        grid = grid.push(current_row);
    }

    grid.into()
}

// ── Spinner ──────────────────────────────────────────────

/// A simple spinning arc indicator.
///
/// Uses the frame's cache invalidation to animate. The spinner re-renders
/// on every frame while visible because the canvas `Program` always
/// returns `canvas::Action::request_redraw()`.
pub fn spinner<'a, M: 'a>(size: f32) -> Element<'a, M> {
    Canvas::new(SpinnerPainter {
        start: std::time::Instant::now(),
    })
    .width(size)
    .height(size)
    .into()
}

struct SpinnerPainter {
    start: std::time::Instant,
}

impl<M> canvas::Program<M> for SpinnerPainter {
    type State = ();

    fn draw(
        &self,
        _state: &Self::State,
        renderer: &Renderer,
        theme: &Theme,
        bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> Vec<canvas::Geometry<Renderer>> {
        let palette = theme.palette();
        let mut frame = canvas::Frame::new(renderer, bounds.size());
        let center = frame.center();
        let radius = bounds.width.min(bounds.height) / 2.0 - 2.0;

        let elapsed = self.start.elapsed().as_secs_f32();
        let start_angle = elapsed * 4.0; // rotations per second
        let sweep = std::f32::consts::FRAC_PI_2 * 3.0; // 270 degrees

        let path = canvas::path::Builder::new();
        let mut builder = canvas::path::Builder::new();
        builder.arc(canvas::path::Arc {
            center,
            radius,
            start_angle: iced::Radians(start_angle),
            end_angle: iced::Radians(start_angle + sweep),
        });
        let arc_path = builder.build();

        frame.stroke(
            &arc_path,
            canvas::Stroke::default()
                .with_color(palette.primary.base.color)
                .with_width(2.5),
        );
        drop(path);

        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        _state: &Self::State,
        _bounds: Rectangle,
        _cursor: mouse::Cursor,
    ) -> mouse::Interaction {
        mouse::Interaction::default()
    }
}
