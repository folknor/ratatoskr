use std::collections::HashSet;

use chrono::{Datelike, Local, NaiveDate, TimeZone, Weekday};

use crate::ui::calendar_month;
use crate::ui::calendar_time_grid;
use crate::ui::undoable::UndoableText;

/// Which calendar view is active in the main content area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalendarView {
    Day,
    WorkWeek,
    Week,
    Month,
}

impl CalendarView {
    pub fn label(self) -> &'static str {
        match self {
            Self::Day => "D",
            Self::WorkWeek => "5",
            Self::Week => "7",
            Self::Month => "M",
        }
    }
}

/// What the user is currently doing in the calendar feature.
///
/// Source of truth for event lifecycle meaning. Surfaces
/// (`active_popover`, `active_modal`) are presentation caches
/// synchronized from this state.
///
/// Which presentation surface is active for a `ViewingEvent` workflow.
///
/// Exists because `ExpandPopoverToModal` is the one transition where
/// workflow identity stays the same while presentation changes - the
/// user is still viewing the same event, only the surface differs.
#[derive(Debug, Clone)]
pub enum ViewingSurface {
    /// Quick-glance popover.
    Popover,
    /// Full event detail modal.
    FullModal,
}

/// **Invariant:** Handlers update workflow state, then call
/// `sync_surfaces()`. Surfaces are written exclusively by
/// `sync_surfaces()`, never independently by handlers.
/// Reads of lifecycle meaning and identity come from workflow only.
#[derive(Debug, Clone)]
pub enum CalendarWorkflow {
    /// No active event workflow. User is browsing/navigating.
    Idle,
    /// Viewing an existing event (popover or full modal).
    /// Identity is read from `event_data.id` / `event_data.account_id`.
    ViewingEvent {
        event_data: CalendarEventData,
        surface: ViewingSurface,
    },
    /// Creating a new event in the editor.
    ///
    /// `account_id` is authoritative for mutation dispatch.
    /// `session.draft.account_id` is a synced display copy - updated
    /// alongside this field by the `CalendarSelected` handler, never
    /// read for lifecycle decisions.
    CreatingEvent {
        account_id: Option<String>,
        session: EditorSession,
    },
    /// Editing an existing event in the editor.
    ///
    /// `event_id` and `account_id` are authoritative for mutation dispatch.
    /// `session.draft.id` / `session.draft.account_id` are carried display
    /// copies - consistent by construction, never read for lifecycle decisions.
    EditingEvent {
        event_id: String,
        account_id: String,
        session: EditorSession,
    },
    /// Confirming discard of unsaved editor changes.
    /// Carries the full session so cancel-discard can restore the editor.
    ConfirmingDiscard {
        /// `true` = was creating, `false` = was editing.
        was_creating: bool,
        event_id: Option<String>,
        account_id: Option<String>,
        session: EditorSession,
    },
    /// Confirming deletion of a persisted event.
    ConfirmingDelete {
        event_id: String,
        account_id: String,
        title: String,
    },
}

/// The active calendar popover, if any.
#[derive(Debug, Clone)]
pub enum CalendarPopover {
    /// Quick-glance event popover (~300px compact card).
    EventDetail { event: CalendarEventData },
}

/// The active calendar modal, if any.
///
/// Presentation cache, not workflow state. Written exclusively by
/// `CalendarState::sync_surfaces()`, derived from `CalendarWorkflow`.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
pub enum CalendarModal {
    /// Full event detail modal (two-panel: 70% detail + 30% day view).
    EventFull { event: CalendarEventData },
    /// Editing or creating an event. Create-vs-edit semantics
    /// and the mutable draft come from `CalendarWorkflow`, not from
    /// this variant. This is a unit marker - the draft lives on
    /// `EditorSession` in the workflow state.
    ///
    /// **Invariant:** `EventEditor` without `CreatingEvent` or
    /// `EditingEvent` in workflow state is a contract violation.
    EventEditor,
    /// Delete confirmation dialog.
    #[allow(dead_code)] // event_id + account_id captured here so the modal can dispatch the delete
    ConfirmDelete {
        event_id: String,
        title: String,
        account_id: Option<String>,
    },
    /// Discard-unsaved-changes confirmation dialog.
    ConfirmDiscard { title: String },
}

