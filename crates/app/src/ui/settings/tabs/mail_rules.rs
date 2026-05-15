use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Element, Length, Padding};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::theme::RowPosition;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{AccountLabelRow, AccountLabelsGroup};

pub(super) fn mail_rules_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Labels",
        vec![labels_list_builder(&state.labels_by_account)],
    ));

    if !state.demo_filters.is_empty() {
        col = col.push(section(
            "Filters",
            vec![editable_list(
                "filters",
                &state.demo_filters,
                "Add Filter",
                &state.drag_state,
            )],
        ));
    }
    col.into()
}

/// One RowBuilder that emits the whole labels list as a single column.
/// For each account: a fat header row + a per-account drag-enabled
/// mouse_area wrapping that account's label rows. Single "+ Add Label"
/// at the bottom. Matches the structural pattern used by `editable_list`
/// so internal/outer corner-rounding composes cleanly, and per-account
/// drag works because each account has its own mouse_area + list_id.
fn labels_list_builder<'a>(
    groups: &'a [AccountLabelsGroup],
) -> RowBuilder<'a> {
    Box::new(move |outer_position| {
        let internal_n = groups
            .iter()
            .map(|g| 1 + g.labels.len())
            .sum::<usize>()
            + 1; // +1 for the trailing Add row
        let mut col = column![].width(Length::Fill);

        let mut internal_index: usize = 0;
        for group in groups {
            if internal_index > 0 {
                col = col.push(
                    iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
                );
            }
            let header_pos = compose_positions(
                outer_position,
                position_for(internal_index, internal_n),
            );
            col = col.push(account_header_element(group, header_pos));
            internal_index += 1;

            let mut sub = column![].width(Length::Fill);
            for (sub_idx, lbl) in group.labels.iter().enumerate() {
                sub = sub.push(
                    iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
                );
                let row_pos = compose_positions(
                    outer_position,
                    position_for(internal_index, internal_n),
                );
                sub = sub.push(label_row_element(lbl, sub_idx, row_pos));
                internal_index += 1;
            }

            let list_id = format!("labels:{}", group.account_id);
            let on_move_id = list_id.clone();
            let on_end_id = list_id.clone();
            col = col.push(
                mouse_area(sub)
                    .on_move(move |point| {
                        SettingsMessage::ListDragMove(on_move_id.clone(), point)
                    })
                    .on_release(SettingsMessage::ListDragEnd(on_end_id.clone()))
                    .on_exit(SettingsMessage::ListDragEnd(on_end_id)),
            );
        }

        if internal_index > 0 {
            col = col.push(
                iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
            );
        }
        let add_pos = compose_positions(
            outer_position,
            position_for(internal_n.saturating_sub(1), internal_n),
        );
        col = col.push(add_label_row(add_pos));

        col.into()
    })
}

fn account_header_element<'a>(
    group: &'a AccountLabelsGroup,
    position: RowPosition,
) -> Element<'a, SettingsMessage> {
    let dot: Element<'a, SettingsMessage> = group
        .account_color
        .as_deref()
        .map(|hex| widgets::color_dot::<SettingsMessage>(theme::hex_to_color(hex)))
        .unwrap_or_else(|| Space::new().width(SPACE_SM).height(SPACE_SM).into());

    let content = row![
        dot,
        text(group.account_name.clone()).size(TEXT_LG).style(text::base),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let _ = position;
    container(content)
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .height(SETTINGS_SECTION_HEADER_HEIGHT)
        .align_y(Alignment::Center)
        .into()
}

fn label_row_element<'a>(
    lbl: &AccountLabelRow,
    sub_index: usize,
    position: RowPosition,
) -> Element<'a, SettingsMessage> {
    let list_id = format!("labels:{}", lbl.account_id);
    let grip = mouse_area(
        container(
            icon::grip_vertical()
                .size(ICON_MD)
                .style(theme::TextClass::Tertiary.style()),
        )
        .width(GRIP_SLOT_WIDTH)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::ListGripPress(list_id, sub_index))
    .interaction(iced::mouse::Interaction::Grab);

    let pill = container(Space::new().width(28.0).height(16.0))
        .style({
            let bg = theme::hex_to_color(&lbl.color_bg);
            move |_theme: &iced::Theme| iced::widget::container::Style {
                background: Some(bg.into()),
                border: iced::Border {
                    radius: RADIUS_LG.into(),
                    ..Default::default()
                },
                ..Default::default()
            }
        });

    let identity = text(lbl.name.clone()).size(TEXT_LG).style(text::base);

    let content = row![
        grip,
        identity,
        Space::new().width(Length::Fill),
        pill,
        Space::new().width(SPACE_XS),
        container(
            icon::chevron_right()
                .size(ICON_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let inner = container(content)
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .height(SETTINGS_ROW_HEIGHT)
        .align_y(Alignment::Center);

    let account_id = lbl.account_id.clone();
    let label_id = lbl.label_id.clone();

    button(inner)
        .on_press(SettingsMessage::OpenLabelEditor {
            account_id,
            label_id,
        })
        .padding(0)
        .style(settings_row_style(position))
        .width(Length::Fill)
        .into()
}

fn add_label_row<'a>(position: RowPosition) -> Element<'a, SettingsMessage> {
    button(
        container(
            row![
                icon::plus().size(ICON_MD).style(text::base),
                text("Add Label")
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
    .on_press(SettingsMessage::OpenLabelEditor {
        account_id: String::new(),
        label_id: String::new(),
    })
    .padding(Padding::ZERO)
    .style(settings_row_style(position))
    .width(Length::Fill)
    .height(SETTINGS_ROW_HEIGHT)
    .into()
}
