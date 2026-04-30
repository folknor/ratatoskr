use iced::time::Instant;
use iced::widget::{
    Space, button, column, container, mouse_area, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::undoable_text_input::undoable_text_input;
use crate::ui::widgets;

use rte::{Action as RteAction, BlockKind, EditAction, InlineStyle, rich_text_editor};

use super::row_widgets::*;
use super::types::*;

// ── View ────────────────────────────────────────────────

pub(super) fn settings_view(state: &Settings) -> Element<'_, SettingsMessage> {
    let nav = tab_nav(state.active_tab);
    let content = match state.active_tab {
        Tab::Accounts => accounts_tab(state),
        Tab::General => general_tab(state),
        Tab::Theme => theme_tab(state),
        Tab::Notifications => notifications_tab(state),
        Tab::Composing => composing_tab(state),
        Tab::MailRules => mail_rules_tab(state),
        Tab::People => people_tab(state),
        Tab::Shortcuts => shortcuts_tab(),
        Tab::Ai => ai_tab(state),
        Tab::About => about_tab(),
    };

    let now = Instant::now();
    let sheet_t: f32 = state.sheet_anim.interpolate(0.0, 1.0, now);
    let show_sheet = state.active_sheet.is_some() || sheet_t > 0.001;

    // Float the scrollbar so its appearing/disappearing doesn't shift the
    // content horizontally. We instead reserve a fixed right gutter on the
    // outer container that's wide enough to host the scrollbar so the
    // bar overlays empty space rather than the content.
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
            Some(SettingsSheetPage::CreateFilter) => create_filter_sheet(),
            Some(SettingsSheetPage::AccountEditor) => account_editor_sheet(state),
            Some(SettingsSheetPage::EditSignature { .. }) => signature_editor_sheet(state),
            Some(SettingsSheetPage::EditContact { .. }) => contact_editor_sheet(state),
            Some(SettingsSheetPage::EditGroup { .. }) => group_editor_sheet(state),
            Some(SettingsSheetPage::ImportContacts) => import_wizard_sheet(state),
            None => column![].into(), // closing animation
        };

        // Sheet panel: back button header + scrollable content
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

        // Slide from right: use a large fixed offset (2000px) scaled by (1-t).
        // The stack clips to bounds so overshooting doesn't matter.
        let offset = ((1.0 - sheet_t) * 2000.0).round();

        crate::ui::modal_overlay::modal_overlay(
            content_area,
            sheet_panel,
            crate::ui::modal_overlay::ModalSurface::Sheet { offset },
            SettingsMessage::Noop,
        )
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

// ── Tab navigation ──────────────────────────────────────

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

// ── General tab ─────────────────────────────────────────

fn general_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Appearance",
        vec![
            setting_row(
                "Theme",
                widgets::select(
                    &["System", "Light", "Dark", "Theme"],
                    &state.theme,
                    state.open_select == Some(SelectField::Theme),
                    SettingsMessage::ToggleSelect(SelectField::Theme),
                    SettingsMessage::ThemeChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::Theme),
            ),
            setting_row(
                "Email Density",
                widgets::select(
                    &["Compact", "Default", "Spacious"],
                    &state.density,
                    state.open_select == Some(SelectField::Density),
                    SettingsMessage::ToggleSelect(SelectField::Density),
                    SettingsMessage::DensityChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::Density),
            ),
            setting_row(
                "Font Size",
                widgets::select(
                    &["Small", "Default", "Large", "XLarge"],
                    &state.font_size,
                    state.open_select == Some(SelectField::FontSize),
                    SettingsMessage::ToggleSelect(SelectField::FontSize),
                    SettingsMessage::FontSizeChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::FontSize),
            ),
            setting_row(
                "Email Body Background",
                widgets::select(
                    &["Always White", "Match Theme", "Auto"],
                    state.email_body_background.label(),
                    state.open_select == Some(SelectField::EmailBodyBg),
                    SettingsMessage::ToggleSelect(SelectField::EmailBodyBg),
                    SettingsMessage::EmailBodyBgChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::EmailBodyBg),
            ),
            slider_row(
                "Scale",
                None,
                1.0..=4.0,
                state.scale_preview.unwrap_or(state.scale),
                1.0,
                0.125,
                SettingsMessage::ScaleDragged,
                Some(SettingsMessage::ScaleReleased),
            ),
            setting_row_with_description(
                "Message Dates",
                Some("Relative offset shows \"2h ago\"; Absolute shows the calendar date."),
                widgets::select(
                    &["Relative Offset", "Absolute"],
                    match state.date_display {
                        crate::db::DateDisplay::RelativeOffset => "Relative Offset",
                        crate::db::DateDisplay::Absolute => "Absolute",
                    },
                    state.open_select == Some(SelectField::DateDisplay),
                    SettingsMessage::ToggleSelect(SelectField::DateDisplay),
                    SettingsMessage::DateDisplayChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::DateDisplay),
            ),
            toggle_row(
                "Show Sync Status Bar",
                "Display sync progress in the status bar",
                state.sync_status_bar,
                SettingsMessage::ToggleSyncStatusBar,
            ),
        ],
    ));

    col = col.push(section(
        "Reading Pane",
        radio_group(
            &[
                ("Right", "Right"),
                ("Bottom", "Bottom"),
                ("Hidden", "Hidden"),
            ],
            Some(state.reading_pane_position.as_str()),
            |v| SettingsMessage::ReadingPaneChanged(v.to_string()),
        ),
    ));

    let privacy_help_id = "privacy-security";
    let privacy_help_visible = state.hovered_help.as_deref() == Some(privacy_help_id);
    col = col.push(section_with_help("Privacy & Security", SectionHelp {
        id: privacy_help_id,
        content: column![
            text("Remote images can be used to track when you open an email. Blocking them prevents this but some emails may not display correctly.")
                .size(TEXT_SM)
                .style(theme::TextClass::OnPrimary.style()),
            Space::new().height(SPACE_XS),
            text("Phishing detection analyzes incoming emails for suspicious links, sender spoofing, and social engineering patterns.")
                .size(TEXT_SM)
                .style(theme::TextClass::OnPrimary.style()),
        ]
        .into(),
        visible: privacy_help_visible,
    }, vec![
        toggle_row("Block Remote Images", "Don't load remote images in email bodies", state.block_remote_images, SettingsMessage::ToggleBlockRemoteImages),
        toggle_row("Phishing Detection", "Warn about suspicious emails", state.phishing_detection, SettingsMessage::TogglePhishingDetection),
    ]));

    col.into()
}

// ── Theme tab ───────────────────────────────────────────

