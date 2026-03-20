//! Time grid widget for Day, Work Week, and Week calendar views.
//!
//! Renders a vertical time axis with day columns, hour labels, all-day
//! event bars, positioned event blocks, and a current-time indicator.
//! Built from iced primitives (no custom `advanced::Widget`).

use chrono::{Datelike, NaiveDate, Timelike, Weekday};
use iced::widget::{button, column, container, row, scrollable, text, Space};
use iced::{Alignment, Element, Length, Padding, Theme};

use super::layout::*;
use super::theme;

// ── Data types ──────────────────────────────────────────

/// An event to render on the time grid.
pub struct TimeGridEvent {
    pub id: String,
    pub title: String,
    /// Unix timestamp (seconds).
    pub start_time: i64,
    /// Unix timestamp (seconds).
    pub end_time: i64,
    pub all_day: bool,
    /// Hex color string (e.g. "#4285f4").
    pub color: String,
    pub calendar_name: Option<String>,
}

/// A single day column in the time grid.
pub struct TimeGridDay {
    pub date: NaiveDate,
    pub events: Vec<TimeGridEvent>,
    pub is_today: bool,
}

/// Configuration for the time grid.
pub struct TimeGridConfig {
    pub days: Vec<TimeGridDay>,
    pub hour_start: u32,
    pub hour_end: u32,
    pub pixels_per_hour: f32,
}

// ── Overlap layout types ────────────────────────────────

/// An event with computed position within its day column.
struct LayoutEvent {
    index: usize,
    start_minutes: f32,
    end_minutes: f32,
    column: usize,
    total_columns: usize,
}

// ── Builder functions ───────────────────────────────────

/// Build a day view config (single column).
pub fn build_day_view(
    date: NaiveDate,
    events: &[TimeGridEvent],
    today: NaiveDate,
) -> TimeGridConfig {
    let day_events = events_for_date(events, date);
    TimeGridConfig {
        days: vec![TimeGridDay {
            date,
            events: day_events,
            is_today: date == today,
        }],
        hour_start: 0,
        hour_end: 24,
        pixels_per_hour: TIME_GRID_PIXELS_PER_HOUR,
    }
}

/// Build a work week view config (Mon-Fri of the week containing `date`).
pub fn build_work_week_view(
    date: NaiveDate,
    events: &[TimeGridEvent],
    today: NaiveDate,
) -> TimeGridConfig {
    let monday = rewind_to_weekday(date, Weekday::Mon);
    let days = (0..5)
        .map(|i| {
            let d = monday + chrono::Duration::days(i);
            TimeGridDay {
                date: d,
                events: events_for_date(events, d),
                is_today: d == today,
            }
        })
        .collect();
    TimeGridConfig {
        days,
        hour_start: 0,
        hour_end: 24,
        pixels_per_hour: TIME_GRID_PIXELS_PER_HOUR,
    }
}

/// Build a week view config (7 columns starting from `week_start`).
pub fn build_week_view(
    date: NaiveDate,
    events: &[TimeGridEvent],
    today: NaiveDate,
    week_start: Weekday,
) -> TimeGridConfig {
    let start = rewind_to_weekday(date, week_start);
    let days = (0..7)
        .map(|i| {
            let d = start + chrono::Duration::days(i);
            TimeGridDay {
                date: d,
                events: events_for_date(events, d),
                is_today: d == today,
            }
        })
        .collect();
    TimeGridConfig {
        days,
        hour_start: 0,
        hour_end: 24,
        pixels_per_hour: TIME_GRID_PIXELS_PER_HOUR,
    }
}

// ── Main view function ──────────────────────────────────

/// Render the time grid view.
pub fn time_grid_view<'a, M: 'a + Clone>(
    config: &'a TimeGridConfig,
    on_event_click: impl Fn(&str) -> M + 'a,
    on_slot_click: impl Fn(NaiveDate, u32) -> M + 'a,
) -> Element<'a, M> {
    let header = build_header(config, &on_slot_click);
    let all_day = build_all_day_bar(config, &on_event_click);
    let grid = build_time_grid(config, &on_event_click, &on_slot_click);

    let scrollable_grid = scrollable(grid)
        .height(Length::Fill)
        .width(Length::Fill);

    column![header, all_day, scrollable_grid]
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
}