/// An attendee for display in event detail.
#[derive(Debug, Clone)]
pub struct AttendeeEntry {
    pub email: String,
    pub name: Option<String>,
    pub rsvp_status: String,
    pub is_organizer: bool,
}

/// A reminder for display in event detail.
#[derive(Debug, Clone)]
pub struct ReminderEntry {
    pub minutes_before: i64,
    pub method: String,
}

#[derive(Debug, Clone)]
pub struct CalendarEventData {
    /// DB row id. `None` for new events that haven't been saved.
    pub id: Option<String>,
    pub title: String,
    pub start_date: NaiveDate,
    pub start_hour: String,
    pub start_minute: String,
    pub end_hour: String,
    pub end_minute: String,
    pub all_day: bool,
    pub location: String,
    pub description: String,
    pub calendar_id: Option<String>,
    pub account_id: Option<String>,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub organizer_name: Option<String>,
    pub organizer_email: Option<String>,
    pub rsvp_status: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
    pub attendees: Vec<AttendeeEntry>,
    pub reminders: Vec<ReminderEntry>,
    pub calendar_name: Option<String>,
    pub color: Option<String>,
}

impl CalendarEventData {
    /// Build a blank event pre-filled with the given date/hour.
    pub fn new_at(date: NaiveDate, hour: u32) -> Self {
        let (end_hour, end_minute) = if hour >= 23 {
            ("23".to_string(), "59".to_string())
        } else {
            (format!("{}", hour + 1), "00".to_string())
        };
        Self {
            id: None,
            title: String::new(),
            start_date: date,
            start_hour: format!("{hour}"),
            start_minute: "00".to_string(),
            end_hour,
            end_minute,
            all_day: false,
            location: String::new(),
            description: String::new(),
            calendar_id: None,
            account_id: None,
            timezone: None,
            recurrence_rule: None,
            organizer_name: None,
            organizer_email: None,
            rsvp_status: None,
            availability: None,
            visibility: None,
            attendees: Vec::new(),
            reminders: Vec::new(),
            calendar_name: None,
            color: None,
        }
    }

    /// Parse start hour as u32 (defaults to 0 on invalid input).
    pub fn start_hour_u32(&self) -> u32 {
        self.start_hour.parse().unwrap_or(0).min(23)
    }

    /// Parse start minute as u32 (defaults to 0 on invalid input).
    pub fn start_minute_u32(&self) -> u32 {
        self.start_minute.parse().unwrap_or(0).min(59)
    }

    /// Parse end hour as u32 (defaults to 0 on invalid input).
    pub fn end_hour_u32(&self) -> u32 {
        self.end_hour.parse().unwrap_or(0).min(23)
    }

    /// Parse end minute as u32 (defaults to 0 on invalid input).
    pub fn end_minute_u32(&self) -> u32 {
        self.end_minute.parse().unwrap_or(0).min(59)
    }

    /// Snapshot the editable persisted fields for dirty detection.
    ///
    /// Excludes identity (`id`, `account_id`) and display-only fields
    /// (`organizer_*`, `rsvp_status`, `calendar_name`, `color`,
    /// `attendees`, `reminders`).
    pub fn snapshot(&self) -> EventSnapshot {
        EventSnapshot {
            title: self.title.clone(),
            start_date: self.start_date,
            start_hour: self.start_hour.clone(),
            start_minute: self.start_minute.clone(),
            end_hour: self.end_hour.clone(),
            end_minute: self.end_minute.clone(),
            all_day: self.all_day,
            location: self.location.clone(),
            description: self.description.clone(),
            calendar_id: self.calendar_id.clone(),
            timezone: self.timezone.clone(),
            recurrence_rule: self.recurrence_rule.clone(),
            availability: self.availability.clone(),
            visibility: self.visibility.clone(),
        }
    }
}

