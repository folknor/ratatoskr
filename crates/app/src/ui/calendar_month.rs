//! Month view grid widget and mini-month calendar for the calendar feature.
//!
//! Provides data types, grid builder, and view functions for rendering
//! a full month grid and a compact mini-month navigation widget.

use chrono::{Datelike, NaiveDate, Weekday};
use iced::widget::{button, column, container, row, text};
use iced::{Alignment, Element, Length, Padding, Theme};

use super::layout::*;
use super::theme;

// ── Data types ──────────────────────────────────────────

/// A single event to render in the month grid.
pub struct MonthEvent {
    pub id: String,
    pub title: String,
    /// Unix timestamp (seconds).
    pub start_time: i64,
    /// Unix timestamp (seconds).
    pub end_time: i64,
    pub all_day: bool,
    /// Hex color string from the calendar (e.g. "#4285f4").
    pub color: String,
}

/// A day cell in the month grid.
pub struct MonthDay {
    pub date: NaiveDate,
    pub events: Vec<MonthEvent>,
    pub is_today: bool,
    pub is_current_month: bool,
}

/// Complete month grid data.
pub struct MonthGridData {
    pub year: i32,
    pub month: u32,
    pub weeks: Vec<[MonthDay; 7]>,
    pub week_start: Weekday,
}

// ── Grid builder ────────────────────────────────────────

/// Build a `MonthGridData` from a year/month, distributing events to the
/// correct day cells based on their `start_time`.
pub fn build_month_grid(
    year: i32,
    month: u32,
    events: &[MonthEvent],
    week_start: Weekday,
    today: NaiveDate,
) -> MonthGridData {
    // Find the first day of the month.
    let Some(first_of_month) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return MonthGridData { year, month, weeks: Vec::new(), week_start };
    };

    // Walk backward to find the grid start (the week_start day on or before first_of_month).
    let grid_start = rewind_to_weekday(first_of_month, week_start);

    // The grid needs 5 or 6 rows. We figure out how many by checking whether
    // the last day of the month lands in row 5 or row 6.
    let last_of_month = last_day_of_month(year, month);
    let total_days_shown = days_between(grid_start, last_of_month) + 1;
    let num_weeks = (total_days_shown + 6) / 7; // ceiling division
    let num_weeks = num_weeks.max(5); // always at least 5 rows

    let mut weeks = Vec::with_capacity(num_weeks);

    for week_idx in 0..num_weeks {
        let mut week: [MonthDay; 7] = std::array::from_fn(|day_idx| {
            #[allow(clippy::cast_possible_wrap)]
            let offset = (week_idx * 7 + day_idx) as i64;
            let date = grid_start + chrono::Duration::days(offset);
            MonthDay {
                date,
                events: Vec::new(),
                is_today: date == today,
                is_current_month: date.month() == month && date.year() == year,
            }
        });

        // Distribute events into this week's cells.
        // Multi-day events appear on every day they span, not just the start date.
        for event in events {
            let Some(start_dt) = chrono::DateTime::from_timestamp(event.start_time, 0) else {
                continue;
            };
            let Some(end_dt) = chrono::DateTime::from_timestamp(event.end_time, 0) else {
                continue;
            };
            let event_start = start_dt.date_naive();
            let event_end = end_dt.date_naive();
            for day in &mut week {
                if day.date >= event_start && day.date <= event_end {
                    day.events.push(MonthEvent {
                        id: event.id.clone(),
                        title: event.title.clone(),
                        start_time: event.start_time,
                        end_time: event.end_time,
                        all_day: event.all_day,
                        color: event.color.clone(),
                    });
                }
            }
        }

        // Sort events: all-day first, then by start_time.
        for day in &mut week {
            day.events.sort_by(|a, b| {
                b.all_day.cmp(&a.all_day).then(a.start_time.cmp(&b.start_time))
            });
        }

        weeks.push(week);
    }

    MonthGridData { year, month, weeks, week_start }
}

// ── Month view widget ───────────────────────────────────

