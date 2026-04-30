use iced::widget::{
    Space, button, column, container, mouse_area, radio, row, slider, text, text_input,
};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::animated_toggler::animated_toggler;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::theme::RowPosition;
use crate::ui::undoable_text_input::undoable_text_input;

use super::types::*;

// ── Row builder type ────────────────────────────────────
//
// Each section item is a closure that receives the row's `RowPosition`
// (Top/Middle/Bottom/Only) so its hover background can match the section's
// `RADIUS_LG` outer corners while keeping `RADIUS_SM` on inner seams.
pub(super) type RowBuilder<'a> =
    Box<dyn FnOnce(RowPosition) -> Element<'a, SettingsMessage> + 'a>;

/// Wrap a pre-built `Element` so it can be passed alongside position-aware
/// row builders. Position is ignored - used for items that don't have hover
/// styling (sliders, info rows, externally-built elements).
pub(super) fn static_row<'a, E>(elem: E) -> RowBuilder<'a>
where
    E: Into<Element<'a, SettingsMessage>> + 'a,
{
    Box::new(move |_pos| elem.into())
}

/// Materialize a `RowBuilder` as an `Element` outside of a section context
/// (assumes `RowPosition::Only`). Use when a builder is pushed onto a plain
/// `column!` rather than into a `section()` items vec.
pub(super) fn build_row<'a>(b: RowBuilder<'a>) -> Element<'a, SettingsMessage> {
    b(RowPosition::Only)
}

/// Returns the position for index `i` in a list of length `n`.
fn position_for(i: usize, n: usize) -> RowPosition {
    match (n, i) {
        (1, _) => RowPosition::Only,
        (_, 0) => RowPosition::Top,
        (_, last) if last + 1 == n => RowPosition::Bottom,
        _ => RowPosition::Middle,
    }
}

/// Closure factory: builds the action-button style for a settings row at
/// the given position, capturing `position` so the closure satisfies
/// iced's `'static`-friendly style fn signature.
fn settings_row_style(
    position: RowPosition,
) -> impl Fn(&iced::Theme, button::Status) -> button::Style + 'static {
    move |theme, status| theme::style_settings_row_button(theme, status, position)
}

// ── Shared setting widgets ──────────────────────────────

pub(super) fn section<'a>(
    title: &'a str,
    items: Vec<RowBuilder<'a>>,
) -> Element<'a, SettingsMessage> {
    section_inner(Some(title), None, None, items)
}

pub(super) fn section_with_subtitle<'a>(
    title: &'a str,
    subtitle: &'a str,
    items: Vec<RowBuilder<'a>>,
) -> Element<'a, SettingsMessage> {
    section_inner(Some(title), Some(subtitle), None, items)
}

pub(super) fn section_with_help<'a>(
    title: &'a str,
    help: SectionHelp<'a>,
    items: Vec<RowBuilder<'a>>,
) -> Element<'a, SettingsMessage> {
    section_inner(Some(title), None, Some(help), items)
}

pub(super) fn section_untitled<'a>(
    items: Vec<RowBuilder<'a>>,
) -> Element<'a, SettingsMessage> {
    section_inner(None, None, None, items)
}

/// Help tooltip configuration for a section header.
pub(super) struct SectionHelp<'a> {
    pub id: &'a str,
    pub content: Element<'a, SettingsMessage>,
    pub visible: bool,
}

fn section_inner<'a>(
    title: Option<&'a str>,
    subtitle: Option<&'a str>,
    help: Option<SectionHelp<'a>>,
    items: Vec<RowBuilder<'a>>,
) -> Element<'a, SettingsMessage> {
    let n = items.len();
    let mut col = column![].width(Length::Fill).padding(1);
    for (i, builder) in items.into_iter().enumerate() {
        if i > 0 {
            col =
                col.push(iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()));
        }
        col = col.push(builder(position_for(i, n)));
    }
    let section_box = container(col)
        .width(Length::Fill)
        .style(theme::ContainerClass::SettingsSection.style());

    if let Some(title) = title {
        let title_text: Element<'a, SettingsMessage> = text(title)
            .size(TEXT_XL)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            })
            .into();

        let header_row: Element<'a, SettingsMessage> = if let Some(help_cfg) = help {
            let help_id = help_cfg.id.to_string();
            let help_id_hover = help_id.clone();
            let help_id_unhover = help_id.clone();
            let help_icon = mouse_area(
                button(
                    container(icon::help_circle().size(ICON_XL).style(theme::TextClass::Muted.style()))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center),
                )
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::BareIcon.style()),
            )
            .on_enter(SettingsMessage::HelpHover(help_id_hover))
            .on_exit(SettingsMessage::HelpUnhover(help_id_unhover));

            let mut pop = crate::ui::anchored_overlay::anchored_overlay(help_icon)
                .position(crate::ui::anchored_overlay::AnchorPosition::BelowRight)
                .popup_width(HELP_TOOLTIP_WIDTH);

            if help_cfg.visible {
                pop = pop
                    .popup(
                        container(help_cfg.content)
                            .padding(PAD_SETTINGS_ROW)
                            .width(Length::Fill)
                            .style(theme::ContainerClass::Floating.style()),
                    );
            }

            row![title_text, Space::new().width(Length::Fill), pop,]
                .align_y(Alignment::Center)
                .into()
        } else {
            title_text
        };

        let mut header = column![header_row].spacing(SPACE_XXXS);

        if let Some(subtitle) = subtitle {
            header = header.push(
                text(subtitle)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            );
        }

        column![header, section_box].spacing(SPACE_XS)
    } else {
        column![section_box]
    }
    .into()
}

