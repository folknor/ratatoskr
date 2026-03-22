use iced::time::Instant;
use iced::widget::{button, column, container, mouse_area, row, scrollable, text, text_input, Space};
use iced::{Alignment, Element, Length};

use crate::icon;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::undoable_text_input::undoable_text_input;
use crate::ui::widgets;

use ratatoskr_rich_text_editor::{
    rich_text_editor, Action as RteAction, EditAction, InlineStyle,
    BlockKind,
};

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
    let overlay_t: f32 = state.overlay_anim.interpolate(0.0, 1.0, now);
    let show_overlay = state.overlay.is_some() || overlay_t > 0.001;

    let content_area = container(
        scrollable(
            container(content)
                .padding(PAD_SETTINGS_CONTENT)
                .align_x(Alignment::Center)
        ).spacing(SCROLLBAR_SPACING).height(Length::Fill)
    )
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style());

    let main_content: Element<'_, SettingsMessage> = if show_overlay {
        let overlay_content = match state.overlay {
            Some(SettingsOverlay::CreateFilter) => create_filter_overlay(),
            Some(SettingsOverlay::AccountEditor) => account_editor_overlay(state),
            Some(SettingsOverlay::EditSignature { .. }) => signature_editor_overlay(state),
            Some(SettingsOverlay::EditContact { .. }) => contact_editor_overlay(state),
            Some(SettingsOverlay::EditGroup { .. }) => group_editor_overlay(state),
            Some(SettingsOverlay::ImportContacts) => import_wizard_overlay(state),
            None => column![].into(), // closing animation
        };

        // Overlay panel: back button header + scrollable content
        let overlay_panel = container(
            column![
                container(
                    button(
                        row![
                            container(icon::arrow_left().size(ICON_XL).style(text::base))
                                .align_y(Alignment::Center),
                            text("Back").size(TEXT_LG).style(text::base)
                                .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
                        ]
                        .spacing(SPACE_XS)
                        .align_y(Alignment::Center),
                    )
                    .on_press(SettingsMessage::CloseOverlay)
                    .padding(PAD_NAV_ITEM)
                    .style(theme::ButtonClass::BareIcon.style()),
                )
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
                scrollable(
                    container(overlay_content)
                        .padding(PAD_SETTINGS_CONTENT)
                        .align_x(Alignment::Center)
                ).spacing(SCROLLBAR_SPACING).height(Length::Fill),
            ]
        )
        .width(Length::Fill)
        .height(Length::Fill)
        .style(theme::ContainerClass::Content.style());

        // Slide from right: use a large fixed offset (2000px) scaled by (1-t).
        // The stack clips to bounds so overshooting doesn't matter.
        let offset = ((1.0 - overlay_t) * 2000.0).round();

        // Event blocker: opaque mouse_area between content and overlay
        // prevents clicks/hovers from reaching the content underneath.
        let blocker = mouse_area(
            container(Space::new().width(Length::Fill).height(Length::Fill))
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(SettingsMessage::CloseOverlay)
        .interaction(iced::mouse::Interaction::default());

        iced::widget::stack![
            content_area,
            blocker,
            container(overlay_panel)
                .width(Length::Fill)
                .height(Length::Fill)
                .padding(iced::Padding { top: 0.0, right: 0.0, bottom: 0.0, left: offset }),
        ]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
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

    container(scrollable(col).spacing(SCROLLBAR_SPACING).height(Length::Fill))
        .padding(PAD_SIDEBAR)
        .height(Length::Fill)
        .style(theme::ContainerClass::Sidebar.style())
        .into()
}

// ── General tab ─────────────────────────────────────────

fn general_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Appearance", vec![
        setting_row("Theme", widgets::select(
            &["System", "Light", "Dark", "Theme"], &state.theme,
            state.open_select == Some(SelectField::Theme),
            SettingsMessage::ToggleSelect(SelectField::Theme),
            SettingsMessage::ThemeChanged,
        ), SettingsMessage::ToggleSelect(SelectField::Theme)),
        setting_row("Email Density", widgets::select(
            &["Compact", "Default", "Spacious"], &state.density,
            state.open_select == Some(SelectField::Density),
            SettingsMessage::ToggleSelect(SelectField::Density),
            SettingsMessage::DensityChanged,
        ), SettingsMessage::ToggleSelect(SelectField::Density)),
        setting_row("Font Size", widgets::select(
            &["Small", "Default", "Large", "XLarge"], &state.font_size,
            state.open_select == Some(SelectField::FontSize),
            SettingsMessage::ToggleSelect(SelectField::FontSize),
            SettingsMessage::FontSizeChanged,
        ), SettingsMessage::ToggleSelect(SelectField::FontSize)),
        slider_row("Scale", None, 1.0..=4.0, state.scale_preview.unwrap_or(state.scale), 1.0, 0.125, SettingsMessage::ScaleDragged, Some(SettingsMessage::ScaleReleased)),
        setting_row("Message Dates", widgets::select(
            &["Relative Offset", "Absolute"],
            match state.date_display {
                crate::db::DateDisplay::RelativeOffset => "Relative Offset",
                crate::db::DateDisplay::Absolute => "Absolute",
            },
            state.open_select == Some(SelectField::DateDisplay),
            SettingsMessage::ToggleSelect(SelectField::DateDisplay),
            SettingsMessage::DateDisplayChanged,
        ), SettingsMessage::ToggleSelect(SelectField::DateDisplay)),
        toggle_row("Show Sync Status Bar", "Display sync progress in the status bar", state.sync_status_bar, SettingsMessage::ToggleSyncStatusBar),
    ]));

    col = col.push(section("Reading Pane", radio_group(
        &[("Right", "Right"), ("Bottom", "Bottom"), ("Hidden", "Hidden")],
        Some(state.reading_pane_position.as_str()),
        |v| SettingsMessage::ReadingPaneChanged(v.to_string()),
    )));

    let privacy_help_id = "privacy-security";
    let privacy_help_visible = state.hovered_help.as_deref() == Some(privacy_help_id)
        || state.pinned_help.as_deref() == Some(privacy_help_id);
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
        pinned: state.pinned_help.as_deref() == Some(privacy_help_id),
    }, vec![
        toggle_row("Block Remote Images", "Don't load remote images in email bodies", state.block_remote_images, SettingsMessage::ToggleBlockRemoteImages),
        toggle_row("Phishing Detection", "Warn about suspicious emails", state.phishing_detection, SettingsMessage::TogglePhishingDetection),
    ]));

    col.into()
}

