//! Labels tab.
//!
//! Two sections:
//! - Top: user-visible label groups (the things that render in the sidebar
//!   LABELS section). Flat list, drag-to-reorder, click to edit.
//! - Bottom: raw provider labels grouped by account. Informational; no
//!   drag, no colour pill - users do not interact with raw labels directly,
//!   they add them to a group above.
//!
//! See `reference/glossary/folders-labels.md` for the underlying model.

use iced::widget::{Space, button, column, container, mouse_area, row, scrollable, text};
use iced::{Alignment, Element, Length, Padding};

use crate::icon;
use crate::ui::label_paint::LabelPaint;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::theme::RowPosition;
use crate::ui::widgets;
use rtsk::db::queries_extra::navigation::{
    AccountLabelRow, AccountLabelsGroup, SettingsLabelGroupRow,
};

pub(super) fn labels_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section_with_subtitle(
        "Labels",
        "These are your labels. They appear in the sidebar, in the command palette, and on every thread you have tagged. A label is a coloured name that bundles one or more underlying tags from your accounts - applying it to a thread writes the right tag in each account.",
        vec![label_groups_list_builder(&state.label_groups, &state.drag_state)],
    ));

    col = col.push(section_with_subtitle(
        "From your accounts",
        "These are the raw tags your providers expose - Gmail labels, Outlook categories, IMAP keywords. Nothing here appears anywhere else in Ratatoskr until you add it to a label above. Listed for reference, and for editing each tag's name or colour at the provider.",
        vec![provider_labels_list_builder(&state.labels_by_account)],
    ));

    col.into()
}

// ── Top section: label groups ──────────────────────────────

fn label_groups_list_builder<'a>(
    groups: &'a [SettingsLabelGroupRow],
    drag: &'a Option<DragState>,
) -> RowBuilder<'a> {
    Box::new(move |outer_position| {
        let n = groups.len() + 1; // +1 for the Add row
        let mut col = column![].width(Length::Fill);

        let list_id = "label-groups".to_owned();
        let mut sub = column![].width(Length::Fill);
        for (idx, group) in groups.iter().enumerate() {
            if idx > 0 {
                sub = sub.push(
                    iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
                );
            }
            let row_pos = compose_positions(outer_position, position_for(idx, n));
            sub = sub.push(label_group_row(group, idx, row_pos, drag));
        }

        let on_move_id = list_id.clone();
        let on_end_id = list_id.clone();
        col = col.push(
            mouse_area(sub)
                .on_move(move |point| SettingsMessage::ListDragMove(on_move_id.clone(), point))
                .on_release(SettingsMessage::ListDragEnd(on_end_id.clone()))
                .on_exit(SettingsMessage::ListDragEnd(on_end_id)),
        );

        if !groups.is_empty() {
            col =
                col.push(iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()));
        }
        let add_pos = compose_positions(outer_position, position_for(n - 1, n));
        col = col.push(add_label_group_row(add_pos));

        col.into()
    })
}

fn label_group_row<'a>(
    group: &SettingsLabelGroupRow,
    sub_index: usize,
    position: RowPosition,
    drag: &Option<DragState>,
) -> Element<'a, SettingsMessage> {
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
    .on_press(SettingsMessage::ListGripPress(
        "label-groups".to_owned(),
        sub_index,
    ))
    .interaction(iced::mouse::Interaction::Grab);

    let pill = container(Space::new().width(28.0).height(16.0)).style({
        let paint = LabelPaint::from_hex_pair(&group.color_bg, &group.color_fg);
        move |_theme: &iced::Theme| iced::widget::container::Style {
            background: Some(paint.bg().into()),
            border: iced::Border {
                radius: RADIUS_LG.into(),
                ..Default::default()
            },
            ..Default::default()
        }
    });

    let subtitle = if group.member_count == 1 {
        "Bundles 1 tag".to_owned()
    } else {
        format!("Bundles {} tags", group.member_count)
    };
    let identity = column![
        text(group.name.clone()).size(TEXT_LG).style(text::base),
        text(subtitle).size(TEXT_SM).style(text::secondary),
    ]
    .spacing(SPACE_XXXS)
    .width(Length::Fill);

    let content = row![
        grip,
        identity,
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

    let is_being_dragged = drag.as_ref().is_some_and(|d| {
        d.list_id == "label-groups" && d.dragging_index == sub_index && d.is_dragging
    });

    let mut inner = container(content)
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .height(SETTINGS_TOGGLE_ROW_HEIGHT)
        .align_y(Alignment::Center);

    if is_being_dragged {
        inner = inner.style(theme::ContainerClass::DraggingRow.style());
    }

    button(inner)
        .on_press(SettingsMessage::OpenLabelGroupEditor {
            group_id: Some(group.id),
        })
        .padding(0)
        .style(settings_row_style(position))
        .width(Length::Fill)
        .into()
}

