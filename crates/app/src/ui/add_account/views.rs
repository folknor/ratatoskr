use iced::widget::{button, column, container, row, text, text_input};
use iced::{Element, Length};

use crate::ui::layout::*;
use crate::ui::theme;

use super::state::{AddAccountMessage, ManualAuthMethod, SecurityOption};

pub(super) fn primary_button<'a>(
    label: &'a str,
    on_press: AddAccountMessage,
) -> Element<'a, AddAccountMessage> {
    button(container(text(label).size(TEXT_LG).color(theme::ON_AVATAR)).center_x(Length::Fill))
        .on_press(on_press)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Primary.style())
        .width(Length::Fill)
        .into()
}

pub(super) fn ghost_button<'a>(
    label: &'a str,
    on_press: AddAccountMessage,
) -> Element<'a, AddAccountMessage> {
    button(container(text(label).size(TEXT_LG).style(text::secondary)).center_x(Length::Fill))
        .on_press(on_press)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Ghost.style())
        .width(Length::Fill)
        .into()
}

pub(super) fn labeled_input<'a>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> AddAccountMessage + 'a,
) -> Element<'a, AddAccountMessage> {
    column![
        text(label).size(TEXT_SM).style(text::secondary),
        text_input(placeholder, value)
            .on_input(on_input)
            .size(TEXT_LG)
            .padding(PAD_INPUT)
            .style(theme::TextInputClass::Settings.style()),
    ]
    .spacing(SPACE_XXXS)
    .width(Length::Fill)
    .into()
}

pub(super) fn server_port_row<'a>(
    server_placeholder: &'a str,
    server_value: &'a str,
    port_placeholder: &'a str,
    port_value: &'a str,
    on_server: impl Fn(String) -> AddAccountMessage + 'a,
    on_port: impl Fn(String) -> AddAccountMessage + 'a,
) -> Element<'a, AddAccountMessage> {
    row![
        column![
            text("Server").size(TEXT_SM).style(text::secondary),
            text_input(server_placeholder, server_value)
                .on_input(on_server)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::FillPortion(3)),
        column![
            text("Port").size(TEXT_SM).style(text::secondary),
            text_input(port_placeholder, port_value)
                .on_input(on_port)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::FillPortion(1)),
    ]
    .spacing(SPACE_SM)
    .into()
}

pub(super) fn security_selector<'a>(
    current: SecurityOption,
    on_change: impl Fn(SecurityOption) -> AddAccountMessage + 'a + Copy,
) -> Element<'a, AddAccountMessage> {
    row![
        iced::widget::radio(
            SecurityOption::Tls.label(),
            SecurityOption::Tls,
            Some(current),
            on_change,
        )
        .size(RADIO_SIZE)
        .text_size(TEXT_LG)
        .spacing(SPACE_XXS)
        .style(theme::RadioClass::Settings.style()),
        iced::widget::radio(
            SecurityOption::StartTls.label(),
            SecurityOption::StartTls,
            Some(current),
            on_change,
        )
        .size(RADIO_SIZE)
        .text_size(TEXT_LG)
        .spacing(SPACE_XXS)
        .style(theme::RadioClass::Settings.style()),
        iced::widget::radio(
            SecurityOption::None.label(),
            SecurityOption::None,
            Some(current),
            on_change,
        )
        .size(RADIO_SIZE)
        .text_size(TEXT_LG)
        .spacing(SPACE_XXS)
        .style(theme::RadioClass::Settings.style()),
    ]
    .spacing(SPACE_MD)
    .into()
}

pub(super) fn auth_method_selector(
    current: ManualAuthMethod,
) -> Element<'static, AddAccountMessage> {
    row![
        iced::widget::radio(
            ManualAuthMethod::OAuth.label(),
            ManualAuthMethod::OAuth,
            Some(current),
            AddAccountMessage::ManualAuthMethodChanged,
        )
        .size(RADIO_SIZE)
        .text_size(TEXT_LG)
        .spacing(SPACE_XXS)
        .style(theme::RadioClass::Settings.style()),
        iced::widget::radio(
            ManualAuthMethod::Password.label(),
            ManualAuthMethod::Password,
            Some(current),
            AddAccountMessage::ManualAuthMethodChanged,
        )
        .size(RADIO_SIZE)
        .text_size(TEXT_LG)
        .spacing(SPACE_XXS)
        .style(theme::RadioClass::Settings.style()),
    ]
    .spacing(SPACE_MD)
    .into()
}

pub(super) fn titlecase(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(c) => {
            let mut result = c.to_uppercase().to_string();
            result.extend(chars);
            result
        }
    }
}