// ── Theme tab ───────────────────────────────────────────

fn theme_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

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
            container(
                text(entry.name).size(TEXT_SM).style(if selected { text::base } else { text::secondary }),
            )
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
            current_row = current_row.push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1)));
            col_count += 1;
        }
        grid = grid.push(current_row);
    }

    col = col.push(section("Themes", vec![
        container(grid).padding(PAD_SETTINGS_ROW).width(Length::Fill).into(),
    ]));

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
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Experiment { variant: *idx }.style()).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Primary.style()).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS);

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
            current_row = current_row.push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1)));
            col_count += 1;
        }
        grid = grid.push(current_row);
    }

    col = col.push(section("Button Experiments (section bg)", vec![
        container(grid).padding(PAD_SETTINGS_ROW).width(Length::Fill).into(),
    ]));

    // Same grid on content/main area background
    let mut grid2 = column![].spacing(SPACE_XS);
    let mut current_row2 = row![].spacing(SPACE_XS);
    let mut col_count2 = 0;
    for (label, idx) in &experiments {
        let btn_width = Length::Fixed(120.0);
        let pair = row![
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Experiment { variant: *idx }.style()).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Primary.style()).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS);
        current_row2 = current_row2.push(container(pair).width(Length::FillPortion(1)));
        col_count2 += 1;
        if col_count2 == 2 {
            grid2 = grid2.push(current_row2);
            current_row2 = row![].spacing(SPACE_XS);
            col_count2 = 0;
        }
    }
    if col_count2 > 0 {
        while col_count2 < 2 { current_row2 = current_row2.push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1))); col_count2 += 1; }
        grid2 = grid2.push(current_row2);
    }

    let content_bg_box = container(
        column![
            text("Content / main area background").size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
            grid2,
        ].spacing(SPACE_SM),
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
            button(container(text(*label).size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Experiment { variant: *idx }.style()).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Primary.style()).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS);
        current_row3 = current_row3.push(container(pair).width(Length::FillPortion(1)));
        col_count3 += 1;
        if col_count3 == 2 {
            grid3 = grid3.push(current_row3);
            current_row3 = row![].spacing(SPACE_XS);
            col_count3 = 0;
        }
    }
    if col_count3 > 0 {
        while col_count3 < 2 { current_row3 = current_row3.push(container(Space::new().width(0).height(0)).width(Length::FillPortion(1))); col_count3 += 1; }
        grid3 = grid3.push(current_row3);
    }

    let sidebar_bg_box = container(
        column![
            text("Sidebar background").size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
            grid3,
        ].spacing(SPACE_SM),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .style(theme::ContainerClass::Sidebar.style());

    col = col.push(sidebar_bg_box);

    // Semantic color pairs
    let btn_width = Length::Fixed(120.0);
    let semantic_grid = column![
        row![
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Primary.style()).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Primary.style()).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS),
        row![
            button(container(text("Success").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::ExperimentSemantic { variant: 0 }.style()).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Primary.style()).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS),
        row![
            button(container(text("Warning").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::ExperimentSemantic { variant: 1 }.style()).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Primary.style()).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS),
        row![
            button(container(text("Danger").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::ExperimentSemantic { variant: 2 }.style()).padding(PAD_BUTTON).width(btn_width),
            button(container(text("Primary").size(TEXT_MD)).center_x(Length::Fill)).on_press(SettingsMessage::Noop).style(theme::ButtonClass::Primary.style()).padding(PAD_BUTTON).width(btn_width),
        ].spacing(SPACE_XXS),
    ].spacing(SPACE_XS);

    col = col.push(section("Semantic Color Pairs", vec![
        container(semantic_grid).padding(PAD_SETTINGS_ROW).width(Length::Fill).into(),
    ]));

    col.into()
}

// ── Composing tab ────────────────────────────────────────

fn composing_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Sending", vec![
        setting_row("Undo Send Delay", widgets::select(
            &["None", "5 seconds", "10 seconds", "30 seconds"], &state.undo_delay,
            state.open_select == Some(SelectField::UndoDelay),
            SettingsMessage::ToggleSelect(SelectField::UndoDelay),
            SettingsMessage::UndoDelayChanged,
        ), SettingsMessage::ToggleSelect(SelectField::UndoDelay)),
        toggle_row("Send & Archive", "Archive a thread immediately after sending a reply",
            state.send_and_archive, SettingsMessage::ToggleSendAndArchive),
    ]));

    col = col.push(section("Behavior", vec![
        setting_row("Default Reply Action", widgets::select(
            &["Reply", "Reply All"], &state.default_reply_mode,
            state.open_select == Some(SelectField::DefaultReply),
            SettingsMessage::ToggleSelect(SelectField::DefaultReply),
            SettingsMessage::DefaultReplyChanged,
        ), SettingsMessage::ToggleSelect(SelectField::DefaultReply)),
        setting_row("Mark as Read", widgets::select(
            &["Instantly", "After 2 Seconds", "Manually"], &state.mark_as_read,
            state.open_select == Some(SelectField::MarkAsRead),
            SettingsMessage::ToggleSelect(SelectField::MarkAsRead),
            SettingsMessage::MarkAsReadChanged,
        ), SettingsMessage::ToggleSelect(SelectField::MarkAsRead)),
    ]));

    col = col.push(signature_list_section(state));

    col = col.push(section("Templates", vec![
        coming_soon_row("Template management"),
    ]));

    col.into()
}

// ── Notifications tab ────────────────────────────────────

