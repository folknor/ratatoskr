use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Element, Length};

use rte::{Action as RteAction, BlockKind, EditAction, InlineStyle, rich_text_editor};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::undoable_text_input::undoable_text_input;
use crate::ui::widgets;

pub(super) fn signature_list_section(state: &Settings) -> Element<'_, SettingsMessage> {
    if state.signatures.is_empty() && state.managed_accounts.is_empty() {
        return section(
            "Signatures",
            vec![coming_soon_row("No accounts configured")],
        );
    }

    let mut items: Vec<RowBuilder<'_>> = Vec::new();

    for account in &state.managed_accounts {
        let account_sigs: Vec<&SignatureEntry> = state
            .signatures
            .iter()
            .filter(|s| s.account_id == account.id)
            .collect();

        let account_name = account
            .account_name
            .as_deref()
            .or(account.display_name.as_deref())
            .unwrap_or(&account.email);

        let mut header_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);
        if let Some(ref hex) = account.account_color {
            let color = crate::ui::theme::hex_to_color(hex);
            header_row = header_row.push(widgets::color_dot::<SettingsMessage>(color));
        }
        header_row = header_row.push(
            text(account_name)
                .size(TEXT_SM)
                .style(text::secondary)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..crate::font::text()
                }),
        );

        items.push(static_row(
            container(header_row)
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
        ));

        for sig in &account_sigs {
            let global_idx = state
                .signatures
                .iter()
                .position(|s| s.id == sig.id)
                .unwrap_or(0);
            items.push(signature_row(sig, global_idx));
        }

        let aid = account.id.clone();
        items.push(Box::new(move |position| {
            button(
                container(
                    row![
                        icon::plus().size(ICON_MD).style(text::base),
                        text("Add Signature")
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
            .on_press(SettingsMessage::SignatureCreate(aid))
            .padding(PAD_SETTINGS_ROW)
            .style(move |t, s| theme::style_settings_row_button(t, s, position))
            .width(Length::Fill)
            .height(SETTINGS_ROW_HEIGHT)
            .into()
        }));
    }

    let sig_section = section("Signatures", items);

    mouse_area(sig_section)
        .on_move(SettingsMessage::SignatureDragMove)
        .on_release(SettingsMessage::SignatureDragEnd)
        .into()
}

fn signature_row<'a>(sig: &'a SignatureEntry, global_index: usize) -> RowBuilder<'a> {
    Box::new(move |position| {
        let sig_id = sig.id.clone();

        let mut label_parts =
            column![text(&sig.name).size(TEXT_LG).style(text::base),].spacing(SPACE_XXXS);

        let preview = sig.body_text.as_deref().unwrap_or(&sig.body_html);
        let snippet: String = preview.chars().take(60).collect();
        if !snippet.is_empty() {
            label_parts = label_parts.push(
                text(snippet)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            );
        }

        let mut content = row![].spacing(SPACE_SM).align_y(Alignment::Center);

        content = content.push(
            mouse_area(
                container(icon::grip_vertical().size(ICON_MD).style(text::secondary))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::SignatureDragGripPress(global_index))
            .interaction(iced::mouse::Interaction::Grab),
        );

        content = content.push(
            container(label_parts)
                .align_y(Alignment::Center)
                .width(Length::Fill),
        );

        if sig.is_default {
            content = content.push(
                container(text("Default").size(TEXT_XS).style(text::secondary))
                    .padding(PAD_BADGE)
                    .style(theme::ContainerClass::KeyBadge.style()),
            );
        }
        if sig.is_reply_default {
            content = content.push(
                container(text("Reply default").size(TEXT_XS).style(text::secondary))
                    .padding(PAD_BADGE)
                    .style(theme::ContainerClass::KeyBadge.style()),
            );
        }

        let del_id = sig.id.clone();
        content = content.push(
            button(
                container(icon::x().size(ICON_MD).style(text::secondary))
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::SignatureDelete(del_id))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::BareIcon.style()),
        );

        button(
            container(content)
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill)
                .height(SETTINGS_TOGGLE_ROW_HEIGHT)
                .align_y(Alignment::Center),
        )
        .on_press(SettingsMessage::SignatureEdit(sig_id))
        .padding(0)
        .style(move |t, s| theme::style_settings_row_button(t, s, position))
        .width(Length::Fill)
        .into()
    })
}

