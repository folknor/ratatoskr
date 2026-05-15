use iced::widget::{button, column, container, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};

use crate::Message;
use crate::font;
use crate::icon;
use crate::ui::dialog::{DialogAction, alert_dialog, form_dialog};
use crate::ui::layout::*;
use crate::ui::theme;

use crate::pop_out::PopOutMessage;

use super::messages::ComposeMessage;
use super::state::ComposeState;
use super::types::{
    BccNudgeBanner, BulkPasteBanner, ContextMenuKind, RecipientField, TokenContextMenuState,
};

pub(super) fn bcc_nudge_banner<'a>(
    window_id: iced::window::Id,
    nudge: &BccNudgeBanner,
) -> Element<'a, Message> {
    let tid = nudge.token_id;
    let label = format!(
        "\u{2139} \"{}\" is a group. Move to Bcc to hide member addresses?",
        nudge.group_name,
    );

    let move_btn = button(text("Move").size(TEXT_SM).font(font::text_semibold()))
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::BccNudgeAccept(tid)),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Primary.style());

    let dismiss_btn = button(text("Dismiss").size(TEXT_SM))
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::BccNudgeDismiss(tid)),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    container(
        row![
            text(label).size(TEXT_SM).width(Length::Fill),
            move_btn,
            dismiss_btn,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CONTENT)
    .width(Length::Fill)
    .style(theme::ContainerClass::Elevated.style())
    .into()
}

pub(super) fn bulk_paste_banner_view<'a>(
    window_id: iced::window::Id,
    banner: &BulkPasteBanner,
) -> Element<'a, Message> {
    let label = format!(
        "\u{2139} {} addresses pasted. Save as a contact group?",
        banner.count,
    );

    let save_btn = button(text("Save as group").size(TEXT_SM))
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::BulkPasteSaveAsGroup),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Primary.style());

    let dismiss_btn = button(text("Dismiss").size(TEXT_SM))
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::BulkPasteDismiss),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    container(
        row![
            text(label).size(TEXT_SM).width(Length::Fill),
            save_btn,
            dismiss_btn,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center),
    )
    .padding(PAD_CONTENT)
    .width(Length::Fill)
    .style(theme::ContainerClass::Elevated.style())
    .into()
}

pub(super) fn save_as_group_dialog<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let count = state
        .bulk_paste_banner
        .as_ref()
        .map_or(0, |b| b.count);

    let name_input = text_input("Group name", &state.save_group_name)
        .on_input(move |s| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::GroupSaveNameChanged(s)),
            )
        })
        .on_submit(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::GroupSaveConfirm),
        ))
        .size(TEXT_MD)
        .padding(PAD_INPUT);

    let mut body = column![
        text(format!(
            "{count} addresses will be saved as a new contact group."
        ))
        .size(TEXT_MD)
        .style(text::secondary),
        column![
            text("Group name").size(TEXT_SM).style(text::secondary),
            name_input,
        ]
        .spacing(SPACE_XXS),
    ]
    .spacing(SPACE_SM);

    if let Some(ref err) = state.save_group_error {
        body = body.push(text(err.as_str()).size(TEXT_SM).style(text::danger));
    }

    let save_disabled =
        state.save_group_name.trim().is_empty() || state.save_group_in_flight;
    let save_label = if state.save_group_in_flight {
        "Saving..."
    } else {
        "Save"
    };

    form_dialog(
        "Save as contact group",
        body,
        vec![
            DialogAction::default_action(
                "Cancel",
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::GroupSaveCancel),
                ),
            ),
            DialogAction::suggested(
                save_label,
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::GroupSaveConfirm),
                ),
            )
            .disabled_when(save_disabled),
        ],
        None,
    )
}

pub(super) fn token_context_menu<'a>(
    window_id: iced::window::Id,
    ctx: &TokenContextMenuState,
) -> Element<'a, Message> {
    let mk = |label: &'a str, msg: ComposeMessage| {
        button(
            container(text(label).size(TEXT_MD).style(text::base))
                .width(Length::Fill)
                .align_y(Alignment::Center),
        )
        .on_press(Message::PopOut(window_id, PopOutMessage::Compose(msg)))
        .padding(PAD_NAV_ITEM)
        .height(DROPDOWN_ITEM_HEIGHT)
        .style(theme::ButtonClass::Dropdown { selected: false }.style())
        .width(Length::Fill)
    };

    let field = ctx.field;
    let mut items = column![].spacing(SPACE_XXS);

    match ctx.kind {
        ContextMenuKind::Token { token_id, is_group } => {
            items = items.push(mk(
                "Cut",
                ComposeMessage::ContextMenuCut { field, token_id },
            ));
            items = items.push(mk(
                "Copy",
                ComposeMessage::ContextMenuCopy { field, token_id },
            ));
            items = items.push(mk("Paste", ComposeMessage::ContextMenuPaste { field }));
            items = items.push(mk(
                "Delete",
                ComposeMessage::ContextMenuDelete { field, token_id },
            ));
            if is_group {
                items = items.push(mk(
                    "Expand group",
                    ComposeMessage::ContextMenuExpandGroup { field, token_id },
                ));
            }
            if field != RecipientField::To {
                items = items.push(mk(
                    "Move to To",
                    ComposeMessage::ContextMenuMoveTo {
                        token_id,
                        from: field,
                        to_field: RecipientField::To,
                    },
                ));
            }
            if field != RecipientField::Cc {
                items = items.push(mk(
                    "Move to Cc",
                    ComposeMessage::ContextMenuMoveTo {
                        token_id,
                        from: field,
                        to_field: RecipientField::Cc,
                    },
                ));
            }
            if field != RecipientField::Bcc {
                items = items.push(mk(
                    "Move to Bcc",
                    ComposeMessage::ContextMenuMoveTo {
                        token_id,
                        from: field,
                        to_field: RecipientField::Bcc,
                    },
                ));
            }
        }
        ContextMenuKind::Field => {
            items = items.push(mk("Paste", ComposeMessage::ContextMenuPaste { field }));
        }
    }

    container(items.width(Length::Fill))
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .width(180.0)
        .into()
}

