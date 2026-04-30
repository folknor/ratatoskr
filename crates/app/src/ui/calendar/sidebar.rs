use chrono::Local;
use iced::widget::{Space, button, checkbox, column, container, row, text};
use iced::{Alignment, Element, Length, Theme};

use crate::icon;
use crate::ui::calendar_month;
use crate::ui::layout::*;
use crate::ui::theme;

use super::format::parse_hex_color;
use super::messages::CalendarMessage;
use super::types::{CalendarState, CalendarView};

/// Calendar sidebar: mini-month, view switcher, calendar list placeholder.
pub(super) fn calendar_sidebar(state: &CalendarState) -> Element<'_, CalendarMessage> {
    let today = Local::now().date_naive();

    let mini = calendar_month::mini_month(
        state.mini_month_year,
        state.mini_month_month,
        Some(state.selected_date),
        today,
        state.week_start,
        &state.dates_with_events,
        CalendarMessage::SelectDate,
        CalendarMessage::PrevMonth,
        CalendarMessage::NextMonth,
    );

    let today_btn = button(text("Today").size(TEXT_SM).style(text::secondary))
        .on_press(CalendarMessage::Today)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let new_event_btn = button(text("+ New Event").size(TEXT_SM).style(text::primary))
        .on_press(CalendarMessage::CreateEvent)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let mut calendar_list_col = column![
        text("Calendars")
            .size(TEXT_XS)
            .font(crate::font::text_semibold())
            .style(theme::TextClass::Muted.style()),
    ]
    .spacing(SPACE_XXS);

    if state.calendars.is_empty() {
        calendar_list_col = calendar_list_col.push(
            text("No calendars synced")
                .size(TEXT_XS)
                .style(theme::TextClass::Muted.style()),
        );
    } else {
        for cal in &state.calendars {
            let cal_id = cal.id.clone();
            let is_visible = cal.is_visible;

            let color_dot = text("\u{25CF}")
                .size(TEXT_SM)
                .color(parse_hex_color(&cal.color));

            let name = text(&cal.display_name).size(TEXT_XS).style(text::base);

            let toggle = checkbox(is_visible).size(12).spacing(0);

            let cal_row = button(
                row![toggle, color_dot, name]
                    .spacing(SPACE_XXS)
                    .align_y(Alignment::Center),
            )
            .on_press(CalendarMessage::ToggleCalendarVisibility(
                cal_id.clone(),
                !is_visible,
            ))
            .padding([SPACE_XXXS, SPACE_XS])
            .style(theme::ButtonClass::Ghost.style())
            .width(Length::Fill);

            calendar_list_col = calendar_list_col.push(cal_row);
        }
    }

    let calendar_list = container(calendar_list_col).padding(SPACE_XS);

    let mode_btn = container(
        button(
            container(icon::mail().size(ICON_HERO).style(text::primary))
                .center_x(Length::Fill)
                .center_y(Length::Fill),
        )
        .on_press(CalendarMessage::SwitchToMail)
        .height(Length::Fill)
        .width(Length::Fill)
        .style(theme::ButtonClass::Experiment { variant: 10 }.style()),
    )
    .width(SIDEBAR_HEADER_HEIGHT)
    .height(Length::Fill);

    let views = [
        CalendarView::Day,
        CalendarView::WorkWeek,
        CalendarView::Week,
        CalendarView::Month,
    ];
    let mut view_row = row![].spacing(SPACE_XXS);
    for v in views {
        let is_active = v == state.active_view;
        let (btn_style, txt_style): (_, fn(&Theme) -> text::Style) = if is_active {
            (theme::ButtonClass::Primary.style(), |_| text::Style {
                color: Some(theme::ON_AVATAR),
            })
        } else {
            (
                theme::ButtonClass::Experiment { variant: 8 }.style(),
                text::primary,
            )
        };
        view_row = view_row.push(
            button(
                container(text(v.label()).size(TEXT_SM).style(txt_style))
                    .center_x(Length::Fill)
                    .center_y(Length::Fill),
            )
            .on_press(CalendarMessage::SetView(v))
            .width(Length::Fill)
            .height(Length::Fill)
            .style(btn_style),
        );
    }
    let right_stack = container(view_row.height(Length::Fill))
        .width(Length::Fill)
        .height(Length::Fill);

    let header = container(
        row![mode_btn, right_stack]
            .spacing(SPACE_XXS)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .height(SIDEBAR_HEADER_HEIGHT)
    .width(Length::Fill);

    let pop_out_btn = iced::widget::tooltip(
        button(
            container(
                row![
                    container(icon::external_link().size(ICON_LG).style(text::primary))
                        .align_y(Alignment::Center),
                    container(text("Pop Out").size(TEXT_LG).style(text::primary))
                        .align_y(Alignment::Center),
                ]
                .spacing(SPACE_XXS)
                .align_y(Alignment::Center),
            )
            .center_x(Length::Fill),
        )
        .on_press(CalendarMessage::PopOutCalendar)
        .style(theme::ButtonClass::Experiment { variant: 10 }.style())
        .padding(PAD_BUTTON)
        .width(Length::Fill),
        text("Open calendar in a separate window")
            .size(TEXT_XS)
            .style(theme::TextClass::OnPrimary.style()),
        iced::widget::tooltip::Position::Top,
    )
    .gap(SPACE_XXS)
    .style(theme::ContainerClass::Floating.style());

    let content = column![
        header,
        Space::new().height(SPACE_XS),
        mini,
        Space::new().height(SPACE_XXS),
        row![today_btn, new_event_btn].spacing(SPACE_XXS),
        Space::new().height(SPACE_SM),
        calendar_list,
        Space::new().height(Length::Fill),
        pop_out_btn,
    ]
    .spacing(0)
    .width(Length::Fill);

    container(content)
        .width(SIDEBAR_MIN_WIDTH)
        .height(Length::Fill)
        .padding(PAD_SIDEBAR)
        .style(theme::ContainerClass::Sidebar.style())
        .into()
}
