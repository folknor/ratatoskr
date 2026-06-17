use iced::widget::{button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;

pub(super) fn group_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.group_editor else {
        return column![].into();
    };

    let is_new = editor.group_id.is_none();
    let title = if is_new { "New Group" } else { "Edit Group" };

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        column![
            text(title)
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..crate::font::text()
                }),
            text("Group changes are not saved automatically. Use the Save button at the bottom.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXS),
    );

    col = col.push(section(
        "Name",
        vec![input_row(
            "group-name",
            "Name",
            "Group name",
            editor.name.text(),
            SettingsMessage::GroupEditorNameChanged,
            InputField::GroupName,
        )],
    ));

    col = col.push(group_add_members_section(editor, state));

    col = col.push(group_members_list_section(editor, state));

    col = col.push(group_editor_buttons(editor, &state.confirm_delete_group));

    col.into()
}

fn group_add_members_section<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let items: Vec<RowBuilder<'a>> = vec![
        static_row(
            container(super::people::filter_row(
                "group-add-filter",
                "Filter contacts...",
                &editor.filter,
                SettingsMessage::GroupEditorFilterChanged,
                FilterId::GroupAddMembers,
                state.focused_filter == Some(FilterId::GroupAddMembers),
            ))
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        ),
        static_row(group_add_candidates_panel(editor, state)),
    ];

    section_with_subtitle(
        "Add Members",
        "You can paste a large list of email addresses here.",
        items,
    )
}

fn group_add_candidate_pill<'a>(
    contact: &'a crate::db::ContactEntry,
) -> Element<'a, SettingsMessage> {
    let email_for_press = contact.email.clone();
    let label: &str = contact.display_name.as_deref().unwrap_or(&contact.email);
    button(
        container(
            row![
                container(icon::plus().size(ICON_SM).style(text::primary))
                    .align_y(Alignment::Center),
                container(text(label).size(TEXT_LG).style(text::base))
                    .align_y(Alignment::Center)
                    .width(Length::Fill),
                container(
                    text(&contact.email)
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .align_y(Alignment::Center),
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::GroupEditorAddMember(email_for_press))
    .padding(0)
    .style(theme::style_pill_card_button)
    .width(Length::Fill)
    .into()
}

fn group_add_candidates_panel<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let filter_lower = editor.filter.to_lowercase();
    let candidates: Vec<&crate::db::ContactEntry> = state
        .contacts
        .iter()
        .filter(|c| !editor.members.contains(&c.email))
        .filter(|c| {
            filter_lower.is_empty()
                || c.email.to_lowercase().contains(&filter_lower)
                || c.display_name
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&filter_lower)
        })
        .collect();

    let panel: Element<'_, SettingsMessage> = if candidates.is_empty() {
        let msg = if filter_lower.is_empty() {
            "All contacts are already in this group."
        } else {
            "No matching contacts."
        };
        container(
            text(msg)
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
        for contact in candidates {
            col = col.push(group_add_candidate_pill(contact));
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

fn group_members_list_section<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let title = format!("Members ({})", editor.members.len());

    let filter = container(super::people::filter_row(
        "group-members-filter",
        "Filter members...",
        &editor.members_filter,
        SettingsMessage::GroupEditorMembersFilterChanged,
        FilterId::GroupMembers,
        state.focused_filter == Some(FilterId::GroupMembers),
    ))
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill);

    let panel: Element<'_, SettingsMessage> = if editor.members.is_empty() {
        let empty_panel = container(
            text("No members yet. Use the list above or paste emails to add them.")
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
        group_members_list_panel(editor, state)
    };

    section_dynamic_with_subtitle(
        title,
        "Click a member to remove them from the group.".to_string(),
        vec![static_row(filter), static_row(panel)],
    )
}

fn group_members_list_panel<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let filter_lower = editor.members_filter.to_lowercase();
    let visible: Vec<&'a String> = editor
        .members
        .iter()
        .filter(|email| {
            if filter_lower.is_empty() {
                return true;
            }
            if email.to_lowercase().contains(&filter_lower) {
                return true;
            }
            state
                .contacts
                .iter()
                .find(|c| &c.email == *email)
                .and_then(|c| c.display_name.as_deref())
                .map(|n| n.to_lowercase().contains(&filter_lower))
                .unwrap_or(false)
        })
        .collect();

    if visible.is_empty() {
        let empty = container(
            text("No members match the filter.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .height(PEOPLE_PANEL_HEIGHT)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(theme::style_recessed_list_panel);
        return container(empty)
            .padding(SPACE_XS)
            .width(Length::Fill)
            .into();
    }

    let mut col = column![].spacing(PEOPLE_PILL_SPACING).width(Length::Fill);
    for email in visible {
        col = col.push(member_pill(email, state));
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

fn member_pill<'a>(email: &'a str, state: &'a Settings) -> Element<'a, SettingsMessage> {
    let email_for_press = email.to_string();

    let display_name: Option<&str> = state
        .contacts
        .iter()
        .find(|c| c.email == email)
        .and_then(|c| c.display_name.as_deref());

    let label_col: Element<'a, SettingsMessage> = if let Some(name) = display_name {
        row![
            container(text(name).size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center)
                .width(Length::Fill),
            container(
                text(email)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .align_y(Alignment::Center),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center)
        .into()
    } else {
        container(text(email).size(TEXT_LG).style(text::base))
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into()
    };

    button(
        container(
            row![
                container(icon::trash().size(ICON_SM).style(text::danger))
                    .align_y(Alignment::Center),
                label_col,
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::GroupEditorRemoveMember(email_for_press))
    .padding(0)
    .style(theme::style_pill_card_button)
    .width(Length::Fill)
    .into()
}

fn group_editor_buttons<'a>(
    editor: &'a GroupEditorState,
    confirm_delete: &'a Option<String>,
) -> Element<'a, SettingsMessage> {
    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if let Some(ref id) = editor.group_id {
        let is_confirming = confirm_delete.as_deref() == Some(id.as_str());
        if is_confirming {
            btn_row = btn_row.push(
                button(text("Confirm delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::GroupConfirmDelete(id.clone()))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
            btn_row = btn_row.push(
                button(text("Cancel").size(TEXT_LG))
                    .on_press(SettingsMessage::GroupCancelDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Ghost.style()),
            );
        } else {
            btn_row = btn_row.push(
                button(text("Delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::GroupDelete(id.clone()))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
        }
    }

    btn_row = btn_row.push(iced::widget::Space::new().width(Length::Fill));

    let can_save = !editor.name.text().trim().is_empty();
    let mut save_btn = button(container(text("Save").size(TEXT_LG)).center_x(Length::Fill))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
    if can_save {
        save_btn = save_btn.on_press(SettingsMessage::GroupEditorSave);
    }
    btn_row = btn_row.push(save_btn);

    btn_row.into()
}