fn notifications_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Notifications", vec![
        toggle_row("Enable Notifications", "Receive desktop notifications for new email",
            state.notifications_enabled, SettingsMessage::ToggleNotifications),
        toggle_row("Smart Notifications", "Only notify about important emails",
            state.smart_notifications, SettingsMessage::ToggleSmartNotifications),
    ]));

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
        let chips_row = iced::widget::row(chips).spacing(SPACE_XS).align_y(Alignment::Center);
        col = col.push(section("Notify for Categories", vec![
            settings_row_container(SETTINGS_ROW_HEIGHT, chips_row),
        ]));

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

        col = col.push(section("VIP Senders", vec![vip_col.into()]));
    }

    col.into()
}

// ── Mail Rules tab ───────────────────────────────────────

fn mail_rules_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Labels", vec![
        editable_list("labels", &state.demo_labels, "Add Label", &state.drag_state),
    ]));
    col = col.push(section("Filters", vec![
        action_row("Create Filter", Some("Add a new mail filter rule"), Some(icon::filter()), ActionKind::InApp, SettingsMessage::OpenOverlay(SettingsOverlay::CreateFilter)),
    ]));
    if !state.demo_filters.is_empty() {
        col = col.push(section_untitled(vec![
            editable_list("filters", &state.demo_filters, "Add Filter", &state.drag_state),
        ]));
    }
    col = col.push(section("Smart Labels", vec![coming_soon_row("Smart label management")]));
    col = col.push(section("Smart Folders", vec![coming_soon_row("Smart folder management")]));
    col = col.push(section("Quick Steps", vec![coming_soon_row("Quick step management")]));

    col.into()
}

// ── Overlays ─────────────────────────────────────────────

fn create_filter_overlay<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text("Create Filter")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
    );

    col = col.push(section("Conditions", vec![
        coming_soon_row("Match conditions"),
    ]));

    col = col.push(section("Actions", vec![
        coming_soon_row("Filter actions"),
    ]));

    col.into()
}

// ── Account editor overlay ───────────────────────────────

fn account_editor_overlay(state: &Settings) -> Element<'_, SettingsMessage> {
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
        vec![container(
            column![
                text("Display Name").size(TEXT_SM).style(text::secondary),
                text_input("Your Name", &editor.display_name)
                    .on_input(SettingsMessage::DisplayNameEditorChanged)
                    .size(TEXT_LG)
                    .padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style()),
            ]
            .spacing(SPACE_XXXS)
            .width(Length::Fill),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .into()],
    ));

    // Account color
    col = col.push(account_editor_color_section(state, editor));

    // CalDAV settings
    col = col.push(account_editor_caldav_section(editor));

    // Re-authenticate action
    col = col.push(section("Authentication", vec![
        action_row(
            "Re-authenticate",
            Some("Sign in again to refresh credentials"),
            None,
            ActionKind::InApp,
            SettingsMessage::ReauthenticateAccount(editor.account_id.clone()),
        ),
    ]));

    // Delete section
    col = col.push(account_editor_delete_section(editor));

    // Save button
    if editor.dirty {
        col = col.push(
            button(
                container(
                    text("Save Changes")
                        .size(TEXT_LG)
                        .color(theme::ON_AVATAR),
                )
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

fn account_editor_name_section(
    editor: &AccountEditor,
) -> Element<'_, SettingsMessage> {
    section(
        "Account Name",
        vec![container(
            column![
                text("Account Name").size(TEXT_SM).style(text::secondary),
                text_input("e.g. Work", &editor.account_name)
                    .on_input(SettingsMessage::AccountNameEditorChanged)
                    .size(TEXT_LG)
                    .padding(PAD_INPUT)
                    .style(theme::TextInputClass::Settings.style()),
            ]
            .spacing(SPACE_XXXS)
            .width(Length::Fill),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .into()],
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
        vec![container(grid)
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .into()],
    )
}

fn account_editor_caldav_section(
    editor: &AccountEditor,
) -> Element<'_, SettingsMessage> {
    let fields = column![
        column![
            text("CalDAV URL").size(TEXT_SM).style(text::secondary),
            text_input("https://", &editor.caldav_url)
                .on_input(SettingsMessage::CaldavUrlChanged)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
        column![
            text("Username").size(TEXT_SM).style(text::secondary),
            text_input("", &editor.caldav_username)
                .on_input(SettingsMessage::CaldavUsernameChanged)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
        column![
            text("Password").size(TEXT_SM).style(text::secondary),
            text_input("", &editor.caldav_password)
                .on_input(SettingsMessage::CaldavPasswordChanged)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style()),
        ]
        .spacing(SPACE_XXXS)
        .width(Length::Fill),
    ]
    .spacing(SPACE_SM);

    section(
        "Calendar (CalDAV)",
        vec![container(fields)
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .into()],
    )
}

fn account_editor_delete_section(
    editor: &AccountEditor,
) -> Element<'_, SettingsMessage> {
    if editor.show_delete_confirmation {
        section(
            "Danger Zone",
            vec![container(
                column![
                    text("Are you sure you want to delete this account?")
                        .size(TEXT_LG)
                        .style(text::danger),
                    text("All data for this account will be permanently removed.")
                        .size(TEXT_SM)
                        .style(text::secondary),
                    Space::new().height(SPACE_SM),
                    row![
                        button(
                            text("Delete Account")
                                .size(TEXT_LG)
                                .style(text::danger),
                        )
                        .on_press(SettingsMessage::DeleteAccountConfirmed(
                            editor.account_id.clone(),
                        ))
                        .padding(PAD_BUTTON)
                        .style(
                            theme::ButtonClass::ExperimentSemantic { variant: 2 }
                                .style(),
                        ),
                        Space::new().width(SPACE_SM),
                        button(
                            text("Cancel").size(TEXT_LG).style(text::secondary),
                        )
                        .on_press(SettingsMessage::DeleteAccountCancelled)
                        .padding(PAD_BUTTON)
                        .style(theme::ButtonClass::Ghost.style()),
                    ],
                ]
                .spacing(SPACE_XS),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .into()],
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
        return section("Signatures", vec![
            coming_soon_row("No accounts configured"),
        ]);
    }

    // Group signatures by account_id
    let mut items: Vec<Element<'_, SettingsMessage>> = Vec::new();

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
            text(account_name).size(TEXT_SM).style(text::secondary)
                .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
        );

        items.push(
            container(header_row)
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill)
                .into(),
        );

        // Signature rows for this account (with global indices for drag)
        for sig in &account_sigs {
            let global_idx = state.signatures.iter().position(|s| s.id == sig.id).unwrap_or(0);
            items.push(signature_row(sig, global_idx));
        }

        // Add Signature button for this account
        let aid = account.id.clone();
        items.push(
            button(
                container(
                    row![
                        icon::plus().size(ICON_MD).style(text::base),
                        text("Add Signature").size(TEXT_LG).style(text::base)
                            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
                    ]
                    .spacing(SPACE_XS)
                    .align_y(Alignment::Center),
                )
                .center_x(Length::Fill)
                .align_y(Alignment::Center),
            )
            .on_press(SettingsMessage::SignatureCreate(aid))
            .padding(PAD_SETTINGS_ROW)
            .style(theme::ButtonClass::Action.style())
            .width(Length::Fill)
            .height(SETTINGS_ROW_HEIGHT)
            .into(),
        );
    }

    let sig_section = section("Signatures", items);

    // Wrap in mouse_area for drag-move tracking.
    mouse_area(sig_section)
        .on_move(SettingsMessage::SignatureDragMove)
        .on_release(SettingsMessage::SignatureDragEnd)
        .into()
}

fn signature_row<'a>(sig: &'a SignatureEntry, global_index: usize) -> Element<'a, SettingsMessage> {
    let sig_id = sig.id.clone();

    let mut label_parts = column![
        text(&sig.name).size(TEXT_LG).style(text::base),
    ]
    .spacing(SPACE_XXXS);

    // Show a preview snippet of the body (plain text, first 60 chars)
    let preview = sig.body_text.as_deref().unwrap_or(&sig.body_html);
    let snippet: String = preview.chars().take(60).collect();
    if !snippet.is_empty() {
        label_parts = label_parts.push(
            text(snippet).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
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
        container(label_parts).align_y(Alignment::Center).width(Length::Fill),
    );

    // Default / Reply default badges
    if sig.is_default {
        content = content.push(
            container(
                text("Default").size(TEXT_XS).style(text::secondary),
            )
            .padding(PAD_BADGE)
            .style(theme::ContainerClass::KeyBadge.style()),
        );
    }
    if sig.is_reply_default {
        content = content.push(
            container(
                text("Reply default").size(TEXT_XS).style(text::secondary),
            )
            .padding(PAD_BADGE)
            .style(theme::ContainerClass::KeyBadge.style()),
        );
    }

    // Remove button — opens editor overlay with delete confirmation
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
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .into()
}

