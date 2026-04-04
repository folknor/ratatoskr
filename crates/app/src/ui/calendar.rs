//! Calendar view skeleton: layout, state, and messages.
//!
//! When the app is in Calendar mode, this module renders the two-panel
//! calendar layout: a sidebar (mini-month, view switcher, calendar list)
//! and a main content area dispatched by the active view.
//! Includes event detail popover and creation/editing overlays.

use std::collections::HashSet;

use chrono::{Datelike, Local, NaiveDate, Weekday};
use iced::widget::{
    Space, button, checkbox, column, container, mouse_area, pick_list, row, scrollable, text,
    text_input,
};
use iced::{Alignment, Element, Length, Theme};

use super::calendar_month;
use super::calendar_time_grid;
use super::layout::*;
use super::theme;
use super::undoable::UndoableText;
use super::undoable_text_input::undoable_text_input;
use crate::icon;

// ── Calendar view enum ─────────────────────────────────

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

// ── Workflow state ────────────────────────────────────

/// What the user is currently doing in the calendar feature.
///
/// Source of truth for event lifecycle meaning. Surfaces
/// (`active_popover`, `active_modal`) are presentation caches
/// synchronized from this state.
///
/// Which presentation surface is active for a `ViewingEvent` workflow.
///
/// Exists because `ExpandPopoverToModal` is the one transition where
/// workflow identity stays the same while presentation changes — the
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
    CreatingEvent {
        account_id: Option<String>,
        session: EditorSession,
    },
    /// Editing an existing event in the editor.
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

// ── Surface state ─────────────────────────────────────

/// The active calendar popover, if any.
#[derive(Debug, Clone)]
pub enum CalendarPopover {
    /// Quick-glance event popover (~300px compact card).
    EventDetail { event: CalendarEventData },
}

/// The active calendar modal, if any.
///
/// These are presentation state, not workflow state. The workflow
/// enum is the source of truth for lifecycle meaning. Modals are
/// synchronized from workflow state by the handler.
#[derive(Debug, Clone)]
pub enum CalendarModal {
    /// Full event detail modal (two-panel: 70% detail + 30% day view).
    EventFull { event: CalendarEventData },
    /// Editing or creating an event. Create-vs-edit semantics
    /// and the mutable draft come from `CalendarWorkflow`, not from
    /// this variant. This is a unit marker — the draft lives on
    /// `EditorSession` in the workflow state.
    ///
    /// **Invariant:** `EventEditor` without `CreatingEvent` or
    /// `EditingEvent` in workflow state is a contract violation.
    EventEditor,
    /// Delete confirmation dialog.
    ConfirmDelete {
        event_id: String,
        title: String,
        account_id: Option<String>,
    },
    /// Discard-unsaved-changes confirmation dialog.
    ConfirmDiscard { title: String },
}

/// Data for a calendar event in the UI layer.
///
/// Used for both detail display and editor form state.
/// Time fields are stored as raw strings during editing to avoid
/// fighting the user on intermediate keystroke states (e.g., typing
/// "12" would be parsed as "1" then "2" if parsed per-keystroke).
/// Parsed to integers only on save.

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
        // If starting at 23, end at 23:59 (can't wrap to next day in V1).
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

// ── Editor session types ──────────────────────────────

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
    /// The mutable draft — fields are updated as the user types.
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

// ── Calendar list entry ────────────────────────────────

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

// ── Calendar state ─────────────────────────────────────

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
    /// what the user is doing — surfaces are synchronized from this.
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

        // Rebuild dates_with_events for mini-month dot indicators.
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

        // Convert to month events for the month grid.
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
                // Keep existing config for non-grid views.
                calendar_time_grid::build_day_view(self.selected_date, events, today)
            }
        };
    }
}

// ── Messages ───────────────────────────────────────────