/// Snapshot of editable event fields, used for dirty detection.
///
/// Includes only fields that the user can modify in the editor.
/// Excludes identity (`id`, `account_id`), display-only fields
/// (`organizer_*`, `rsvp_status`, `calendar_name`, `color`),
/// and read-only collections (`attendees`, `reminders`).
///
/// `calendar_id` is included because the draft is the authoritative
/// editable source for calendar assignment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventSnapshot {
    pub title: String,
    pub start_date: NaiveDate,
    pub start_hour: String,
    pub start_minute: String,
    pub end_hour: String,
    pub end_minute: String,
    pub all_day: bool,
    pub location: String,
    pub description: String,
    pub calendar_id: Option<String>,
    pub timezone: Option<String>,
    pub recurrence_rule: Option<String>,
    pub availability: Option<String>,
    pub visibility: Option<String>,
}

/// Bundles all editor state for a calendar event being created or edited.
///
/// Lives on the workflow state (`CreatingEvent` / `EditingEvent`).
/// The single source of truth for all editable event state during editing.
#[derive(Debug, Clone)]
pub struct EditorSession {
    /// The mutable draft - fields are updated as the user types.
    pub draft: CalendarEventData,
    /// Snapshot of editable fields at editor open time, for dirty detection.
    pub original: EventSnapshot,
    /// Per-text-field undo/redo history.
    pub undo_title: UndoableText,
    pub undo_location: UndoableText,
    pub undo_description: UndoableText,
}

impl EditorSession {
    /// Create a new editor session from an event.
    ///
    /// Takes a snapshot of the editable fields as the original baseline
    /// and initializes undo buffers for text fields.
    pub fn new(event: CalendarEventData) -> Self {
        let original = event.snapshot();
        let undo_title = UndoableText::with_initial(&event.title);
        let undo_location = UndoableText::with_initial(&event.location);
        let undo_description = UndoableText::with_initial(&event.description);
        Self {
            draft: event,
            original,
            undo_title,
            undo_location,
            undo_description,
        }
    }

    /// Whether the draft has been modified from its original state.
    pub fn is_dirty(&self) -> bool {
        self.draft.snapshot() != self.original
    }
}

/// A calendar for the sidebar list with visibility toggle.
/// Also used in the editor's calendar selector dropdown.
#[derive(Debug, Clone, PartialEq)]
pub struct CalendarListEntry {
    pub id: String,
    pub account_id: String,
    pub display_name: String,
    pub color: String,
    pub is_visible: bool,
}

fn first_day_of_month(year: i32, month: u32) -> Option<NaiveDate> {
    NaiveDate::from_ymd_opt(year, month, 1)
}

fn local_midnight_unix(date: NaiveDate) -> i64 {
    let naive = match date.and_hms_opt(0, 0, 0) {
        Some(d) => d,
        None => return 0,
    };
    Local
        .from_local_datetime(&naive)
        .single()
        .map(|dt| dt.timestamp())
        // Spring-forward midnight is non-existent in some zones (rare; no
        // current IANA zone springs forward at 00:00 but the API permits
        // it). The window is approximate so we round up to the next
        // representable minute.
        .unwrap_or_else(|| {
            naive
                .and_utc()
                .timestamp()
        })
}

/// Persistent calendar state (survives mode switches).
pub struct CalendarState {
    /// The currently selected/focused date.
    pub selected_date: NaiveDate,
    /// The selected time slot hour (for event creation pre-fill).
    pub selected_hour: Option<u32>,
    /// The active view (day/work-week/week/month).
    pub active_view: CalendarView,
    /// Which month is displayed in the mini-month sidebar.
    pub mini_month_year: i32,
    pub mini_month_month: u32,
    /// Start-of-week preference.
    pub week_start: Weekday,
    /// Cached month grid data for the month view (rebuilt on state change).
    pub month_grid: calendar_month::MonthGridData,
    /// Cached time grid config for day/work-week/week views.
    pub time_grid_config: calendar_time_grid::TimeGridConfig,
    /// Current event lifecycle workflow state. Source of truth for
    /// what the user is doing - surfaces are synchronized from this.
    pub workflow: CalendarWorkflow,
    /// Current quick-glance popover, if any (presentation cache).
    pub active_popover: Option<CalendarPopover>,
    /// Current blocking modal, if any (presentation cache).
    pub active_modal: Option<CalendarModal>,
    /// Cached events from the DB. Reloaded after CRUD operations.
    pub events: Vec<calendar_time_grid::TimeGridEvent>,
    /// All calendars across accounts (for sidebar list).
    pub calendars: Vec<CalendarListEntry>,
    /// Set of dates that have at least one event (for mini-month dots).
    pub dates_with_events: HashSet<NaiveDate>,
    /// Generation counter for async load staleness detection.
    /// Incremented before each load dispatch; results carry the generation
    /// they were dispatched with and are dropped if it no longer matches.
    pub load_generation: rtsk::generation::GenerationCounter<rtsk::generation::Calendar>,
}