// ── Signature editor overlay ────────────────────────────

fn signature_editor_overlay(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.signature_editor else {
        return column![].into();
    };

    let is_new = editor.signature_id.is_none();
    let title = if is_new { "New Signature" } else { "Edit Signature" };

    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text(title)
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
    );

    // Name field
    col = col.push(section("Name", vec![
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
        .width(Length::Fill)
        .into(),
    ]));

    // Default checkboxes
    col = col.push(section("Defaults", vec![
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
    ]));

    // Formatting toolbar + rich text editor
    col = col.push(section("Content", vec![
        container(
            column![
                text("Signature body").size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
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
            ]
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .into(),
    ]));

    // Action buttons
    let mut btn_row = row![].spacing(SPACE_SM).align_y(Alignment::Center);

    if !is_new {
        let del_id = editor.signature_id.clone().unwrap_or_default();
        let is_confirming = state
            .confirm_delete_signature
            .as_deref() == Some(del_id.as_str());

        if is_confirming {
            btn_row = btn_row.push(
                text("Delete this signature?").size(TEXT_LG).style(text::danger),
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
    let mut save_btn = button(
        container(text("Save").size(TEXT_LG)).center_x(Length::Fill),
    )
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
        .on_press(SettingsMessage::SignatureEditorAction(
            RteAction::Edit(EditAction::ToggleInlineStyle(style_bit)),
        ))
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
        .on_press(SettingsMessage::SignatureEditorAction(
            RteAction::Edit(EditAction::SetBlockType(block_kind)),
        ))
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
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    // ── Contacts section ──
    let mut contact_items: Vec<Element<'_, SettingsMessage>> = Vec::new();

    // Filter input
    contact_items.push(contact_filter_row(&state.contact_filter));

    // Contact cards
    for contact in &state.contacts {
        contact_items.push(contact_card(contact));
    }

    // New Contact button
    contact_items.push(people_add_button("New Contact", SettingsMessage::ContactCreate));

    col = col.push(section("Contacts", contact_items));

    // ── Import section ──
    col = col.push(section("Import", vec![
        action_row(
            "Import Contacts",
            Some("Import from CSV or vCard file"),
            Some(icon::upload()),
            ActionKind::InApp,
            SettingsMessage::ImportContactsOpen,
        ),
    ]));

    // ── Groups section ──
    let mut group_items: Vec<Element<'_, SettingsMessage>> = Vec::new();

    // Filter input
    group_items.push(group_filter_row(&state.group_filter));

    // Group cards
    for group in &state.groups {
        group_items.push(group_card(group));
    }

    // New Group button
    group_items.push(people_add_button("New Group", SettingsMessage::GroupCreate));

    col = col.push(section("Groups", group_items));

    col.into()
}

fn contact_filter_row(filter: &str) -> Element<'_, SettingsMessage> {
    let filter_owned = filter.to_string();
    container(
        text_input("Filter contacts...", &filter_owned)
            .on_input(SettingsMessage::ContactFilterChanged)
            .size(TEXT_LG)
            .padding(PAD_INPUT)
            .style(theme::TextInputClass::Settings.style())
            .width(Length::Fill),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn group_filter_row(filter: &str) -> Element<'_, SettingsMessage> {
    let filter_owned = filter.to_string();
    container(
        text_input("Filter groups...", &filter_owned)
            .on_input(SettingsMessage::GroupFilterChanged)
            .size(TEXT_LG)
            .padding(PAD_INPUT)
            .style(theme::TextInputClass::Settings.style())
            .width(Length::Fill),
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn contact_card(contact: &crate::db::ContactEntry) -> Element<'_, SettingsMessage> {
    let name = contact.display_name.as_deref().unwrap_or("(no name)");
    let id = contact.id.clone();

    let mut left_col = column![].spacing(SPACE_XXXS);
    left_col = left_col.push(text(name).size(TEXT_LG).style(text::base));

    if let Some(ref phone) = contact.phone {
        left_col = left_col.push(
            text(format!("Phone: {phone}")).size(TEXT_SM).style(text::secondary),
        );
    }
    if let Some(ref company) = contact.company {
        left_col = left_col.push(
            text(format!("Company: {company}")).size(TEXT_SM).style(text::secondary),
        );
    }
    if let Some(ref notes) = contact.notes {
        left_col = left_col.push(
            text(format!("Notes: {notes}")).size(TEXT_SM).style(text::secondary),
        );
    }

    // Group pills
    if !contact.groups.is_empty() {
        let group_text = contact.groups.join(", ");
        left_col = left_col.push(
            text(format!("Groups: {group_text}")).size(TEXT_XS).style(text::primary),
        );
    }

    let mut right_col = column![].spacing(SPACE_XXXS).align_x(Alignment::End);
    right_col = right_col.push(text(&contact.email).size(TEXT_SM).style(text::secondary));
    if let Some(ref email2) = contact.email2 {
        right_col = right_col.push(text(email2).size(TEXT_SM).style(text::secondary));
    }

    let content = row![
        left_col.width(Length::Fill),
        right_col,
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    button(
        container(content)
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::ContactClick(id))
    .padding(0)
    .style(theme::ButtonClass::Action.style())
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
    let updated_label = chrono::DateTime::from_timestamp(group.updated_at, 0)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%b %d, %Y").to_string())
        .unwrap_or_default();

    let content = row![
        column![
            text(&group.name).size(TEXT_LG).style(text::base),
        ]
        .width(Length::Fill),
        column![
            text(member_label).size(TEXT_SM).style(text::secondary),
            text(updated_label).size(TEXT_SM).style(theme::TextClass::Muted.style()),
        ]
        .align_x(Alignment::End)
        .spacing(SPACE_XXXS),
    ]
    .spacing(SPACE_SM)
    .align_y(Alignment::Center);

    button(
        container(content)
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .height(SETTINGS_ROW_HEIGHT)
            .align_y(Alignment::Center),
    )
    .on_press(SettingsMessage::GroupClick(id))
    .padding(0)
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .into()
}

fn people_add_button(label: &str, on_press: SettingsMessage) -> Element<'_, SettingsMessage> {
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
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .height(SETTINGS_ROW_HEIGHT)
    .into()
}

// ── Contact editor overlay ──────────────────────────────

fn contact_editor_overlay(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.contact_editor else {
        return column![].into();
    };

    let is_new = editor.contact_id.is_none();
    let title = if is_new { "New Contact" } else { "Edit Contact" };

    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text(title)
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
    );

    // Account selector
    col = col.push(contact_account_selector(editor, &state.managed_accounts));

    col = col.push(contact_editor_fields(editor));
    col = col.push(contact_editor_buttons(editor, &state.confirm_delete_contact));

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

    container(
        column![
            text("Account").size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
            Space::new().height(SPACE_XXXS),
            btn_row,
        ],
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn contact_editor_fields(editor: &ContactEditorState) -> Element<'_, SettingsMessage> {
    let fields = vec![
        contact_field_input("Display name", "Name", &editor.display_name, ContactField::DisplayName),
        contact_field_input("Email", "email@example.com", &editor.email, ContactField::Email),
        contact_field_input("Email 2", "Optional second email", &editor.email2, ContactField::Email2),
        contact_field_input("Phone", "Optional phone number", &editor.phone, ContactField::Phone),
        contact_field_input("Company", "Optional company", &editor.company, ContactField::Company),
        contact_field_input("Notes", "Optional notes", &editor.notes, ContactField::Notes),
    ];
    section("Details", fields)
}

fn contact_field_input<'a>(
    label: &'a str,
    placeholder: &'a str,
    value: &'a str,
    field: ContactField,
) -> Element<'a, SettingsMessage> {
    container(
        column![
            text(label).size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
            Space::new().height(SPACE_XXXS),
            text_input(placeholder, value)
                .on_input(move |v| SettingsMessage::ContactEditorFieldChanged(field.clone(), v))
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style())
                .width(Length::Fill),
        ],
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
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

    let can_save = !editor.email.trim().is_empty();
    let mut save_btn = button(
        container(text("Save").size(TEXT_LG)).center_x(Length::Fill),
    )
    .padding(PAD_BUTTON)
    .style(theme::ButtonClass::Primary.style())
    .width(Length::Fixed(EDITOR_BUTTON_WIDTH));
    if can_save {
        save_btn = save_btn.on_press(SettingsMessage::ContactEditorSave);
    }
    btn_row = btn_row.push(save_btn);

    btn_row.into()
}

// ── Group editor overlay ────────────────────────────────

fn group_editor_overlay(state: &Settings) -> Element<'_, SettingsMessage> {
    let Some(ref editor) = state.group_editor else {
        return column![].into();
    };

    let is_new = editor.group_id.is_none();
    let title = if is_new { "New Group" } else { "Edit Group" };

    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(
        text(title)
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
    );

    // Group name
    col = col.push(section("Name", vec![
        container(
            text_input("Group name", &editor.name)
                .on_input(SettingsMessage::GroupEditorNameChanged)
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style())
                .width(Length::Fill),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .into(),
    ]));

    // Members section
    col = col.push(group_member_section(editor, state));

    // Action buttons
    col = col.push(group_editor_buttons(editor, &state.confirm_delete_group));

    col.into()
}

fn group_member_section<'a>(
    editor: &'a GroupEditorState,
    state: &'a Settings,
) -> Element<'a, SettingsMessage> {
    let mut member_items: Vec<Element<'a, SettingsMessage>> = Vec::new();

    // Add member filter
    member_items.push(
        container(
            text_input("Add member by email...", &editor.filter)
                .on_input(SettingsMessage::GroupEditorFilterChanged)
                .on_submit(SettingsMessage::GroupEditorAddMember(editor.filter.clone()))
                .size(TEXT_LG)
                .padding(PAD_INPUT)
                .style(theme::TextInputClass::Settings.style())
                .width(Length::Fill),
        )
        .padding(PAD_SETTINGS_ROW)
        .width(Length::Fill)
        .into(),
    );

    // Show matching contacts (not already members)
    let filter_lower = editor.filter.to_lowercase();
    if !filter_lower.is_empty() {
        for contact in &state.contacts {
            let dominated = contact.email.to_lowercase().contains(&filter_lower)
                || contact.display_name.as_deref().unwrap_or("").to_lowercase().contains(&filter_lower);
            if dominated && !editor.members.contains(&contact.email) {
                let email = contact.email.clone();
                let label = contact.display_name.as_deref().unwrap_or(&contact.email);
                member_items.push(
                    button(
                        container(
                            row![
                                icon::plus().size(ICON_SM).style(text::primary),
                                text(label).size(TEXT_MD).style(text::base),
                                Space::new().width(Length::Fill),
                                text(&contact.email).size(TEXT_SM).style(text::secondary),
                            ]
                            .spacing(SPACE_XS)
                            .align_y(Alignment::Center),
                        )
                        .padding(PAD_SETTINGS_ROW)
                        .width(Length::Fill),
                    )
                    .on_press(SettingsMessage::GroupEditorAddMember(email))
                    .padding(0)
                    .style(theme::ButtonClass::Action.style())
                    .width(Length::Fill)
                    .into(),
                );
            }
        }
    }

    // Current members as removable tiles
    for email in &editor.members {
        let email_clone = email.clone();
        member_items.push(
            button(
                container(
                    row![
                        text(email).size(TEXT_MD).style(text::base),
                        Space::new().width(Length::Fill),
                        icon::trash().size(ICON_SM).style(text::danger),
                    ]
                    .spacing(SPACE_XS)
                    .align_y(Alignment::Center),
                )
                .padding(PAD_SETTINGS_ROW)
                .width(Length::Fill),
            )
            .on_press(SettingsMessage::GroupEditorRemoveMember(email_clone))
            .padding(0)
            .style(theme::ButtonClass::Action.style())
            .width(Length::Fill)
            .into(),
        );
    }

    // Build the section manually because the title is a dynamic String.
    let title_text = text(format!("Members ({})", editor.members.len()))
        .size(TEXT_XL)
        .style(text::base)
        .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() });

    let mut items_col = column![].width(Length::Fill).padding(1);
    for (i, item) in member_items.into_iter().enumerate() {
        if i > 0 {
            items_col = items_col.push(
                iced::widget::rule::horizontal(1).style(theme::RuleClass::Subtle.style()),
            );
        }
        items_col = items_col.push(item);
    }
    let section_box = container(items_col)
        .width(Length::Fill)
        .style(theme::ContainerClass::SettingsSection.style());

    column![title_text, section_box].spacing(SPACE_XS).into()
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

    let can_save = !editor.name.trim().is_empty();
    let mut save_btn = button(
        container(text("Save").size(TEXT_LG)).center_x(Length::Fill),
    )
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
        ("Navigation", &[
            ("Next thread", "j"),
            ("Previous thread", "k"),
            ("Go to Inbox", "g i"),
            ("Search", "/"),
            ("Close / dismiss", "Esc"),
        ]),
        ("Thread", &[
            ("Archive", "e"),
            ("Delete", "#"),
            ("Reply", "r"),
            ("Reply All", "a"),
            ("Forward", "f"),
            ("Star / unstar", "s"),
            ("Mute thread", "m"),
            ("Mark as unread", "u"),
        ]),
        ("Composing", &[
            ("New message", "c"),
        ]),
    ];

    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    for (category, items) in sections {
        let rows: Vec<Element<'_, SettingsMessage>> = items
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
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Provider", vec![
        setting_row("AI Provider", widgets::select(
            &["Claude", "OpenAI", "Gemini", "Ollama", "Copilot"],
            &state.ai_provider,
            state.open_select == Some(SelectField::AiProvider),
            SettingsMessage::ToggleSelect(SelectField::AiProvider),
            SettingsMessage::AiProviderChanged,
        ), SettingsMessage::ToggleSelect(SelectField::AiProvider)),
    ]));

    if state.ai_provider == "Ollama" {
        col = col.push(section("Local Server", vec![
            input_row("ollama-url", "Server URL", "http://localhost:11434", state.ai_ollama_url.text(), SettingsMessage::OllamaUrlChanged, InputField::OllamaUrl),
            input_row("ollama-model", "Model Name", "e.g. llama3.2", state.ai_ollama_model.text(), SettingsMessage::OllamaModelChanged, InputField::OllamaModel),
        ]));
    } else {
        let key_label = match state.ai_provider.as_str() {
            "OpenAI" => "OpenAI API Key",
            "Gemini" => "Google AI API Key",
            "Copilot" => "GitHub Personal Access Token",
            _ => "Anthropic API Key",
        };

        let model_options: &[&str] = match state.ai_provider.as_str() {
            "OpenAI" => &["gpt-4o", "gpt-4o-mini", "o4-mini"],
            "Gemini" => &["gemini-2.0-flash", "gemini-2.5-flash-preview-05-20", "gemini-2.5-pro"],
            "Copilot" => &["openai/gpt-4o", "openai/gpt-4o-mini"],
            _ => &["claude-haiku-4-5-20251001", "claude-sonnet-4-5", "claude-sonnet-4-6", "claude-opus-4-6"],
        };

        col = col.push(section("API Key", vec![
            container(
                column![
                    text(key_label).size(TEXT_LG).style(text::base),
                    Space::new().height(SPACE_XXS),
                    row![
                        undoable_text_input("", state.ai_api_key.text())
                            .on_input(SettingsMessage::AiApiKeyChanged)
                            .on_undo(SettingsMessage::UndoInput(InputField::AiApiKey))
                            .on_redo(SettingsMessage::RedoInput(InputField::AiApiKey))
                            .secure(true)
                            .size(TEXT_LG)
                            .padding(PAD_INPUT)
                            .style(theme::TextInputClass::Settings.style())
                            .width(Length::Fill),
                        Space::new().width(SPACE_XS),
                        button(
                            text(if state.ai_key_saved { "Saved" } else { "Save" }).size(TEXT_LG),
                        )
                        .on_press(SettingsMessage::SaveAiSettings)
                        .padding(PAD_ICON_BTN)
                        .style(theme::ButtonClass::Secondary.style()),
                    ]
                    .align_y(Alignment::Center),
                ]
                .spacing(SPACE_XXXS),
            )
            .padding(PAD_SETTINGS_ROW)
            .width(Length::Fill)
            .into(),
            setting_row("Model", widgets::select(
                model_options, &state.ai_model,
                state.open_select == Some(SelectField::AiModel),
                SettingsMessage::ToggleSelect(SelectField::AiModel),
                SettingsMessage::AiModelChanged,
            ), SettingsMessage::ToggleSelect(SelectField::AiModel)),
        ]));
    }

    col = col.push(section_with_subtitle("Features", "AI-powered tools to help manage your inbox", vec![
        toggle_row("Enable AI Features", "Use AI-powered features across the app",
            state.ai_enabled, SettingsMessage::ToggleAiEnabled),
        toggle_row("Auto-Categorize", "Automatically categorize incoming emails",
            state.ai_auto_categorize, SettingsMessage::ToggleAiAutoCategorize),
        toggle_row("Auto-Summarize", "Generate summaries for long email threads",
            state.ai_auto_summarize, SettingsMessage::ToggleAiAutoSummarize),
    ]));

    col = col.push(section("Auto-Draft Replies", vec![
        toggle_row("Auto-Draft", "Automatically draft replies based on email content",
            state.ai_auto_draft, SettingsMessage::ToggleAiAutoDraft),
        toggle_row("Learn Writing Style", "Analyze your sent emails to match your writing style",
            state.ai_writing_style, SettingsMessage::ToggleAiWritingStyle),
    ]));

    col = col.push(section("Auto-Archive Categories", vec![
        toggle_row("Updates", "Automatically archive update emails",
            state.ai_auto_archive_updates, SettingsMessage::ToggleAiAutoArchiveUpdates),
        toggle_row("Promotions", "Automatically archive promotional emails",
            state.ai_auto_archive_promotions, SettingsMessage::ToggleAiAutoArchivePromotions),
        toggle_row("Social", "Automatically archive social notification emails",
            state.ai_auto_archive_social, SettingsMessage::ToggleAiAutoArchiveSocial),
        toggle_row("Newsletters", "Automatically archive newsletters",
            state.ai_auto_archive_newsletters, SettingsMessage::ToggleAiAutoArchiveNewsletters),
    ]));

    col.into()
}

