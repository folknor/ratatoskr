use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Timelike};
use iced::Task;

use crate::ui::calendar::{
    AttendeeEntry, CalendarEventData, CalendarMessage, CalendarOverlay, EventField,
    EventTextField, ReminderEntry,
};
use crate::{App, Message};

impl App {
    pub(crate) fn handle_calendar(&mut self, cal_msg: CalendarMessage) -> Task<Message> {
        match cal_msg {
            CalendarMessage::SelectDate(date) => {
                self.calendar.selected_date = date;
                self.calendar.mini_month_year = date.year();
                self.calendar.mini_month_month = date.month();
                self.calendar.rebuild_view_data();
                Task::none()
            }
            CalendarMessage::SetView(view) => {
                self.calendar.active_view = view;
                self.calendar.rebuild_view_data();
                Task::none()
            }
            CalendarMessage::PrevMonth => {
                self.calendar.prev_month();
                Task::none()
            }
            CalendarMessage::NextMonth => {
                self.calendar.next_month();
                Task::none()
            }
            CalendarMessage::Today => {
                self.calendar.go_to_today();
                Task::none()
            }
            CalendarMessage::SelectSlot(date, hour) => {
                self.calendar.selected_date = date;
                self.calendar.selected_hour = Some(hour);
                self.calendar.rebuild_view_data();
                Task::none()
            }
            CalendarMessage::DoubleClickSlot(date, hour) => {
                // Double-click opens event creation dialog with time pre-filled.
                let event = CalendarEventData::new_at(date, hour);
                self.calendar.reset_editor_undo(&event);
                self.calendar.overlay = CalendarOverlay::EventEditor {
                    event,
                    is_new: true,
                    original_title: String::new(),
                };
                Task::none()
            }
            CalendarMessage::EventClicked(event_id) => {
                let db = Arc::clone(&self.db);
                let eid = event_id.clone();
                // Also find the event in cached events for color/calendar_name.
                let cached_event = self.calendar.events.iter().find(|e| e.id == eid).cloned();
                Task::perform(
                    async move {
                        let ev = db.get_calendar_event(eid).await?;
                        let Some(ev) = ev else {
                            return Err(format!("Event not found: {event_id}"));
                        };
                        let attendees = db.get_event_attendees(
                            ev.account_id.clone(),
                            ev.id.clone(),
                        ).await.unwrap_or_default();
                        let reminders = db.get_event_reminders(
                            ev.account_id.clone(),
                            ev.id.clone(),
                        ).await.unwrap_or_default();
                        let mut data = db_event_to_calendar_data(&ev);
                        data.attendees = attendees;
                        data.reminders = reminders;
                        if let Some(cached) = cached_event {
                            data.calendar_name = cached.calendar_name;
                            data.color = Some(cached.color);
                        }
                        Ok(data)
                    },
                    |result| Message::Calendar(Box::new(CalendarMessage::EventLoaded(result))),
                )
            }
            CalendarMessage::EventLoaded(result) => {
                match result {
                    Ok(data) => {
                        self.calendar.overlay =
                            CalendarOverlay::EventDetail { event: data };
                    }
                    Err(e) => {
                        log::error!("Failed to load calendar event: {e}");
                        self.status = format!("Failed to load event: {e}");
                    }
                }
                Task::none()
            }
            CalendarMessage::CloseOverlay => {
                // Check for unsaved changes in the editor.
                if let CalendarOverlay::EventEditor { ref event, ref original_title, .. } = self.calendar.overlay {
                    if event.title != *original_title
                        || !event.description.is_empty()
                        || !event.location.is_empty()
                    {
                        // Has changes — show confirmation.
                        self.calendar.overlay = CalendarOverlay::ConfirmDelete {
                            event_id: "__discard__".to_string(),
                            title: "Discard unsaved changes?".to_string(),
                        };
                        return Task::none();
                    }
                }
                self.calendar.overlay = CalendarOverlay::None;
                Task::none()
            }
            CalendarMessage::ExpandToFullModal => {
                if let CalendarOverlay::EventDetail { event } = &self.calendar.overlay {
                    let event = event.clone();
                    self.calendar.overlay = CalendarOverlay::EventFullModal { event };
                }
                Task::none()
            }
            CalendarMessage::OpenEventEditor(data) => {
                let (event, is_new) = match data {
                    Some(e) => {
                        let new = e.id.is_none();
                        (e, new)
                    }
                    None => {
                        let date = self.calendar.selected_date;
                        let hour = self.calendar.selected_hour.unwrap_or(9);
                        (CalendarEventData::new_at(date, hour), true)
                    }
                };
                self.calendar.reset_editor_undo(&event);
                let original_title = event.title.clone();
                self.calendar.overlay =
                    CalendarOverlay::EventEditor { event, is_new, original_title };
                Task::none()
            }
            CalendarMessage::CreateEvent => {
                let date = self.calendar.selected_date;
                let hour = self.calendar.selected_hour.unwrap_or(9);
                let event = CalendarEventData::new_at(date, hour);
                self.calendar.reset_editor_undo(&event);
                self.calendar.overlay = CalendarOverlay::EventEditor {
                    event,
                    is_new: true,
                    original_title: String::new(),
                };
                Task::none()
            }
            CalendarMessage::EventFieldChanged(field) => {
                self.handle_event_field_changed(field);
                Task::none()
            }
            CalendarMessage::EventFieldUndo(text_field) => {
                self.handle_event_field_undo(text_field);
                Task::none()
            }
            CalendarMessage::EventFieldRedo(text_field) => {
                self.handle_event_field_redo(text_field);
                Task::none()
            }
            CalendarMessage::SaveEvent => self.handle_save_event(),
            CalendarMessage::EventSaved(result) => {
                match result {
                    Ok(()) => {
                        log::info!("Calendar event saved");
                        self.calendar.overlay = CalendarOverlay::None;
                        return self.reload_calendar_events();
                    }
                    Err(e) => {
                        log::error!("Failed to save calendar event: {e}");
                        self.status = format!("Save failed: {e}");
                    }
                }
                Task::none()
            }
            CalendarMessage::ConfirmDeleteEvent(id, title) => {
                self.calendar.overlay =
                    CalendarOverlay::ConfirmDelete { event_id: id, title };
                Task::none()
            }
            CalendarMessage::DeleteEvent(event_id) => {
                // Handle discard-unsaved-changes sentinel.
                if event_id == "__discard__" {
                    self.calendar.overlay = CalendarOverlay::None;
                    return Task::none();
                }
                let Some(ctx) = self.action_ctx() else {
                    return Task::none();
                };
                // Resolve account_id from the event editor overlay if available,
                // fall back to first account.
                let account_id = match &self.calendar.overlay {
                    CalendarOverlay::ConfirmDelete { .. } | CalendarOverlay::EventEditor { .. } => {
                        // Try to get account_id from the event data
                        if let CalendarOverlay::EventEditor { event, .. } = &self.calendar.overlay {
                            event.account_id.clone()
                        } else {
                            None
                        }
                    }
                    _ => None,
                }
                .or_else(|| self.sidebar.accounts.first().map(|a| a.id.clone()))
                .unwrap_or_default();

                self.calendar.overlay = CalendarOverlay::None;
                Task::perform(
                    async move {
                        let outcome = ratatoskr_calendar::actions::delete_calendar_event(
                            &ctx, &account_id, &event_id,
                        )
                        .await;
                        calendar_outcome_to_result(outcome)
                    },
                    |r| Message::Calendar(Box::new(CalendarMessage::EventDeleted(r))),
                )
            }
            CalendarMessage::EventDeleted(result) => {
                match result {
                    Ok(()) => {
                        log::info!("Calendar event deleted");
                        return self.reload_calendar_events();
                    }
                    Err(e) => {
                        log::error!("Failed to delete calendar event: {e}");
                        self.status = format!("Delete failed: {e}");
                    }
                }
                Task::none()
            }
            CalendarMessage::SwitchToMail => {
                return self.update(Message::SetAppMode(crate::AppMode::Mail));
            }
            CalendarMessage::PopOutCalendar => {
                // Check if a calendar pop-out already exists.
                let existing = self.pop_out_windows.values().any(|w| {
                    matches!(w, crate::pop_out::PopOutWindow::Calendar)
                });
                if existing {
                    // Bring existing pop-out to foreground.
                    // (iced doesn't have a bring-to-front API, so this is a no-op for now)
                    return Task::none();
                }
                // Open new calendar pop-out window.
                let settings = iced::window::Settings {
                    size: iced::Size::new(1024.0, 768.0),
                    ..Default::default()
                };
                let (id, open_task) = iced::window::open(settings);
                self.pop_out_windows.insert(id, crate::pop_out::PopOutWindow::Calendar);
                // Switch main window back to mail mode.
                self.app_mode = crate::AppMode::Mail;
                open_task.discard()
            }
            CalendarMessage::EventsLoaded(load_generation, result) => {
                if !self.calendar.load_generation.is_current(load_generation) {
                    // Stale result from a previous load — discard.
                    return Task::none();
                }
                match result {
                    Ok(events) => {
                        self.calendar.events = events;
                        self.calendar.rebuild_view_data();
                    }
                    Err(e) => {
                        log::error!("Failed to load calendar events: {e}");
                        self.status = format!("Load events error: {e}");
                    }
                }
                Task::none()
            }
            CalendarMessage::ToggleCalendarVisibility(calendar_id, visible) => {
                // Update local state immediately for responsiveness.
                if let Some(cal) = self.calendar.calendars.iter_mut().find(|c| c.id == calendar_id) {
                    cal.is_visible = visible;
                }
                // Persist to DB and reload events.
                let db = Arc::clone(&self.db);
                Task::perform(
                    async move {
                        db.set_calendar_visibility(calendar_id, visible).await
                    },
                    |_| Message::Calendar(Box::new(CalendarMessage::EventSaved(Ok(())))),
                )
            }
            CalendarMessage::CalendarsLoaded(load_generation, result) => {
                if !self.calendar.load_generation.is_current(load_generation) {
                    // Stale result from a previous load — discard.
                    return Task::none();
                }
                match result {
                    Ok(calendars) => {
                        self.calendar.calendars = calendars;
                    }
                    Err(e) => {
                        log::error!("Failed to load calendars: {e}");
                    }
                }
                Task::none()
            }
        }
    }

