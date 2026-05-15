use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;

pub(super) fn contact_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.contact_editor else {
        return column![].into();
    };

    let is_new = editor.contact_id.is_none();
    let title = if is_new {
        "New Contact"
    } else {
        "Edit Contact"
    };

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text(title)
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    );

    col = col.push(contact_account_selector(editor, &state.managed_accounts));

    col = col.push(contact_editor_fields(editor));
    col = col.push(contact_editor_buttons(
        editor,
        &state.confirm_delete_contact,
    ));

    col.into()
}

fn contact_account_selector<'a>(
    editor: &'a ContactEditorState,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let selected_id = editor.account_id.as_deref();

    let mut btn_row = row![].spacing(SPACE_XS).align_y(Alignment::Center);

    let is_local = selected_id.is_none();
    let local_style = if is_local {
        theme::ButtonClass::Primary
    } else {
        theme::ButtonClass::Ghost
    };
    btn_row = btn_row.push(
        button(text("Local").size(TEXT_SM))
            .style(local_style.style())
            .on_press(SettingsMessage::ContactEditorAccountChanged(None))
            .padding(PAD_ICON_BTN),
    );

    for account in accounts {
        let is_selected = selected_id == Some(account.id.as_str());
        let style = if is_selected {
            theme::ButtonClass::Primary
        } else {
            theme::ButtonClass::Ghost
        };
        let aid = Some(account.id.clone());
        btn_row = btn_row.push(
            button(text(&account.email).size(TEXT_SM))
                .style(style.style())
                .on_press(SettingsMessage::ContactEditorAccountChanged(aid))
                .padding(PAD_ICON_BTN),
        );
    }

    container(column![
        text("Account")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
        Space::new().height(SPACE_XXXS),
        btn_row,
    ])
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn contact_editor_fields(editor: &ContactEditorState) -> Element<'_, SettingsMessage> {
    let fields = vec![
        contact_field_input(
            "contact-display-name",
            "Display name",
            "Name",
            editor.display_name.text(),
            ContactField::DisplayName,
            InputField::ContactDisplayName,
        ),
        contact_field_input(
            "contact-email",
            "Email",
            "email@example.com",
            editor.email.text(),
            ContactField::Email,
            InputField::ContactEmail,
        ),
        contact_field_input(
            "contact-email2",
            "Email 2",
            "Optional second email",
            editor.email2.text(),
            ContactField::Email2,
            InputField::ContactEmail2,
        ),
        contact_field_input(
            "contact-phone",
            "Phone",
            "Optional phone number",
            editor.phone.text(),
            ContactField::Phone,
            InputField::ContactPhone,
        ),
        contact_field_input(
            "contact-company",
            "Company",
            "Optional company",
            editor.company.text(),
            ContactField::Company,
            InputField::ContactCompany,
        ),
        contact_field_input(
            "contact-notes",
            "Notes",
            "Optional notes",
            editor.notes.text(),
            ContactField::Notes,
            InputField::ContactNotes,
        ),
    ];
    section("Details", fields)
}

fn contact_field_input(
    id: &str,
    display_text: &str,
    placeholder: &str,
    value: &str,
    contact_field: ContactField,
    input_field: InputField,
) -> RowBuilder<'static> {
    input_row(
        id,
        display_text,
        placeholder,
        value,
        move |v| SettingsMessage::ContactEditorFieldChanged(contact_field.clone(), v),
        input_field,
    )
}

fn contact_editor_buttons<'a>(
    editor: &'a ContactEditorState,
    confirm_delete: &'a Option<String>,
) -> Element<'a, SettingsMessage> {
    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if let Some(ref id) = editor.contact_id {
        let is_confirming = confirm_delete.as_deref() == Some(id.as_str());
        if is_confirming {
            btn_row = btn_row.push(
                button(text("Confirm delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::ContactConfirmDelete(id.clone()))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
            btn_row = btn_row.push(
                button(text("Cancel").size(TEXT_LG))
                    .on_press(SettingsMessage::ContactCancelDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Ghost.style()),
            );
        } else {
            btn_row = btn_row.push(
                button(text("Delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::ContactDelete(id.clone()))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
        }
    }

    btn_row = btn_row.push(Space::new().width(Length::Fill));

    let is_local = editor.source.as_deref().is_none_or(|s| s == "user");
    if is_local {
        if editor.contact_id.is_some() {
            btn_row = btn_row.push(text("Auto-saved").size(TEXT_SM).style(text::secondary));
        } else {
            let can_save = !editor.email.text().trim().is_empty();
            let mut save_btn =
                button(container(text("Create").size(TEXT_LG)).center_x(Length::Fill))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Primary.style())
                    .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
            if can_save {
                save_btn = save_btn.on_press(SettingsMessage::ContactEditorSave);
            }
            btn_row = btn_row.push(save_btn);
        }
    } else {
        let can_save = !editor.email.text().trim().is_empty() && editor.dirty;
        let mut save_btn = button(container(text("Save").size(TEXT_LG)).center_x(Length::Fill))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Primary.style())
            .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
        if can_save {
            save_btn = save_btn.on_press(SettingsMessage::ContactEditorSave);
        }
        btn_row = btn_row.push(save_btn);
    }

    btn_row.into()
}
