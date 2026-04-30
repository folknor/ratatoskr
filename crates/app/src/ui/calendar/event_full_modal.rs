use chrono::Datelike;
use iced::widget::{Space, button, column, container, row, scrollable, text};
use iced::{Alignment, Element, Length};

use crate::ui::calendar_time_grid;
use crate::ui::layout::*;
use crate::ui::theme;

use super::format::{
    format_event_time_range, format_recurrence_rule, format_reminder, month_short, parse_hex_color,
    weekday_short,
};
use super::messages::CalendarMessage;
use super::types::{CalendarEventData, CalendarState};

/// Full event detail modal (two-panel: ~70% detail + ~30% mini day view).
pub(super) fn event_full_modal<'a>(
    event: &'a CalendarEventData,
    state: &'a CalendarState,
) -> Element<'a, CalendarMessage> {
    let mut detail = column![].spacing(SPACE_SM);

    let close_btn = button(text("\u{2715}").size(TEXT_SM))
        .on_press(CalendarMessage::CloseModal)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    if let Some(ref cal_name) = event.calendar_name {
        let color_dot = if let Some(ref hex) = event.color {
            text("\u{25CF}").size(TEXT_MD).color(parse_hex_color(hex))
        } else {
            text("\u{25CF}").size(TEXT_MD)
        };
        detail = detail.push(
            row![
                color_dot,
                text(cal_name).size(TEXT_SM).style(text::secondary)
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        );
    }

    let title_text = if event.title.is_empty() {
        "(Untitled event)"
    } else {
        &event.title
    };
    detail = detail.push(
        text(title_text)
            .size(TEXT_HEADING)
            .font(crate::font::text_semibold()),
    );

    let date_str = format!(
        "{}, {} {}, {}",
        weekday_short(event.start_date.weekday()),
        month_short(event.start_date.month()),
        event.start_date.day(),
        event.start_date.year(),
    );
    let time_str = format_event_time_range(event);
    let datetime_label = if event.all_day {
        format!("{date_str} \u{2014} All day")
    } else {
        format!("{date_str}  {time_str}")
    };
    let mut datetime_row = row![text(datetime_label).size(TEXT_MD).style(text::secondary),]
        .spacing(SPACE_SM)
        .align_y(Alignment::Center);

    if let Some(ref tz) = event.timezone
        && !tz.is_empty()
    {
        datetime_row = datetime_row.push(
            text(tz)
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        );
    }
    detail = detail.push(datetime_row);

    if let Some(ref rrule) = event.recurrence_rule {
        detail = detail.push(
            row![
                text("\u{1F501}").size(TEXT_SM),
                text(format_recurrence_rule(rrule))
                    .size(TEXT_SM)
                    .style(text::secondary),
            ]
            .spacing(SPACE_XXS)
            .align_y(Alignment::Center),
        );
    }

    if !event.location.is_empty() {
        let loc_text =
            if event.location.starts_with("http://") || event.location.starts_with("https://") {
                text(&event.location).size(TEXT_MD).style(text::primary)
            } else {
                text(&event.location).size(TEXT_MD).style(text::secondary)
            };
        detail = detail.push(loc_text);
    }

    if let Some(ref name) = event.organizer_name {
        if !name.is_empty() {
            detail = detail.push(
                text(format!("Organizer: {name}"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
    } else if let Some(ref email) = event.organizer_email
        && !email.is_empty()
    {
        detail = detail.push(
            text(format!("Organizer: {email}"))
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }

    if !event.attendees.is_empty() {
        detail = detail.push(
            text("Attendees:")
                .size(TEXT_SM)
                .font(crate::font::text_semibold()),
        );
        for att in &event.attendees {
            let icon = match att.rsvp_status.as_str() {
                "accepted" => "\u{2713}",
                "declined" => "\u{2717}",
                "tentative" => "~",
                _ => "?",
            };
            let display = att.name.as_deref().unwrap_or(&att.email);
            let suffix = if att.is_organizer { " (organizer)" } else { "" };
            detail = detail.push(
                text(format!("  {icon} {display}{suffix}"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
    }

    if !event.description.is_empty() {
        detail = detail.push(Space::new().height(SPACE_XXS));
        detail = detail.push(
            text(&event.description)
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }

    if !event.reminders.is_empty() {
        let reminder_text = event
            .reminders
            .iter()
            .map(|r| format_reminder(r.minutes_before))
            .collect::<Vec<_>>()
            .join(", ");
        detail = detail.push(
            text(format!("Reminders: {reminder_text}"))
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }

    if let Some(ref status) = event.rsvp_status {
        let display = match status.as_str() {
            "accepted" => "Accepted",
            "declined" => "Declined",
            "tentative" => "Tentative",
            "needs-action" => "No response",
            _ => status.as_str(),
        };
        detail = detail.push(
            text(format!("Your RSVP: {display}"))
                .size(TEXT_SM)
                .font(crate::font::text_semibold())
                .style(text::secondary),
        );
    }

    detail = detail.push(Space::new().height(SPACE_XS));
    let edit_btn = button(text("Edit").size(TEXT_SM))
        .on_press(CalendarMessage::OpenEventEditor(Some(event.clone())))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Ghost.style());

    let mut action_row = row![edit_btn].spacing(SPACE_XS);
    if let Some(ref id) = event.id {
        action_row = action_row.push(
            button(text("Delete").size(TEXT_SM))
                .on_press(CalendarMessage::ConfirmDeleteEvent {
                    event_id: id.clone(),
                    title: event.title.clone(),
                    account_id: event.account_id.clone(),
                })
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style()),
        );
    }
    detail = detail.push(action_row);

    let detail_scroll = scrollable(detail).height(Length::Fill);

    let day_events = calendar_time_grid::events_for_date(&state.events, event.start_date);

    let day_label = format!(
        "{}, {} {}",
        weekday_short(event.start_date.weekday()),
        month_short(event.start_date.month()),
        event.start_date.day(),
    );

    let mut day_col = column![
        text(day_label)
            .size(TEXT_SM)
            .font(crate::font::text_semibold()),
        Space::new().height(SPACE_XS),
    ]
    .spacing(SPACE_XXXS);

    if day_events.is_empty() {
        day_col = day_col.push(
            text("No other events")
                .size(TEXT_XS)
                .style(theme::TextClass::Muted.style()),
        );
    } else {
        for ev in day_events {
            let time_str = if ev.all_day {
                "All day".to_string()
            } else {
                let start = chrono::DateTime::from_timestamp(ev.start_time, 0)
                    .map(|dt| dt.with_timezone(&chrono::Local));
                let end = chrono::DateTime::from_timestamp(ev.end_time, 0)
                    .map(|dt| dt.with_timezone(&chrono::Local));
                match (start, end) {
                    (Some(s), Some(e)) => {
                        format!("{} \u{2013} {}", s.format("%H:%M"), e.format("%H:%M"))
                    }
                    _ => String::new(),
                }
            };
            let is_current = event.id.as_deref() == Some(ev.id.as_str());
            let ev_color = parse_hex_color(&ev.color);
            let dot = text("\u{25CF}").size(TEXT_XS).color(ev_color);
            let title_style: fn(&iced::Theme) -> text::Style = if is_current {
                text::primary
            } else {
                text::base
            };
            let ev_row = column![
                row![dot, text(ev.title).size(TEXT_XS).style(title_style)]
                    .spacing(SPACE_XXS)
                    .align_y(Alignment::Center),
                text(time_str)
                    .size(8.0)
                    .style(theme::TextClass::Muted.style()),
            ]
            .spacing(0);
            day_col = day_col.push(ev_row);
        }
    }

    let right_panel = container(scrollable(day_col))
        .width(Length::FillPortion(3))
        .height(Length::Fill)
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Sidebar.style());

    let header = row![Space::new().width(Length::Fill), close_btn,].width(Length::Fill);

    let left_panel = container(column![header, detail_scroll].spacing(SPACE_XXS))
        .width(Length::FillPortion(7))
        .height(Length::Fill)
        .padding(PAD_CARD);

    let two_panel = row![left_panel, right_panel].height(Length::Fill);

    container(two_panel)
        .width(Length::FillPortion(4))
        .max_width(1200.0)
        .height(Length::Fill)
        .padding(iced::Padding::from([SPACE_LG, 0.0]))
        .style(theme::ContainerClass::Elevated.style())
        .into()
}