fn theme_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    // Build a 3-column grid of theme previews
    let mut grid = column![].spacing(SPACE_XS);
    let mut current_row = row![].spacing(SPACE_XS);
    let mut col_count = 0;

    for (i, entry) in theme::THEMES.iter().enumerate() {
        let selected = state.selected_theme == Some(i)
            || (state.selected_theme.is_none() && state.theme == entry.name);

        let card = column![
            widgets::theme_preview(&entry.palette, selected, crate::Message::Noop)
                .map(move |_| SettingsMessage::ThemeSelected(i)),
            container(text(entry.name).size(TEXT_SM).style(if selected {
                text::base
            } else {
                text::secondary
            }),)
            .width(Length::Fill)
            .align_x(Alignment::Center),
        ]
        .spacing(SPACE_XXS)
        .align_x(Alignment::Center);

        current_row = current_row.push(container(card).width(Length::FillPortion(1)));
        col_count += 1;

        if col_count == 3 {
            grid = grid.push(current_row);
            current_row = row![].spacing(SPACE_XS);
            col_count = 0;
        }
    }

    // Push remaining items with empty spacers to fill the row
    if col_count > 0 {
        while col_count < 3 {
            current_row = current_row
                .push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1)));
            col_count += 1;
        }
        grid = grid.push(current_row);
    }

    col = col.push(section(
        "Themes",
        vec![static_row(
            container(grid)
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
        )],
    ));

    // Button experiment grid: each candidate next to a Primary for comparison
    let experiments: Vec<(&str, usize)> = vec![
        ("pri border", 8),
        ("text border", 9),
        ("pri+fill", 10),
        ("muted border", 11),
        ("mix 15%", 20),
        ("text 10%", 19),
    ];

    let mut grid = column![].spacing(SPACE_XS);
    let mut current_row = row![].spacing(SPACE_XS);
    let mut col_count = 0;

    for (label, idx) in &experiments {
        let btn_width = Length::Fixed(120.0);
        let pair = row![
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Experiment { variant: *idx }.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Primary.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
        ]
        .spacing(SPACE_XXS);

        current_row = current_row.push(container(pair).width(Length::FillPortion(1)));
        col_count += 1;

        if col_count == 2 {
            grid = grid.push(current_row);
            current_row = row![].spacing(SPACE_XS);
            col_count = 0;
        }
    }
    if col_count > 0 {
        while col_count < 2 {
            current_row = current_row
                .push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1)));
            col_count += 1;
        }
        grid = grid.push(current_row);
    }

    col = col.push(section(
        "Button Experiments (section bg)",
        vec![static_row(
            container(grid)
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
        )],
    ));

    // Same grid on content/main area background
    let mut grid2 = column![].spacing(SPACE_XS);
    let mut current_row2 = row![].spacing(SPACE_XS);
    let mut col_count2 = 0;
    for (label, idx) in &experiments {
        let btn_width = Length::Fixed(120.0);
        let pair = row![
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Experiment { variant: *idx }.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Primary.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
        ]
        .spacing(SPACE_XXS);
        current_row2 = current_row2.push(container(pair).width(Length::FillPortion(1)));
        col_count2 += 1;
        if col_count2 == 2 {
            grid2 = grid2.push(current_row2);
            current_row2 = row![].spacing(SPACE_XS);
            col_count2 = 0;
        }
    }
    if col_count2 > 0 {
        while col_count2 < 2 {
            current_row2 = current_row2
                .push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1)));
            col_count2 += 1;
        }
        grid2 = grid2.push(current_row2);
    }

    let content_bg_box = container(
        column![
            text("Content / main area background")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
            grid2,
        ]
        .spacing(SPACE_SM),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .style(theme::ContainerClass::Content.style());

    col = col.push(content_bg_box);

    // Same grid on sidebar background
    let mut grid3 = column![].spacing(SPACE_XS);
    let mut current_row3 = row![].spacing(SPACE_XS);
    let mut col_count3 = 0;
    for (label, idx) in &experiments {
        let btn_width = Length::Fixed(120.0);
        let pair = row![
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Experiment { variant: *idx }.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Primary.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
        ]
        .spacing(SPACE_XXS);
        current_row3 = current_row3.push(container(pair).width(Length::FillPortion(1)));
        col_count3 += 1;
        if col_count3 == 2 {
            grid3 = grid3.push(current_row3);
            current_row3 = row![].spacing(SPACE_XS);
            col_count3 = 0;
        }
    }
    if col_count3 > 0 {
        while col_count3 < 2 {
            current_row3 = current_row3
                .push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1)));
            col_count3 += 1;
        }
        grid3 = grid3.push(current_row3);
    }

    let sidebar_bg_box = container(
        column![
            text("Sidebar background")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
            grid3,
        ]
        .spacing(SPACE_SM),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .style(theme::ContainerClass::Sidebar.style());

    col = col.push(sidebar_bg_box);

    // Semantic color pairs
    let btn_width = Length::Fixed(120.0);
    let semantic_grid = column![
        row![
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Primary.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Primary.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
        ]
        .spacing(SPACE_XXS),
        row![
            button(container(text("Success").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::ExperimentSemantic { variant: 0 }.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Primary.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
        ]
        .spacing(SPACE_XXS),
        row![
            button(container(text("Warning").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::ExperimentSemantic { variant: 1 }.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Primary.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
        ]
        .spacing(SPACE_XXS),
        row![
            button(container(text("Danger").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::ExperimentSemantic { variant: 2 }.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill))
                .on_press(SettingsMessage::Noop)
                .style(theme::ButtonClass::Primary.style())
                .padding(PAD_BUTTON)
                .width(btn_width),
        ]
        .spacing(SPACE_XXS),
    ]
    .spacing(SPACE_XS);

    col = col.push(section(
        "Semantic Color Pairs",
        vec![static_row(
            container(semantic_grid)
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
        )],
    ));

    col.into()
}

// ── Composing tab ────────────────────────────────────────

fn composing_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Sending",
        vec![
            setting_row(
                "Undo Send Delay",
                widgets::select(
                    &["None", "5 seconds", "10 seconds", "30 seconds"],
                    &state.undo_delay,
                    state.open_select == Some(SelectField::UndoDelay),
                    SettingsMessage::ToggleSelect(SelectField::UndoDelay),
                    SettingsMessage::UndoDelayChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::UndoDelay),
            ),
            toggle_row(
                "Send & Archive",
                "Archive a thread immediately after sending a reply",
                state.send_and_archive,
                SettingsMessage::ToggleSendAndArchive,
            ),
        ],
    ));

    col = col.push(section(
        "Behavior",
        vec![
            setting_row(
                "Default Reply Action",
                widgets::select(
                    &["Reply", "Reply All"],
                    &state.default_reply_mode,
                    state.open_select == Some(SelectField::DefaultReply),
                    SettingsMessage::ToggleSelect(SelectField::DefaultReply),
                    SettingsMessage::DefaultReplyChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::DefaultReply),
            ),
            setting_row(
                "Mark as Read",
                widgets::select(
                    &["Instantly", "After 2 Seconds", "Manually"],
                    &state.mark_as_read,
                    state.open_select == Some(SelectField::MarkAsRead),
                    SettingsMessage::ToggleSelect(SelectField::MarkAsRead),
                    SettingsMessage::MarkAsReadChanged,
                ),
                SettingsMessage::ToggleSelect(SelectField::MarkAsRead),
            ),
        ],
    ));

    col = col.push(signature_list_section(state));

    col = col.push(section(
        "Templates",
        vec![coming_soon_row("Template management")],
    ));

    col.into()
}

// ── Notifications tab ────────────────────────────────────

fn notifications_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Notifications",
        vec![
            toggle_row(
                "Enable Notifications",
                "Receive desktop notifications for new email",
                state.notifications_enabled,
                SettingsMessage::ToggleNotifications,
            ),
            toggle_row(
                "Smart Notifications",
                "Only notify about important emails",
                state.smart_notifications,
                SettingsMessage::ToggleSmartNotifications,
            ),
        ],
    ));

    if state.smart_notifications {
        let chips: Vec<Element<'_, SettingsMessage>> =
            ["Primary", "Updates", "Promotions", "Social", "Newsletters"]
                .iter()
                .map(|cat| {
                    let active = state.notify_categories.contains(&(*cat).to_string());
                    button(text(*cat).size(TEXT_SM))
                        .on_press(SettingsMessage::ToggleNotifyCategory((*cat).to_string()))
                        .padding(PAD_ICON_BTN)
                        .style(theme::ButtonClass::Chip { active }.style())
                        .into()
                })
                .collect();
        let chips_row = iced::widget::row(chips)
            .spacing(SPACE_XS)
            .align_y(Alignment::Center);
        col = col.push(section(
            "Notify for Categories",
            vec![settings_row_container(SETTINGS_ROW_HEIGHT, chips_row)],
        ));

        let mut vip_col = column![].spacing(SPACE_XXXS).width(Length::Fill);

        vip_col = vip_col.push(
            container(
                text("Always notify when email arrives from a VIP sender.")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        );

        for email in &state.vip_senders {
            vip_col = vip_col.push(
                container(
                    row![
                        container(text(email.clone()).size(TEXT_LG).style(text::base))
                            .align_y(Alignment::Center)
                            .width(Length::Fill),
                        button(text("Remove").size(TEXT_SM).style(text::danger))
                            .on_press(SettingsMessage::RemoveVipSender(email.clone()))
                            .padding(PAD_ICON_BTN)
                            .style(theme::ButtonClass::Action.style()),
                    ]
                    .align_y(Alignment::Center),
                )
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
            );
        }

        vip_col = vip_col.push(
            container(
                row![
                    undoable_text_input("email@example.com", state.vip_email_input.text())
                        .on_input(SettingsMessage::VipEmailChanged)
                        .on_submit(SettingsMessage::AddVipSender)
                        .on_undo(SettingsMessage::UndoInput(InputField::VipEmail))
                        .on_redo(SettingsMessage::RedoInput(InputField::VipEmail))
                        .size(TEXT_LG)
                        .padding(PAD_INPUT)
                        .style(theme::TextInputClass::Settings.style())
                        .width(Length::Fill),
                    Space::new().width(SPACE_XS),
                    button(text("Add").size(TEXT_LG))
                        .on_press(SettingsMessage::AddVipSender)
                        .padding(PAD_ICON_BTN)
                        .style(theme::ButtonClass::Secondary.style()),
                ]
                .align_y(Alignment::Center),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        );

        col = col.push(section("VIP Senders", vec![static_row(vip_col)]));
    }

    col.into()
}

// ── Mail Rules tab ───────────────────────────────────────

fn mail_rules_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Labels",
        vec![editable_list(
            "labels",
            &state.demo_labels,
            "Add Label",
            &state.drag_state,
        )],
    ));
    col = col.push(section(
        "Filters",
        vec![action_row(
            "Create Filter",
            Some("Add a new mail filter rule"),
            Some(icon::filter()),
            ActionKind::InApp,
            SettingsMessage::OpenSheet(SettingsSheetPage::CreateFilter),
        )],
    ));
    if !state.demo_filters.is_empty() {
        col = col.push(section_untitled(vec![editable_list(
            "filters",
            &state.demo_filters,
            "Add Filter",
            &state.drag_state,
        )]));
    }
    col = col.push(section(
        "Smart Labels",
        vec![coming_soon_row("Smart label management")],
    ));
    col = col.push(section(
        "Smart Folders",
        vec![coming_soon_row("Smart folder management")],
    ));
    col = col.push(section(
        "Quick Steps",
        vec![coming_soon_row("Quick step management")],
    ));

    col.into()
}

