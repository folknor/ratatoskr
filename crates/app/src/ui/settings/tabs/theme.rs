use iced::widget::{Space, button, column, container, row, text};
use iced::{Alignment, Element, Length};

use crate::ui::layout::*;
use crate::ui::settings::row_widgets::*;
use crate::ui::settings::types::*;
use crate::ui::theme;
use crate::ui::widgets;

pub(super) fn theme_tab(state: &Settings) -> Element<'_, SettingsMessage> {
    let mut col = column![]
        .spacing(SPACE_LG)
        .width(Length::Fill)
        .max_width(SETTINGS_CONTENT_MAX_WIDTH);

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
