//! Cross-account label editor sheet (Mail Rules > Labels).

use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::widgets;

pub(super) fn label_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.editing_label else {
        return column![].into();
    };

    let is_new = editor.label_id.is_empty();
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
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..crate::font::text()
                }),
            text("Labels apply to messages on every account that supports them. \
                  Renaming or recoloring is cross-account.")
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
            "Label name",
            &editor.name,
            SettingsMessage::LabelEditorNameChanged,
            InputField::LabelName,
        )],
    ));

    col = col.push(section("Color", vec![color_preview_row(editor)]));

    col = col.push(label_editor_buttons(editor));

    col.into()
}

fn color_preview_row<'a>(editor: &'a LabelEditorState) -> RowBuilder<'a> {
    let color_bg = editor.color_bg.clone();
    let display_name = editor.name.clone();
    let has_override = editor.has_override;
    Box::new(move |position| {
        let bg = theme::hex_to_color(&color_bg);
        let dot = widgets::color_dot::<SettingsMessage>(bg);

        let descriptor = if has_override {
            "User color (override)"
        } else {
            "Default color"
        };

        let preview = row![
            dot,
            text(if display_name.is_empty() {
                "Preview".to_owned()
            } else {
                display_name
            })
            .size(TEXT_LG)
            .style(text::base),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center)
        .width(Length::Fill);

        let descriptor_text = text(descriptor)
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style());

        let inner = container(
            row![
                preview,
                descriptor_text,
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .height(SETTINGS_TOGGLE_ROW_HEIGHT)
        .align_y(Alignment::Center);

        button(inner)
            .on_press(SettingsMessage::Noop)
            .padding(0)
            .style(settings_row_style(position))
            .width(Length::Fill)
            .into()
    })
}

fn label_editor_buttons<'a>(editor: &'a LabelEditorState) -> Element<'a, SettingsMessage> {
    let is_new = editor.label_id.is_empty();

    let cancel = button(
        container(text("Cancel").size(TEXT_LG).style(text::base))
            .padding(PAD_SETTINGS_ROW),
    )
    .on_press(SettingsMessage::LabelEditorCancel)
    .style(theme::ButtonClass::Secondary.style());

    let save = button(
        container(
            text(if is_new { "Create" } else { "Save" })
                .size(TEXT_LG)
                .style(text::base),
        )
        .padding(PAD_SETTINGS_ROW),
    )
    .on_press(SettingsMessage::LabelEditorSave)
    .style(theme::ButtonClass::Primary.style());

    let mut row = row![cancel].spacing(SPACE_SM);

    if !is_new {
        let delete = button(
            container(text("Delete").size(TEXT_LG).style(text::base))
                .padding(PAD_SETTINGS_ROW),
        )
        .on_press(SettingsMessage::LabelEditorConfirmDelete)
        .style(theme::ButtonClass::Destructive.style());
        row = row.push(delete);
    }

    row = row.push(iced::widget::Space::new().width(Length::Fill));
    row = row.push(save);
    row.into()
}