// ── Header row ──────────────────────────────────────────

/// Day column headers with date and weekday label.
fn build_header<'a, M: 'a + Clone>(
    config: &'a TimeGridConfig,
    on_slot_click: &(impl Fn(NaiveDate, u32) -> M + 'a),
) -> Element<'a, M> {
    let mut header_row = row![]
        .spacing(SPACE_0)
        .height(TIME_GRID_HEADER_HEIGHT);

    // Left spacer for hour label column.
    header_row = header_row.push(
        container(Space::new())
            .width(TIME_GRID_HOUR_LABEL_WIDTH)
            .height(TIME_GRID_HEADER_HEIGHT),
    );

    for day in &config.days {
        let label = format_day_header(day.date);
        let style_fn = if day.is_today {
            theme::ContainerClass::TimeGridTodayHeader.style()
        } else {
            theme::ContainerClass::TimeGridCell.style()
        };
        let text_style: fn(&Theme) -> text::Style = if day.is_today {
            text::primary
        } else {
            text::base
        };
        let font = if day.is_today {
            crate::font::text_semibold()
        } else {
            crate::font::text()
        };

        let header_btn = button(
            container(
                text(label).size(TEXT_SM).style(text_style).font(font),
            )
            .width(Length::Fill)
            .height(Length::Fill)
            .align_x(Alignment::Center)
            .align_y(Alignment::Center),
        )
        .on_press(on_slot_click(day.date, 8))
        .padding(0)
        .width(Length::Fill)
        .style(theme::ButtonClass::Ghost.style());

        header_row = header_row.push(
            container(header_btn)
                .width(Length::Fill)
                .height(TIME_GRID_HEADER_HEIGHT)
                .style(style_fn),
        );
    }

    header_row.into()
}

// ── All-day bar ─────────────────────────────────────────

/// Bar above the time grid showing all-day events.
fn build_all_day_bar<'a, M: 'a + Clone>(
    config: &'a TimeGridConfig,
    on_event_click: &(impl Fn(&str) -> M + 'a),
) -> Element<'a, M> {
    let has_all_day = config.days.iter().any(|d| d.events.iter().any(|e| e.all_day));
    if !has_all_day {
        return Space::new().height(SPACE_0).into();
    }

    // Use Shrink height so the bar expands to fit multiple all-day events,
    // but cap visible events at 2 per day with "+N more" overflow.
    let max_visible_all_day = 2;

    let mut bar_row = row![].spacing(SPACE_0);

    // Label column.
    bar_row = bar_row.push(
        container(
            text("All day").size(TEXT_XS).style(theme::TextClass::Muted.style()),
        )
        .width(TIME_GRID_HOUR_LABEL_WIDTH)
        .padding(Padding::from([SPACE_XXXS, SPACE_XXS]))
        .align_y(Alignment::Center),
    );

    for day in &config.days {
        let all_day_events: Vec<&TimeGridEvent> =
            day.events.iter().filter(|e| e.all_day).collect();
        let cell = if all_day_events.is_empty() {
            container(Space::new().height(TIME_GRID_ALL_DAY_HEIGHT))
                .width(Length::Fill)
                .style(theme::ContainerClass::TimeGridCell.style())
        } else {
            let mut col = column![].spacing(SPACE_XXXS);
            let visible = all_day_events.len().min(max_visible_all_day);
            for ev in &all_day_events[..visible] {
                col = col.push(all_day_event_chip(ev, on_event_click(&ev.id)));
            }
            let overflow = all_day_events.len().saturating_sub(max_visible_all_day);
            if overflow > 0 {
                col = col.push(
                    text(format!("+{overflow} more"))
                        .size(TEXT_XS)
                        .style(theme::TextClass::Muted.style()),
                );
            }
            container(col)
                .width(Length::Fill)
                .padding(Padding::from([SPACE_XXXS, SPACE_XXS]))
                .style(theme::ContainerClass::TimeGridCell.style())
        };
        bar_row = bar_row.push(cell);
    }

    bar_row.into()
}