// ── Overlays ─────────────────────────────────────────────

fn create_filter_sheet<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text("Create Filter")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    );

    col = col.push(section(
        "Conditions",
        vec![coming_soon_row("Match conditions")],
    ));

    col = col.push(section("Actions", vec![coming_soon_row("Filter actions")]));

    col.into()
}

// ── Account editor sheet ────────────────────────────────

fn account_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
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

    // Account name
    col = col.push(account_editor_name_section(editor));

    // Display name
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

    // Account color
    col = col.push(account_editor_color_section(state, editor));

    // CalDAV settings
    col = col.push(account_editor_caldav_section(editor));

    // Re-authenticate action
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

    // Delete section
    col = col.push(account_editor_delete_section(editor));

    // Save button
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

// ── Signature list section ───────────────────────────────

fn signature_list_section(state: &Settings) -> Element<'_, SettingsMessage> {
    if state.signatures.is_empty() && state.managed_accounts.is_empty() {
        return section(
            "Signatures",
            vec![coming_soon_row("No accounts configured")],
        );
    }

    // Group signatures by account_id
    let mut items: Vec<RowBuilder<'_>> = Vec::new();

    for account in &state.managed_accounts {
        let account_sigs: Vec<&SignatureEntry> = state
            .signatures
            .iter()
            .filter(|s| s.account_id == account.id)
            .collect();

        // Account header row
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

        // Signature rows for this account (with global indices for drag)
        for sig in &account_sigs {
            let global_idx = state
                .signatures
                .iter()
                .position(|s| s.id == sig.id)
                .unwrap_or(0);
            items.push(signature_row(sig, global_idx));
        }

        // Add Signature button for this account
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

    // Wrap in mouse_area for drag-move tracking.
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

        // Show a preview snippet of the body (plain text, first 60 chars)
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

        // Drag grip handle
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

        // Default / Reply default badges
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

        // Remove button - opens editor sheet with delete confirmation
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

// ── Signature editor sheet ─────────────────────────────

fn signature_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
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

    // Name field
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

    // Default checkboxes
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

    // Formatting toolbar + rich text editor
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

    // Action buttons
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

/// Formatting toolbar for the signature rich text editor.
///
/// B/I/U/S, list toggles, blockquote, and link buttons.
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
        block_btn(icon::text_quote(), BlockKind::BlockQuote),
    ]
    .spacing(SPACE_XXXS)
    .align_y(Alignment::Center);

    toolbar.into()
}

// ── People tab ───────────────────────────────────────────

fn people_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    // ── Contacts section ──
    // ── Import action (top of the contact management UI per spec) ──
    col = col.push(section_untitled(vec![action_row(
        "Import Contacts",
        Some("Import from CSV or vCard file"),
        Some(icon::upload()),
        ActionKind::InApp,
        SettingsMessage::ImportContactsOpen,
    )]));

    let mut contact_items: Vec<RowBuilder<'_>> = Vec::new();

    // Filter input
    contact_items.push(static_row(contact_filter_row(
        &state.contact_filter,
        state.focused_filter == Some(FilterId::Contacts),
    )));

    // Recessed scrollable panel of contact pills
    contact_items.push(static_row(contact_list_panel(state)));

    // New Contact button
    contact_items.push(people_add_button(
        "New Contact",
        SettingsMessage::ContactCreate,
    ));

    col = col.push(section("Contacts", contact_items));

    // ── Groups section ──
    let mut group_items: Vec<RowBuilder<'_>> = Vec::new();

    // Filter input
    group_items.push(static_row(group_filter_row(
        &state.group_filter,
        state.focused_filter == Some(FilterId::Groups),
    )));

    // Recessed scrollable panel of group pills
    group_items.push(static_row(group_list_panel(state)));

    // New Group button
    group_items.push(people_add_button(
        "New Group",
        SettingsMessage::GroupCreate,
    ));

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

