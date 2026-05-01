use iced::widget::{Space, button, column, container, mouse_area, row, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::widgets;

pub(super) fn accounts_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    let mut card_col = column![].width(Length::Fill);
    for (i, account) in state.managed_accounts.iter().enumerate() {
        if i > 0 {
            card_col = card_col
                .push(iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()));
        }
        card_col = card_col.push(account_card(account, i, &state.account_drag));
    }

    let account_list: Element<'_, SettingsMessage> = if state.managed_accounts.len() > 1 {
        mouse_area(card_col)
            .on_move(SettingsMessage::AccountDragMove)
            .on_release(SettingsMessage::AccountDragEnd)
            .on_exit(SettingsMessage::AccountDragEnd)
            .into()
    } else {
        card_col.into()
    };

    let add_btn = button(
        container(
            row![
                icon::plus().size(ICON_MD).style(text::base),
                text("Add Account")
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
    .on_press(SettingsMessage::AddAccountFromSettings)
    .padding(PAD_SETTINGS_ROW)
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .height(SETTINGS_ROW_HEIGHT);

    col = col.push(section(
        "Accounts",
        vec![static_row(account_list), static_row(add_btn)],
    ));

    col.into()
}

fn account_card<'a>(
    account: &'a ManagedAccount,
    index: usize,
    drag: &'a Option<AccountDragState>,
) -> Element<'a, SettingsMessage> {
    let name = account
        .account_name
        .as_deref()
        .or(account.display_name.as_deref())
        .unwrap_or(&account.email);

    let provider_label = format_provider_label(&account.provider);
    let sync_label = format_last_sync(account.last_sync_at);

    let grip_slot = mouse_area(
        container(
            icon::grip_vertical()
                .size(ICON_MD)
                .style(theme::TextClass::Tertiary.style()),
        )
        .width(GRIP_SLOT_WIDTH)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::AccountGripPress(index))
    .interaction(iced::mouse::Interaction::Grab);

    let mut left = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if let Some(ref hex) = account.account_color {
        let color = crate::ui::theme::hex_to_color(hex);
        left = left.push(crate::ui::widgets::color_dot(color));
    }

    left = left.push(
        column![
            text(name).size(TEXT_LG).style(text::base),
            text(&account.email).size(TEXT_SM).style(text::secondary),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    );

    let health_dot = health_indicator(account.health);

    let right = column![
        text(provider_label).size(TEXT_SM).style(text::secondary),
        row![
            text(sync_label).size(TEXT_XS).style(text::secondary),
            Space::new().width(SPACE_XS),
            health_dot,
        ]
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XXXS)
    .align_x(Alignment::End);

    let content = row![
        grip_slot,
        left,
        right,
        container(
            icon::chevron_right()
                .size(ICON_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .align_y(Alignment::Center),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    let is_being_dragged = drag
        .as_ref()
        .is_some_and(|d| d.dragging_index == index && d.is_dragging);

    let id = account.id.clone();
    let mut inner_container = container(content)
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .height(SETTINGS_TOGGLE_ROW_HEIGHT)
        .align_y(Alignment::Center);

    if is_being_dragged {
        inner_container = inner_container.style(theme::ContainerClass::DraggingRow.style());
    }

    button(inner_container)
        .on_press(SettingsMessage::AccountCardClicked(id))
        .padding(0)
        .style(theme::ButtonClass::Action.style())
        .width(Length::Fill)
        .into()
}

fn health_indicator<'a>(health: AccountHealth) -> Element<'a, SettingsMessage> {
    let color = match health {
        AccountHealth::Healthy => iced::Color::from_rgb(0.2, 0.8, 0.3),
        AccountHealth::Warning => iced::Color::from_rgb(1.0, 0.75, 0.0),
        AccountHealth::Error => iced::Color::from_rgb(0.9, 0.2, 0.2),
        AccountHealth::Disabled => iced::Color::from_rgb(0.5, 0.5, 0.5),
    };
    widgets::color_dot(color)
}

fn format_provider_label(provider: &str) -> String {
    match provider {
        "gmail_api" => "Gmail".to_string(),
        "graph" => "Microsoft 365".to_string(),
        "jmap" => "JMAP".to_string(),
        "imap" => "IMAP".to_string(),
        other => other.to_string(),
    }
}

fn format_last_sync(last_sync_at: Option<i64>) -> String {
    match last_sync_at {
        None => "Never synced".to_string(),
        Some(ts) => {
            let Some(dt) = chrono::DateTime::from_timestamp(ts, 0) else {
                return "Unknown".to_string();
            };
            let now = chrono::Utc::now();
            let diff = now.signed_duration_since(dt);
            if diff.num_minutes() < 1 {
                "Just now".to_string()
            } else if diff.num_minutes() < 60 {
                format!("{} min ago", diff.num_minutes())
            } else if diff.num_hours() < 24 {
                format!("{} hours ago", diff.num_hours())
            } else {
                format!("{} days ago", diff.num_days())
            }
        }
    }
}

pub(super) fn account_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let editor = match &state.editing_account {
        Some(e) => e,
        None => return column![].into(),
    };

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text("Edit Account")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    );

    col = col.push(
        text(&editor.account_email)
            .size(TEXT_LG)
            .style(text::secondary),
    );

    col = col.push(account_editor_name_section(editor));

    col = col.push(section(
        "Display Name",
        vec![input_row(
            "account-display-name",
            "Display Name",
            "Your Name",
            editor.display_name.text(),
            SettingsMessage::DisplayNameEditorChanged,
            InputField::AccountDisplayName,
        )],
    ));

    col = col.push(account_editor_color_section(state, editor));

    col = col.push(account_editor_caldav_section(editor));

    col = col.push(section(
        "Authentication",
        vec![action_row(
            "Re-authenticate",
            Some("Sign in again to refresh credentials"),
            None,
            ActionKind::InApp,
            SettingsMessage::ReauthenticateAccount(editor.account_id.clone()),
        )],
    ));

    col = col.push(account_editor_delete_section(editor));

    if editor.dirty {
        col = col.push(
            button(
                container(text("Save Changes").size(TEXT_LG).color(theme::ON_AVATAR))
                    .center_x(Length::Fill),
            )
            .on_press(SettingsMessage::SaveAccountEditor)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Primary.style())
            .width(Length::Fill),
        );
    }

    col.into()
}

fn account_editor_name_section(editor: &AccountEditor) -> Element<'_, SettingsMessage> {
    section(
        "Account Name",
        vec![input_row(
            "account-name",
            "Account Name",
            "e.g. Work",
            editor.account_name.text(),
            SettingsMessage::AccountNameEditorChanged,
            InputField::AccountName,
        )],
    )
}

