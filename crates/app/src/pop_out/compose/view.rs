use iced::widget::{Space, button, column, container, row, text, text_input};
use iced::{Alignment, Element, Length};
use rte::rich_text_editor;

use crate::Message;
use crate::font;
use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::token_input::{self, TokenId, TokenInputMessage, TokenInputValue};

use crate::pop_out::PopOutMessage;

use super::messages::ComposeMessage;
use super::modals::{
    attachment_list, autocomplete_dropdown, bcc_nudge_banner, bulk_paste_banner_view,
    discard_confirmation, link_dialog, save_as_group_dialog, token_context_menu,
};
use super::state::ComposeState;
use super::types::RecipientField;

pub fn view_compose_window<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let header = compose_header(window_id, state);
    let toolbar = formatting_toolbar(window_id);
    let body = compose_body(window_id, state);
    let footer = compose_footer(window_id, state);

    let mut content = column![header,].spacing(SPACE_0);

    for nudge in &state.bcc_nudges {
        content = content.push(bcc_nudge_banner(window_id, nudge));
    }

    if let Some(ref banner) = state.bulk_paste_banner {
        content = content.push(bulk_paste_banner_view(window_id, banner));
    }

    content = content
        .push(crate::ui::widgets::divider())
        .push(toolbar)
        .push(crate::ui::widgets::divider())
        .push(body);

    if !state.attachments.is_empty() {
        content = content.push(crate::ui::widgets::divider());
        content = content.push(attachment_list(window_id, state));
    }

    content = content.push(crate::ui::widgets::divider());
    content = content.push(footer);

    let base = container(content)
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style());

    let with_context_menu: Element<'a, Message> = if let Some(ref ctx) = state.context_menu {
        crate::ui::anchored_overlay::anchored_overlay(base)
            .popup(token_context_menu(window_id, ctx))
            .popup_width(180.0)
            .anchor_point(ctx.position)
            .on_dismiss(Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::DismissContextMenu),
            ))
            .into()
    } else {
        base.into()
    };

    if state.discard_confirm_open {
        let noop = Message::PopOut(window_id, PopOutMessage::Compose(ComposeMessage::Noop));
        crate::ui::modal_overlay::modal_overlay(
            with_context_menu,
            discard_confirmation(window_id),
            crate::ui::modal_overlay::ModalSurface::Modal,
            noop,
        )
    } else if state.link_dialog_open {
        let noop = Message::PopOut(window_id, PopOutMessage::Compose(ComposeMessage::Noop));
        crate::ui::modal_overlay::modal_overlay(
            with_context_menu,
            link_dialog(window_id, state),
            crate::ui::modal_overlay::ModalSurface::Modal,
            noop,
        )
    } else if state.save_group_dialog_open {
        let noop = Message::PopOut(window_id, PopOutMessage::Compose(ComposeMessage::Noop));
        crate::ui::modal_overlay::modal_overlay(
            with_context_menu,
            save_as_group_dialog(window_id, state),
            crate::ui::modal_overlay::ModalSurface::Modal,
            noop,
        )
    } else {
        with_context_menu
    }
}

fn compose_header<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let mut fields = column![].spacing(SPACE_XS);

    let from_row = build_from_row(window_id, state);
    fields = fields.push(from_row);

    fields = fields.push(build_to_row(window_id, state));

    if let Some(err) = state.recipients_error.as_deref() {
        // Inline validation error directly under the recipient rows.
        // Indented to line up with the field column (skipping the label
        // gutter) so it visually belongs to the To/Cc/Bcc cluster
        // rather than floating loose at the bottom of the form.
        fields = fields.push(
            row![
                Space::new().width(COMPOSE_LABEL_WIDTH),
                text(err).size(TEXT_SM).style(text::danger),
            ]
            .spacing(SPACE_XS),
        );
    }

    if state.show_cc {
        fields = fields.push(build_cc_row(window_id, state));
    }

    if state.show_bcc {
        fields = fields.push(build_bcc_row(window_id, state));
    }

    let subject_input = text_input("Subject", &state.subject)
        .on_input(move |s| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::SubjectChanged(s)),
            )
        })
        .size(TEXT_LG)
        .padding(PAD_INPUT);

    let subject_row = row![
        container(
            text("Subject")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .width(COMPOSE_LABEL_WIDTH)
        .align_x(Alignment::End)
        .align_y(Alignment::Center),
        subject_input,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);
    fields = fields.push(subject_row);

    if let Some(ref status) = state.status {
        fields = fields.push(
            text(status.as_str())
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        );
    }

    container(fields)
        .padding(PAD_CONTENT)
        .width(Length::Fill)
        .into()
}