/// Shared search/filter input. The outer container owns the bg + border so
/// the search icon (left), inline text input (center), and clear button
/// (right, when value is non-empty) all read as one unified field. Wraps
/// the input in a `mouse_area` so a click sets `focused_filter` even when
/// the user hasn't typed yet (Escape must know which filter to clear).
fn filter_row<'a>(
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

    // Trailing slot is fixed-size so the row height stays constant whether
    // the clear button is showing or not. Width = PAD_ICON_BTN (l+r) + ICON_SM.
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
        container(icon::search().size(ICON_MD).style(text::secondary))
            .align_y(Alignment::Center),
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

/// Recessed, fixed-height scrollable panel holding the contact pills. Used
/// inside the "Contacts" section between the filter row and the New Contact
/// button so the list stays compact regardless of how many contacts exist.
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
        let mut col = column![]
            .spacing(PEOPLE_PILL_SPACING)
            .width(Length::Fill);
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

    // Inset the panel from the section walls so its rounded corners + border
    // have breathing room.
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

        // ── Header row: display name (left) + email (right) ──
        let header = row![
            container(text(name).size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center)
                .width(Length::Fill),
            text(&contact.email)
                .size(TEXT_SM)
                .style(text::secondary),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center);

        // ── Optional secondary email (right-aligned, below header) ──
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

        // ── Detail lines (left-aligned, only if present) ──
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

        // ── Group pills (primary-tinted) ──
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

        // ── Account pill (colored dot + account name) ──
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

        // Assemble. Header always present; later rows optional.
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

    // Top row: name (left) + member count (right).
    let top_row = row![
        container(text(&group.name).size(TEXT_LG).style(text::base))
            .align_y(Alignment::Center)
            .width(Length::Fill),
        text(member_label).size(TEXT_SM).style(text::secondary),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    // Bottom row: created date (left) + last updated date (right).
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

/// Recessed scrollable panel of group pills, mirroring `contact_list_panel`.
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
        let mut col = column![]
            .spacing(PEOPLE_PILL_SPACING)
            .width(Length::Fill);
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

// ── Contact editor sheet ───────────────────────────────

fn contact_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.contact_editor else {
        return column![].into();
    };

    let is_new = editor.contact_id.is_none();
    let title = if is_new {
        "New Contact"
    } else {
        "Edit Contact"
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

    // Account selector
    col = col.push(contact_account_selector(editor, &state.managed_accounts));

    col = col.push(contact_editor_fields(editor));
    col = col.push(contact_editor_buttons(
        editor,
        &state.confirm_delete_contact,
    ));

    col.into()
}

fn contact_account_selector<'a>(
    editor: &'a ContactEditorState,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let selected_id = editor.account_id.as_deref();

    let mut btn_row = row![].spacing(SPACE_XS).align_y(Alignment::Center);

    // "Local" option
    let is_local = selected_id.is_none();
    let local_style = if is_local {
        theme::ButtonClass::Primary
    } else {
        theme::ButtonClass::Ghost
    };
    btn_row = btn_row.push(
        button(text("Local").size(TEXT_SM))
            .style(local_style.style())
            .on_press(SettingsMessage::ContactEditorAccountChanged(None))
            .padding(PAD_ICON_BTN),
    );

    // Account options
    for account in accounts {
        let is_selected = selected_id == Some(account.id.as_str());
        let style = if is_selected {
            theme::ButtonClass::Primary
        } else {
            theme::ButtonClass::Ghost
        };
        let aid = Some(account.id.clone());
        btn_row = btn_row.push(
            button(text(&account.email).size(TEXT_SM))
                .style(style.style())
                .on_press(SettingsMessage::ContactEditorAccountChanged(aid))
                .padding(PAD_ICON_BTN),
        );
    }

    container(column![
        text("Account")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
        Space::new().height(SPACE_XXXS),
        btn_row,
    ])
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn contact_editor_fields(editor: &ContactEditorState) -> Element<'_, SettingsMessage> {
    let fields = vec![
        contact_field_input(
            "contact-display-name",
            "Display name",
            "Name",
            editor.display_name.text(),
            ContactField::DisplayName,
            InputField::ContactDisplayName,
        ),
        contact_field_input(
            "contact-email",
            "Email",
            "email@example.com",
            editor.email.text(),
            ContactField::Email,
            InputField::ContactEmail,
        ),
        contact_field_input(
            "contact-email2",
            "Email 2",
            "Optional second email",
            editor.email2.text(),
            ContactField::Email2,
            InputField::ContactEmail2,
        ),
        contact_field_input(
            "contact-phone",
            "Phone",
            "Optional phone number",
            editor.phone.text(),
            ContactField::Phone,
            InputField::ContactPhone,
        ),
        contact_field_input(
            "contact-company",
            "Company",
            "Optional company",
            editor.company.text(),
            ContactField::Company,
            InputField::ContactCompany,
        ),
        contact_field_input(
            "contact-notes",
            "Notes",
            "Optional notes",
            editor.notes.text(),
            ContactField::Notes,
            InputField::ContactNotes,
        ),
    ];
    section("Details", fields)
}

fn contact_field_input(
    id: &str,
    label: &str,
    placeholder: &str,
    value: &str,
    contact_field: ContactField,
    input_field: InputField,
) -> RowBuilder<'static> {
    input_row(
        id,
        label,
        placeholder,
        value,
        move |v| SettingsMessage::ContactEditorFieldChanged(contact_field.clone(), v),
        input_field,
    )
}