fn account_editor_color_section<'a>(
    state: &'a Settings,
    editor: &'a AccountEditor,
) -> Element<'a, SettingsMessage> {
    let used_colors: Vec<String> = state
        .managed_accounts
        .iter()
        .filter(|a| a.id != editor.account_id)
        .filter_map(|a| a.account_color.clone())
        .collect();

    let grid = widgets::color_palette_grid(
        editor.account_color_index,
        &used_colors,
        SettingsMessage::AccountColorEditorChanged,
    );

    section(
        "Account Color",
        vec![static_row(
            container(grid)
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
        )],
    )
}

fn account_editor_caldav_section(editor: &AccountEditor) -> Element<'_, SettingsMessage> {
    section(
        "Calendar (CalDAV)",
        vec![
            input_row(
                "caldav-url",
                "CalDAV URL",
                "https://",
                editor.caldav_url.text(),
                SettingsMessage::CaldavUrlChanged,
                InputField::CaldavUrl,
            ),
            input_row(
                "caldav-username",
                "Username",
                "",
                editor.caldav_username.text(),
                SettingsMessage::CaldavUsernameChanged,
                InputField::CaldavUsername,
            ),
            input_row_secure(
                "caldav-password",
                "Password",
                "",
                editor.caldav_password.text(),
                SettingsMessage::CaldavPasswordChanged,
                InputField::CaldavPassword,
            ),
        ],
    )
}

fn account_editor_delete_section(editor: &AccountEditor) -> Element<'_, SettingsMessage> {
    if editor.show_delete_confirmation {
        section(
            "Danger Zone",
            vec![static_row(
                container(
                    column![
                        text("Are you sure you want to delete this account?")
                            .size(TEXT_LG)
                            .style(text::danger),
                        text("All data for this account will be permanently removed.")
                            .size(TEXT_SM)
                            .style(text::secondary),
                        Space::new().height(SPACE_SM),
                        row![
                            button(text("Delete Account").size(TEXT_LG).style(text::danger),)
                                .on_press(SettingsMessage::DeleteAccountConfirmed(
                                    editor.account_id.clone(),
                                ))
                                .padding(PAD_BUTTON)
                                .style(
                                    theme::ButtonClass::ExperimentSemantic { variant: 2 }.style(),
                                ),
                            Space::new().width(SPACE_SM),
                            button(text("Cancel").size(TEXT_LG).style(text::secondary),)
                                .on_press(SettingsMessage::DeleteAccountCancelled)
                                .padding(PAD_BUTTON)
                                .style(theme::ButtonClass::Ghost.style()),
                        ],
                    ]
                    .spacing(SPACE_XS),
                )
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
            )],
        )
    } else {
        section(
            "Danger Zone",
            vec![action_row(
                "Delete Account",
                Some("Remove this account and all its data"),
                None,
                ActionKind::InApp,
                SettingsMessage::DeleteAccountRequested(editor.account_id.clone()),
            )],
        )
    }
}