// ── About tab ───────────────────────────────────────────

fn about_tab<'a>() -> Element<'a, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);

    col = col.push(section("Application", vec![
        info_row("Version", "0.1.0-dev"),
        info_row("Iced Version", "0.15.0-dev"),
        info_row("Platform", std::env::consts::OS),
        info_row("Architecture", std::env::consts::ARCH),
    ]));

    col = col.push(section("Updates", vec![
        action_row("Software Updates", Some("Check for new versions"), None, ActionKind::InApp, SettingsMessage::CheckForUpdates),
    ]));

    col = col.push(section("License", vec![
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
        ).padding(PAD_SETTINGS_ROW).into(),
    ]));

    col = col.push(section("Links", vec![
        action_row("GitHub Repository", Some("folknor/ratatoskr"), Some(icon::globe()), ActionKind::Url, SettingsMessage::OpenGithub),
    ]));

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
            card_col = card_col.push(
                iced::widget::rule::horizontal(1)
                    .style(theme::RuleClass::Subtle.style()),
            );
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

    col = col.push(section("Accounts", vec![account_list, add_btn.into()]));

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
        inner_container =
            inner_container.style(theme::ContainerClass::DraggingRow.style());
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

// ── Import wizard overlay ───────────────────────────────

fn import_wizard_overlay(state: &Settings) -> Element<'_, SettingsMessage> {
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

    let mut col = column![].spacing(SPACE_LG).width(Length::Fill).max_width(SETTINGS_CONTENT_MAX_WIDTH);
    col = col.push(
        text("Import Contacts")
            .size(TEXT_HEADING)
            .style(text::base)
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
    );
    col = col.push(content);
    col.into()
}