pub(super) fn settings_row_container<'a>(
    height: f32,
    content: impl Into<iced::Element<'a, SettingsMessage>>,
) -> RowBuilder<'a> {
    let content = content.into();
    Box::new(move |_pos| {
        container(content)
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(height)
            .align_y(Alignment::Center)
            .into()
    })
}

pub(super) fn setting_row<'a>(
    label: &'a str,
    control: Element<'a, SettingsMessage>,
    on_press: SettingsMessage,
) -> RowBuilder<'a> {
    setting_row_with_description(label, None, control, on_press)
}

/// `setting_row` with an optional secondary line beneath the label, matching
/// `toggle_row`'s two-line layout. The row grows to `SETTINGS_TOGGLE_ROW_HEIGHT`
/// when a description is present.
pub(super) fn setting_row_with_description<'a>(
    label: &'a str,
    description: Option<&'a str>,
    control: Element<'a, SettingsMessage>,
    on_press: SettingsMessage,
) -> RowBuilder<'a> {
    Box::new(move |position| {
        let label_col: Element<'a, SettingsMessage> = if let Some(desc) = description {
            column![
                text(label).size(TEXT_LG).style(text::base),
                text(desc)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ]
            .spacing(SPACE_XXXS)
            .into()
        } else {
            text(label).size(TEXT_LG).style(text::base).into()
        };

        let height = if description.is_some() {
            SETTINGS_TOGGLE_ROW_HEIGHT
        } else {
            SETTINGS_ROW_HEIGHT
        };

        button(
            container(
                row![
                    container(label_col).align_y(Alignment::Center),
                    Space::new().width(Length::Fill),
                    control,
                ]
                .align_y(Alignment::Center),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(height)
            .align_y(Alignment::Center),
        )
        .on_press(on_press)
        .padding(0)
        .style(settings_row_style(position))
        .width(Length::Fill)
        .into()
    })
}

pub(super) fn toggle_row<'a>(
    label: &'a str,
    description: &'a str,
    value: bool,
    on_toggle: impl Fn(bool) -> SettingsMessage + 'a,
) -> RowBuilder<'a> {
    Box::new(move |position| {
        // Compute the button's press message before on_toggle is moved into the toggler.
        // The toggler captures its own click events, so the button only fires when the
        // user clicks outside the knob (e.g. on the label). No double-firing.
        let on_press_msg = on_toggle(!value);
        button(
            container(
                row![
                    column![
                        text(label).size(TEXT_LG).style(text::base),
                        text(description)
                            .size(TEXT_SM)
                            .style(theme::TextClass::Tertiary.style()),
                    ]
                    .spacing(SPACE_XXXS),
                    Space::new().width(Length::Fill),
                    animated_toggler(value)
                        .size(TEXT_HEADING)
                        .on_toggle(on_toggle)
                        .style(theme::TogglerClass::Settings.style()),
                ]
                .align_y(Alignment::Center),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(SETTINGS_TOGGLE_ROW_HEIGHT)
            .align_y(Alignment::Center),
        )
        .on_press(on_press_msg)
        .padding(0)
        .style(settings_row_style(position))
        .width(Length::Fill)
        .into()
    })
}