/// A small chip for an all-day event.
fn all_day_event_chip<'a, M: 'a + Clone>(
    event: &'a TimeGridEvent,
    on_click: M,
) -> Element<'a, M> {
    let bg_color = theme::hex_to_color(&event.color);
    let text_color = contrasting_text_color(bg_color);

    button(
        container(
            text(&event.title)
                .size(TEXT_XS)
                .color(text_color)
                .wrapping(text::Wrapping::None),
        )
        .padding(Padding::from([0.0, SPACE_XXS]))
        .width(Length::Fill)
        .align_y(Alignment::Center)
        .style(move |_theme: &Theme| container::Style {
            background: Some(bg_color.into()),
            border: iced::Border {
                radius: RADIUS_SM.into(),
                ..Default::default()
            },
            ..Default::default()
        }),
    )
    .on_press(on_click)
    .padding(0)
    .style(theme::ButtonClass::BareTransparent.style())
    .width(Length::Fill)
    .into()
}

// ── Time grid body ──────────────────────────────────────

/// The scrollable time grid with hour rows and event blocks.
fn build_time_grid<'a, M: 'a + Clone>(
    config: &'a TimeGridConfig,
    on_event_click: &(impl Fn(&str) -> M + 'a),
    on_slot_click: &(impl Fn(NaiveDate, u32) -> M + 'a),
) -> Element<'a, M> {
    let total_hours = config.hour_end.saturating_sub(config.hour_start);
    let grid_height = total_hours as f32 * config.pixels_per_hour;

    let mut grid_row = row![].spacing(SPACE_0);

    // Hour label column.
    grid_row = grid_row.push(build_hour_labels(config, grid_height));

    // Day columns.
    for day in &config.days {
        grid_row = grid_row.push(build_day_column(
            day,
            config,
            grid_height,
            on_event_click,
            on_slot_click,
        ));
    }

    container(grid_row)
        .width(Length::Fill)
        .height(grid_height)
        .into()
}

/// Left column of hour labels (8:00, 9:00, etc.).
fn build_hour_labels<'a, M: 'a>(
    config: &'a TimeGridConfig,
    grid_height: f32,
) -> Element<'a, M> {
    let mut col = column![].spacing(SPACE_0);

    for hour in config.hour_start..config.hour_end {
        let label = format_hour(hour);
        col = col.push(
            container(
                text(label)
                    .size(TEXT_XS)
                    .style(theme::TextClass::Muted.style()),
            )
            .width(TIME_GRID_HOUR_LABEL_WIDTH)
            .height(config.pixels_per_hour)
            .padding(Padding::from([SPACE_XXXS, SPACE_XXS]))
            .align_y(Alignment::Start)
            .style(theme::ContainerClass::TimeGridHourLabel.style()),
        );
    }

    container(col)
        .width(TIME_GRID_HOUR_LABEL_WIDTH)
        .height(grid_height)
        .into()
}

/// A single day column with hour slot backgrounds and positioned events.
fn build_day_column<'a, M: 'a + Clone>(
    day: &'a TimeGridDay,
    config: &'a TimeGridConfig,
    grid_height: f32,
    on_event_click: &(impl Fn(&str) -> M + 'a),
    on_slot_click: &(impl Fn(NaiveDate, u32) -> M + 'a),
) -> Element<'a, M> {
    // Background: hour slot cells as click targets.
    let slots = build_hour_slots(day, config, on_slot_click);

    // Overlay: positioned events + now line via a stack.
    let timed_events: Vec<&TimeGridEvent> =
        day.events.iter().filter(|e| !e.all_day).collect();

    if timed_events.is_empty() {
        // No events: just the slot background.
        let col = column![slots].width(Length::Fill);
        return container(col)
            .width(Length::Fill)
            .height(grid_height)
            .into();
    }

    // Lay out events with overlap algorithm, render, and stack on top of slots.
    let layouts = compute_overlap_layout(&timed_events, config, day.date);
    let events_layer = build_events_layer(
        &timed_events,
        &layouts,
        config,
        grid_height,
        on_event_click,
    );

    // Build now-line for today's column.
    let now_line: Element<'a, M> = if day.is_today {
        let now = chrono::Local::now();
        let now_minutes = now.time().hour() as f32 * 60.0 + now.time().minute() as f32;
        let grid_start_m = config.hour_start as f32 * 60.0;
        let grid_end_m = config.hour_end as f32 * 60.0;
        if now_minutes >= grid_start_m && now_minutes <= grid_end_m {
            let top = (now_minutes - grid_start_m) / 60.0 * config.pixels_per_hour;
            column![
                Space::new().height(top),
                container(Space::new())
                    .width(Length::Fill)
                    .height(TIME_GRID_NOW_LINE_WIDTH)
                    .style(theme::ContainerClass::TimeGridNowLine.style()),
            ]
            .width(Length::Fill)
            .into()
        } else {
            Space::new().width(0).height(0).into()
        }
    } else {
        Space::new().width(0).height(0).into()
    };

    // Use a stack: slots underneath, events on top, now-line on top of everything.
    let stack = iced::widget::stack![slots, events_layer, now_line]
        .width(Length::Fill)
        .height(grid_height);

    container(stack)
        .width(Length::Fill)
        .height(grid_height)
        .into()
}

