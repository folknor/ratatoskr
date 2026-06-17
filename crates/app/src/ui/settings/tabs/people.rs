use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::widgets;

pub(super) fn people_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section_untitled(vec![action_row(
        "Import Contacts",
        Some("Import from CSV or vCard file"),
        Some(icon::upload()),
        ActionKind::InApp,
        SettingsMessage::ImportContactsOpen,
    )]));

    let contact_items: Vec<RowBuilder<'_>> = vec![
        static_row(contact_filter_row(
            &state.contact_filter,
            state.focused_filter == Some(FilterId::Contacts),
        )),
        static_row(contact_list_panel(state)),
        people_add_button("New Contact", SettingsMessage::ContactCreate),
    ];

    col = col.push(section("Contacts", contact_items));

    let group_items: Vec<RowBuilder<'_>> = vec![
        static_row(group_filter_row(
            &state.group_filter,
            state.focused_filter == Some(FilterId::Groups),
        )),
        static_row(group_list_panel(state)),
        people_add_button("New Group", SettingsMessage::GroupCreate),
    ];

    col = col.push(section("Groups", group_items));

    col.into()
}

fn contact_filter_row(filter: &str, focused: bool) -> Element<'_, SettingsMessage> {
    container(filter_row(
        "contact-filter",
        "Filter contacts...",
        filter,
        SettingsMessage::ContactFilterChanged,
        FilterId::Contacts,
        focused,
    ))
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn group_filter_row(filter: &str, focused: bool) -> Element<'_, SettingsMessage> {
    container(filter_row(
        "group-filter",
        "Filter groups...",
        filter,
        SettingsMessage::GroupFilterChanged,
        FilterId::Groups,
        focused,
    ))
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

pub(super) fn filter_row<'a>(
    id: &'a str,
    placeholder: &'a str,
    value: &'a str,
    on_input: impl Fn(String) -> SettingsMessage + 'static,
    filter_id: FilterId,
    focused: bool,
) -> Element<'a, SettingsMessage> {
    let value_owned = value.to_string();
    let id_owned = id.to_string();
    let has_value = !value_owned.is_empty();

    let trailing_slot_width = ICON_SM + PAD_ICON_BTN.left + PAD_ICON_BTN.right;
    let trailing: Element<'a, SettingsMessage> = if has_value {
        button(
            container(icon::x().size(ICON_SM).style(text::secondary))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(SettingsMessage::FilterCleared(filter_id))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::BareIcon.style())
        .into()
    } else {
        Space::new()
            .width(Length::Fixed(trailing_slot_width))
            .height(Length::Fixed(
                ICON_SM + PAD_ICON_BTN.top + PAD_ICON_BTN.bottom,
            ))
            .into()
    };

    let content = row![
        container(icon::search().size(ICON_MD).style(text::secondary)).align_y(Alignment::Center),
        text_input(placeholder, &value_owned)
            .id(id_owned)
            .on_input(on_input)
            .size(TEXT_LG)
            .padding(0)
            .style(theme::TextInputClass::Inline.style())
            .width(Length::Fill),
        trailing,
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center);

    let body = container(content)
        .padding(PAD_INPUT)
        .width(Length::Fill)
        .style(move |theme| theme::style_filter_container(theme, focused));

    mouse_area(body)
        .on_press(SettingsMessage::FilterFocused(filter_id))
        .into()
}