/// Render the full month view grid.
///
/// Layout: header row of day-of-week labels, followed by 5-6 week rows
/// each containing 7 day cells. Each cell shows the date number and
/// up to N event entries (with a "+N more" indicator when truncated).
pub fn month_view<'a, M: 'a + Clone>(
    data: &'a MonthGridData,
    on_date_click: impl Fn(NaiveDate) -> M + 'a,
    on_event_click: impl Fn(&str) -> M + 'a,
) -> Element<'a, M> {
    let max_events_per_cell = max_visible_events();

    // Header row: week# label + day-of-week labels.
    let header = day_of_week_header_with_week_num(data.week_start);

    // Week rows.
    let mut grid = column![header].spacing(SPACE_0);

    for week in &data.weeks {
        // ISO week number from the first day of the row.
        let week_num = week.first()
            .map(|d| d.date.iso_week().week())
            .unwrap_or(0);

        let week_label = button(
            container(
                text(format!("{week_num}"))
                    .size(TEXT_XS)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .width(Length::Fixed(WEEK_NUM_COL_WIDTH))
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Start)
            .padding(Padding::from([SPACE_XXS, 0.0])),
        )
        .on_press(on_date_click(week[0].date))
        .padding(0)
        .style(theme::ButtonClass::Ghost.style());

        let mut week_row = row![week_label].spacing(SPACE_0);
        for day in week {
            let cell = day_cell(
                day,
                max_events_per_cell,
                &on_date_click,
                &on_event_click,
            );
            week_row = week_row.push(cell);
        }
        grid = grid.push(week_row);
    }

    container(grid)
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

/// Width of the ISO week number column.
const WEEK_NUM_COL_WIDTH: f32 = 28.0;

/// Header row with week# column + abbreviated day-of-week labels.
fn day_of_week_header_with_week_num<'a, M: 'a>(week_start: Weekday) -> Element<'a, M> {
    // Empty cell for the week number column.
    let wk_header = container(
        text("Wk")
            .size(TEXT_XS)
            .style(theme::TextClass::Tertiary.style()),
    )
    .width(Length::Fixed(WEEK_NUM_COL_WIDTH))
    .height(CALENDAR_HEADER_HEIGHT)
    .align_x(Alignment::Center)
    .align_y(Alignment::Center);

    let mut header_row = row![wk_header].spacing(SPACE_0);
    for i in 0..7 {
        let day = weekday_offset(week_start, i);
        let label = weekday_short(day);
        let cell = container(
            text(label)
                .size(TEXT_XS)
                .style(theme::TextClass::Tertiary.style()),
        )
        .width(Length::Fill)
        .height(CALENDAR_HEADER_HEIGHT)
        .padding(Padding::from([0.0, SPACE_XXS]))
        .align_y(Alignment::Center);
        header_row = header_row.push(cell);
    }
    header_row.into()
}

/// A single day cell in the month grid.
fn day_cell<'a, M: 'a + Clone>(
    day: &'a MonthDay,
    max_events: usize,
    on_date_click: &(impl Fn(NaiveDate) -> M + 'a),
    on_event_click: &(impl Fn(&str) -> M + 'a),
) -> Element<'a, M> {
    let cell_style = if day.is_today {
        theme::ContainerClass::CalendarCellToday
    } else if day.is_current_month {
        theme::ContainerClass::CalendarCell
    } else {
        theme::ContainerClass::CalendarCellMuted
    };

    // Date number.
    let date_num = day.date.day();
    let date_text_style: fn(&Theme) -> text::Style = if day.is_today {
        text::primary
    } else if day.is_current_month {
        text::base
    } else {
        theme::TextClass::Muted.style()
    };
    let date_font = if day.is_today {
        crate::font::text_semibold()
    } else {
        crate::font::text()
    };

    let date_label = button(
        text(format!("{date_num}"))
            .size(TEXT_SM)
            .style(date_text_style)
            .font(date_font),
    )
    .on_press(on_date_click(day.date))
    .padding(Padding::from([SPACE_XXXS, SPACE_XXS]))
    .style(theme::ButtonClass::Ghost.style());

    let mut content = column![date_label].spacing(SPACE_XXXS);

    // Event entries (capped).
    let total = day.events.len();
    let visible = total.min(max_events);
    let overflow = total.saturating_sub(max_events);

    for event in day.events.iter().take(visible) {
        content = content.push(event_entry(event, on_event_click(&event.id)));
    }

    if overflow > 0 {
        content = content.push(
            button(
                text(format!("+{overflow} more"))
                    .size(TEXT_XS)
                    .style(theme::TextClass::Muted.style()),
            )
            .on_press(on_date_click(day.date))
            .padding(Padding::from([0.0, SPACE_XXS]))
            .style(theme::ButtonClass::Ghost.style()),
        );
    }

    container(content)
        .width(Length::Fill)
        .height(Length::Fixed(CALENDAR_CELL_MIN_HEIGHT))
        .padding(SPACE_XXXS)
        .style(cell_style.style())
        .into()
}

// ── Event entry ─────────────────────────────────────────

/// A single event entry inside a day cell.
///
/// Small container with colored background and event title text.
/// Text color is chosen based on background luminance for readability.
fn event_entry<'a, M: 'a + Clone>(
    event: &'a MonthEvent,
    on_click: M,
) -> Element<'a, M> {
    let bg_color = theme::hex_to_color(&event.color);
    let text_color = contrasting_text_color(bg_color);

    let label = text(&event.title)
        .size(TEXT_XS)
        .color(text_color)
        .wrapping(text::Wrapping::None);

    button(
        container(label)
            .padding(Padding::from([0.0, SPACE_XXS]))
            .width(Length::Fill)
            .height(CALENDAR_EVENT_HEIGHT)
            .align_y(Alignment::Center)
            .style(move |_theme: &Theme| {
                container::Style {
                    background: Some(bg_color.into()),
                    border: iced::Border {
                        radius: RADIUS_SM.into(),
                        ..Default::default()
                    },
                    ..Default::default()
                }
            }),
    )
    .on_press(on_click)
    .padding(0)
    .style(theme::ButtonClass::BareTransparent.style())
    .width(Length::Fill)
    .into()
}