fn import_step_file_select(_wizard: &ImportWizardState) -> Element<'_, SettingsMessage> {
    let items = vec![
        import_file_select_row(),
    ];
    section("Select File", items)
}

fn import_file_select_row() -> Element<'static, SettingsMessage> {
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
    section_untitled(vec![
        settings_row_container(
            SETTINGS_ROW_HEIGHT,
            row![
                icon::file().size(ICON_XL).style(text::secondary),
                Space::new().width(SPACE_XS),
                text(path).size(TEXT_LG).style(text::base),
            ]
            .align_y(Alignment::Center),
        ),
    ])
}

fn import_header_toggle(has_header: bool) -> Element<'static, SettingsMessage> {
    toggle_row(
        "First row is a header",
        "Enable if the first row contains column names, not data.",
        has_header,
        SettingsMessage::ImportToggleHeader,
    )
}

fn import_mapping_table<'a>(
    preview: &'a ratatoskr_contact_import::ImportPreview,
    mappings: &'a [ImportContactField],
) -> Element<'a, SettingsMessage> {
    let mut items: Vec<Element<'a, SettingsMessage>> = Vec::new();

    // Header row: column names with mapping dropdowns
    for (i, header) in preview.headers.iter().enumerate() {
        let current_field = mappings.get(i).copied().unwrap_or(ImportContactField::Ignore);
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
) -> Element<'_, SettingsMessage> {
    let header_owned = header.to_string();
    let selected = current.label().to_string();

    button(
        container(
            row![
                container(text(header_owned).size(TEXT_LG).style(text::base))
                    .align_y(Alignment::Center)
                    .width(Length::FillPortion(1)),
                container(
                    text(format!("-> {selected}")).size(TEXT_LG).style(text::primary),
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
    .style(theme::ButtonClass::Action.style())
    .width(Length::Fill)
    .into()
}

/// Cycle through import field options on click.
fn cycle_import_field(current: ImportContactField) -> ImportContactField {
    let all = ImportContactField::ALL_OPTIONS;
    let current_idx = all.iter().position(|&f| f == current).unwrap_or(0);
    let next_idx = (current_idx + 1) % all.len();
    all[next_idx]
}

fn import_sample_header() -> Element<'static, SettingsMessage> {
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text("Preview (first rows)")
            .size(TEXT_SM)
            .style(theme::TextClass::Tertiary.style())
            .font(iced::Font { weight: iced::font::Weight::Bold, ..crate::font::text() }),
    )
}

fn import_sample_row(row: &[String]) -> Element<'_, SettingsMessage> {
    let display = row.join("  |  ");
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(display).size(TEXT_SM).style(text::secondary),
    )
}