impl CalendarState {
    #[allow(dead_code)] // default constructor kept; callers go through with_default_view today
    pub fn new() -> Self {
        Self::with_default_view(CalendarView::Month)
    }

    /// Create calendar state with a specific default view (read from settings).
    pub fn with_default_view(default_view: CalendarView) -> Self {
        let today = Local::now().date_naive();
        let month_grid =
            calendar_month::build_month_grid(today.year(), today.month(), &[], Weekday::Mon, today);
        let time_grid_config = calendar_time_grid::build_day_view(today, &[], today);
        Self {
            selected_date: today,
            selected_hour: None,
            active_view: default_view,
            mini_month_year: today.year(),
            mini_month_month: today.month(),
            week_start: Weekday::Mon,
            month_grid,
            time_grid_config,
            workflow: CalendarWorkflow::Idle,
            active_popover: None,
            active_modal: None,
            events: Vec::new(),
            calendars: Vec::new(),
            dates_with_events: HashSet::new(),
            load_generation: rtsk::generation::GenerationCounter::new(),
        }
    }

    /// Compute the (start, end) Unix-second window the current view needs
    /// loaded.
    ///
    /// We use a generous 3-month band centered on the currently-displayed
    /// mini-month rather than the active view's tight range. Reasons:
    /// - The month-view grid spans up to 6 weeks (mini_month +- a few days
    ///   from neighbouring months), so a tight month window would clip the
    ///   leading/trailing rows.
    /// - The user navigates nearby weeks frequently; loading +-1 month
    ///   means most arrow-key navigation hits already-loaded data without
    ///   re-running the SELECT under the connection mutex.
    /// - Recurring rows are loaded regardless of window (they pass the
    ///   `recurrence_rule IS NOT NULL` branch in SQL), so the window only
    ///   bounds non-recurring rows. A wider window costs almost nothing.
    ///
    /// The window is also used inside `expand_view_events` to clip the
    /// recurrence-expansion output, so unbounded RRULEs don't pour years
    /// of instances into the view.
    pub fn current_view_window(&self) -> (i64, i64) {
        let prev = first_day_of_month(self.mini_month_year, self.mini_month_month)
            .and_then(|d| d.checked_sub_months(chrono::Months::new(1)))
            .unwrap_or_else(|| {
                first_day_of_month(self.mini_month_year, self.mini_month_month)
                    .unwrap_or_else(|| Local::now().date_naive())
            });
        let next = first_day_of_month(self.mini_month_year, self.mini_month_month)
            .and_then(|d| d.checked_add_months(chrono::Months::new(2)))
            .unwrap_or_else(|| {
                first_day_of_month(self.mini_month_year, self.mini_month_month)
                    .unwrap_or_else(|| Local::now().date_naive())
            });
        // Anchor in chrono::Local. The window is approximate by design so
        // a host-zone offset of a few hours doesn't matter; the exact
        // boundary is meaningless for the SQL filter (it's just bounding
        // the result set).
        let start = local_midnight_unix(prev);
        let end = local_midnight_unix(next);
        (start, end)
    }

    /// Parse a view name string from the settings table.
    pub fn parse_view_name(name: &str) -> CalendarView {
        match name {
            "day" => CalendarView::Day,
            "work_week" => CalendarView::WorkWeek,
            "week" => CalendarView::Week,
            _ => CalendarView::Month,
        }
    }