pub(super) fn discard_confirmation<'a>(window_id: iced::window::Id) -> Element<'a, Message> {
    let cancel_msg = Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::ToggleDiscardConfirm),
    );
    let discard_msg = Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::Discard),
    );
    alert_dialog(
        "Discard this draft?",
        "Your unsaved changes will be lost.",
        vec![
            DialogAction::default_action("Keep editing", cancel_msg),
            DialogAction::destructive("Discard", discard_msg),
        ],
        None,
    )
}

pub(super) fn attachment_list<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let mut items = column![].spacing(SPACE_XXS);

    for (idx, att) in state.attachments.iter().enumerate() {
        let size_label = att.display_size();
        let remove_btn = button(icon::x().size(ICON_XS).style(text::secondary))
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::RemoveAttachment(idx)),
            ))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::BareIcon.style());

        let att_row = row![
            icon::paperclip().size(ICON_SM).style(text::secondary),
            text(&att.name).size(TEXT_SM),
            text(size_label)
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style()),
            remove_btn,
        ]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center);

        items = items.push(att_row);
    }

    container(items)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

pub(super) fn autocomplete_dropdown<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let mut items = column![].spacing(SPACE_0);

    for (idx, entry) in state.autocomplete.results.iter().enumerate() {
        let is_highlighted = state.autocomplete.highlighted == Some(idx);

        let row_style = if is_highlighted {
            theme::ButtonClass::Primary.style()
        } else {
            theme::ButtonClass::Ghost.style()
        };

        let content: Element<'_, Message> = if entry.is_group {
            let member_suffix = entry
                .member_count
                .map(|n| format!(" ({n})"))
                .unwrap_or_default();
            let name = entry
                .display_name
                .as_deref()
                .unwrap_or(&entry.email);
            row![
                icon::users().size(ICON_SM).style(text::secondary),
                text(format!("{name}{member_suffix}"))
                    .size(TEXT_SM)
                    .style(text::base),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center)
            .into()
        } else if let Some(ref name) = entry.display_name {
            column![
                text(name).size(TEXT_SM).style(text::base),
                text(&entry.email)
                    .size(TEXT_XS)
                    .style(theme::TextClass::Tertiary.style()),
            ]
            .spacing(SPACE_XXXS)
            .into()
        } else {
            text(&entry.email).size(TEXT_SM).style(text::base).into()
        };

        let row_btn = button(content)
            .on_press(Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::AutocompleteSelect(idx)),
            ))
            .width(Length::Fill)
            .padding(PAD_INPUT)
            .style(row_style);

        items = items.push(container(row_btn).width(Length::Fill));
    }

    container(scrollable(items).height(Length::Shrink))
        .max_height(AUTOCOMPLETE_MAX_HEIGHT)
        .width(Length::Fill)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

pub(super) fn link_dialog<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let url_input = text_input("https://...", &state.link_url)
        .on_input(move |s| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::LinkUrlChanged(s)),
            )
        })
        .size(TEXT_MD)
        .padding(PAD_INPUT);

    let text_input_field = text_input("Display text (optional)", &state.link_text)
        .on_input(move |s| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::LinkTextChanged(s)),
            )
        })
        .size(TEXT_MD)
        .padding(PAD_INPUT);

    let body = column![
        column![text("URL").size(TEXT_SM).style(text::secondary), url_input,]
            .spacing(SPACE_XXS),
        column![
            text("Display text").size(TEXT_SM).style(text::secondary),
            text_input_field,
        ]
        .spacing(SPACE_XXS),
    ]
    .spacing(SPACE_SM);

    let insert_disabled = state.link_url.trim().is_empty();

    form_dialog(
        "Insert link",
        body,
        vec![
            DialogAction::default_action(
                "Cancel",
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::ToggleLinkDialog),
                ),
            ),
            DialogAction::suggested(
                "Insert",
                Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::LinkInsert),
                ),
            )
            .disabled_when(insert_disabled),
        ],
        None,
    )
}
