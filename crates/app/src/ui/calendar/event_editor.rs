use chrono::Datelike;
use iced::widget::{Space, button, column, container, pick_list, row, scrollable, text, text_input};
use iced::{Alignment, Element, Length};

use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::undoable_text_input::undoable_text_input;

use super::format::{format_recurrence_rule, month_short, weekday_short};
use super::messages::{CalendarMessage, EventField, EventTextField};
use super::types::{CalendarEventData, CalendarListEntry};

/// Event creation/editing form (rendered as a centered modal).
pub(super) fn event_editor_card<'a>(
    event: &'a CalendarEventData,
    is_creating: bool,
    calendars: &'a [CalendarListEntry],
) -> Element<'a, CalendarMessage> {
    let heading = if is_creating {
        "New Event"
    } else {
        "Edit Event"
    };

    let mut content = column![].spacing(SPACE_SM);

    let close_btn = button(text("\u{2715}").size(TEXT_SM))
        .on_press(CalendarMessage::CloseModal)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());
    content = content.push(
        row![
            text(heading)
                .size(TEXT_HEADING)
                .font(crate::font::text_semibold()),
            Space::new().width(Length::Fill),
            close_btn,
        ]
        .align_y(Alignment::Center),
    );

    if is_creating {
        let selected = calendars
            .iter()
            .find(|c| Some(&c.id) == event.calendar_id.as_ref())
            .cloned();
        let options: Vec<CalendarListEntry> = calendars.to_vec();
        let picker = pick_list(selected, options, |c: &CalendarListEntry| {
            c.display_name.clone()
        })
        .on_select(|entry: CalendarListEntry| {
            CalendarMessage::EventFieldChanged(EventField::CalendarSelected {
                calendar_id: Some(entry.id),
                account_id: Some(entry.account_id),
            })
        })
        .placeholder("Select calendar...")
        .text_size(TEXT_MD)
        .padding(PAD_INPUT)
        .width(Length::Fill)
        .style(theme::PickListClass::Ghost.style());
        content = content.push(form_field("Calendar", picker.into()));
    } else {
        let label = event
            .calendar_name
            .as_deref()
            .or(event.calendar_id.as_deref())
            .unwrap_or("Unknown calendar");
        content = content.push(form_field("Calendar", text(label).size(TEXT_MD).into()));
    }

    content = content.push(form_field(
        "Title",
        undoable_text_input("Event title", &event.title)
            .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Title(s)))
            .on_undo(CalendarMessage::EventFieldUndo(EventTextField::Title))
            .on_redo(CalendarMessage::EventFieldRedo(EventTextField::Title))
            .padding(PAD_INPUT)
            .size(TEXT_MD)
            .into(),
    ));

    let date_str = format!(
        "{}, {} {}, {}",
        weekday_short(event.start_date.weekday()),
        month_short(event.start_date.month()),
        event.start_date.day(),
        event.start_date.year(),
    );
    content = content.push(form_field("Date", text(date_str).size(TEXT_MD).into()));

    if !event.all_day {
        let time_row = time_input_row(event);
        content = content.push(form_field("Time", time_row));
    }

    let all_day_label = if event.all_day {
        "All day: Yes"
    } else {
        "All day: No"
    };
    let all_day_btn = button(text(all_day_label).size(TEXT_SM))
        .on_press(CalendarMessage::EventFieldChanged(EventField::AllDay(
            !event.all_day,
        )))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());
    content = content.push(all_day_btn);

    content = content.push(form_field(
        "Location",
        undoable_text_input("Location (optional)", &event.location)
            .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Location(s)))
            .on_undo(CalendarMessage::EventFieldUndo(EventTextField::Location))
            .on_redo(CalendarMessage::EventFieldRedo(EventTextField::Location))
            .padding(PAD_INPUT)
            .size(TEXT_MD)
            .into(),
    ));

    content = content.push(form_field(
        "Description",
        undoable_text_input("Description (optional)", &event.description)
            .on_input(|s| CalendarMessage::EventFieldChanged(EventField::Description(s)))
            .on_undo(CalendarMessage::EventFieldUndo(EventTextField::Description))
            .on_redo(CalendarMessage::EventFieldRedo(EventTextField::Description))
            .padding(PAD_INPUT)
            .size(TEXT_MD)
            .into(),
    ));

    let tz_display = event.timezone.as_deref().unwrap_or("Local");
    let tz_input = text_input("Timezone (e.g. Europe/Oslo)", tz_display)
        .on_input(|s| {
            let tz = if s.is_empty() { None } else { Some(s) };
            CalendarMessage::EventFieldChanged(EventField::Timezone(tz))
        })
        .padding(PAD_INPUT)
        .size(TEXT_SM);
    content = content.push(form_field("Timezone", tz_input.into()));

    let avail = event.availability.as_deref().unwrap_or("busy");
    let avail_options = ["busy", "free", "tentative", "oof"];
    let mut avail_row = row![].spacing(SPACE_XXS);
    for opt in &avail_options {
        let is_active = avail == *opt;
        let label = match *opt {
            "busy" => "Busy",
            "free" => "Free",
            "tentative" => "Tentative",
            "oof" => "OOO",
            _ => opt,
        };
        avail_row = avail_row.push(
            button(text(label).size(TEXT_XS).style(if is_active {
                text::primary
            } else {
                text::secondary
            }))
            .on_press(CalendarMessage::EventFieldChanged(
                EventField::Availability(Some((*opt).to_string())),
            ))
            .padding(PAD_ICON_BTN)
            .style(if is_active {
                theme::ButtonClass::Nav { active: true }.style()
            } else {
                theme::ButtonClass::Ghost.style()
            }),
        );
    }
    content = content.push(form_field("Availability", avail_row.into()));

    let vis = event.visibility.as_deref().unwrap_or("default");
    let vis_options = ["default", "public", "private"];
    let mut vis_row = row![].spacing(SPACE_XXS);
    for opt in &vis_options {
        let is_active = vis == *opt;
        let label = match *opt {
            "default" => "Default",
            "public" => "Public",
            "private" => "Private",
            _ => opt,
        };
        vis_row = vis_row.push(
            button(text(label).size(TEXT_XS).style(if is_active {
                text::primary
            } else {
                text::secondary
            }))
            .on_press(CalendarMessage::EventFieldChanged(EventField::Visibility(
                Some((*opt).to_string()),
            )))
            .padding(PAD_ICON_BTN)
            .style(if is_active {
                theme::ButtonClass::Nav { active: true }.style()
            } else {
                theme::ButtonClass::Ghost.style()
            }),
        );
    }
    content = content.push(form_field("Visibility", vis_row.into()));

    let has_recurrence = event.recurrence_rule.is_some();
    let recurrence_label = if has_recurrence {
        format!(
            "Recurring: {}",
            format_recurrence_rule(event.recurrence_rule.as_deref().unwrap_or(""),)
        )
    } else {
        "Not recurring".to_string()
    };
    let recurrence_toggle = button(text(recurrence_label).size(TEXT_SM))
        .on_press(CalendarMessage::EventFieldChanged(
            EventField::RecurrenceRule(if has_recurrence {
                None
            } else {
                Some("RRULE:FREQ=WEEKLY".to_string())
            }),
        ))
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());
    content = content.push(form_field("Recurrence", recurrence_toggle.into()));

    let can_save = event.calendar_id.is_some();
    let save_btn = button(text("Save").size(TEXT_SM))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Nav { active: true }.style());
    let save_btn = if can_save {
        save_btn.on_press(CalendarMessage::SaveEvent)
    } else {
        save_btn
    };

    let cancel_btn = button(text("Cancel").size(TEXT_SM))
        .on_press(CalendarMessage::CloseModal)
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Ghost.style());

    let mut action_row = row![save_btn, cancel_btn].spacing(SPACE_XS);

    if !is_creating
        && let Some(ref id) = event.id
    {
        action_row = action_row.push(Space::new().width(Length::Fill));
        action_row = action_row.push(
            button(text("Delete").size(TEXT_SM).style(text::danger))
                .on_press(CalendarMessage::ConfirmDeleteEvent {
                    event_id: id.clone(),
                    title: event.title.clone(),
                    account_id: event.account_id.clone(),
                })
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style()),
        );
    }

    content = content.push(Space::new().height(SPACE_XS));
    content = content.push(action_row);

    let scrollable_content = scrollable(content).height(Length::Shrink);

    container(scrollable_content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .max_height(CALENDAR_OVERLAY_MAX_HEIGHT)
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

/// A labeled form field row: label on left, widget on right.
fn form_field<'a>(
    display_text: &'a str,
    widget: Element<'a, CalendarMessage>,
) -> Element<'a, CalendarMessage> {
    row![
        container(
            text(display_text)
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        )
        .width(Length::Fixed(CALENDAR_FORM_LABEL_WIDTH))
        .height(CALENDAR_FORM_ROW_HEIGHT)
        .align_y(Alignment::Center),
        container(widget)
            .width(Length::Fill)
            .align_y(Alignment::Center),
    ]
    .spacing(SPACE_XS)
    .align_y(Alignment::Center)
    .into()
}

/// Time input row with start and end hour:minute text inputs.
fn time_input_row(event: &CalendarEventData) -> Element<'_, CalendarMessage> {
    let start_h = text_input("HH", &event.start_hour)
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::StartHour(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let start_m = text_input("MM", &event.start_minute)
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::StartMinute(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let end_h = text_input("HH", &event.end_hour)
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::EndHour(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    let end_m = text_input("MM", &event.end_minute)
        .on_input(|s| CalendarMessage::EventFieldChanged(EventField::EndMinute(s)))
        .padding(PAD_INPUT)
        .width(Length::Fixed(48.0))
        .size(TEXT_MD);

    row![
        start_h,
        text(":").size(TEXT_MD),
        start_m,
        text("\u{2013}").size(TEXT_MD),
        end_h,
        text(":").size(TEXT_MD),
        end_m,
    ]
    .spacing(SPACE_XXS)
    .align_y(Alignment::Center)
    .into()
}