fn contact_editor_buttons<'a>(
    editor: &'a ContactEditorState,
    confirm_delete: &'a Option<String>,
) -> Element<'a, SettingsMessage> {
    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if let Some(ref id) = editor.contact_id {
        // Show confirmation buttons if this contact is pending delete
        let is_confirming = confirm_delete.as_deref() == Some(id.as_str());
        if is_confirming {
            btn_row = btn_row.push(
                button(text("Confirm delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::ContactConfirmDelete(id.clone()))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
            btn_row = btn_row.push(
                button(text("Cancel").size(TEXT_LG))
                    .on_press(SettingsMessage::ContactCancelDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Ghost.style()),
            );
        } else {
            btn_row = btn_row.push(
                button(text("Delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::ContactDelete(id.clone()))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
        }
    }

    btn_row = btn_row.push(Space::new().width(Length::Fill));

    // Local contacts auto-save on field change - no Save button needed.
    // Synced contacts need an explicit Save button (enabled when dirty).
    let is_local = editor.source.as_deref().is_none_or(|s| s == "user");
    if is_local {
        // Show a subtle "Auto-saved" indicator when dirty
        if editor.contact_id.is_some() {
            btn_row = btn_row.push(text("Auto-saved").size(TEXT_SM).style(text::secondary));
        } else {
            // New contact: still need a Create button
            let can_save = !editor.email.text().trim().is_empty();
            let mut save_btn =
                button(container(text("Create").size(TEXT_LG)).center_x(Length::Fill))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Primary.style())
                    .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
            if can_save {
                save_btn = save_btn.on_press(SettingsMessage::ContactEditorSave);
            }
            btn_row = btn_row.push(save_btn);
        }
    } else {
        // Synced contact: explicit Save button, enabled when dirty
        let can_save = !editor.email.text().trim().is_empty() && editor.dirty;
        let mut save_btn = button(container(text("Save").size(TEXT_LG)).center_x(Length::Fill))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Primary.style())
            .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
        if can_save {
            save_btn = save_btn.on_press(SettingsMessage::ContactEditorSave);
        }
        btn_row = btn_row.push(save_btn);
    }

    btn_row.into()
}

// ── Group editor sheet ─────────────────────────────────

fn group_editor_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.group_editor else {
        return column![].into();
    };

    let is_new = editor.group_id.is_none();
    let title = if is_new { "New Group" } else { "Edit Group" };

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
            text("Group changes are not saved automatically. Use the Save button at the bottom.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ]
        .spacing(SPACE_XXS),
    );

    // Group name
    col = col.push(section(
        "Name",
        vec![input_row(
            "group-name",
            "Name",
            "Group name",
            editor.name.text(),
            SettingsMessage::GroupEditorNameChanged,
            InputField::GroupName,
        )],
    ));

    // Add Members section (filter + paste hint + non-member contacts)
    col = col.push(group_add_members_section(editor, state));

    // Members grid (dynamic title + tile grid; click to remove)
    col = col.push(group_members_list_section(editor, state));

    // Action buttons
    col = col.push(group_editor_buttons(editor, &state.confirm_delete_group));

    col.into()
}

/// Add Members section: filter input + recessed panel of non-member contact
/// pills, with a paste hint subtitle.
fn group_add_members_section<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let items: Vec<RowBuilder<'a>> = vec![
        static_row(
            container(filter_row(
                "group-add-filter",
                "Filter contacts...",
                &editor.filter,
                SettingsMessage::GroupEditorFilterChanged,
                FilterId::GroupAddMembers,
                state.focused_filter == Some(FilterId::GroupAddMembers),
            ))
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill),
        ),
        static_row(group_add_candidates_panel(editor, state)),
    ];

    section_with_subtitle(
        "Add Members",
        "You can paste a large list of email addresses here.",
        items,
    )
}