// ── Mini-month calendar ─────────────────────────────────

/// A compact month grid for the calendar sidebar (date navigation).
///
/// Shows a header with prev/next arrows and month/year label,
/// followed by a 7-column grid of date numbers.
pub fn mini_month<'a, M: 'a + Clone>(
    year: i32,
    month: u32,
    selected_date: Option<NaiveDate>,
    today: NaiveDate,
    week_start: Weekday,
    dates_with_events: &'a std::collections::HashSet<NaiveDate>,
    on_date_click: impl Fn(NaiveDate) -> M + 'a,
    on_prev_month: M,
    on_next_month: M,
) -> Element<'a, M> {
    // Month/year header with arrows.
    let month_name = month_label(month);
    let header = row![
        button(
            container(text("\u{25C0}").size(TEXT_SM).style(text::secondary))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(on_prev_month)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::BareIcon.style()),
        container(
            text(format!("{month_name} {year}"))
                .size(TEXT_SM)
                .style(text::base)
                .font(crate::font::text_semibold()),
        )
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center),
        button(
            container(text("\u{25B6}").size(TEXT_SM).style(text::secondary))
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
        )
        .on_press(on_next_month)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::BareIcon.style()),
    ]
    .align_y(Alignment::Center);

    // Day-of-week header (two-letter abbreviations).
    let mut dow_row = row![].spacing(SPACE_0);
    for i in 0..7 {
        let day = weekday_offset(week_start, i);
        let label = weekday_two_letter(day);
        dow_row = dow_row.push(
            container(
                text(label)
                    .size(TEXT_XS)
                    .style(theme::TextClass::Tertiary.style()),
            )
            .width(MINI_MONTH_CELL_SIZE)
            .height(MINI_MONTH_CELL_SIZE)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
        );
    }

    // Date grid.
    let Some(first_of_month) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return container(column![header, dow_row]).into();
    };
    let grid_start = rewind_to_weekday(first_of_month, week_start);
    let last = last_day_of_month(year, month);
    let total = days_between(grid_start, last) + 1;
    let num_weeks = ((total + 6) / 7).max(5);

    let mut grid = column![].spacing(SPACE_0);
    for w in 0..num_weeks {
        let mut week_row = row![].spacing(SPACE_0);
        for d in 0..7 {
            #[allow(clippy::cast_possible_wrap)]
            let offset = (w * 7 + d) as i64;
            let date = grid_start + chrono::Duration::days(offset);
            let in_month = date.month() == month && date.year() == year;
            let is_today = date == today;
            let is_selected = selected_date == Some(date);

            let num_str = format!("{}", date.day());
            let text_style: fn(&Theme) -> text::Style = if is_today {
                text::primary
            } else if in_month {
                text::base
            } else {
                theme::TextClass::Muted.style()
            };
            let font = if is_today {
                crate::font::text_semibold()
            } else {
                crate::font::text()
            };

            let has_events = dates_with_events.contains(&date);
            let label = text(num_str)
                .size(TEXT_XS)
                .style(text_style)
                .font(font);

            // Stack date number + optional event dot.
            let cell_content: Element<'_, M> = if has_events && in_month {
                let dot = container(text("\u{2022}").size(4.0).style(text::primary))
                    .align_x(Alignment::Center);
                column![label, dot].spacing(0).align_x(Alignment::Center).into()
            } else {
                label.into()
            };

            let cell_container = if is_selected {
                container(cell_content)
                    .width(MINI_MONTH_CELL_SIZE)
                    .height(MINI_MONTH_CELL_SIZE)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
                    .style(theme::ContainerClass::MiniMonthSelected.style())
            } else {
                container(cell_content)
                    .width(MINI_MONTH_CELL_SIZE)
                    .height(MINI_MONTH_CELL_SIZE)
                    .align_x(Alignment::Center)
                    .align_y(Alignment::Center)
            };

            let cell_btn = button(cell_container)
                .on_press(on_date_click(date))
                .padding(0)
                .style(theme::ButtonClass::Ghost.style());

            week_row = week_row.push(cell_btn);
        }
        grid = grid.push(week_row);
    }

    column![header, dow_row, grid]
        .spacing(SPACE_XXS)
        .into()
}