fn contact_list_panel(state: &Settings) -> Element<'_, SettingsMessage> {
    let panel: Element<'_, SettingsMessage> = if state.contacts.is_empty() {
        let msg = if state.contact_filter.trim().is_empty() {
            "No contacts yet."
        } else {
            "No contacts match the filter."
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
        for contact in &state.contacts {
            col = col.push(contact_card(contact, &state.managed_accounts));
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

fn contact_card<'a>(
    contact: &'a crate::db::ContactEntry,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let name = contact.display_name.as_deref().unwrap_or("(no name)");
    let id = contact.id.clone();

    let header = row![
        container(text(name).size(TEXT_LG).style(text::base))
            .align_y(Alignment::Center)
            .width(Length::Fill),
        text(&contact.email).size(TEXT_SM).style(text::secondary),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let email2_row: Option<Element<'a, SettingsMessage>> = contact.email2.as_ref().map(|e2| {
        row![
            Space::new().width(Length::Fill),
            text(e2)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_SM)
        .into()
    });

    let mut details = column![].spacing(SPACE_XXXS);
    if let Some(ref phone) = contact.phone {
        details = details.push(
            text(format!("Phone: {phone}"))
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }
    if let Some(ref company) = contact.company {
        details = details.push(
            text(format!("Company: {company}"))
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }
    if let Some(ref notes) = contact.notes {
        details = details.push(
            text(format!("Notes: {notes}"))
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }

    let groups_row: Option<Element<'a, SettingsMessage>> = if contact.groups.is_empty() {
        None
    } else {
        let mut r = row![].spacing(SPACE_XXS).align_y(Alignment::Center);
        for group_name in &contact.groups {
            r = r.push(
                container(text(group_name).size(TEXT_XS).style(text::primary))
                    .padding(iced::Padding {
                        top: 1.0,
                        right: 6.0,
                        bottom: 1.0,
                        left: 6.0,
                    })
                    .style(theme::ContainerClass::Badge.style()),
            );
        }
        Some(r.into())
    };

    let account_row: Option<Element<'a, SettingsMessage>> =
        contact.account_id.as_deref().and_then(|aid| {
            let account = accounts.iter().find(|a| a.id == aid)?;
            let account_label = account
                .account_name
                .as_deref()
                .or(account.display_name.as_deref())
                .unwrap_or(&account.email);
            let pill = match account.account_color.as_deref() {
                Some(hex) => {
                    let color = crate::ui::theme::hex_to_color(hex);
                    container(
                        row![
                            widgets::color_dot::<SettingsMessage>(color),
                            text(account_label.to_string())
                                .size(TEXT_XS)
                                .style(text::secondary),
                        ]
                        .spacing(SPACE_XXS)
                        .align_y(Alignment::Center),
                    )
                    .padding(iced::Padding {
                        top: 1.0,
                        right: 6.0,
                        bottom: 1.0,
                        left: 6.0,
                    })
                    .style(theme::ContainerClass::Badge.style())
                }
                None => container(
                    text(account_label.to_string())
                        .size(TEXT_XS)
                        .style(text::secondary),
                )
                .padding(iced::Padding {
                    top: 1.0,
                    right: 6.0,
                    bottom: 1.0,
                    left: 6.0,
                })
                .style(theme::ContainerClass::Badge.style()),
            };
            Some(
                row![pill]
                    .spacing(SPACE_XXS)
                    .align_y(Alignment::Center)
                    .into(),
            )
        });

    let mut col = column![header].spacing(SPACE_XXXS);
    if let Some(e2) = email2_row {
        col = col.push(e2);
    }
    col = col.push(details);
    if let Some(gr) = groups_row {
        col = col.push(gr);
    }
    if let Some(ar) = account_row {
        col = col.push(ar);
    }

    button(
        container(col)
            .padding(PAD_CARD)
            .width(Length::Fill)
            .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::ContactClick(id))
    .padding(0)
    .style(theme::style_pill_card_button)
    .width(Length::Fill)
    .into()
}

fn group_card(group: &crate::db::GroupEntry) -> Element<'_, SettingsMessage> {
    let id = group.id.clone();
    let member_label = if group.member_count == 1 {
        "1 member".to_string()
    } else {
        format!("{} members", group.member_count)
    };
    let format_date = |ts: i64| -> String {
        chrono::DateTime::from_timestamp(ts, 0)
            .map(|dt| {
                dt.with_timezone(&chrono::Local)
                    .format("%b %d, %Y")
                    .to_string()
            })
            .unwrap_or_default()
    };
    let created_label = format!("Created: {}", format_date(group.created_at));
    let updated_label = format!("Last updated: {}", format_date(group.updated_at));

    let top_row = row![
        container(text(&group.name).size(TEXT_LG).style(text::base))
            .align_y(Alignment::Center)
            .width(Length::Fill),
        text(member_label).size(TEXT_SM).style(text::secondary),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let bottom_row = row![
        container(
            text(created_label)
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        )
        .align_y(Alignment::Center)
        .width(Length::Fill),
        text(updated_label)
            .size(TEXT_SM)
            .style(theme::TextClass::Muted.style()),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let content = column![top_row, bottom_row].spacing(SPACE_XXXS);

    button(
        container(content)
            .padding(PAD_CARD)
            .width(Length::Fill)
            .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::GroupClick(id))
    .padding(0)
    .style(theme::style_pill_card_button)
    .width(Length::Fill)
    .into()
}

fn group_list_panel(state: &Settings) -> Element<'_, SettingsMessage> {
    let panel: Element<'_, SettingsMessage> = if state.groups.is_empty() {
        let msg = if state.group_filter.trim().is_empty() {
            "No groups yet."
        } else {
            "No groups match the filter."
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
        for group in &state.groups {
            col = col.push(group_card(group));
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

fn people_add_button(label: &str, on_press: SettingsMessage) -> RowBuilder<'_> {
    Box::new(move |position| {
        button(
            container(
                row![
                    icon::plus().size(ICON_MD).style(text::base),
                    text(label)
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
        .on_press(on_press)
        .padding(PAD_SETTINGS_ROW)
        .style(move |t, s| theme::style_settings_row_button(t, s, position))
        .width(Length::Fill)
        .height(SETTINGS_ROW_HEIGHT)
        .into()
    })
}