fn group_add_candidate_pill<'a>(
    contact: &'a crate::db::ContactEntry,
) -> Element<'a, SettingsMessage> {
    let email_for_press = contact.email.clone();
    let label: &str = contact.display_name.as_deref().unwrap_or(&contact.email);
    button(
        container(
            row![
                container(icon::plus().size(ICON_SM).style(text::primary))
                    .align_y(Alignment::Center),
                container(text(label).size(TEXT_LG).style(text::base))
                    .align_y(Alignment::Center)
                    .width(Length::Fill),
                container(
                    text(&contact.email)
                        .size(TEXT_SM)
                        .style(theme::TextClass::Tertiary.style()),
                )
                .align_y(Alignment::Center),
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::GroupEditorAddMember(email_for_press))
    .padding(0)
    .style(theme::style_pill_card_button)
    .width(Length::Fill)
    .into()
}

/// Recessed scrollable panel of candidate contact pills (non-members
/// optionally filtered).
fn group_add_candidates_panel<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let filter_lower = editor.filter.to_lowercase();
    let candidates: Vec<&crate::db::ContactEntry> = state
        .contacts
        .iter()
        .filter(|c| !editor.members.contains(&c.email))
        .filter(|c| {
            filter_lower.is_empty()
                || c.email.to_lowercase().contains(&filter_lower)
                || c.display_name
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&filter_lower)
        })
        .collect();

    let panel: Element<'_, SettingsMessage> = if candidates.is_empty() {
        let msg = if filter_lower.is_empty() {
            "All contacts are already in this group."
        } else {
            "No matching contacts."
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
        let mut col = column![]
            .spacing(PEOPLE_PILL_SPACING)
            .width(Length::Fill);
        for contact in candidates {
            col = col.push(group_add_candidate_pill(contact));
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

/// Members section: dynamic title `Members (N)` over a recessed scrollable
/// panel of full-width member pills. Mirrors the Add Members layout, just
/// with a delete (trash) icon instead of a plus icon. Clicking a pill
/// removes that member from the group.
fn group_members_list_section<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let title = format!("Members ({})", editor.members.len());

    let filter = container(filter_row(
        "group-members-filter",
        "Filter members...",
        &editor.members_filter,
        SettingsMessage::GroupEditorMembersFilterChanged,
        FilterId::GroupMembers,
        state.focused_filter == Some(FilterId::GroupMembers),
    ))
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill);

    let panel: Element<'_, SettingsMessage> = if editor.members.is_empty() {
        let empty_panel = container(
            text("No members yet. Use the list above or paste emails to add them.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .height(PEOPLE_PANEL_HEIGHT)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(theme::style_recessed_list_panel);
        container(empty_panel)
            .padding(SPACE_XS)
            .width(Length::Fill)
            .into()
    } else {
        group_members_list_panel(editor, state)
    };

    section_dynamic_with_subtitle(
        title,
        "Click a member to remove them from the group.".to_string(),
        vec![static_row(filter), static_row(panel)],
    )
}

/// Recessed scrollable panel housing the member pills (one per row).
/// Honours `editor.members_filter` so the list narrows as the user types
/// in the filter input above the panel.
fn group_members_list_panel<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let filter_lower = editor.members_filter.to_lowercase();
    let visible: Vec<&'a String> = editor
        .members
        .iter()
        .filter(|email| {
            if filter_lower.is_empty() {
                return true;
            }
            if email.to_lowercase().contains(&filter_lower) {
                return true;
            }
            // Also match against the contact's display name when known.
            state
                .contacts
                .iter()
                .find(|c| &c.email == *email)
                .and_then(|c| c.display_name.as_deref())
                .map(|n| n.to_lowercase().contains(&filter_lower))
                .unwrap_or(false)
        })
        .collect();

    if visible.is_empty() {
        let empty = container(
            text("No members match the filter.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .height(PEOPLE_PANEL_HEIGHT)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .style(theme::style_recessed_list_panel);
        return container(empty)
            .padding(SPACE_XS)
            .width(Length::Fill)
            .into();
    }

    let mut col = column![]
        .spacing(PEOPLE_PILL_SPACING)
        .width(Length::Fill);
    for email in visible {
        col = col.push(member_pill(email, state));
    }

    let panel = container(
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
    .style(theme::style_recessed_list_panel);

    container(panel)
        .padding(SPACE_XS)
        .width(Length::Fill)
        .into()
}

/// A single member rendered as a pill: trash icon (left, danger) + display
/// name (when known) + email (right). Clicking removes the member.
fn member_pill<'a>(email: &'a str, state: &'a Settings) -> Element<'a, SettingsMessage> {
    let email_for_press = email.to_string();

    // Look up the contact for a display name; fall back to email-only.
    let display_name: Option<&str> = state
        .contacts
        .iter()
        .find(|c| c.email == email)
        .and_then(|c| c.display_name.as_deref());

    let label_col: Element<'a, SettingsMessage> = if let Some(name) = display_name {
        row![
            container(text(name).size(TEXT_LG).style(text::base))
                .align_y(Alignment::Center)
                .width(Length::Fill),
            container(
                text(email)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .align_y(Alignment::Center),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center)
        .into()
    } else {
        container(text(email).size(TEXT_LG).style(text::base))
            .align_y(Alignment::Center)
            .width(Length::Fill)
            .into()
    };

    button(
        container(
            row![
                container(icon::trash().size(ICON_SM).style(text::danger))
                    .align_y(Alignment::Center),
                label_col,
            ]
            .spacing(SPACE_SM)
            .align_y(Alignment::Center),
        )
        .padding(PAD_CARD)
        .width(Length::Fill)
        .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::GroupEditorRemoveMember(email_for_press))
    .padding(0)
    .style(theme::style_pill_card_button)
    .width(Length::Fill)
    .into()
}

fn group_editor_buttons<'a>(
    editor: &'a GroupEditorState,
    confirm_delete: &'a Option<String>,
) -> Element<'a, SettingsMessage> {
    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if let Some(ref id) = editor.group_id {
        let is_confirming = confirm_delete.as_deref() == Some(id.as_str());
        if is_confirming {
            btn_row = btn_row.push(
                button(text("Confirm delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::GroupConfirmDelete(id.clone()))
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Action.style()),
            );
            btn_row = btn_row.push(
                button(text("Cancel").size(TEXT_LG))
                    .on_press(SettingsMessage::GroupCancelDelete)
                    .padding(PAD_BUTTON)
                    .style(theme::ButtonClass::Ghost.style()),
            );
        } else {
            btn_row = btn_row.push(
                button(text("Delete").size(TEXT_LG).style(text::danger))
                    .on_press(SettingsMessage::GroupDelete(id.clone()))
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
        save_btn = save_btn.on_press(SettingsMessage::GroupEditorSave);
    }
    btn_row = btn_row.push(save_btn);

    btn_row.into()
}

// ── Shortcuts tab ────────────────────────────────────────

fn shortcuts_tab<'a>() -> Element<'a, SettingsMessage> {
    let sections: &[(&str, &[(&str, &str)])] = &[
        (
            "Navigation",
            &[
                ("Next thread", "j"),
                ("Previous thread", "k"),
                ("Go to Inbox", "g i"),
                ("Search", "/"),
                ("Close / dismiss", "Esc"),
            ],
        ),
        (
            "Thread",
            &[
                ("Archive", "e"),
                ("Delete", "#"),
                ("Reply", "r"),
                ("Reply All", "a"),
                ("Forward", "f"),
                ("Star / unstar", "s"),
                ("Mute thread", "m"),
                ("Mark as unread", "u"),
            ],
        ),
        ("Composing", &[("New message", "c")]),
    ];

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    for (category, items) in sections {
        let rows: Vec<RowBuilder<'_>> = items
            .iter()
            .map(|(desc, key)| {
                settings_row_container(
                    SETTINGS_ROW_HEIGHT,
                    row![
                        container(text(*desc).size(TEXT_LG).style(text::secondary))
                            .align_y(Alignment::Center)
                            .width(Length::Fill),
                        container(text(*key).size(TEXT_SM).style(text::secondary))
                            .padding(PAD_ICON_BTN)
                            .style(theme::ContainerClass::KeyBadge.style()),
                    ]
                    .align_y(Alignment::Center),
                )
            })
            .collect();

        col = col.push(section(category, rows));
    }

    col.into()
}

// ── AI tab ───────────────────────────────────────────────

fn ai_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Provider",
        vec![setting_row(
            "AI Provider",
            widgets::select(
                &["Claude", "OpenAI", "Gemini", "Ollama", "Copilot"],
                &state.ai_provider,
                state.open_select == Some(SelectField::AiProvider),
                SettingsMessage::ToggleSelect(SelectField::AiProvider),
                SettingsMessage::AiProviderChanged,
            ),
            SettingsMessage::ToggleSelect(SelectField::AiProvider),
        )],
    ));

    if state.ai_provider == "Ollama" {
        col = col.push(section(
            "Local Server",
            vec![
                input_row(
                    "ollama-url",
                    "Server URL",
                    "http://localhost:11434",
                    state.ai_ollama_url.text(),
                    SettingsMessage::OllamaUrlChanged,
                    InputField::OllamaUrl,
                ),
                input_row(
                    "ollama-model",
                    "Model Name",
                    "e.g. llama3.2",
                    state.ai_ollama_model.text(),
                    SettingsMessage::OllamaModelChanged,
                    InputField::OllamaModel,
                ),
            ],
        ));
    } else {
        let key_label = match state.ai_provider.as_str() {
            "OpenAI" => "OpenAI API Key",
            "Gemini" => "Google AI API Key",
            "Copilot" => "GitHub Personal Access Token",
            _ => "Anthropic API Key",
        };

        let model_options: &[&str] = match state.ai_provider.as_str() {
            "OpenAI" => &["gpt-4o", "gpt-4o-mini", "o4-mini"],
            "Gemini" => &[
                "gemini-2.0-flash",
                "gemini-2.5-flash-preview-05-20",
                "gemini-2.5-pro",
            ],
            "Copilot" => &["openai/gpt-4o", "openai/gpt-4o-mini"],
            _ => &[
                "claude-haiku-4-5-20251001",
                "claude-sonnet-4-5",
                "claude-sonnet-4-6",
                "claude-opus-4-6",
            ],
        };

        col = col.push(section(
            "API Key",
            vec![
                input_row_secure(
                    "ai-api-key",
                    key_label,
                    "",
                    state.ai_api_key.text(),
                    SettingsMessage::AiApiKeyChanged,
                    InputField::AiApiKey,
                ),
                setting_row(
                    "Model",
                    widgets::select(
                        model_options,
                        &state.ai_model,
                        state.open_select == Some(SelectField::AiModel),
                        SettingsMessage::ToggleSelect(SelectField::AiModel),
                        SettingsMessage::AiModelChanged,
                    ),
                    SettingsMessage::ToggleSelect(SelectField::AiModel),
                ),
            ],
        ));
    }

    col = col.push(section_with_subtitle(
        "Features",
        "AI-powered tools to help manage your inbox",
        vec![
            toggle_row(
                "Enable AI Features",
                "Use AI-powered features across the app",
                state.ai_enabled,
                SettingsMessage::ToggleAiEnabled,
            ),
            toggle_row(
                "Auto-Categorize",
                "Automatically categorize incoming emails",
                state.ai_auto_categorize,
                SettingsMessage::ToggleAiAutoCategorize,
            ),
            toggle_row(
                "Auto-Summarize",
                "Generate summaries for long email threads",
                state.ai_auto_summarize,
                SettingsMessage::ToggleAiAutoSummarize,
            ),
        ],
    ));

    col = col.push(section(
        "Auto-Draft Replies",
        vec![
            toggle_row(
                "Auto-Draft",
                "Automatically draft replies based on email content",
                state.ai_auto_draft,
                SettingsMessage::ToggleAiAutoDraft,
            ),
            toggle_row(
                "Learn Writing Style",
                "Analyze your sent emails to match your writing style",
                state.ai_writing_style,
                SettingsMessage::ToggleAiWritingStyle,
            ),
        ],
    ));

    col = col.push(section(
        "Auto-Archive Categories",
        vec![
            toggle_row(
                "Updates",
                "Automatically archive update emails",
                state.ai_auto_archive_updates,
                SettingsMessage::ToggleAiAutoArchiveUpdates,
            ),
            toggle_row(
                "Promotions",
                "Automatically archive promotional emails",
                state.ai_auto_archive_promotions,
                SettingsMessage::ToggleAiAutoArchivePromotions,
            ),
            toggle_row(
                "Social",
                "Automatically archive social notification emails",
                state.ai_auto_archive_social,
                SettingsMessage::ToggleAiAutoArchiveSocial,
            ),
            toggle_row(
                "Newsletters",
                "Automatically archive newsletters",
                state.ai_auto_archive_newsletters,
                SettingsMessage::ToggleAiAutoArchiveNewsletters,
            ),
        ],
    ));

    col.into()
}

// ── About tab ───────────────────────────────────────────

fn about_tab<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section(
        "Application",
        vec![
            info_row("Version", "0.1.0-dev"),
            info_row("Iced Version", "0.15.0-dev"),
            info_row("Platform", std::env::consts::OS),
            info_row("Architecture", std::env::consts::ARCH),
        ],
    ));

    col = col.push(section(
        "Updates",
        vec![action_row(
            "Software Updates",
            Some("Check for new versions"),
            None,
            ActionKind::InApp,
            SettingsMessage::CheckForUpdates,
        )],
    ));

    col = col.push(section("License", vec![static_row(
        container(
            column![
                text("Apache License 2.0").size(TEXT_LG).style(text::base),
                Space::new().height(SPACE_XS),
                text("Licensed under the Apache License, Version 2.0. You may obtain a copy of the License at:")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
                Space::new().height(SPACE_XXS),
                text("https://www.apache.org/licenses/LICENSE-2.0")
                    .size(TEXT_SM)
                    .style(text::primary),
                Space::new().height(SPACE_SM),
                text("Copyright 2024-2026 Ratatoskr contributors.")
                    .size(TEXT_SM)
                    .style(theme::TextClass::Tertiary.style()),
            ]
        ).padding(PAD_SETTINGS_ROW),
    )]));

    col = col.push(section(
        "Links",
        vec![action_row(
            "GitHub Repository",
            Some("folknor/ratatoskr"),
            Some(icon::globe()),
            ActionKind::Url,
            SettingsMessage::OpenGithub,
        )],
    ));

    col.into()
}