    fn handle_event_field_changed(&mut self, field: EventField) {
        if let CalendarOverlay::EventEditor { ref mut event, .. } = self.calendar.overlay {
            match field {
                EventField::Title(s) => {
                    self.calendar.editor_undo_title.set_text(s.clone());
                    event.title = s;
                }
                EventField::Location(s) => {
                    self.calendar.editor_undo_location.set_text(s.clone());
                    event.location = s;
                }
                EventField::Description(s) => {
                    self.calendar.editor_undo_description.set_text(s.clone());
                    event.description = s;
                }
                EventField::StartHour(s) => event.start_hour = s,
                EventField::StartMinute(s) => event.start_minute = s,
                EventField::EndHour(s) => event.end_hour = s,
                EventField::EndMinute(s) => event.end_minute = s,
                EventField::AllDay(b) => event.all_day = b,
                EventField::CalendarId(id) => event.calendar_id = id,
                EventField::Timezone(tz) => event.timezone = tz,
                EventField::Availability(a) => event.availability = a,
                EventField::Visibility(v) => event.visibility = v,
                EventField::RecurrenceRule(r) => event.recurrence_rule = r,
            }
        }
    }

    fn handle_event_field_undo(&mut self, text_field: EventTextField) {
        if let CalendarOverlay::EventEditor { ref mut event, .. } = self.calendar.overlay {
            match text_field {
                EventTextField::Title => {
                    if let Some(t) = self.calendar.editor_undo_title.undo() {
                        event.title = t.to_owned();
                    }
                }
                EventTextField::Location => {
                    if let Some(t) = self.calendar.editor_undo_location.undo() {
                        event.location = t.to_owned();
                    }
                }
                EventTextField::Description => {
                    if let Some(t) = self.calendar.editor_undo_description.undo() {
                        event.description = t.to_owned();
                    }
                }
            }
        }
    }

