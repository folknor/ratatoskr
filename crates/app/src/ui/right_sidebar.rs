use chrono::{Datelike, Local, NaiveDate};
use iced::widget::{button, column, container, scrollable, text, Space};
use iced::{Element, Length, Padding};

use crate::db::Thread;
use crate::ui::calendar::{CalendarMessage, CalendarState};
use crate::ui::calendar_month;
use crate::ui::layout::*;
use crate::ui::theme;
use crate::ui::widgets;
use crate::Message;

/// Maximum number of starred threads to display in the sidebar.
const MAX_STARRED_ITEMS: usize = 5;

/// Data required by the right sidebar, gathered by the caller.
pub struct RightSidebarData<'a> {
    pub calendar: &'a CalendarState,
    pub threads: &'a [Thread],
}

pub fn view<'a>(open: bool, data: &RightSidebarData<'a>) -> Element<'a, Message> {
    if !open {
        return Space::new().width(0).height(0).into();
    }

    let content = column![
        calendar_section(data.calendar),
        widgets::divider(),
        agenda_section(data.calendar),
        widgets::divider(),
        pinned_section(data.threads),
    ]
    .spacing(0)
    .width(Length::Fill);

    container(scrollable(content).spacing(SCROLLBAR_SPACING).height(Length::Fill))
        .width(RIGHT_SIDEBAR_WIDTH)
        .height(Length::Fill)
        .style(theme::ContainerClass::Sidebar.style())
        .into()
}

fn calendar_section(cal: &CalendarState) -> Element<'_, Message> {
    let today = Local::now().date_naive();

    let mini = calendar_month::mini_month(
        cal.mini_month_year,
        cal.mini_month_month,
        Some(cal.selected_date),
        today,
        cal.week_start,
        &cal.dates_with_events,
        |date| Message::Calendar(Box::new(CalendarMessage::SelectDate(date))),
        Message::Calendar(Box::new(CalendarMessage::PrevMonth)),
        Message::Calendar(Box::new(CalendarMessage::NextMonth)),
    );

    container(
        column![widgets::section_header("CALENDAR"), mini].spacing(SPACE_XXS),
    )
    .padding(PAD_RIGHT_SIDEBAR)
    .into()
}

/// Show today's calendar events as a compact time + title list.
fn agenda_section(cal: &CalendarState) -> Element<'_, Message> {
    let today = Local::now().date_naive();
    let today_events = events_for_date(&cal.events, today);

    let body: Element<'_, Message> = if today_events.is_empty() {
        container(
            text("No events today")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_ICON_BTN)
        .into()
    } else {
        let mut items = column![].spacing(SPACE_XXXS);
        for ev in &today_events {
            items = items.push(agenda_item(ev));
        }
        container(items).padding(PAD_ICON_BTN).into()
    };

    container(
        column![widgets::section_header("TODAY'S AGENDA"), body].spacing(SPACE_XXS),
    )
    .padding(PAD_RIGHT_SIDEBAR)
    .into()
}

/// A single agenda row: time range + title. Clicking opens the event detail.
fn agenda_item(event: &crate::ui::calendar_time_grid::TimeGridEvent) -> Element<'_, Message> {
    let time_label = if event.all_day {
        "All day".to_string()
    } else {
        format_time_range(event.start_time, event.end_time)
    };

    let time_text = text(time_label)
        .size(TEXT_XS)
        .style(theme::TextClass::Tertiary.style());

    let title_text = text(&event.title)
        .size(TEXT_SM)
        .style(text::base)
        .wrapping(text::Wrapping::None);

    let event_id = event.id.clone();
    button(
        column![time_text, title_text].spacing(SPACE_XXXS),
    )
    .on_press(Message::Calendar(Box::new(CalendarMessage::EventClicked(event_id))))
    .padding(Padding::from([SPACE_XXXS, 0.0]))
    .width(Length::Fill)
    .style(theme::ButtonClass::Ghost.style())
    .into()
}

fn pinned_section(threads: &[Thread]) -> Element<'_, Message> {
    let starred: Vec<&Thread> = threads
        .iter()
        .filter(|t| t.is_starred)
        .take(MAX_STARRED_ITEMS)
        .collect();

    let body: Element<'_, Message> = if starred.is_empty() {
        container(
            text("No pinned items")
                .size(TEXT_SM)
                .style(theme::TextClass::Tertiary.style()),
        )
        .padding(PAD_ICON_BTN)
        .into()
    } else {
        let mut items = column![].spacing(SPACE_XS);
        for thread in &starred {
            items = items.push(starred_item(thread));
        }
        container(items).padding(PAD_ICON_BTN).into()
    };

    container(
        column![widgets::section_header("PINNED ITEMS"), body].spacing(SPACE_XXS),
    )
    .padding(PAD_RIGHT_SIDEBAR)
    .into()
}

/// A compact starred thread row: sender on line 1, subject on line 2.
fn starred_item(thread: &Thread) -> Element<'_, Message> {
    let sender = thread
        .from_name
        .as_deref()
        .or(thread.from_address.as_deref())
        .unwrap_or("Unknown");

    let subject = thread.subject.as_deref().unwrap_or("(no subject)");

    let sender_text = text(sender)
        .size(TEXT_XS)
        .style(theme::TextClass::Tertiary.style())
        .wrapping(text::Wrapping::None);

    let subject_text = text(subject)
        .size(TEXT_SM)
        .style(text::base)
        .wrapping(text::Wrapping::None);

    column![sender_text, subject_text].spacing(SPACE_XXXS).into()
}

// ── Helpers ─────────────────────────────────────────────

/// Filter calendar events to those occurring on a specific date.
fn events_for_date<'a>(
    events: &'a [crate::ui::calendar_time_grid::TimeGridEvent],
    date: NaiveDate,
) -> Vec<&'a crate::ui::calendar_time_grid::TimeGridEvent> {
    events
        .iter()
        .filter(|ev| {
            let Some(start_dt) = chrono::DateTime::from_timestamp(ev.start_time, 0) else {
                return false;
            };
            let Some(end_dt) = chrono::DateTime::from_timestamp(ev.end_time, 0) else {
                return false;
            };
            let start_date = start_dt.date_naive();
            let end_date = end_dt.date_naive();
            date >= start_date && date <= end_date
        })
        .collect()
}

/// Format a Unix timestamp range as "HH:MM -- HH:MM".
fn format_time_range(start: i64, end: i64) -> String {
    use chrono::TimeZone;
    let local = Local;
    let start_dt = local.timestamp_opt(start, 0).single();
    let end_dt = local.timestamp_opt(end, 0).single();
    match (start_dt, end_dt) {
        (Some(s), Some(e)) => format!("{} \u{2013} {}", s.format("%H:%M"), e.format("%H:%M")),
        (Some(s), None) => s.format("%H:%M").to_string(),
        _ => String::new(),
    }
}