// ── Helpers ─────────────────────────────────────────────

/// How many events can fit in a single day cell, leaving room for the date
/// number and a potential "+N more" row.
fn max_visible_events() -> usize {
    // Cell height minus date row (~20px) minus "+N more" row (~16px),
    // divided by event row height.
    let available = CALENDAR_CELL_MIN_HEIGHT - CALENDAR_EVENT_HEIGHT - CALENDAR_EVENT_HEIGHT;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let count = (available / CALENDAR_EVENT_HEIGHT).floor() as usize;
    count.max(1)
}

/// Rewind a date to the previous (or same) occurrence of `target` weekday.
fn rewind_to_weekday(date: NaiveDate, target: Weekday) -> NaiveDate {
    let current = date.weekday();
    let diff = (current.num_days_from_monday() as i64 - target.num_days_from_monday() as i64 + 7)
        % 7;
    date - chrono::Duration::days(diff)
}

/// Last day of a given year/month.
fn last_day_of_month(year: i32, month: u32) -> NaiveDate {
    if month == 12 {
        NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(year, month + 1, 1)
    }
    .map(|d| d - chrono::Duration::days(1))
    .unwrap_or_else(|| NaiveDate::from_ymd_opt(year, month, 28).unwrap_or_default())
}

/// Number of days between two dates (inclusive would be +1 at call site).
fn days_between(a: NaiveDate, b: NaiveDate) -> usize {
    let diff = (b - a).num_days();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    if diff < 0 { 0 } else { diff as usize }
}

/// Get the weekday that is `offset` days after `start`.
fn weekday_offset(start: Weekday, offset: usize) -> Weekday {
    let base = start.num_days_from_monday();
    let target = (base as usize + offset) % 7;
    match target {
        0 => Weekday::Mon,
        1 => Weekday::Tue,
        2 => Weekday::Wed,
        3 => Weekday::Thu,
        4 => Weekday::Fri,
        5 => Weekday::Sat,
        _ => Weekday::Sun,
    }
}

/// Three-letter weekday abbreviation.
fn weekday_short(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    }
}

/// Two-letter weekday abbreviation for the mini-month.
fn weekday_two_letter(day: Weekday) -> &'static str {
    match day {
        Weekday::Mon => "Mo",
        Weekday::Tue => "Tu",
        Weekday::Wed => "We",
        Weekday::Thu => "Th",
        Weekday::Fri => "Fr",
        Weekday::Sat => "Sa",
        Weekday::Sun => "Su",
    }
}

/// Full month name.
fn month_label(month: u32) -> &'static str {
    match month {
        1 => "January",
        2 => "February",
        3 => "March",
        4 => "April",
        5 => "May",
        6 => "June",
        7 => "July",
        8 => "August",
        9 => "September",
        10 => "October",
        11 => "November",
        12 => "December",
        _ => "???",
    }
}

/// Choose white or dark text based on background luminance.
/// Uses the ITU-R BT.601 luma formula (0.299R + 0.587G + 0.114B).
fn contrasting_text_color(bg: iced::Color) -> iced::Color {
    let luminance = 0.299 * bg.r + 0.587 * bg.g + 0.114 * bg.b;
    if luminance > 0.5 {
        iced::Color::BLACK
    } else {
        iced::Color::WHITE
    }
}