fn import_preview_stats<'a>(
    preview: &'a ratatoskr_contact_import::ImportPreview,
    mappings: &'a [ImportContactField],
) -> Element<'a, SettingsMessage> {
    let has_email = mappings.iter().any(|m| *m == ImportContactField::Email);
    let status = if has_email {
        format!("{} rows to import.", preview.total_rows)
    } else {
        "No Email column mapped. Map at least one column to Email.".to_string()
    };
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(status).size(TEXT_LG).style(if has_email { text::base } else { text::danger }),
    )
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

    container(
        column![
            text("Import to account").size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
            Space::new().height(SPACE_XXXS),
            btn_row,
        ],
    )
    .padding(PAD_SETTINGS_ROW)
    .width(Length::Fill)
    .into()
}

fn import_update_toggle(update_existing: bool) -> Element<'static, SettingsMessage> {
    toggle_row(
        "Update existing contacts",
        "When a duplicate email is found, update the existing contact with imported data.",
        update_existing,
        SettingsMessage::ImportToggleUpdateExisting,
    )
}

fn import_execute_button() -> Element<'static, SettingsMessage> {
    container(
        button(
            container(text("Import").size(TEXT_LG)).center_x(Length::Fill),
        )
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
    let valid_count = wizard.vcf_contacts.iter().filter(|c| c.has_valid_email()).count();
    let total = wizard.vcf_contacts.len();
    let skipped = total - valid_count;

    let stat_text = format!(
        "{total} contacts found. {valid_count} with valid email, {skipped} without.",
    );
    col = col.push(settings_row_container(
        SETTINGS_ROW_HEIGHT,
        text(stat_text).size(TEXT_LG).style(text::base),
    ));

    // Preview first 10 contacts
    let mut preview_items: Vec<Element<'a, SettingsMessage>> = Vec::new();
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

fn import_vcf_contact_row(contact: &ratatoskr_contact_import::ImportedContact) -> Element<'_, SettingsMessage> {
    let name = contact.effective_display_name().unwrap_or_else(|| "(no name)".to_string());
    let email = contact.normalized_email().unwrap_or_else(|| "(no email)".to_string());

    let email_style: fn(&iced::Theme) -> text::Style = if contact.has_valid_email() {
        text::secondary
    } else {
        text::danger
    };

    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            text(name).size(TEXT_LG).style(text::base).width(Length::FillPortion(1)),
            text(email).size(TEXT_SM).style(email_style).width(Length::FillPortion(1)),
        ]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center),
    )
}