// ── Accounts tab ────────────────────────────────────────

fn accounts_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

    // Account cards with drag-to-reorder grip handles.
    // Wrapped in a single mouse_area so on_move fires continuously during drag.
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
            .on_move(|point| SettingsMessage::AccountDragMove(point))
            .on_release(SettingsMessage::AccountDragEnd)
            .on_exit(SettingsMessage::AccountDragEnd)
            .into()
    } else {
        card_col.into()
    };

    // Add Account button at the bottom
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

    // Grip handle for drag-to-reorder
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

    // Color indicator
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
        container(icon::chevron_right().size(ICON_XL).style(text::secondary))
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

/// A tiny colored dot indicating account health.
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

// ── Import wizard sheet ────────────────────────────────

fn import_wizard_sheet(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref wizard) = state.import_wizard else {
        return column![].into();
    };

    let content: Element<'_, SettingsMessage> = match wizard.step {
        ImportStep::FileSelect => import_step_file_select(wizard),
        ImportStep::Mapping => import_step_mapping(wizard, &state.managed_accounts),
        ImportStep::VcfPreview => import_step_vcf_preview(wizard, &state.managed_accounts),
        ImportStep::Importing => import_step_importing(),
        ImportStep::Summary => import_step_summary(wizard),
    };

    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);
    col = col.push(
        text("Import Contacts")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    );
    col = col.push(content);
    col.into()
}

fn import_step_file_select(_wizard: &ImportWizardState) -> Element<'_, SettingsMessage> {
    let items = vec![import_file_select_row()];
    section("Select File", items)
}

fn import_file_select_row() -> RowBuilder<'static> {
    let description = "Select a .csv or .vcf file to import contacts from.";
    settings_row_container(
        SETTINGS_TOGGLE_ROW_HEIGHT,
        column![
            text("Choose a CSV or vCard file to import.")
                .size(TEXT_LG)
                .style(text::base),
            Space::new().height(SPACE_XS),
            text(description)
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
            Space::new().height(SPACE_SM),
            text("Use the file browser to select a file. Supported formats: .csv, .vcf")
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        ],
    )
}

fn import_step_mapping<'a>(
    wizard: &'a ImportWizardState,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill);

    // File info
    if let Some(ref path) = wizard.file_path {
        col = col.push(import_file_info_row(path));
    }

    // Header toggle
    col = col.push(import_header_toggle(wizard.has_header));

    // Column mapping table
    if let Some(ref preview) = wizard.preview {
        col = col.push(import_mapping_table(preview, &wizard.mappings));
        col = col.push(import_preview_stats(preview, &wizard.mappings));
    }

    // Account selector
    col = col.push(import_account_selector(wizard, accounts));

    // Update existing toggle
    col = col.push(import_update_toggle(wizard.update_existing));

    // Import button
    col = col.push(import_execute_button());

    col.into()
}

fn import_file_info_row(path: &str) -> Element<'_, SettingsMessage> {
    section_untitled(vec![settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            icon::file().size(ICON_XL).style(text::secondary),
            Space::new().width(SPACE_XS),
            text(path).size(TEXT_LG).style(text::base),
        ]
        .align_y(Alignment::Center),
    )])
}

fn import_header_toggle(has_header: bool) -> Element<'static, SettingsMessage> {
    build_row(toggle_row(
        "First row is a header",
        "Enable if the first row contains column names, not data.",
        has_header,
        SettingsMessage::ImportToggleHeader,
    ))
}

fn import_mapping_table<'a>(
    preview: &'a import::ImportPreview,
    mappings: &'a [ImportContactField],
) -> Element<'a, SettingsMessage> {
    let mut items: Vec<RowBuilder<'a>> = Vec::new();

    // Header row: column names with mapping dropdowns
    for (i, header) in preview.headers.iter().enumerate() {
        let current_field = mappings
            .get(i)
            .copied()
            .unwrap_or(ImportContactField::Ignore);
        items.push(import_column_mapping_row(i, header, current_field));
    }

    // Sample data rows (first 5)
    let sample_count = preview.sample_rows.len().min(5);
    if sample_count > 0 {
        items.push(import_sample_header());
        for row in preview.sample_rows.iter().take(sample_count) {
            items.push(import_sample_row(row));
        }
    }

    section("Column Mapping", items)
}

fn import_column_mapping_row(
    index: usize,
    header: &str,
    current: ImportContactField,
) -> RowBuilder<'_> {
    let header_owned = header.to_string();
    let selected = current.label().to_string();

    Box::new(move |position| {
        button(
            container(
                row![
                    container(text(header_owned).size(TEXT_LG).style(text::base))
                        .align_y(Alignment::Center)
                        .width(Length::FillPortion(1)),
                    container(
                        text(format!("-> {selected}"))
                            .size(TEXT_LG)
                            .style(text::primary),
                    )
                    .align_y(Alignment::Center)
                    .width(Length::FillPortion(1)),
                ]
                .align_y(Alignment::Center),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(SETTINGS_ROW_HEIGHT)
            .align_y(Alignment::Center),
        )
        .on_press(SettingsMessage::ImportMappingChanged(
            index,
            cycle_import_field(current),
        ))
        .padding(0)
        .style(move |theme, status| {
            theme::style_settings_row_button(theme, status, position)
        })
        .width(Length::Fill)
        .into()
    })
}