pub(super) fn signature_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.signature_editor else {
        return column![].into();
    };

    let is_new = editor.signature_id.is_none();
    let title = if is_new {
        "New Signature"
    } else {
        "Edit Signature"
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

    col = col.push(section(
        "Name",
        vec![static_row(
            container(
                undoable_text_input("Signature name", editor.name.text())
                    .id("sig-name")
                    .on_input(SettingsMessage::SignatureEditorNameChanged)
                    .on_undo(SettingsMessage::UndoInput(InputField::SignatureName))
                    .on_redo(SettingsMessage::RedoInput(InputField::SignatureName))
                    .size(TEXT_LG)
                    .padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style())
                    .width(Length::Fill),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        )],
    ));

    col = col.push(section(
        "Defaults",
        vec![
            toggle_row(
                "Default for new messages",
                "Use this signature when composing new emails",
                editor.is_default,
                SettingsMessage::SignatureEditorToggleDefault,
            ),
            toggle_row(
                "Default for replies & forwards",
                "Use this signature when replying or forwarding",
                editor.is_reply_default,
                SettingsMessage::SignatureEditorToggleReplyDefault,
            ),
        ],
    ));

    col = col.push(section(
        "Content",
        vec![static_row(
            container(column![
                text("Signature body")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
                Space::new().height(SPACE_XXS),
                signature_formatting_toolbar(editor),
                Space::new().height(SPACE_XXS),
                container(
                    rich_text_editor(&editor.body_editor)
                        .on_action(SettingsMessage::SignatureEditorAction)
                        .font(crate::font::text())
                        .height(Length::Fixed(200.0))
                        .width(Length::Fill)
                        .padding(PAD_INPUT),
                )
                .style(theme::ContainerClass::Surface.style())
                .width(Length::Fill),
            ])
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        )],
    ));

    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if !is_new {
        let del_id = editor.signature_id.clone().unwrap_or_default();
        let is_confirming = state.confirm_delete_signature.as_deref() == Some(del_id.as_str());

        if is_confirming {
            btn_row = btn_row.push(
                text("Delete this signature?")
                    .size(TEXT_LG)
                    .style(text::danger),
            );
            btn_row = btn_row.push(
                button(text("Cancel").size(TEXT_LG).style(text::base))
                    .on_press(SettingsMessage::SignatureDeleteCancelled)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
            btn_row = btn_row.push(
                button(text("Confirm").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::SignatureDeleteConfirmed(del_id))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
        } else {
            btn_row = btn_row.push(
                button(text("Delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::SignatureDelete(del_id))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
        }
    }

    btn_row = btn_row.push(Space::new().width(Length::Fill));

    let can_save = !editor.name.text().trim().is_empty();
    let mut save_btn = button(container(text("Save").size(TEXT_LG)).center_x(Length::Fill))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
    if can_save {
        save_btn = save_btn.on_press(SettingsMessage::SignatureEditorSave);
    }
    btn_row = btn_row.push(save_btn);

    col = col.push(btn_row);

    col.into()
}

fn signature_formatting_toolbar<'a>(
    _editor: &'a SignatureEditorState,
) -> Element<'a, SettingsMessage> {
    let inline_btn = |icon_widget: iced::widget::Text<'a>, style_bit: InlineStyle| {
        button(
            container(icon_widget.size(ICON_MD).style(text::base))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .width(SETTINGS_ROW_HEIGHT)
                .height(SETTINGS_ROW_HEIGHT),
        )
        .on_press(SettingsMessage::SignatureEditorAction(RteAction::Edit(
            EditAction::ToggleInlineStyle(style_bit),
        )))
        .padding(0)
        .style(theme::ButtonClass::Action.style())
    };

    let block_btn = |icon_widget: iced::widget::Text<'a>, block_kind: BlockKind| {
        button(
            container(icon_widget.size(ICON_MD).style(text::base))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center)
                .width(SETTINGS_ROW_HEIGHT)
                .height(SETTINGS_ROW_HEIGHT),
        )
        .on_press(SettingsMessage::SignatureEditorAction(RteAction::Edit(
            EditAction::SetBlockType(block_kind),
        )))
        .padding(0)
        .style(theme::ButtonClass::Action.style())
    };

    let toolbar = row![
        inline_btn(icon::bold(), InlineStyle::BOLD),
        inline_btn(icon::italic(), InlineStyle::ITALIC),
        inline_btn(icon::underline(), InlineStyle::UNDERLINE),
        inline_btn(icon::strikethrough(), InlineStyle::STRIKETHROUGH),
        Space::new().width(SPACE_XS),
        block_btn(icon::list(), BlockKind::ListItem { ordered: false }),
        block_btn(icon::list_ordered(), BlockKind::ListItem { ordered: true }),
    ]
    .spacing(SPACE_XXXS)
    .align_y(Alignment::Center);

    toolbar.into()
}