fn import_step_importing() -> Element<'static, SettingsMessage> {
    settings_row_container(
        SETTINGS_TOGGLE_ROW_HEIGHT,
        column![
            text("Importing contacts...").size(TEXT_LG).style(text::base),
            Space::new().height(SPACE_XS),
            text("Please wait.").size(TEXT_SM).style(theme::TextClass::Tertiary.style()),
        ],
    )
}

fn import_step_summary(wizard: &ImportWizardState) -> Element<'_, SettingsMessage> {
    let mut col = column![].spacing(SPACE_LG).width(Length::Fill);

    if let Some(ref result) = wizard.result {
        let mut stats: Vec<Element<'_, SettingsMessage>> = Vec::new();
        stats.push(import_stat_row("Imported", result.imported));
        if result.updated > 0 {
            stats.push(import_stat_row("Updated", result.updated));
        }
        if result.skipped_no_email > 0 {
            stats.push(import_stat_row("Skipped (no email)", result.skipped_no_email));
        }
        if result.skipped_duplicate > 0 {
            stats.push(import_stat_row("Skipped (duplicate)", result.skipped_duplicate));
        }
        if result.groups_created > 0 {
            stats.push(import_stat_row("Groups created", result.groups_created));
        }
        col = col.push(section("Import Complete", stats));
    }

    col = col.push(
        container(
            button(
                container(text("Done").size(TEXT_LG)).center_x(Length::Fill),
            )
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

fn import_stat_row(label: &str, count: usize) -> Element<'_, SettingsMessage> {
    settings_row_container(
        SETTINGS_ROW_HEIGHT,
        row![
            text(label).size(TEXT_LG).style(text::base).width(Length::Fill),
            text(count.to_string()).size(TEXT_LG).style(text::primary),
        ]
        .align_y(Alignment::Center),
    )
}
