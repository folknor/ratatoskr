#![allow(dead_code)]

use iced::widget::{button, container, row, text};
use iced::{Alignment, Element, Length, Theme};

use cmdk::{BindingTable, CommandContext, CommandId, CommandRegistry};

use crate::Message;
use crate::icon;
use crate::ui::layout::{
    ICON_LG, ICON_MD, ICON_SM, PAD_BUTTON, PAD_ICON_BTN, SPACE_XXS, TEXT_LG, TEXT_SM,
};
use crate::ui::theme;

/// Build a toolbar button for a registered command.
///
/// Pulls label (including toggle resolution like Star/Unstar),
/// availability, and keybinding hint from the registry and
/// binding table. Unavailable commands render as disabled buttons
/// with muted text. Keybinding hints appear as tooltips.
pub fn command_button<'a>(
    id: CommandId,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let desc = registry.get(id);
    let (label, available) = desc.map_or(("???", false), |d| {
        (d.resolved_label(ctx), (d.is_available)(ctx))
    });
    let keybinding = binding_table.display_binding(id);

    let label_style: fn(&Theme) -> text::Style = if available {
        text::secondary
    } else {
        theme::TextClass::Tertiary.style()
    };

    let label_text = text(label).size(TEXT_SM).style(label_style);
    let mut btn = button(container(label_text).align_y(Alignment::Center))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Action.style());

    if available {
        btn = btn.on_press(Message::ExecuteCommand(id));
    }

    let _ = keybinding;
    btn.into()
}

/// Build a toolbar button for a registered command, with an icon.
///
/// Same as [`command_button`] but prepends an icon glyph before the label.
pub fn command_icon_button<'a>(
    id: CommandId,
    ico: iced::widget::Text<'a>,
    registry: &CommandRegistry,
    binding_table: &BindingTable,
    ctx: &CommandContext,
) -> Element<'a, Message> {
    let desc = registry.get(id);
    let (label, available) = desc.map_or(("???", false), |d| {
        (d.resolved_label(ctx), (d.is_available)(ctx))
    });
    let keybinding = binding_table.display_binding(id);

    let label_style: fn(&Theme) -> text::Style = if available {
        text::secondary
    } else {
        theme::TextClass::Tertiary.style()
    };
    let icon_style: fn(&Theme) -> text::Style = label_style;

    let content = row![
        container(ico.size(ICON_MD).style(icon_style)).align_y(Alignment::Center),
        container(text(label).size(TEXT_SM).style(label_style)).align_y(Alignment::Center),
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center);

    let mut btn = button(content)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Action.style());

    if available {
        btn = btn.on_press(Message::ExecuteCommand(id));
    }

    let _ = keybinding;
    btn.into()
}

pub fn action_icon_button<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: M,
) -> Element<'a, M> {
    button(
        row![
            container(ico.size(ICON_MD).style(text::secondary)).align_y(Alignment::Center),
            container(text(label).size(TEXT_SM).style(text::secondary))
                .align_y(Alignment::Center),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Action.style())
    .into()
}

pub fn reply_button<'a, M: Clone + 'a>(
    ico: iced::widget::Text<'a>,
    label: &'a str,
    on_press: M,
) -> Element<'a, M> {
    button(
        row![
            ico.size(ICON_SM).style(text::secondary),
            text(label).size(TEXT_SM).style(text::secondary),
        ]
        .spacing(SPACE_XXS)
        .align_y(Alignment::Center),
    )
    .on_press(on_press)
    .padding(PAD_ICON_BTN)
    .style(theme::ButtonClass::Ghost.style())
    .into()
}

pub fn compose_button<'a, M: Clone + 'a>(on_press: M) -> Element<'a, M> {
    button(
        container(
            row![
                container(icon::pencil().size(ICON_LG).color(theme::ON_AVATAR))
                    .align_y(Alignment::Center),
                container(text("Compose").size(TEXT_LG).color(theme::ON_AVATAR))
                    .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill)
        .center_y(Length::Fill),
    )
    .on_press(on_press)
    .padding(PAD_BUTTON)
    .style(theme::ButtonClass::Primary.style())
    .width(Length::Fill)
    .into()
}

pub fn settings_button<'a, M: Clone + 'a>(on_press: M) -> Element<'a, M> {
    button(
        container(
            row![
                container(icon::settings().size(ICON_LG).style(text::primary))
                    .align_y(Alignment::Center),
                container(text("Settings").size(TEXT_LG).style(text::primary))
                    .align_y(Alignment::Center),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        )
        .center_x(Length::Fill),
    )
    .on_press(on_press)
    .style(theme::ButtonClass::Experiment { variant: 10 }.style())
    .padding(PAD_BUTTON)
    .width(Length::Fill)
    .into()
}
