use iced::time::Instant;
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::dialog::{DialogAction, alert_dialog};
use crate::ui::layout::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::widgets;

mod accounts;
mod ai;
mod behavior;
mod contacts;
mod general;
mod groups;
mod import;
mod mail_rules;
mod people;
mod reference;
mod signatures;
#[path = "theme.rs"]
mod theme_panel;

pub(super) fn settings_view(state: &Settings) -> Element<'_, SettingsMessage> {
    let nav = tab_nav(state.active_tab);
    let content = match state.active_tab {
        Tab::Accounts => accounts::accounts_tab(state),
        Tab::General => general::general_tab(state),
        Tab::Theme => theme_panel::theme_tab(state),
        Tab::Notifications => behavior::notifications_tab(state),
        Tab::Composing => behavior::composing_tab(state),
        Tab::MailRules => mail_rules::mail_rules_tab(state),
        Tab::People => people::people_tab(state),
        Tab::Shortcuts => reference::shortcuts_tab(),
        Tab::Ai => ai::ai_tab(state),
        Tab::About => reference::about_tab(),
    };

    let now = Instant::now();
    let sheet_t: f32 = state.sheet_anim.interpolate(0.0, 1.0, now);
    let show_sheet = state.active_sheet.is_some() || sheet_t > 0.001;

    let scrollbar_gutter = SCROLLBAR_SPACING + 8.0;
    let content_area = container(
        scrollable(
            container(content)
                .padding(PAD_SETTINGS_CONTENT)
                .align_x(Alignment::Center),
        )
        .height(Length::Fill),
    )
    .padding(iced::Padding {
        top: 0.0,
        right: scrollbar_gutter,
        bottom: 0.0,
        left: 0.0,
    })
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style());

    let main_content: Element<'_, SettingsMessage> = if show_sheet {
        let sheet_content = match state.active_sheet {
            Some(SettingsSheetPage::CreateFilter) => mail_rules::create_filter_sheet(),
            Some(SettingsSheetPage::AccountEditor) => accounts::account_editor_sheet(state),
            Some(SettingsSheetPage::EditSignature { .. }) => {
                signatures::signature_editor_sheet(state)
            }
            Some(SettingsSheetPage::EditContact { .. }) => contacts::contact_editor_sheet(state),
            Some(SettingsSheetPage::EditGroup { .. }) => groups::group_editor_sheet(state),
            Some(SettingsSheetPage::ImportContacts) => import::import_wizard_sheet(state),
            None => column![].into(),
        };

        let sheet_panel = container(column![
            container(
                button(
                    row![
                        container(icon::arrow_left().size(ICON_XL).style(text::base))
                            .align_y(Alignment::Center),
                        text("Back")
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
                .on_press(SettingsMessage::CloseSheet)
                .padding(PAD_NAV_ITEM)
                .style(theme::ButtonClass::BareIcon.style()),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
            scrollable(
                container(sheet_content)
                    .padding(PAD_SETTINGS_CONTENT)
                    .align_x(Alignment::Center)
            )
            .spacing(SCROLLBAR_SPACING)
            .height(Length::Fill),
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style());

        let offset = ((1.0 - sheet_t) * 2000.0).round();

        let with_sheet = crate::ui::modal_overlay::modal_overlay(
            content_area,
            sheet_panel,
            crate::ui::modal_overlay::ModalSurface::Sheet { offset },
            SettingsMessage::Noop,
        );

        if state.pending_discard.is_some() {
            crate::ui::modal_overlay::modal_overlay(
                with_sheet,
                discard_changes_dialog(),
                crate::ui::modal_overlay::ModalSurface::Modal,
                SettingsMessage::Noop,
            )
        } else {
            with_sheet
        }
    } else {
        content_area.into()
    };

    row![
        nav,
        iced::widget::rule::vertical(1).style(theme::RuleClass::SidebarDivider.style()),
        main_content,
    ]
    .into()
}

fn discard_changes_dialog<'a>() -> Element<'a, SettingsMessage> {
    alert_dialog(
        "Discard unsaved changes?",
        "Your edits to this item will be lost.",
        vec![
            DialogAction::default_action(
                "Keep editing",
                SettingsMessage::CancelDiscardEditorChanges,
            ),
            DialogAction::destructive(
                "Discard",
                SettingsMessage::ConfirmDiscardEditorChanges,
            ),
        ],
        None,
    )
}

fn tab_nav(active: Tab) -> Element<'static, SettingsMessage> {
    let mut col = column![].spacing(SPACE_XXS).width(SETTINGS_NAV_WIDTH);

    col = col.push(widgets::nav_button(
        Some(icon::arrow_left()),
        "Settings",
        false,
        widgets::NavSize::Regular,
        None,
        SettingsMessage::Close,
    ));
    col = col.push(Space::new().height(SPACE_XS));

    for tab in Tab::ALL {
        let is_active = *tab == active;
        col = col.push(widgets::nav_button(
            Some(tab.icon()),
            tab.label(),
            is_active,
            widgets::NavSize::Regular,
            None,
            SettingsMessage::SelectTab(*tab),
        ));
    }

    container(
        scrollable(col)
            .spacing(SCROLLBAR_SPACING)
            .height(Length::Fill),
    )
    .padding(PAD_SIDEBAR)
    .height(Length::Fill)
    .style(theme::ContainerClass::Sidebar.style())
    .into()
}