/// Cycle through import field options on click.
fn cycle_import_field(current: ImportContactField) -> ImportContactField {
    let all = ImportContactField::ALL_OPTIONS;
    let current_idx = all.iter().position(|&f| f == current).unwrap_or(0);
    let next_idx = (current_idx + 1) % all.len();
    all[next_idx]
}

fn import_sample_header<'a>() -> RowBuilder<'a> {
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text("Preview (first rows)")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style())
            .font(iced::Font {
                weight: iced::font::Weight::Bold,
                ..crate::font::text()
            }),
    )
}

fn import_sample_row(row: &[String]) -> RowBuilder<'_> {
    let display = row.join("  |  ");
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(display).size(TEXT_SM).style(text::secondary),
    )
}

fn import_preview_stats<'a>(
    preview: &'a import::ImportPreview,
    mappings: &'a [ImportContactField],
) -> Element<'a, SettingsMessage> {
    let has_email = mappings.iter().any(|m| *m == ImportContactField::Email);
    let status = if has_email {
        format!("{} rows to import.", preview.total_rows)
    } else {
        "No Email column mapped. Map at least one column to Email.".to_string()
    };
    build_row(settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(status)
            .size(TEXT_LG)
            .style(if has_email { text::base } else { text::danger }),
    ))
}

fn import_account_selector<'a>(
    wizard: &'a ImportWizardState,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let selected_id = wizard.account_id.as_deref();

    let mut btn_row = row![].spacing(SPACE_XS).align_y(Alignment::Center);

    // "Local" option
    let is_local = selected_id.is_none();
    let local_style = if is_local {
        theme::ButtonClass::Primary
    } else {
        theme::ButtonClass::Ghost
    };
    btn_row = btn_row.push(
        button(text("Local").size(TEXT_SM))
            .style(local_style.style())
            .on_press(SettingsMessage::ImportAccountChanged(None))
            .padding(PAD_ICON_BTN),
    );

    for account in accounts {
        let is_selected = selected_id == Some(account.id.as_str());
        let style = if is_selected {
            theme::ButtonClass::Primary
        } else {
            theme::ButtonClass::Ghost
        };
        let aid = Some(account.id.clone());
        btn_row = btn_row.push(
            button(text(&account.email).size(TEXT_SM))
                .style(style.style())
                .on_press(SettingsMessage::ImportAccountChanged(aid))
                .padding(PAD_ICON_BTN),
        );
    }

    container(column![
        text("Import to account")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style()),
        Space::new().height(SPACE_XXXS),
        btn_row,
    ])
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn import_update_toggle(update_existing: bool) -> Element<'static, SettingsMessage> {
    build_row(toggle_row(
        "Update existing contacts",
        "When a duplicate email is found, update the existing contact with imported data.",
        update_existing,
        SettingsMessage::ImportToggleUpdateExisting,
    ))
}

fn import_execute_button() -> Element<'static, SettingsMessage> {
    container(
        button(container(text("Import").size(TEXT_LG)).center_x(Length::Fill))
            .on_press(SettingsMessage::ImportExecute)
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Primary.style())
            .width(Length::Fixed(EDITOR_BUTTON_WIDTH)),
    )
    .width(Length::Fill)
    .align_x(Alignment::End)
    .padding(PAD_SETTINGS_ROW)
    .into()
}

fn import_step_vcf_preview<'a>(
    wizard: &'a ImportWizardState,
    accounts: &'a [ManagedAccount],
) -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill);

    // File info
    if let Some(ref path) = wizard.file_path {
        col = col.push(import_file_info_row(path));
    }

    // Contact list preview
    let valid_count = wizard
        .vcf_contacts
        .iter()
        .filter(|c| c.has_valid_email())
        .count();
    let total = wizard.vcf_contacts.len();
    let skipped = total - valid_count;

    let stat_text =
        format!("{total} contacts found. {valid_count} with valid email, {skipped} without.",);
    col = col.push(build_row(settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(stat_text).size(TEXT_LG).style(text::base),
    )));

    // Preview first 10 contacts
    let mut preview_items: Vec<RowBuilder<'a>> = Vec::new();
    for contact in wizard.vcf_contacts.iter().take(10) {
        preview_items.push(import_vcf_contact_row(contact));
    }
    if !preview_items.is_empty() {
        col = col.push(section("Preview", preview_items));
    }

    // Account selector
    col = col.push(import_account_selector(wizard, accounts));

    // Update existing toggle
    col = col.push(import_update_toggle(wizard.update_existing));

    // Import button
    col = col.push(import_execute_button());

    col.into()
}

fn import_vcf_contact_row(contact: &import::ImportedContact) -> RowBuilder<'_> {
    let name = contact
        .effective_display_name()
        .unwrap_or_else(|| "(no name)".to_string());
    let email = contact
        .normalized_email()
        .unwrap_or_else(|| "(no email)".to_string());

    let email_style: fn(&iced::Theme) -> text::Style = if contact.has_valid_email() {
        text::secondary
    } else {
        text::danger
    };

    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            text(name)
                .size(TEXT_LG)
                .style(text::base)
                .width(Length::FillPortion(1)),
            text(email)
                .size(TEXT_SM)
                .style(email_style)
                .width(Length::FillPortion(1)),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
}

fn import_step_importing() -> Element<'static, SettingsMessage> {
    build_row(settings_row_container(
        SETTINGS_TOGGLE_ROW_HEIGHT,
        column![
            text("Importing contacts...")
                .size(TEXT_LG)
                .style(text::base),
            Space::new().height(SPACE_XS),
            text("Please wait.")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        ],
    ))
}

fn import_step_summary(wizard: &ImportWizardState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill);

    if let Some(ref result) = wizard.result {
        let mut stats: Vec<RowBuilder<'_>> = Vec::new();
        stats.push(import_stat_row("Imported", result.imported));
        if result.updated > 0 {
            stats.push(import_stat_row("Updated", result.updated));
        }
        if result.skipped_no_email > 0 {
            stats.push(import_stat_row(
                "Skipped (no email)",
                result.skipped_no_email,
            ));
        }
        if result.skipped_duplicate > 0 {
            stats.push(import_stat_row(
                "Skipped (duplicate)",
                result.skipped_duplicate,
            ));
        }
        if result.groups_created > 0 {
            stats.push(import_stat_row("Groups created", result.groups_created));
        }
        col = col.push(section("Import Complete", stats));
    }

    col = col.push(
        container(
            button(container(text("Done").size(TEXT_LG)).center_x(Length::Fill))
                .on_press(SettingsMessage::ImportBack)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Primary.style())
                .width(Length::Fixed(EDITOR_BUTTON_WIDTH)),
        )
        .width(Length::Fill)
        .align_x(Alignment::End)
        .padding(PAD_SETTINGS_ROW),
    );

    col.into()
}

fn import_stat_row(label: &str, count: usize) -> RowBuilder<'_> {
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            text(label)
                .size(TEXT_LG)
                .style(text::base)
                .width(Length::Fill),
            text(count.to_string()).size(TEXT_LG).style(text::primary),
        ]
        .align_y(Alignment::Center),
    )
}