/// Which field changed in the event editor form.
#[derive(Debug, Clone)]
pub enum EventField {
    Title(String),
    Location(String),
    Description(String),
    StartHour(String),
    StartMinute(String),
    EndHour(String),
    EndMinute(String),
    AllDay(bool),
    /// Calendar selection carrying both calendar and account ownership.
    /// `account_id` comes from `CalendarListEntry.account_id` at selection
    /// time — not reconstructed from a later lookup.
    CalendarSelected {
        calendar_id: Option<String>,
        account_id: Option<String>,
    },
    Timezone(Option<String>),
    Availability(Option<String>),
    Visibility(Option<String>),
    RecurrenceRule(Option<String>),
}

/// Identifies a text field in the event editor for undo/redo.
#[derive(Debug, Clone, Copy)]
pub enum EventTextField {
    Title,
    Location,
    Description,
}

#[derive(Debug, Clone)]
pub enum CalendarMessage {
    /// A date was clicked in the mini-month or main view.
    SelectDate(NaiveDate),
    /// A time slot was clicked in day/week views (for event creation pre-fill).
    SelectSlot(NaiveDate, u32),
    /// A time slot was double-clicked — open event creation dialog.
    DoubleClickSlot(NaiveDate, u32),
    /// Switch the active calendar view.
    SetView(CalendarView),
    /// Navigate mini-month backward.
    PrevMonth,
    /// Navigate mini-month forward.
    NextMonth,
    /// Jump to today.
    Today,
    /// An event was clicked (event ID).
    EventClicked(String),
    /// Close the active popover.
    ClosePopover,
    /// Close the active modal.
    CloseModal,
    /// Expand the event-detail popover into a full modal.
    ExpandPopoverToModal,
    /// Open the event editor. `None` = create new event.
    OpenEventEditor(Option<CalendarEventData>),
    /// A field in the event editor changed.
    EventFieldChanged(EventField),
    /// Undo the last edit to a text field in the event editor.
    EventFieldUndo(EventTextField),
    /// Redo a previously undone edit to a text field in the event editor.
    EventFieldRedo(EventTextField),
    /// Save the event (create or update).
    SaveEvent,
    /// Async save completed.
    EventSaved(Result<(), String>),
    /// Start deleting an event (shows confirmation).
    ConfirmDeleteEvent {
        event_id: String,
        title: String,
        account_id: Option<String>,
    },
    /// User confirmed deletion. The String payload is transitional —
    /// the handler reads identity from workflow state (ConfirmingDelete).
    DeleteEvent(String),
    /// User confirmed discarding unsaved editor changes.
    DiscardChanges,
    /// Async delete completed.
    EventDeleted(Result<(), String>),
    /// Create a new event (from command palette or UI action).
    CreateEvent,
    /// Event detail was loaded from DB after clicking an event.
    EventLoaded(Result<CalendarEventData, String>),
    /// Calendar events loaded from DB for view rendering.
    /// The token is a generation guard — stale results are discarded.
    EventsLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::Calendar>,
        Result<Vec<calendar_time_grid::TimeGridEvent>, String>,
    ),
    /// Switch back to mail mode.
    SwitchToMail,
    /// No-op event sink for modal blocker (iced requires on_press to capture).
    Noop,
    /// Pop out the calendar into a separate window.
    PopOutCalendar,
    /// Toggle visibility of a calendar (checkbox in sidebar).
    ToggleCalendarVisibility(String, bool),
    /// Calendars loaded from DB for sidebar list.
    /// The token is a generation guard — stale results are discarded.
    CalendarsLoaded(
        rtsk::generation::GenerationToken<rtsk::generation::Calendar>,
        Result<Vec<CalendarListEntry>, String>,
    ),
}

// ── View ───────────────────────────────────────────────