    fn handle_event_field_redo(&mut self, text_field: EventTextField) {
        if let CalendarOverlay::EventEditor { ref mut event, .. } = self.calendar.overlay {
            match text_field {
                EventTextField::Title => {
                    if let Some(t) = self.calendar.editor_undo_title.redo() {
                        event.title = t.to_owned();
                    }
                }
                EventTextField::Location => {
                    if let Some(t) = self.calendar.editor_undo_location.redo() {
                        event.location = t.to_owned();
                    }
                }
                EventTextField::Description => {
                    if let Some(t) = self.calendar.editor_undo_description.redo() {
                        event.description = t.to_owned();
                    }
                }
            }
        }
    }

    fn handle_save_event(&mut self) -> Task<Message> {
        let event = match &self.calendar.overlay {
            CalendarOverlay::EventEditor { event, .. } => event.clone(),
            _ => return Task::none(),
        };

        let Some(ctx) = self.action_ctx() else {
            return Task::none();
        };

        let start_ts = calendar_data_to_timestamp(
            event.start_date,
            event.start_hour_u32(),
            event.start_minute_u32(),
        );
        let end_ts = calendar_data_to_timestamp(
            event.start_date,
            event.end_hour_u32(),
            event.end_minute_u32(),
        );

        let account_id = event
            .account_id
            .clone()
            .or_else(|| {
                self.sidebar.accounts.first().map(|a| a.id.clone())
            })
            .unwrap_or_default();

        let input = ratatoskr_calendar::actions::CalendarEventInput {
            title: event.title.clone(),
            description: event.description.clone(),
            location: event.location.clone(),
            start_time: start_ts,
            end_time: end_ts,
            is_all_day: event.all_day,
            timezone: event.timezone.clone(),
            recurrence_rule: event.recurrence_rule.clone(),
            availability: event.availability.clone(),
            visibility: event.visibility.clone(),
        };

        if let Some(id) = event.id.clone() {
            let aid = account_id.clone();
            Task::perform(
                async move {
                    let outcome = ratatoskr_calendar::actions::update_calendar_event(
                        &ctx, &aid, &id, input,
                    )
                    .await;
                    calendar_outcome_to_result(outcome)
                },
                |r| Message::Calendar(Box::new(CalendarMessage::EventSaved(r))),
            )
        } else {
            let cal_id = event.calendar_id.clone().unwrap_or_default();
            let aid = account_id.clone();
            Task::perform(
                async move {
                    let outcome = ratatoskr_calendar::actions::create_calendar_event(
                        &ctx, &aid, &cal_id, input,
                    )
                    .await;
                    calendar_outcome_to_result(outcome)
                },
                |r| Message::Calendar(Box::new(CalendarMessage::EventSaved(r))),
            )
        }
    }