pub(super) fn info_row(label: &str, value: &str) -> RowBuilder<'static> {
    let label_owned = label.to_string();
    let value_owned = value.to_string();
    Box::new(move |_pos| {
        let value_for_clipboard = value_owned.clone();
        container(
            row![
                column![
                    text(label_owned)
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                    text_input("", &value_owned)
                        .on_input(|_| SettingsMessage::Noop)
                        .size(TEXT_LG)
                        .padding(0)
                        .style(theme::TextInputClass::Inline.style()),
                ]
                .spacing(SPACE_XXXS)
                .width(Length::Fill),
                button(
                    container(icon::copy().size(ICON_MD).style(text::base))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center),
                )
                .on_press(SettingsMessage::CopyToClipboard(value_for_clipboard))
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::BareIcon.style()),
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .into()
    })
}

pub(super) fn input_row(
    id: &str,
    label: &str,
    placeholder: &str,
    value: &str,
    on_input: impl Fn(String) -> SettingsMessage + 'static,
    field: InputField,
) -> RowBuilder<'static> {
    let id_owned = id.to_string();
    let label_owned = label.to_string();
    let placeholder_owned = placeholder.to_string();
    let value_owned = value.to_string();
    Box::new(move |position| {
        let id_clone = id_owned.clone();
        mouse_area(
            button(
                container(
                    row![
                        column![
                            text(label_owned.clone())
                                .size(TEXT_SM)
                                .style(theme::TextClass::Tertiary.style()),
                            undoable_text_input(&placeholder_owned, &value_owned)
                                .id(id_clone.clone())
                                .on_input(on_input)
                                .on_undo(SettingsMessage::UndoInput(field))
                                .on_redo(SettingsMessage::RedoInput(field))
                                .size(TEXT_LG)
                                .padding(0)
                                .style(theme::TextInputClass::Inline.style()),
                        ]
                        .spacing(SPACE_XXXS)
                        .width(Length::Fill),
                        container(icon::pencil().size(ICON_MD).style(text::base))
                            .align_x(Alignment::Center)
                            .align_y(Alignment::Center),
                    ]
                    .spacing(SPACE_SM)
                    .align_y(Alignment::Center),
                )
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
            )
            .on_press(SettingsMessage::FocusInput(id_clone))
            .padding(0)
            .style(settings_row_style(position))
            .width(Length::Fill),
        )
        .interaction(iced::mouse::Interaction::Text)
        .into()
    })
}

pub(super) fn coming_soon_row<'a>(feature: &'a str) -> RowBuilder<'a> {
    Box::new(move |_pos| {
        container(
            text(format!("{feature} coming soon."))
                .size(TEXT_LG)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .height(SETTINGS_ROW_HEIGHT)
        .align_y(Alignment::Center)
        .into()
    })
}

/// A row with a label on the left (50%) and an optional icon + slider on the right (50%).
/// No hover effect - only direct slider interaction. The slider has a strong snap toward `default`.
pub(super) fn slider_row<'a>(
    label: &'a str,
    icon: Option<iced::widget::Text<'a>>,
    range: std::ops::RangeInclusive<f32>,
    value: f32,
    default: f32,
    step: f32,
    on_change: impl Fn(f32) -> SettingsMessage + 'a,
    on_release: Option<SettingsMessage>,
) -> RowBuilder<'a> {
    Box::new(move |_pos| {
        let mut slider_widget = slider(range, value, on_change)
            .default(default)
            .step(step)
            .style(theme::SliderClass::Settings.style())
            .width(Length::Fill);
        if let Some(msg) = on_release {
            slider_widget = slider_widget.on_release(msg);
        }

        let right_content: Element<'a, SettingsMessage> = if let Some(ic) = icon {
            row![
                container(ic.size(ICON_XL).style(text::secondary)).align_y(Alignment::Center),
                slider_widget,
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into()
        } else {
            slider_widget.into()
        };

        container(
            row![
                container(text(label).size(TEXT_LG).style(text::base))
                    .align_y(Alignment::Center)
                    .width(Length::FillPortion(1)),
                container(right_content)
                    .align_y(Alignment::Center)
                    .width(Length::FillPortion(1)),
            ]
            .align_y(Alignment::Center),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .height(SETTINGS_ROW_HEIGHT)
        .align_y(Alignment::Center)
        .into()
    })
}