/// Render the full calendar layout (sidebar + main area + calendar surfaces).
///
/// Returns an `Element<CalendarMessage>` — the parent maps this to the
/// top-level app Message.
pub fn calendar_layout(state: &CalendarState) -> Element<'_, CalendarMessage> {
    let sidebar = calendar_sidebar(state);
    let main_view = calendar_main_view(state);

    let base = row![sidebar, main_view].height(Length::Fill);

    if let Some(modal) = &state.active_modal {
        let card = match modal {
            CalendarModal::EventFull { event } => event_full_modal(event, state),
            CalendarModal::EventEditor => {
                let (draft, is_creating) = match &state.workflow {
                    CalendarWorkflow::CreatingEvent { session, .. } => (&session.draft, true),
                    CalendarWorkflow::EditingEvent { session, .. } => (&session.draft, false),
                    other => {
                        debug_assert!(
                            false,
                            "EventEditor modal without editing workflow: {other:?}"
                        );
                        log::error!("EventEditor modal without editing workflow state");
                        return container(text("")).into();
                    }
                };
                event_editor_card(draft, is_creating, &state.calendars)
            }
            CalendarModal::ConfirmDelete {
                event_id, title, ..
            } => delete_confirm_card(event_id, title),
            CalendarModal::ConfirmDiscard { title } => discard_confirm_card(title),
        };
        crate::ui::modal_overlay::modal_overlay(
            base,
            card,
            crate::ui::modal_overlay::ModalSurface::Modal,
            CalendarMessage::Noop,
        )
    } else if let Some(popover) = &state.active_popover {
        match popover {
            CalendarPopover::EventDetail { event } => {
                popover_stack(base.into(), event_detail_popover(event))
            }
        }
    } else {
        base.into()
    }
}

/// Wrap a base layout with a lightweight popover (click-away backdrop, right-aligned).
fn popover_stack<'a>(
    base: Element<'a, CalendarMessage>,
    card: Element<'a, CalendarMessage>,
) -> Element<'a, CalendarMessage> {
    let backdrop = mouse_area(container("").width(Length::Fill).height(Length::Fill))
        .on_press(CalendarMessage::ClosePopover);

    // Position the popover toward the right side of the view.
    let positioned = container(container(card).align_y(Alignment::Center).max_width(320.0))
        .width(Length::Fill)
        .height(Length::Fill)
        .align_x(Alignment::End)
        .align_y(Alignment::Center)
        .padding(iced::Padding::from([SPACE_LG, SPACE_LG]));

    iced::widget::stack![base, backdrop, positioned].into()
}


// ── Event detail card ──────────────────────────────────