fn add_label_group_row<'a>(position: RowPosition) -> Element<'a, SettingsMessage> {
    button(
        container(
            row![
                icon::plus().size(ICON_MD).style(text::base),
                text("Add Label")
                    .size(TEXT_LG)
                    .style(text::base)
                    .font(crate::font::text_bold()),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::OpenLabelGroupEditor { group_id: None })
    .padding(Padding::ZERO)
    .style(settings_row_style(position))
    .width(Length::Fill)
    .height(SETTINGS_ROW_HEIGHT)
    .into()
}

// ── Bottom section: raw provider labels (informational) ────

fn provider_labels_list_builder<'a>(groups: &'a [AccountLabelsGroup]) -> RowBuilder<'a> {
    Box::new(move |outer_position| {
        let internal_n = groups.iter().map(|g| 1 + g.labels.len()).sum::<usize>() + 1; // +1 for the trailing Add row
        let mut col = column![].width(Length::Fill);

        let mut internal_index: usize = 0;
        for group in groups {
            if internal_index > 0 {
                col = col.push(
                    iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
                );
            }
            let header_pos =
                compose_positions(outer_position, position_for(internal_index, internal_n));
            col = col.push(account_header_element(group, header_pos));
            internal_index += 1;

            for lbl in &group.labels {
                col = col.push(
                    iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
                );
                let row_pos =
                    compose_positions(outer_position, position_for(internal_index, internal_n));
                col = col.push(provider_label_row(lbl)(row_pos));
                internal_index += 1;
            }
        }

        if internal_index > 0 {
            col =
                col.push(iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()));
        }
        let add_pos = compose_positions(
            outer_position,
            position_for(internal_n.saturating_sub(1), internal_n),
        );
        col = col.push(add_provider_label_row(add_pos));

        col.into()
    })
}

