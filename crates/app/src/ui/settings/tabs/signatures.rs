use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use rte::{Action as RteAction, BlockKind, EditAction, InlineStyle, rich_text_editor};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::theme::RowPosition;
use crate::ui::widgets;

pub(super) fn signature_list_section(state: &Settings) -> Element<'_, SettingsMessage> {
    if state.managed_accounts.is_empty() {
        return section(
            "Signatures",
            vec![coming_soon_row("No accounts configured")],
        );
    }

    section("Signatures", vec![signatures_section_body(state)])
}

/// Section body that merges the signature rows and the trailing
/// "Add Signature" button into a single block with shared corners,
/// mirroring the Accounts tab's `accounts_section_body`.
fn signatures_section_body<'a>(state: &'a Settings) -> RowBuilder<'a> {
    Box::new(move |outer_position| {
        let n_sigs = state.signatures.len();
        let internal_n = n_sigs + 1;

        let mut col = column![].width(Length::Fill);

        for (i, sig) in state.signatures.iter().enumerate() {
            if i > 0 {
                col = col.push(
                    iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
                );
            }
            let internal_pos = position_for(i, internal_n);
            let effective = compose_positions(outer_position, internal_pos);
            col = col.push(signature_card(sig, &state.managed_accounts, effective));
        }

        if n_sigs > 0 {
            col = col
                .push(iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()));
        }

        let add_internal_pos = position_for(internal_n.saturating_sub(1), internal_n);
        let add_effective = compose_positions(outer_position, add_internal_pos);
        col = col.push(add_signature_button(add_effective));

        col.into()
    })
}

fn add_signature_button<'a>(position: RowPosition) -> Element<'a, SettingsMessage> {
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
    .on_press(SettingsMessage::SignatureCreate)
    .padding(PAD_SETTINGS_ROW)
    .style(settings_row_style(position))
    .width(Length::Fill)
    .height(SETTINGS_ROW_HEIGHT)
    .into()
}

fn signature_card<'a>(
    sig: &'a SignatureEntry,
    accounts: &'a [ManagedAccount],
    position: RowPosition,
) -> Element<'a, SettingsMessage> {
    let mut left = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if let Some(account) = accounts.iter().find(|a| a.id == sig.account_id)
        && let Some(ref hex) = account.account_color
    {
        let color = crate::ui::theme::hex_to_color(hex);
        left = left.push(widgets::color_dot::<SettingsMessage>(color));
    }

    left = left.push(
        text(&sig.name)
            .size(TEXT_LG)
            .style(text::base)
            .width(Length::Fill),
    );

    let mut content = row![left].spacing(SPACE_SM).align_y(Alignment::Center);

    if sig.is_default {
        content = content.push(
            container(text("New messages").size(TEXT_XS).style(text::secondary))
                .padding(PAD_BADGE)
                .style(theme::ContainerClass::KeyBadge.style()),
        );
    }
    if sig.is_reply_default {
        content = content.push(
            container(text("Replies").size(TEXT_XS).style(text::secondary))
                .padding(PAD_BADGE)
                .style(theme::ContainerClass::KeyBadge.style()),
        );
    }

    content = content.push(
        container(
            icon::chevron_right()
                .size(ICON_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    );

    let sig_id = sig.id.clone();
    button(
        container(content)
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(SETTINGS_TOGGLE_ROW_HEIGHT)
            .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::SignatureEdit(sig_id))
    .padding(0)
    .style(settings_row_style(position))
    .width(Length::Fill)
    .into()
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
        column![
            text(title)
                .size(TEXT_HEADING)
                .style(text::base)
                .font(iced::Font {
                    weight: iced::font::Weight::Bold,
                    ..crate::font::text()
                }),
            text("Signature changes are not saved automatically. Use the Save button at the bottom.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXS),
    );

    col = col.push(section_untitled(vec![
        signature_account_row(
            editor,
            &state.managed_accounts,
            state.open_select == Some(SelectField::SignatureAccount),
        ),
        input_row(
            "sig-name",
            "Name",
            "Signature name",
            editor.name.text(),
            SettingsMessage::SignatureEditorNameChanged,
            InputField::SignatureName,
        ),
    ]));

    col = col.push(section_with_subtitle(
        "Defaults",
        "Each account has one default for new messages and one for replies. Setting these here clears them on the account's other signatures, if any.",
        vec![
            toggle_row(
                "New messages",
                "",
                editor.is_default,
                SettingsMessage::SignatureEditorToggleDefault,
            ),
            toggle_row(
                "Replies & forwards",
                "",
                editor.is_reply_default,
                SettingsMessage::SignatureEditorToggleReplyDefault,
            ),
        ],
    ));

    col = col.push(section(
        "Signature",
        vec![static_row(
            container(column![
                signature_formatting_toolbar(editor),
                Space::new().height(SPACE_XXS),
                container(
                    rich_text_editor(&editor.body_editor)
                        .on_action(SettingsMessage::SignatureEditorAction)
                        .font(crate::font::text())
                        .height(Length::Fixed(200.0))
                        .width(Length::Fill),
                )
                .padding(PAD_CONTENT)
                .style(theme::ContainerClass::EmailBody.style())
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

    let can_save = !editor.name.text().trim().is_empty() && !editor.account_id.is_empty();
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

fn signature_account_row<'a>(
    editor: &'a SignatureEditorState,
    accounts: &'a [ManagedAccount],
    open: bool,
) -> RowBuilder<'a> {
    let enabled = editor.signature_id.is_none();
    let selected = if editor.account_id.is_empty() {
        None
    } else {
        Some(editor.account_id.as_str())
    };

    let options: Vec<widgets::SelectOption<'a>> = accounts
        .iter()
        .map(|account| {
            let label = account
                .account_name
                .as_deref()
                .or(account.display_name.as_deref())
                .unwrap_or(&account.email);
            let icon = account
                .account_color
                .as_deref()
                .map(|hex| widgets::SelectIcon::ColorDot(crate::ui::theme::hex_to_color(hex)));
            widgets::SelectOption {
                value: account.id.clone(),
                label,
                icon,
            }
        })
        .collect();

    let dropdown = widgets::select_with_icons(
        options,
        selected,
        open,
        enabled,
        "Choose account",
        SettingsMessage::ToggleSelect(SelectField::SignatureAccount),
        SettingsMessage::SignatureEditorAccountChanged,
    );

    setting_row_with_description(
        "Account",
        Some("When supported, signatures are stored with your cloud provider, and each signature is exclusive to its selected account."),
        dropdown,
        SettingsMessage::ToggleSelect(SelectField::SignatureAccount),
    )
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