fn build_from_row<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let from_picker = from_account_picker(window_id, state);

    let from_label = container(
        text("From")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
    )
    .width(COMPOSE_LABEL_WIDTH)
    .align_x(Alignment::End)
    .align_y(Alignment::Center);

    let mut from_row = row![from_label, from_picker]
        .spacing(SPACE_XS)
        .align_y(Alignment::Center)
        .width(Length::Fill);

    if !state.show_cc {
        from_row = from_row.push(
            button(text("Cc").size(TEXT_SM))
                .style(theme::ButtonClass::BareIcon.style())
                .on_press(Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::ShowCc),
                ))
                .padding(PAD_INPUT),
        );
    }
    if !state.show_bcc {
        from_row = from_row.push(
            button(text("Bcc").size(TEXT_SM))
                .style(theme::ButtonClass::BareIcon.style())
                .on_press(Message::PopOut(
                    window_id,
                    PopOutMessage::Compose(ComposeMessage::ShowBcc),
                ))
                .padding(PAD_INPUT),
        );
    }

    from_row.into()
}

/// Build the From-account dropdown trigger plus its popover menu.
///
/// Trigger layout: `[ name <email> .... ]  [ account_name ]  [ chevron ]`
/// - name+email is `Length::Fill`, ellipsizes when narrow
/// - account_name is `Length::Shrink`, rendered in a tertiary (muted) color
/// - chevron is fixed
fn from_account_picker<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let main_text = match state.from_account.as_ref() {
        Some(a) => match a.display_name.as_deref() {
            Some(n) if !n.is_empty() => format!("{n} <{}>", a.email),
            _ => a.email.clone(),
        },
        None => "Select account".to_string(),
    };
    let account_name = state
        .from_account
        .as_ref()
        .and_then(|a| a.account_name.clone())
        .unwrap_or_default();

    let trigger_row = row![
        container(
            text(main_text)
                .size(TEXT_LG)
                .style(text::base)
                .wrapping(iced::widget::text::Wrapping::None)
                .ellipsis(iced::widget::text::Ellipsis::End)
        )
        .width(Length::Fill)
        .align_y(Alignment::Center),
        container(
            text(account_name)
                .size(TEXT_LG)
                .style(theme::TextClass::Tertiary.style())
        )
        .align_y(Alignment::Center),
        container(
            icon::chevron_down()
                .size(ICON_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let trigger = button(trigger_row)
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::ToggleFromDropdown),
        ))
        .padding(PAD_INPUT)
        .width(Length::Fill)
        .style(theme::ButtonClass::BareIcon.style());

    if !state.from_dropdown_open {
        return trigger.into();
    }

    let selected_id = state.from_account.as_ref().map(|a| a.id.clone());
    let mut items = column![].spacing(SPACE_XXS).width(Length::Fill);
    for account in &state.from_accounts {
        let item_main = match account.display_name.as_deref() {
            Some(n) if !n.is_empty() => format!("{n} <{}>", account.email),
            _ => account.email.clone(),
        };
        let item_meta = account.account_name.clone().unwrap_or_default();
        let is_selected = selected_id.as_deref() == Some(account.id.as_str());
        let acc = account.clone();
        let item = button(
            row![
                container(
                    text(item_main)
                        .size(TEXT_LG)
                        .style(text::base)
                        .wrapping(iced::widget::text::Wrapping::None)
                        .ellipsis(iced::widget::text::Ellipsis::End)
                )
                .width(Length::Fill)
                .align_y(Alignment::Center),
                container(
                    text(item_meta)
                        .size(TEXT_LG)
                        .style(theme::TextClass::Tertiary.style())
                )
                .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XS)
            .align_y(Alignment::Center),
        )
        .on_press(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::FromAccountChanged(acc)),
        ))
        .padding(PAD_INPUT)
        .width(Length::Fill)
        .style(theme::ButtonClass::Dropdown { selected: is_selected }.style());
        items = items.push(item);
    }

    let menu = container(items)
        .padding(PAD_DROPDOWN)
        .style(theme::ContainerClass::SelectMenu.style())
        .width(Length::Fill);

    crate::ui::anchored_overlay::anchored_overlay(trigger)
        .popup(menu)
        .on_dismiss(Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::ToggleFromDropdown),
        ))
        .into()
}

fn build_to_row<'a>(window_id: iced::window::Id, state: &'a ComposeState) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::To
        && !state.autocomplete.results.is_empty();
    let autocomplete_dropdown = ac_open.then(|| autocomplete_dropdown(window_id, state));
    build_recipient_row_inner(
        "To",
        &state.to,
        state.selected_to_token,
        ac_open,
        autocomplete_dropdown,
        window_id,
        "Add recipients...",
        ComposeMessage::ToTokenInput,
    )
}

fn build_cc_row<'a>(window_id: iced::window::Id, state: &'a ComposeState) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::Cc
        && !state.autocomplete.results.is_empty();
    let autocomplete_dropdown = ac_open.then(|| autocomplete_dropdown(window_id, state));
    build_recipient_row_inner(
        "Cc",
        &state.cc,
        state.selected_cc_token,
        ac_open,
        autocomplete_dropdown,
        window_id,
        "Add Cc...",
        ComposeMessage::CcTokenInput,
    )
}