/// "+ Create new tag" row at the bottom of the per-account section.
/// Opens the per-account label editor in create mode; the existing
/// `label.create` stub fans the new tag out across every account.
fn add_provider_label_row<'a>(position: RowPosition) -> Element<'a, SettingsMessage> {
    button(
        container(
            row![
                icon::plus().size(ICON_MD).style(text::base),
                text("Create new tag")
                    .size(TEXT_LG)
                    .style(text::base)
                    .font(crate::font::text_bold()),
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

fn account_header_element<'a>(
    group: &'a AccountLabelsGroup,
    position: RowPosition,
) -> Element<'a, SettingsMessage> {
    let dot: Element<'a, SettingsMessage> = group
        .account_color
        .as_deref()
        .map(|hex| widgets::color_dot::<SettingsMessage>(theme::hex_to_color(hex)))
        .unwrap_or_else(|| Space::new().width(SPACE_SM).height(SPACE_SM).into());

    let name_slot = container(
        text(group.account_name.clone())
            .size(TEXT_LG)
            .style(text::base)
            .font(crate::font::text_bold()),
    )
    .align_y(Alignment::Center)
    .width(Length::Fill);

    let content = row![dot, name_slot]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    let _ = position;
    container(content)
        .padding(iced::Padding {
            top: 0.0,
            right: PAD_SETTINGS_ROW.right,
            bottom: 0.0,
            left: PAD_SETTINGS_ROW.left,
        })
        .width(Length::Fill)
        .height(SETTINGS_SECTION_HEADER_HEIGHT)
        .align_y(Alignment::Center)
        .into()
}

/// Provider-label row. Click opens the per-account label editor (the
/// pre-existing editor for renaming/recolouring/deleting one raw label).
fn provider_label_row<'a>(lbl: &'a AccountLabelRow) -> RowBuilder<'a> {
    let chevron: Element<'a, SettingsMessage> = container(
        icon::chevron_right()
            .size(ICON_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .align_y(Alignment::Center)
    .into();
    setting_row(
        &lbl.name,
        chevron,
        SettingsMessage::OpenLabelEditor {
            account_id: lbl.account_id.clone(),
            label_id: lbl.label_id.clone(),
        },
    )
}

// ── Editor sheets ──────────────────────────────────────────

pub(super) fn label_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.editing_label else {
        return column![].into();
    };

    let is_new = editor.label_id.is_empty();
    let title = if is_new { "New tag" } else { "Edit tag" };

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        column![
            text(title)
                .size(TEXT_HEADING)
                .style(text::base)
                .font(crate::font::text_bold()),
            text(
                "Provider tag - lives on the account itself, not in Ratatoskr. \
                 Edits here change the tag at the provider. To make this tag \
                 appear in the sidebar, add it to a Label. Changes are not \
                 saved automatically - click Save at the bottom when you're done."
            )
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXS),
    );

    col = col.push(section(
        "Name",
        vec![input_row(
            "label-name",
            "Name",
            "Tag name",
            &editor.name,
            SettingsMessage::LabelEditorNameChanged,
            InputField::LabelName,
        )],
    ));

    col = col.push(section_with_subtitle(
        "Colour",
        "Not shown anywhere in Ratatoskr - the tag's colour only matters in your provider's own client (Gmail web, Outlook, etc.). Changing it here updates it there.",
        vec![static_row(
            container(widgets::color_palette_grid(
                editor.color_index,
                &[],
                SettingsMessage::LabelEditorColorChanged,
                Some(SettingsMessage::LabelEditorOpenCustomColor),
            ))
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        )],
    ));

    col = col.push(label_editor_buttons(editor));

    col.into()
}

fn label_editor_buttons<'a>(editor: &'a LabelEditorState) -> Element<'a, SettingsMessage> {
    let is_new = editor.label_id.is_empty();
    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if !is_new && !editor.is_undeletable {
        if editor.show_delete_confirmation {
            btn_row = btn_row.push(
                button(text("Confirm delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::LabelEditorDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
            btn_row = btn_row.push(
                button(text("Cancel").size(TEXT_LG))
                    .on_press(SettingsMessage::LabelEditorCancelDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Ghost.style()),
            );
        } else {
            btn_row = btn_row.push(
                button(text("Delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::LabelEditorConfirmDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
        }
    }

    btn_row = btn_row.push(Space::new().width(Length::Fill));

    btn_row = btn_row.push(
        button(text("Cancel").size(TEXT_LG))
            .on_press(SettingsMessage::LabelEditorCancel)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style()),
    );

    let save_label = if is_new { "Create" } else { "Save" };
    let name_filled = !editor.name.trim().is_empty();
    let can_save = name_filled && (is_new || editor.dirty);
    let mut save_btn = button(container(text(save_label).size(TEXT_LG)).center_x(Length::Fill))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
    if can_save {
        save_btn = save_btn.on_press(SettingsMessage::LabelEditorSave);
    }
    btn_row = btn_row.push(save_btn);

    btn_row.into()
}

/// Stub editor sheet for a user-visible label group. The sections here are
/// placeholders for the real editor; we wire up structure now so we can
/// iterate on layout/UX without designing in the abstract.
pub(super) fn label_group_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.editing_label_group else {
        return column![].into();
    };

    let is_new = editor.group_id.is_none();
    let title = if is_new { "New Label" } else { "Edit Label" };

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        column![
            text(title)
                .size(TEXT_HEADING)
                .style(text::base)
                .font(crate::font::text_bold()),
            text(
                "Give your label a name and colour, then pick the underlying \
                 provider tags it represents. Applying this label to a thread \
                 writes each underlying tag in its account. Changes are not \
                 saved automatically - click Save at the bottom when you're done."
            )
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXS),
    );

    // Name.
    col = col.push(section(
        "Name",
        vec![input_row(
            "label-group-name",
            "Name",
            "Label name",
            &editor.name,
            SettingsMessage::LabelGroupEditorNameChanged,
            InputField::GroupName,
        )],
    ));

    col = col.push(section(
        "Colour",
        vec![static_row(
            container(widgets::color_palette_grid(
                editor.color_index,
                &[],
                SettingsMessage::LabelGroupEditorColorChanged,
                Some(SettingsMessage::LabelGroupEditorOpenCustomColor),
            ))
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        )],
    ));

    col = col.push(label_group_add_tags_section(editor, state));
    col = col.push(label_group_members_section(editor, state));

    col = col.push(label_group_editor_buttons(editor));

    col.into()
}

fn label_group_add_tags_section<'a>(
    editor: &'a LabelGroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    section_with_subtitle(
        "Add Tags",
        "Pick tags from your accounts to bundle into this label.",
        vec![static_row(label_group_add_candidates_panel(editor, state))],
    )
}