/// Hour slot cells (clickable backgrounds for each hour).
fn build_hour_slots<'a, M: 'a + Clone>(
    day: &'a TimeGridDay,
    config: &'a TimeGridConfig,
    on_slot_click: &(impl Fn(NaiveDate, u32) -> M + 'a),
) -> Element<'a, M> {
    let mut col = column![].spacing(SPACE_0);

    for hour in config.hour_start..config.hour_end {
        let slot = button(
            container(Space::new())
                .width(Length::Fill)
                .height(config.pixels_per_hour),
        )
        .on_press(on_slot_click(day.date, hour))
        .padding(0)
        .width(Length::Fill)
        .height(config.pixels_per_hour)
        .style(theme::ButtonClass::Ghost.style());

        col = col.push(
            container(slot)
                .width(Length::Fill)
                .height(config.pixels_per_hour)
                .style(theme::ContainerClass::TimeGridCell.style()),
        );
    }

    container(col).width(Length::Fill).into()
}

// ── Overlap algorithm ───────────────────────────────────

/// Compute column assignments for overlapping events.
fn compute_overlap_layout(
    events: &[&TimeGridEvent],
    config: &TimeGridConfig,
    column_date: NaiveDate,
) -> Vec<LayoutEvent> {
    let mut items: Vec<LayoutEvent> = events
        .iter()
        .enumerate()
        .map(|(i, ev)| {
            let (start_m, end_m) = event_minutes(ev, config, column_date);
            LayoutEvent {
                index: i,
                start_minutes: start_m,
                end_minutes: end_m,
                column: 0,
                total_columns: 1,
            }
        })
        .collect();

    // Sort by start time, then by longer duration first.
    items.sort_by(|a, b| {
        a.start_minutes
            .partial_cmp(&b.start_minutes)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                (b.end_minutes - b.start_minutes)
                    .partial_cmp(&(a.end_minutes - a.start_minutes))
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });

    assign_columns(&mut items);
    items
}

/// Assign columns to overlapping events using a greedy algorithm.
fn assign_columns(items: &mut [LayoutEvent]) {
    // Find groups of mutually overlapping events and assign columns.
    let n = items.len();
    if n == 0 {
        return;
    }

    // For each event, find which column it can go in.
    let mut columns_end: Vec<f32> = Vec::new(); // end time of latest event in each column

    for item in items.iter_mut() {
        // Find the first column where this event doesn't overlap.
        let col = columns_end
            .iter()
            .position(|&end| end <= item.start_minutes);
        match col {
            Some(c) => {
                item.column = c;
                columns_end[c] = item.end_minutes;
            }
            None => {
                item.column = columns_end.len();
                columns_end.push(item.end_minutes);
            }
        }
    }

    // Now set total_columns for each overlap group.
    // We do a second pass: for each event, find all events it overlaps with
    // and take the max column count.
    set_total_columns(items);
}

/// Set `total_columns` for each event based on its overlap group.
fn set_total_columns(items: &mut [LayoutEvent]) {
    let n = items.len();
    for i in 0..n {
        let mut max_col = items[i].column;
        for j in 0..n {
            if i == j {
                continue;
            }
            if events_overlap(&items[i], &items[j]) {
                max_col = max_col.max(items[j].column);
            }
        }
        items[i].total_columns = max_col + 1;
    }
}