    /// Derive surface state from workflow state.
    ///
    /// Call after every workflow state mutation. This is the single
    /// place where `active_popover` and `active_modal` are written.
    pub fn sync_surfaces(&mut self) {
        match &self.workflow {
            CalendarWorkflow::Idle => {
                self.active_popover = None;
                self.active_modal = None;
            }
            CalendarWorkflow::ViewingEvent {
                event_data,
                surface,
            } => match surface {
                ViewingSurface::Popover => {
                    self.active_popover = Some(CalendarPopover::EventDetail {
                        event: event_data.clone(),
                    });
                    self.active_modal = None;
                }
                ViewingSurface::FullModal => {
                    self.active_popover = None;
                    self.active_modal = Some(CalendarModal::EventFull {
                        event: event_data.clone(),
                    });
                }
            },
            CalendarWorkflow::CreatingEvent { .. }
            | CalendarWorkflow::EditingEvent { .. } => {
                self.active_popover = None;
                self.active_modal = Some(CalendarModal::EventEditor);
            }
            CalendarWorkflow::ConfirmingDiscard { .. } => {
                self.active_popover = None;
                self.active_modal = Some(CalendarModal::ConfirmDiscard {
                    title: "Discard unsaved changes?".to_string(),
                });
            }
            CalendarWorkflow::ConfirmingDelete {
                title,
                event_id,
                account_id,
            } => {
                self.active_popover = None;
                self.active_modal = Some(CalendarModal::ConfirmDelete {
                    event_id: event_id.clone(),
                    title: title.clone(),
                    account_id: Some(account_id.clone()),
                });
            }
        }
    }

    /// Navigate the mini-month to the previous month.
    pub fn prev_month(&mut self) {
        if self.mini_month_month == 1 {
            self.mini_month_month = 12;
            self.mini_month_year -= 1;
        } else {
            self.mini_month_month -= 1;
        }
        self.rebuild_view_data();
    }

    /// Navigate the mini-month to the next month.
    pub fn next_month(&mut self) {
        if self.mini_month_month == 12 {
            self.mini_month_month = 1;
            self.mini_month_year += 1;
        } else {
            self.mini_month_month += 1;
        }
        self.rebuild_view_data();
    }

    /// Jump to today, updating both selected date and mini-month.
    pub fn go_to_today(&mut self) {
        let today = Local::now().date_naive();
        self.selected_date = today;
        self.mini_month_year = today.year();
        self.mini_month_month = today.month();
        self.rebuild_view_data();
    }

    /// Rebuild cached view data from `self.events`.
    /// Call after any state change (date, view, events loaded).
    pub fn rebuild_view_data(&mut self) {
        let events = &self.events;
        let today = Local::now().date_naive();

        self.dates_with_events.clear();
        for e in events {
            if let Some(start_dt) = chrono::DateTime::from_timestamp(e.start_time, 0) {
                let start_date = start_dt.with_timezone(&chrono::Local).date_naive();
                let end_dt = chrono::DateTime::from_timestamp(e.end_time, 0)
                    .map(|dt| dt.with_timezone(&chrono::Local).date_naive())
                    .unwrap_or(start_date);
                let mut d = start_date;
                while d <= end_dt {
                    self.dates_with_events.insert(d);
                    d += chrono::Duration::days(1);
                }
            }
        }

        let month_events: Vec<calendar_month::MonthEvent> = events
            .iter()
            .map(|e| calendar_month::MonthEvent {
                id: e.id.clone(),
                title: e.title.clone(),
                start_time: e.start_time,
                end_time: e.end_time,
                all_day: e.all_day,
                color: e.color.clone(),
            })
            .collect();

        self.month_grid = calendar_month::build_month_grid(
            self.mini_month_year,
            self.mini_month_month,
            &month_events,
            self.week_start,
            today,
        );

        self.time_grid_config = match self.active_view {
            CalendarView::Day => {
                calendar_time_grid::build_day_view(self.selected_date, events, today)
            }
            CalendarView::WorkWeek => {
                calendar_time_grid::build_work_week_view(self.selected_date, events, today)
            }
            CalendarView::Week => calendar_time_grid::build_week_view(
                self.selected_date,
                events,
                today,
                self.week_start,
            ),
            CalendarView::Month => {
                calendar_time_grid::build_day_view(self.selected_date, events, today)
            }
        };
    }
}