fn label_group_add_candidates_panel<'a>(
    editor: &'a LabelGroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    // Flatten labels_by_account into a single candidate list, excluding
    // tags already members. Account context (name + colour dot) travels
    // with each candidate so the right-side meta column reads cleanly.
    struct Candidate<'a> {
        label: &'a AccountLabelRow,
        account_name: &'a str,
        account_color: Option<&'a str>,
    }
    let candidates: Vec<Candidate<'a>> = state
        .labels_by_account
        .iter()
        .flat_map(|grp| {
            grp.labels.iter().map(move |lbl| Candidate {
                label: lbl,
                account_name: grp.account_name.as_str(),
                account_color: grp.account_color.as_deref(),
            })
        })
        .filter(|c| {
            !editor
                .members
                .iter()
                .any(|(a, l)| a == &c.label.account_id && l == &c.label.label_id)
        })
        .collect();

    let panel: Element<'_, SettingsMessage> = if candidates.is_empty() {
        container(
            text("Every tag from your accounts is already in this label.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .height(PEOPLE_PANEL_HEIGHT)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(theme::style_recessed_list_panel)
        .into()
    } else {
        let mut col = column![].spacing(PEOPLE_PILL_SPACING).width(Length::Fill);
        for c in candidates {
            col = col.push(tag_add_candidate_pill(
                c.label,
                c.account_name,
                c.account_color,
            ));
        }

        container(
            scrollable(container(col).padding(PAD_CARD).width(Length::Fill))
                .direction(iced::widget::scrollable::Direction::Vertical(
                    iced::widget::scrollable::Scrollbar::new()
                        .width(6)
                        .scroller_width(6)
                        .margin(SPACE_XXS),
                ))
                .height(Length::Fill),
        )
        .padding(iced::Padding {
            top: SPACE_XS,
            right: 0.0,
            bottom: SPACE_XS,
            left: 0.0,
        })
        .width(Length::Fill)
        .height(PEOPLE_PANEL_HEIGHT)
        .clip(true)
        .style(theme::style_recessed_list_panel)
        .into()
    };

    container(panel)
        .padding(SPACE_XS)
        .width(Length::Fill)
        .into()
}