/// Check if two layout events overlap in time.
fn events_overlap(a: &LayoutEvent, b: &LayoutEvent) -> bool {
    a.start_minutes < b.end_minutes && b.start_minutes < a.end_minutes
}

// ── Event rendering ─────────────────────────────────────

/// Build a layer of positioned event blocks using a stack for overlap support.
/// Each event is absolutely positioned via a column with top spacer.
fn build_events_layer<'a, M: 'a + Clone>(
    events: &[&'a TimeGridEvent],
    layouts: &[LayoutEvent],
    config: &'a TimeGridConfig,
    grid_height: f32,
    on_event_click: &(impl Fn(&str) -> M + 'a),
) -> Element<'a, M> {
    let mut layers: Vec<Element<'a, M>> = Vec::new();

    for layout in layouts {
        let event = &events[layout.index];
        let top = (layout.start_minutes - config.hour_start as f32 * 60.0)
            / 60.0
            * config.pixels_per_hour;
        let height = ((layout.end_minutes - layout.start_minutes) / 60.0
            * config.pixels_per_hour)
            .max(TIME_GRID_MIN_EVENT_HEIGHT);

        let block = event_block(event, height, layout, on_event_click(&event.id));

        // Position via a column with top spacer.
        let positioned = column![Space::new().height(top), block]
            .spacing(SPACE_0)
            .width(Length::Fill)
            .height(grid_height);

        layers.push(positioned.into());
    }

    if layers.is_empty() {
        return Space::new().width(Length::Fill).height(grid_height).into();
    }

    iced::widget::stack(layers)
        .width(Length::Fill)
        .height(grid_height)
        .into()
}

/// Render a single event block.
fn event_block<'a, M: 'a + Clone>(
    event: &'a TimeGridEvent,
    height: f32,
    layout: &LayoutEvent,
    on_click: M,
) -> Element<'a, M> {
    let bg_color = theme::hex_to_color(&event.color);
    let text_color = contrasting_text_color(bg_color);

    let time_str = format_event_time(event);
    let label_text = if time_str.is_empty() {
        event.title.clone()
    } else {
        format!("{} {}", time_str, event.title)
    };

    let label = text(label_text)
        .size(TEXT_XS)
        .color(text_color)
        .wrapping(text::Wrapping::None);

    let inner = container(label)
        .padding(Padding::from([SPACE_XXXS, SPACE_XXS]))
        .width(Length::Fill)
        .height(height)
        .align_y(Alignment::Start)
        .style(move |_theme: &Theme| container::Style {
            background: Some(bg_color.into()),
            border: iced::Border {
                radius: RADIUS_SM.into(),
                ..Default::default()
            },
            ..Default::default()
        });

    // Horizontal positioning based on column assignment.
    // We use left padding as a fraction of the available width.
    // Since we can't get the actual width in a pure function,
    // we use FillPortion to divide the space.
    let total_cols = layout.total_columns.max(1);
    let col_idx = layout.column;

    if total_cols == 1 {
        // Full width.
        button(inner)
            .on_press(on_click)
            .padding(0)
            .width(Length::Fill)
            .style(theme::ButtonClass::BareTransparent.style())
            .into()
    } else {
        // Split into portions: spacer(col_idx) + event(1) + spacer(remaining).
        let mut r = row![].spacing(SPACE_0).width(Length::Fill);

        if col_idx > 0 {
            r = r.push(
                Space::new()
                    .width(Length::FillPortion(col_idx as u16))
                    .height(height),
            );
        }

        r = r.push(
            button(inner)
                .on_press(on_click)
                .padding(0)
                .width(Length::FillPortion(1))
                .style(theme::ButtonClass::BareTransparent.style()),
        );

        let remaining = total_cols - col_idx - 1;
        if remaining > 0 {
            r = r.push(
                Space::new()
                    .width(Length::FillPortion(remaining as u16))
                    .height(height),
            );
        }

        r.height(height).into()
    }
}

// ── Helpers ─────────────────────────────────────────────