    /// Reload calendar events from DB and rebuild views.
    ///
    /// Increments the load generation counter so that results from
    /// previously-dispatched (now stale) loads are discarded.
    pub(crate) fn reload_calendar_events(&mut self) -> Task<Message> {
        let load_generation = self.calendar.load_generation.next();
        let db = Arc::clone(&self.db);
        let db2 = Arc::clone(&self.db);
        Task::batch([
            Task::perform(
                async move { db.load_calendar_events_for_view().await },
                move |r| Message::Calendar(Box::new(CalendarMessage::EventsLoaded(load_generation, r))),
            ),
            Task::perform(
                async move { db2.load_calendars_for_sidebar().await },
                move |r| Message::Calendar(Box::new(CalendarMessage::CalendarsLoaded(load_generation, r))),
            ),
        ])
    }
}

/// Map ActionOutcome to the Result<(), String> that CalendarMessage expects.
///
/// LocalOnly maps to Ok(()) — the event is visible locally, the overlay closes.
/// Phase 3 can add richer outcome reporting for the "saved locally, not synced" case.
fn calendar_outcome_to_result(
    outcome: ratatoskr_core::actions::ActionOutcome,
) -> Result<(), String> {
    match outcome {
        ratatoskr_core::actions::ActionOutcome::Success
        | ratatoskr_core::actions::ActionOutcome::NoOp
        | ratatoskr_core::actions::ActionOutcome::LocalOnly { .. } => Ok(()),
        ratatoskr_core::actions::ActionOutcome::Failed { error } => Err(error.user_message()),
    }
}

/// Convert a CalendarEvent from the DB to CalendarEventData for the UI.
fn db_event_to_calendar_data(ev: &crate::db::CalendarEvent) -> CalendarEventData {
    use chrono::TimeZone;
    let start_dt = chrono::Local
        .timestamp_opt(ev.start_time, 0)
        .single();
    let end_dt = chrono::Local
        .timestamp_opt(ev.end_time, 0)
        .single();

    let (date, sh, sm) = match start_dt {
        Some(dt) => (dt.date_naive(), dt.time().hour(), dt.time().minute()),
        None => (chrono::Local::now().date_naive(), 9, 0),
    };
    let (eh, em) = match end_dt {
        Some(dt) => (dt.time().hour(), dt.time().minute()),
        None => ((sh + 1).min(23), 0),
    };

    CalendarEventData {
        id: Some(ev.id.clone()),
        title: ev.summary.clone().unwrap_or_default(),
        start_date: date,
        start_hour: format!("{sh}"),
        start_minute: format!("{sm:02}"),
        end_hour: format!("{eh}"),
        end_minute: format!("{em:02}"),
        all_day: ev.is_all_day,
        location: ev.location.clone().unwrap_or_default(),
        description: ev.description.clone().unwrap_or_default(),
        calendar_id: ev.calendar_id.clone(),
        account_id: Some(ev.account_id.clone()),
        timezone: ev.timezone.clone(),
        recurrence_rule: ev.recurrence_rule.clone(),
        organizer_name: ev.organizer_name.clone(),
        organizer_email: ev.organizer_email.clone(),
        rsvp_status: ev.rsvp_status.clone(),
        availability: ev.availability.clone(),
        visibility: ev.visibility.clone(),
        attendees: Vec::new(),
        reminders: Vec::new(),
        calendar_name: None,
        color: None,
    }
}

/// Convert date + hour + minute to a Unix timestamp (local time).
pub(crate) fn calendar_data_to_timestamp(date: NaiveDate, hour: u32, minute: u32) -> i64 {
    use chrono::TimeZone;
    let naive_time = chrono::NaiveTime::from_hms_opt(hour, minute, 0)
        .unwrap_or_else(|| chrono::NaiveTime::from_hms_opt(0, 0, 0).unwrap_or_default());
    let naive_dt = date.and_time(naive_time);
    chrono::Local
        .from_local_datetime(&naive_dt)
        .single()
        .map_or(0, |dt| dt.timestamp())
}