/// A group of mutually exclusive radio options, rendered as rows with hover effects.
/// Each row has a radio circle on the left, label text a fixed distance away.
/// Radio groups must always have their own `section()` - don't mix with other row types.
pub(super) fn radio_group<'a, V>(
    options: &'a [(&'a str, V)],
    selected: Option<V>,
    on_select: impl Fn(V) -> SettingsMessage + 'a + Copy,
) -> Vec<RowBuilder<'a>>
where
    V: Copy + Eq + 'a,
{
    options
        .iter()
        .map(|(label, value)| {
            let label = *label;
            let value = *value;
            let row_builder: RowBuilder<'a> = Box::new(move |position| {
                let msg = on_select(value);
                button(
                    container(
                        row![
                            radio("", value, selected, on_select)
                                .size(RADIO_SIZE)
                                .spacing(0)
                                .style(theme::RadioClass::Settings.style()),
                            container(text(label).size(TEXT_LG).style(text::base))
                                .align_y(Alignment::Center),
                        ]
                        .spacing(RADIO_LABEL_SPACING)
                        .align_y(Alignment::Center),
                    )
                    .padding(PAD_SETTINGS_ROW)
                    .width(Length::Fill)
                    .height(SETTINGS_ROW_HEIGHT)
                    .align_y(Alignment::Center),
                )
                .on_press(msg)
                .padding(0)
                .style(settings_row_style(position))
                .width(Length::Fill)
                .into()
            });
            row_builder
        })
        .collect()
}

/// An editable, reorderable list with drag handles, optional toggles/menus/remove buttons,
/// and an "Add" button at the bottom. This is the full-featured private implementation;
/// public wrappers will expose only the slots each use case needs.
///
/// Drag reordering uses a single `mouse_area` wrapping the entire list so that
/// `on_move` continues to fire as the cursor leaves individual row bounds.
/// The grip `on_press` initiates the drag; the list-level `on_move` computes
/// the target index from the cursor's Y position relative to the list top.
pub(super) fn editable_list<'a>(
    list_id: &'a str,
    items: &'a [EditableItem],
    add_label: &'a str,
    drag_state: &'a Option<DragState>,
) -> RowBuilder<'a> {
    Box::new(move |outer_position| {
        let id = list_id.to_string();

        let mut col = column![].width(Length::Fill);

        // Internal row count = items + Add button.
        let internal_n = items.len() + 1;

        for (i, item) in items.iter().enumerate() {
            if i > 0 {
                col = col.push(
                    iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
                );
            }

            let is_drag_item = drag_state
                .as_ref()
                .is_some_and(|d| d.list_id == list_id && d.dragging_index == i && d.is_dragging);

            // ── Left half: grip + label ──
            let lid_grip = id.clone();
            let grip_slot = mouse_area(
                container(
                    icon::grip_vertical()
                        .size(ICON_MD)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .width(GRIP_SLOT_WIDTH)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::ListGripPress(lid_grip, i))
            .interaction(iced::mouse::Interaction::Grab);

            let label_slot = container(text(&item.label).size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center)
                .width(Length::Fill);

            let left_half = row![grip_slot, label_slot]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center)
                .width(Length::FillPortion(1));

            // ── Right half: optional toggle, menu, remove - all float right ──
            let mut right_items: Vec<Element<'a, SettingsMessage>> = Vec::new();
            right_items.push(Space::new().width(Length::Fill).into());

            if let Some(enabled) = item.enabled {
                let idx = i;
                let lid = id.clone();
                right_items.push(
                    animated_toggler(enabled)
                        .size(TEXT_HEADING)
                        .on_toggle(move |v| SettingsMessage::ListToggle(lid.clone(), idx, v))
                        .style(theme::TogglerClass::Settings.style())
                        .into(),
                );
            }

            // Menu button (...)
            right_items.push(
                button(
                    container(icon::ellipsis().size(ICON_MD).style(text::secondary))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center),
                )
                .on_press(SettingsMessage::ListMenu(id.clone(), i))
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::BareIcon.style())
                .into(),
            );

            // Remove button
            right_items.push(
                button(
                    container(icon::x().size(ICON_MD).style(text::secondary))
                        .align_x(Alignment::Center)
                        .align_y(Alignment::Center),
                )
                .on_press(SettingsMessage::ListRemove(id.clone(), i))
                .padding(PAD_ICON_BTN)
                .style(theme::ButtonClass::BareIcon.style())
                .into(),
            );

            let right_half = iced::widget::row(right_items)
                .spacing(SPACE_XS)
                .align_y(Alignment::Center)
                .width(Length::FillPortion(1));

            let item_row = row![left_half, right_half].align_y(Alignment::Center);

            // Button for hover effect + row click (toggle).
            let lid_click = id.clone();

            let mut inner_container = container(item_row)
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill)
                .height(SETTINGS_ROW_HEIGHT)
                .align_y(Alignment::Center);

            if is_drag_item {
                inner_container = inner_container.style(theme::ContainerClass::DraggingRow.style());
            }

            // Compose internal position with outer position so that the
            // editable_list as a whole inherits the section's outer corners.
            let internal_pos = position_for(i, internal_n);
            let effective = compose_positions(outer_position, internal_pos);

            let item_btn = button(inner_container)
                .on_press(SettingsMessage::ListRowClick(lid_click, i))
                .padding(0)
                .style(settings_row_style(effective))
                .width(Length::Fill);

            col = col.push(item_btn);
        }

        // Divider before Add button (if there are items)
        if !items.is_empty() {
            col = col
                .push(iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()));
        }

        // Add button - label centered
        let add_id = id.clone();
        let add_internal_pos = position_for(internal_n.saturating_sub(1), internal_n);
        let add_effective = compose_positions(outer_position, add_internal_pos);
        let add_btn = button(
            container(
                row![
                    icon::plus().size(ICON_MD).style(text::base),
                    text(add_label)
                        .size(TEXT_LG)
                        .style(text::base)
                        .font(iced::Font {
                            weight: iced::font::Weight::Bold,
                            ..crate::font::text()
                        }),
                ]
                .spacing(SPACE_XS)
                .align_y(Alignment::Center),
            )
            .center_x(Length::Fill)
            .align_y(Alignment::Center),
        )
        .on_press(SettingsMessage::ListAdd(add_id))
        .padding(PAD_SETTINGS_ROW)
        .style(settings_row_style(add_effective))
        .width(Length::Fill)
        .height(SETTINGS_ROW_HEIGHT);

        col = col.push(add_btn);

        // Wrap entire list in a single mouse_area for drag tracking.
        // on_move gives us Y relative to the list top, so we can compute
        // the target index directly: target = (y / row_height).
        let lid_move = id.clone();
        let lid_release = id;
        let lid_exit = lid_release.clone();
        let list_area = mouse_area(col)
            .on_release(SettingsMessage::ListDragEnd(lid_release))
            .on_exit(SettingsMessage::ListDragEnd(lid_exit))
            .on_move(move |point| SettingsMessage::ListDragMove(lid_move.clone(), point));

        list_area.into()
    })
}