fn tag_add_candidate_pill<'a>(
    lbl: &'a AccountLabelRow,
    account_name: &'a str,
    account_color: Option<&'a str>,
) -> Element<'a, SettingsMessage> {
    button(
        container(
            row![
                container(icon::plus().size(ICON_SM).style(text::primary))
                    .align_y(Alignment::Center),
                container(text(&lbl.name).size(TEXT_LG).style(text::base))
                    .align_y(Alignment::Center)
                    .width(Length::Fill),
                account_meta(account_name, account_color),
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::LabelGroupEditorAddMember(
        lbl.account_id.clone(),
        lbl.label_id.clone(),
    ))
    .padding(0)
    .style(theme::style_pill_card_button)
    .width(Length::Fill)
    .into()
}

fn label_group_members_section<'a>(
    editor: &'a LabelGroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let title = format!("Members ({})", editor.members.len());

    let panel: Element<'_, SettingsMessage> = if editor.members.is_empty() {
        let empty_panel = container(
            text("No tags yet. Use the list above to add some.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .height(PEOPLE_PANEL_HEIGHT)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(theme::style_recessed_list_panel);
        container(empty_panel)
            .padding(SPACE_XS)
            .width(Length::Fill)
            .into()
    } else {
        label_group_members_panel(editor, state)
    };

    section_dynamic_with_subtitle(
        title,
        "Click a tag to remove it from this label.".to_string(),
        vec![static_row(panel)],
    )
}

fn label_group_members_panel<'a>(
    editor: &'a LabelGroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(PEOPLE_PILL_SPACING).width(Length::Fill);
    for (account_id, label_id) in &editor.members {
        let lookup = state
            .labels_by_account
            .iter()
            .find(|g| &g.account_id == account_id)
            .and_then(|g| {
                let lbl = g.labels.iter().find(|l| &l.label_id == label_id)?;
                Some((lbl, g))
            });
        if let Some((lbl, group)) = lookup {
            col = col.push(tag_member_pill(
                lbl,
                group.account_name.as_str(),
                group.account_color.as_deref(),
            ));
        }
    }

    let panel = container(
        scrollable(container(col).padding(PAD_CARD).width(Length::Fill))
            .direction(iced::widget::scrollable::Direction::Vertical(
                iced::widget::scrollable::Scrollbar::new()
                    .width(6)
                    .scroller_width(6)
                    .margin(SPACE_XXS),
            ))
            .height(Length::Fill),
    )
    .padding(iced::Padding {
        top: SPACE_XS,
        right: 0.0,
        bottom: SPACE_XS,
        left: 0.0,
    })
    .width(Length::Fill)
    .height(PEOPLE_PANEL_HEIGHT)
    .clip(true)
    .style(theme::style_recessed_list_panel);

    container(panel)
        .padding(SPACE_XS)
        .width(Length::Fill)
        .into()
}

fn tag_member_pill<'a>(
    lbl: &'a AccountLabelRow,
    account_name: &'a str,
    account_color: Option<&'a str>,
) -> Element<'a, SettingsMessage> {
    button(
        container(
            row![
                container(icon::trash().size(ICON_SM).style(text::danger))
                    .align_y(Alignment::Center),
                container(text(&lbl.name).size(TEXT_LG).style(text::base))
                    .align_y(Alignment::Center)
                    .width(Length::Fill),
                account_meta(account_name, account_color),
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::LabelGroupEditorRemoveMember(
        lbl.account_id.clone(),
        lbl.label_id.clone(),
    ))
    .padding(0)
    .style(theme::style_pill_card_button)
    .width(Length::Fill)
    .into()
}

/// Right-side meta: account-colour dot followed by the account name.
/// Used in both Add Tags and Members pills.
fn account_meta<'a>(
    account_name: &'a str,
    account_color: Option<&'a str>,
) -> Element<'a, SettingsMessage> {
    let dot: Element<'a, SettingsMessage> = account_color
        .map(|hex| widgets::color_dot::<SettingsMessage>(theme::hex_to_color(hex)))
        .unwrap_or_else(|| Space::new().width(SPACE_SM).height(SPACE_SM).into());
    row![
        container(
            text(account_name)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
        container(dot).align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

fn label_group_editor_buttons<'a>(
    editor: &'a LabelGroupEditorState,
) -> Element<'a, SettingsMessage> {
    let is_new = editor.group_id.is_none();
    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if !is_new {
        if editor.show_delete_confirmation {
            btn_row = btn_row.push(
                button(text("Confirm delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::LabelGroupEditorDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
            btn_row = btn_row.push(
                button(text("Cancel").size(TEXT_LG))
                    .on_press(SettingsMessage::LabelGroupEditorCancelDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Ghost.style()),
            );
        } else {
            btn_row = btn_row.push(
                button(text("Delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::LabelGroupEditorConfirmDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
        }
    }

    btn_row = btn_row.push(Space::new().width(Length::Fill));

    btn_row = btn_row.push(
        button(text("Cancel").size(TEXT_LG))
            .on_press(SettingsMessage::LabelGroupEditorCancel)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style()),
    );

    let save_label = if is_new { "Create" } else { "Save" };
    let name_filled = !editor.name.trim().is_empty();
    let can_save = name_filled && (is_new || editor.dirty);
    let mut save_btn = button(container(text(save_label).size(TEXT_LG)).center_x(Length::Fill))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
    if can_save {
        save_btn = save_btn.on_press(SettingsMessage::LabelGroupEditorSave);
    }
    btn_row = btn_row.push(save_btn);

    btn_row.into()
}