/// Compact event detail popover (~300px, quick glance).
fn event_detail_popover(event: &CalendarEventData) -> Element<'_, CalendarMessage> {
    let mut content = column![].spacing(SPACE_SM);

    // Title row with ↗ expand-to-modal button
    let title_text = if event.title.is_empty() {
        "(Untitled event)"
    } else {
        &event.title
    };
    let expand_btn = button(text("\u{2197}").size(TEXT_MD))
        .on_press(CalendarMessage::ExpandPopoverToModal)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let close_btn = button(text("\u{2715}").size(TEXT_SM))
        .on_press(CalendarMessage::ClosePopover)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    let title_row = row![
        container(
            text(title_text)
                .size(TEXT_HEADING)
                .font(crate::font::text_semibold()),
        )
        .width(Length::Fill),
        expand_btn,
        close_btn,
    ]
    .align_y(Alignment::Center);
    content = content.push(title_row);

    // Time
    let time_label = format_event_time_range(event);
    content = content.push(text(time_label).size(TEXT_MD).style(text::secondary));

    // Location (hidden if empty)
    if !event.location.is_empty() {
        content = content.push(text(&event.location).size(TEXT_MD).style(text::secondary));
    }

    // Recurrence (hidden if none)
    if let Some(ref rrule) = event.recurrence_rule {
        content = content.push(
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

    // Organizer (hidden if own event / empty)
    if let Some(ref organizer) = event.organizer_name {
        if !organizer.is_empty() {
            content = content.push(
                text(format!("Invited by {organizer}"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
    } else if let Some(ref email) = event.organizer_email {
        if !email.is_empty() {
            content = content.push(
                text(format!("Invited by {email}"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
    }

    // Attendees (hidden if empty)
    if !event.attendees.is_empty() {
        let mut attendee_col = column![].spacing(SPACE_XXXS);
        for att in &event.attendees {
            let status_icon = match att.rsvp_status.as_str() {
                "accepted" => "\u{2713}",
                "declined" => "\u{2717}",
                "tentative" => "~",
                _ => "?",
            };
            let display = att.name.as_deref().unwrap_or(&att.email);
            let suffix = if att.is_organizer { " (organizer)" } else { "" };
            attendee_col = attendee_col.push(
                text(format!("{status_icon} {display}{suffix}"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
        content = content.push(attendee_col);
    }

    // Description (hidden if empty)
    if !event.description.is_empty() {
        content = content.push(
            text(&event.description)
                .size(TEXT_SM)
                .style(theme::TextClass::Muted.style()),
        );
    }

    // Reminders (hidden if empty)
    if !event.reminders.is_empty() {
        let reminder_text = event
            .reminders
            .iter()
            .map(|r| format_reminder(r.minutes_before))
            .collect::<Vec<_>>()
            .join(", ");
        content = content.push(
            text(format!("Reminders: {reminder_text}"))
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }

    // RSVP status + action buttons (context-dependent)
    if let Some(ref status) = event.rsvp_status {
        let status_display = match status.as_str() {
            "accepted" => "Accepted",
            "declined" => "Declined",
            "tentative" => "Tentative",
            "needs-action" => "No response",
            _ => status.as_str(),
        };
        content = content.push(
            text(format!("Your RSVP: {status_display}"))
                .size(TEXT_SM)
                .font(crate::font::text_semibold())
                .style(text::secondary),
        );
    }

    // Action buttons: Edit, Delete
    let edit_btn = button(text("Edit").size(TEXT_SM))
        .on_press(CalendarMessage::OpenEventEditor(Some(event.clone())))
        .padding(PAD_BUTTON)
        .style(theme::ButtonClass::Ghost.style());

    let delete_btn = if let Some(ref id) = event.id {
        button(text("Delete").size(TEXT_SM))
            .on_press(CalendarMessage::ConfirmDeleteEvent {
                event_id: id.clone(),
                title: event.title.clone(),
                account_id: event.account_id.clone(),
            })
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style())
    } else {
        button(text("Delete").size(TEXT_SM))
            .padding(PAD_BUTTON)
            .style(theme::ButtonClass::Ghost.style())
    };

    content = content.push(Space::new().height(SPACE_XS));
    content = content.push(row![edit_btn, delete_btn].spacing(SPACE_XS));

    let scrollable_content = scrollable(content).height(Length::Shrink);

    container(scrollable_content)
        .width(Length::Fixed(300.0))
        .max_height(CALENDAR_OVERLAY_MAX_HEIGHT)
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

// ── Event full modal ─────────────────────────────────

/// Full event detail modal (two-panel: ~70% detail + ~30% mini day view).
fn event_full_modal<'a>(
    event: &'a CalendarEventData,
    state: &'a CalendarState,
) -> Element<'a, CalendarMessage> {
    // ── Left panel: full event details ──
    let mut detail = column![].spacing(SPACE_SM);

    // Close button row
    let close_btn = button(text("\u{2715}").size(TEXT_SM))
        .on_press(CalendarMessage::CloseModal)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    // Calendar name + color
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

    // Title
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

    // Date and time
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

    // Timezone
    if let Some(ref tz) = event.timezone {
        if !tz.is_empty() {
            datetime_row = datetime_row.push(
                text(tz)
                    .size(TEXT_SM)
                    .style(theme::TextClass::Muted.style()),
            );
        }
    }
    detail = detail.push(datetime_row);

    // Recurrence
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

    // Location (clickable if URL)
    if !event.location.is_empty() {
        let loc_text =
            if event.location.starts_with("http://") || event.location.starts_with("https://") {
                text(&event.location).size(TEXT_MD).style(text::primary)
            } else {
                text(&event.location).size(TEXT_MD).style(text::secondary)
            };
        detail = detail.push(loc_text);
    }

    // Organizer
    if let Some(ref name) = event.organizer_name {
        if !name.is_empty() {
            detail = detail.push(
                text(format!("Organizer: {name}"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
    } else if let Some(ref email) = event.organizer_email {
        if !email.is_empty() {
            detail = detail.push(
                text(format!("Organizer: {email}"))
                    .size(TEXT_SM)
                    .style(text::secondary),
            );
        }
    }

    // Attendees
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

    // Description (full, not truncated)
    if !event.description.is_empty() {
        detail = detail.push(Space::new().height(SPACE_XXS));
        detail = detail.push(
            text(&event.description)
                .size(TEXT_SM)
                .style(text::secondary),
        );
    }

    // Reminders
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

    // RSVP status
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

    // Action buttons
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

    // ── Right panel: mini day view showing conflicts ──
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

    // ── Assemble two-panel layout ──
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

// ── Event editor card ──────────────────────────────────

/// Event creation/editing form (rendered as a centered modal).
fn event_editor_card<'a>(
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

    // Heading + close button
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

    // Calendar selector (spec: first field)
    // Enabled for new events; read-only for existing events (move semantics deferred).
    if is_creating {
        let selected = calendars
            .iter()
            .find(|c| Some(&c.id) == event.calendar_id.as_ref())
            .cloned();
        let options: Vec<CalendarListEntry> = calendars.to_vec();
        let picker = pick_list(selected, options, |c: &CalendarListEntry| {
            if let Some(ref name) = c.name {
                name.clone()
            } else {
                c.id.clone()
            }
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

    // Title
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

    // Date (read-only display for V1)
    let date_str = format!(
        "{}, {} {}, {}",
        weekday_short(event.start_date.weekday()),
        month_short(event.start_date.month()),
        event.start_date.day(),
        event.start_date.year(),
    );
    content = content.push(form_field("Date", text(date_str).size(TEXT_MD).into()));

    // Time row (start hour:minute - end hour:minute)
    if !event.all_day {
        let time_row = time_input_row(event);
        content = content.push(form_field("Time", time_row));
    }

    // All-day toggle
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

    // Location
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

    // Description
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

    // Timezone
    let tz_display = event.timezone.as_deref().unwrap_or("Local");
    let tz_input = text_input("Timezone (e.g. Europe/Oslo)", tz_display)
        .on_input(|s| {
            let tz = if s.is_empty() { None } else { Some(s) };
            CalendarMessage::EventFieldChanged(EventField::Timezone(tz))
        })
        .padding(PAD_INPUT)
        .size(TEXT_SM);
    content = content.push(form_field("Timezone", tz_input.into()));

    // Availability dropdown (free / busy / tentative / out of office)
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

    // Visibility (public / private)
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

    // Recurrence (basic toggle + text display)
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

    // Action buttons — save is disabled when no calendar is selected.
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

    content = content.push(Space::new().height(SPACE_XS));
    content = content.push(row![save_btn, cancel_btn].spacing(SPACE_XS));

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
    label: &'a str,
    widget: Element<'a, CalendarMessage>,
) -> Element<'a, CalendarMessage> {
    row![
        container(
            text(label)
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

// ── Delete confirmation card ───────────────────────────

/// Confirmation dialog before deleting an event.
fn delete_confirm_card<'a>(event_id: &str, title: &str) -> Element<'a, CalendarMessage> {
    let display_title = if title.is_empty() {
        "(Untitled)"
    } else {
        title
    };
    let id = event_id.to_string();

    let content = column![
        text("Delete Event")
            .size(TEXT_HEADING)
            .font(crate::font::text_semibold()),
        Space::new().height(SPACE_XS),
        text(format!(
            "Delete \"{display_title}\"? This cannot be undone."
        ))
        .size(TEXT_MD),
        Space::new().height(SPACE_MD),
        row![
            button(text("Delete").size(TEXT_SM))
                .on_press(CalendarMessage::DeleteEvent(id))
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Nav { active: true }.style()),
            button(text("Cancel").size(TEXT_SM))
                .on_press(CalendarMessage::CloseModal)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style()),
        ]
        .spacing(SPACE_XS),
    ]
    .spacing(SPACE_XXS);

    container(content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

// ── Discard confirmation card ─────────────────────────

/// Confirmation dialog before discarding unsaved editor changes.
fn discard_confirm_card(title: &str) -> Element<'_, CalendarMessage> {
    let display_title = if title.is_empty() {
        "Discard unsaved changes?"
    } else {
        title
    };

    let content = column![
        text("Unsaved Changes")
            .size(TEXT_HEADING)
            .font(crate::font::text_semibold()),
        Space::new().height(SPACE_XS),
        text(display_title).size(TEXT_MD),
        Space::new().height(SPACE_MD),
        row![
            button(text("Discard").size(TEXT_SM))
                .on_press(CalendarMessage::DiscardChanges)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Nav { active: true }.style()),
            button(text("Cancel").size(TEXT_SM))
                .on_press(CalendarMessage::CloseModal)
                .padding(PAD_BUTTON)
                .style(theme::ButtonClass::Ghost.style()),
        ]
        .spacing(SPACE_XS),
    ]
    .spacing(SPACE_XXS);

    container(content)
        .width(Length::Fixed(CALENDAR_OVERLAY_WIDTH))
        .padding(PAD_CARD)
        .style(theme::ContainerClass::Elevated.style())
        .into()
}

// ── Calendar sidebar ───────────────────────────────────

/// Calendar sidebar: mini-month, view switcher, calendar list placeholder.
fn calendar_sidebar(state: &CalendarState) -> Element<'_, CalendarMessage> {
    let today = Local::now().date_naive();

    // Mini-month
    let mini = super::calendar_month::mini_month(
        state.mini_month_year,
        state.mini_month_month,
        Some(state.selected_date),
        today,
        state.week_start,
        &state.dates_with_events,
        |d| CalendarMessage::SelectDate(d),
        CalendarMessage::PrevMonth,
        CalendarMessage::NextMonth,
    );

    // Today button
    let today_btn = button(text("Today").size(TEXT_SM).style(text::secondary))
        .on_press(CalendarMessage::Today)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    // New event button
    let new_event_btn = button(text("+ New Event").size(TEXT_SM).style(text::primary))
        .on_press(CalendarMessage::CreateEvent)
        .padding(PAD_ICON_BTN)
        .style(theme::ButtonClass::Ghost.style());

    // Calendar list with visibility toggles
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

            // Color dot + name + checkbox
            let color_dot = text("\u{25CF}")
                .size(TEXT_SM)
                .color(parse_hex_color(&cal.color));

            let name = text(&cal.display_name).size(TEXT_XS).style(text::base);

            let toggle = checkbox(is_visible)
                .on_toggle(move |checked| {
                    CalendarMessage::ToggleCalendarVisibility(cal_id.clone(), checked)
                })
                .size(12)
                .spacing(0);

            let cal_row = row![toggle, color_dot, name]
                .spacing(SPACE_XXS)
                .align_y(Alignment::Center);

            calendar_list_col = calendar_list_col.push(cal_row);
        }
    }

    let calendar_list = container(calendar_list_col).padding(SPACE_XS);

    // Mode toggle button (tall square, same as mail sidebar)
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
    .width(SIDEBAR_HEADER_HEIGHT) // square
    .height(Length::Fill);

    // View switcher + pop-out stacked to the right
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

    // Fixed-height header so Fill children resolve correctly
    let header = container(
        row![mode_btn, right_stack]
            .spacing(SPACE_XXS)
            .width(Length::Fill)
            .height(Length::Fill),
    )
    .height(SIDEBAR_HEADER_HEIGHT)
    .width(Length::Fill);

    // Pop-out button at the bottom (styled like settings button)
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

/// Calendar main content area: dispatches to the appropriate view.
fn calendar_main_view(state: &CalendarState) -> Element<'_, CalendarMessage> {
    match state.active_view {
        CalendarView::Month => calendar_main_month(state),
        _ => calendar_main_time_grid(state),
    }
}

/// Month view main content area.
fn calendar_main_month(state: &CalendarState) -> Element<'_, CalendarMessage> {
    container(calendar_month::month_view(
        &state.month_grid,
        |d| CalendarMessage::SelectDate(d),
        |id| CalendarMessage::EventClicked(id.to_string()),
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style())
    .into()
}

/// Day / Work Week / Week time grid main content area.
fn calendar_main_time_grid(state: &CalendarState) -> Element<'_, CalendarMessage> {
    container(calendar_time_grid::time_grid_view(
        &state.time_grid_config,
        |id| CalendarMessage::EventClicked(id.to_string()),
        |date, hour| CalendarMessage::SelectSlot(date, hour),
    ))
    .width(Length::Fill)
    .height(Length::Fill)
    .style(theme::ContainerClass::Content.style())
    .into()
}

// ── Helpers ────────────────────────────────────────────

/// Format time range like "10:00 - 11:30" or "All day".
fn format_event_time_range(event: &CalendarEventData) -> String {
    if event.all_day {
        return "All day".to_string();
    }
    format!(
        "{:02}:{:02} \u{2013} {:02}:{:02}",
        event.start_hour_u32(),
        event.start_minute_u32(),
        event.end_hour_u32(),
        event.end_minute_u32(),
    )
}

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

fn month_short(month: u32) -> &'static str {
    match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        12 => "Dec",
        _ => "???",
    }
}

/// Format a recurrence rule for display.
fn format_recurrence_rule(rrule: &str) -> String {
    // Strip "RRULE:" prefix if present.
    let rule = rrule.strip_prefix("RRULE:").unwrap_or(rrule);
    // Basic human-readable parsing of common patterns.
    let mut freq = "";
    let mut interval = 1u32;
    for part in rule.split(';') {
        if let Some(val) = part.strip_prefix("FREQ=") {
            freq = val;
        }
        if let Some(val) = part.strip_prefix("INTERVAL=") {
            interval = val.parse().unwrap_or(1);
        }
    }
    let freq_label = match freq {
        "DAILY" => "day",
        "WEEKLY" => "week",
        "MONTHLY" => "month",
        "YEARLY" => "year",
        _ => return rule.to_string(),
    };
    if interval <= 1 {
        format!("Every {freq_label}")
    } else {
        format!("Every {interval} {freq_label}s")
    }
}

/// Format a reminder as human-readable text.
fn format_reminder(minutes_before: i64) -> String {
    if minutes_before <= 0 {
        "At time of event".to_string()
    } else if minutes_before < 60 {
        format!("{minutes_before} min before")
    } else if minutes_before < 1440 {
        let hours = minutes_before / 60;
        if hours == 1 {
            "1 hour before".to_string()
        } else {
            format!("{hours} hours before")
        }
    } else {
        let days = minutes_before / 1440;
        if days == 1 {
            "1 day before".to_string()
        } else {
            format!("{days} days before")
        }
    }
}

/// Parse a hex color string (e.g. "#4285f4") to an iced Color.
fn parse_hex_color(hex: &str) -> iced::Color {
    let hex = hex.trim_start_matches('#');
    if hex.len() >= 6 {
        let r = u8::from_str_radix(&hex[0..2], 16).unwrap_or(100);
        let g = u8::from_str_radix(&hex[2..4], 16).unwrap_or(100);
        let b = u8::from_str_radix(&hex[4..6], 16).unwrap_or(100);
        iced::Color::from_rgb8(r, g, b)
    } else {
        iced::Color::from_rgb8(100, 100, 200)
    }
}