/// Compose an outer (section-level) position with an inner (sub-list) position
/// so a multi-row helper that fills a section keeps the section's outer corners
/// on its first/last rows. Only the corners aligned with the outer position
/// can be `LG`-rounded; otherwise the inner seams stay `SM`.
fn compose_positions(outer: RowPosition, inner: RowPosition) -> RowPosition {
    match (outer, inner) {
        (RowPosition::Only, p) => p,
        (RowPosition::Top, RowPosition::Top) => RowPosition::Top,
        (RowPosition::Top, RowPosition::Only) => RowPosition::Top,
        (RowPosition::Bottom, RowPosition::Bottom) => RowPosition::Bottom,
        (RowPosition::Bottom, RowPosition::Only) => RowPosition::Bottom,
        _ => RowPosition::Middle,
    }
}

/// The action type determines the trailing icon.
#[derive(Debug, Clone, Copy)]
pub(super) enum ActionKind {
    /// Opens an external URL - shows external_link icon.
    Url,
    /// In-app action or slide-in overlay - shows arrow_right icon.
    InApp,
}

/// A full-row button with optional leading icon, label + optional description,
/// and a trailing icon indicating the action type. The entire row is the click
/// target - no nested buttons. Follows the rule that section rows never contain buttons.
pub(super) fn action_row<'a>(
    label: &'a str,
    description: Option<&'a str>,
    icon: Option<iced::widget::Text<'a>>,
    kind: ActionKind,
    on_press: SettingsMessage,
) -> RowBuilder<'a> {
    Box::new(move |position| {
        let mut content = row![].spacing(SPACE_SM).align_y(Alignment::Center);

        if let Some(ico) = icon {
            content = content.push(
                container(ico.size(ICON_XL).style(text::secondary)).align_y(Alignment::Center),
            );
        }

        let label_col: Element<'a, SettingsMessage> = if let Some(desc) = description {
            column![
                text(label).size(TEXT_LG).style(text::base),
                text(desc)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ]
            .spacing(SPACE_XXXS)
            .into()
        } else {
            text(label).size(TEXT_LG).style(text::base).into()
        };

        content = content.push(label_col);
        content = content.push(Space::new().width(Length::Fill));

        let trailing = match kind {
            ActionKind::Url => icon::external_link(),
            ActionKind::InApp => icon::arrow_right(),
        };
        content = content
            .push(container(trailing.size(ICON_XL).style(text::base)).align_y(Alignment::Center));

        button(content)
            .on_press(on_press)
            .padding(PAD_SETTINGS_ROW)
            .style(settings_row_style(position))
            .width(Length::Fill)
            .into()
    })
}