/// Get start/end as minutes-from-midnight for an event on a specific
/// column date. For multi-day events, the start is clamped to midnight
/// if the event started on a previous day, and the end is clamped to
/// 23:59 if the event continues past this day.
fn event_minutes(
    event: &TimeGridEvent,
    config: &TimeGridConfig,
    column_date: NaiveDate,
) -> (f32, f32) {
    // Convert UTC timestamps to local time for display.
    // The spec says "Display in local time everywhere."
    let start_dt = chrono::DateTime::from_timestamp(event.start_time, 0)
        .map(|dt| dt.with_timezone(&chrono::Local));
    let end_dt = chrono::DateTime::from_timestamp(event.end_time, 0)
        .map(|dt| dt.with_timezone(&chrono::Local));
    let grid_start = config.hour_start as f32 * 60.0;
    let grid_end = config.hour_end as f32 * 60.0;

    let (start_m, end_m) = match (start_dt, end_dt) {
        (Some(s), Some(e)) => {
            let event_start_date = s.date_naive();
            let event_end_date = e.date_naive();

            // Clamp start: if event started before this column's day,
            // treat as starting at midnight.
            let sm = if event_start_date < column_date {
                0.0
            } else {
                s.time().hour() as f32 * 60.0 + s.time().minute() as f32
            };

            // Clamp end: if event ends after this column's day,
            // treat as ending at end-of-day.
            let em = if event_end_date > column_date {
                grid_end
            } else {
                e.time().hour() as f32 * 60.0 + e.time().minute() as f32
            };

            (sm.max(grid_start), em.min(grid_end).max(sm + 1.0))
        }
        _ => (0.0, 60.0),
    };
    (start_m, end_m)
}

/// Filter events to those occurring on a specific date.
fn events_for_date(events: &[TimeGridEvent], date: NaiveDate) -> Vec<TimeGridEvent> {
    events
        .iter()
        .filter(|e| {
            // Use local time for date filtering — an event at 23:00 local
            // should appear on the correct local day, not the UTC day.
            let start = chrono::DateTime::from_timestamp(e.start_time, 0)
                .map(|dt| dt.with_timezone(&chrono::Local).date_naive());
            let end = chrono::DateTime::from_timestamp(e.end_time, 0)
                .map(|dt| dt.with_timezone(&chrono::Local).date_naive());
            match (start, end) {
                (Some(s), Some(e_date)) => date >= s && date <= e_date,
                (Some(s), None) => date == s,
                _ => false,
            }
        })
        .map(|e| TimeGridEvent {
            id: e.id.clone(),
            title: e.title.clone(),
            start_time: e.start_time,
            end_time: e.end_time,
            all_day: e.all_day,
            color: e.color.clone(),
            calendar_name: e.calendar_name.clone(),
        })
        .collect()
}

/// Rewind a date to the previous (or same) occurrence of `target` weekday.
fn rewind_to_weekday(date: NaiveDate, target: Weekday) -> NaiveDate {
    let current = date.weekday();
    let diff = (current.num_days_from_monday() as i64
        - target.num_days_from_monday() as i64
        + 7)
        % 7;
    date - chrono::Duration::days(diff)
}

/// Format an hour number as a time label.
fn format_hour(hour: u32) -> String {
    format!("{hour}:00")
}

/// Format a day header label (e.g. "Wed 19").
fn format_day_header(date: NaiveDate) -> String {
    let weekday = match date.weekday() {
        Weekday::Mon => "Mon",
        Weekday::Tue => "Tue",
        Weekday::Wed => "Wed",
        Weekday::Thu => "Thu",
        Weekday::Fri => "Fri",
        Weekday::Sat => "Sat",
        Weekday::Sun => "Sun",
    };
    format!("{} {}", weekday, date.day())
}

/// Format event start time for display in block.
fn format_event_time(event: &TimeGridEvent) -> String {
    chrono::DateTime::from_timestamp(event.start_time, 0)
        .map(|dt| format!("{}:{:02}", dt.time().hour(), dt.time().minute()))
        .unwrap_or_default()
}

/// Choose white or dark text based on background luminance.
fn contrasting_text_color(bg: iced::Color) -> iced::Color {
    let luminance = 0.299 * bg.r + 0.587 * bg.g + 0.114 * bg.b;
    if luminance > 0.5 {
        iced::Color::BLACK
    } else {
        iced::Color::WHITE
    }
}