fn build_bcc_row<'a>(window_id: iced::window::Id, state: &'a ComposeState) -> Element<'a, Message> {
    let ac_open = state.autocomplete.active_field == RecipientField::Bcc
        && !state.autocomplete.results.is_empty();
    let autocomplete_dropdown = ac_open.then(|| autocomplete_dropdown(window_id, state));
    build_recipient_row_inner(
        "Bcc",
        &state.bcc,
        state.selected_bcc_token,
        ac_open,
        autocomplete_dropdown,
        window_id,
        "Add Bcc...",
        ComposeMessage::BccTokenInput,
    )
}

// TODO(refactor): bundle row params (autocomplete + selection state) into a struct.
#[allow(clippy::too_many_arguments)]
fn build_recipient_row_inner<'a>(
    label: &'a str,
    value: &'a TokenInputValue,
    selected: Option<TokenId>,
    autocomplete_open: bool,
    autocomplete_dropdown: Option<Element<'a, Message>>,
    window_id: iced::window::Id,
    placeholder: &'a str,
    wrap: fn(TokenInputMessage) -> ComposeMessage,
) -> Element<'a, Message> {
    let field = token_input::token_input_field(
        &value.tokens,
        &value.text,
        placeholder,
        selected,
        autocomplete_open,
        move |msg| Message::PopOut(window_id, PopOutMessage::Compose(wrap(msg))),
    );

    let mut overlay = crate::ui::anchored_overlay::anchored_overlay(field).on_dismiss(
        Message::PopOut(
            window_id,
            PopOutMessage::Compose(ComposeMessage::AutocompleteDismiss),
        ),
    );
    if let Some(dropdown) = autocomplete_dropdown {
        overlay = overlay.popup(dropdown);
    }
    let field: Element<'a, Message> = overlay.into();

    row![
        container(
            text(label)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style())
        )
        .width(COMPOSE_LABEL_WIDTH)
        .align_x(Alignment::End)
        .align_y(Alignment::Center),
        field,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

fn formatting_toolbar<'a>(window_id: iced::window::Id) -> Element<'a, Message> {
    let fmt_btn = |ico: iced::widget::Text<'a>, msg: ComposeMessage| {
        button(ico.size(ICON_SM).style(text::secondary))
            .on_press(Message::PopOut(window_id, PopOutMessage::Compose(msg)))
            .padding(PAD_ICON_BTN)
            .style(theme::ButtonClass::BareIcon.style())
    };

    let toolbar = row![
        fmt_btn(icon::bold(), ComposeMessage::FormatBold),
        fmt_btn(icon::italic(), ComposeMessage::FormatItalic),
        fmt_btn(icon::underline(), ComposeMessage::FormatUnderline),
        fmt_btn(icon::list(), ComposeMessage::FormatList),
        fmt_btn(icon::link(), ComposeMessage::FormatLink),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    container(toolbar)
        .padding(PAD_TOOLBAR)
        .width(Length::Fill)
        .into()
}

fn compose_body<'a>(window_id: iced::window::Id, state: &'a ComposeState) -> Element<'a, Message> {
    let editor = rich_text_editor(&state.body)
        .on_action(move |action| {
            Message::PopOut(
                window_id,
                PopOutMessage::Compose(ComposeMessage::BodyChanged(action)),
            )
        })
        .height(Length::Fill)
        .font(font::text());

    let body_card = container(editor)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(PAD_CONTENT)
        .style(theme::ContainerClass::EmailBody.style());

    container(body_card)
        .width(Length::Fill)
        .height(Length::Fill)
        .padding(iced::Padding {
            top: 0.0,
            right: SPACE_SM,
            bottom: 0.0,
            left: SPACE_SM,
        })
        .into()
}

fn compose_footer<'a>(
    window_id: iced::window::Id,
    state: &'a ComposeState,
) -> Element<'a, Message> {
    let discard_msg = if state.has_user_content() {
        ComposeMessage::ToggleDiscardConfirm
    } else {
        ComposeMessage::Discard
    };

    let discard_btn = button(
        row![icon::trash().size(ICON_LG), text("Discard").size(TEXT_LG),]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Action.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(discard_msg),
    ))
    .padding(PAD_BUTTON);

    let send_btn = button(
        row![
            icon::send().size(ICON_LG).color(theme::ON_AVATAR),
            text("Send").size(TEXT_LG).color(theme::ON_AVATAR),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Primary.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::Send),
    ))
    .padding(PAD_BUTTON);

    let attach_btn = button(
        row![
            icon::paperclip().size(ICON_LG),
            text("Attach").size(TEXT_LG),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .style(theme::ButtonClass::Action.style())
    .on_press(Message::PopOut(
        window_id,
        PopOutMessage::Compose(ComposeMessage::AttachFiles),
    ))
    .padding(PAD_BUTTON);

    let footer_row = row![
        discard_btn,
        attach_btn,
        Space::new().width(Length::Fill),
        send_btn,
    ]
    .align_y(Alignment::Center);

    container(footer_row)
        .padding(iced::Padding {
            top: PAD_CONTENT.top,
            right: PAD_CONTENT.right,
            bottom: PAD_CONTENT.bottom,
            left: PAD_CONTENT.left - PAD_BUTTON.left,
        })
        .width(Length::Fill)
        .into()
}
