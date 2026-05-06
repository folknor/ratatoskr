use std::sync::Arc;

use chrono::{Datelike, Timelike};
use iced::Task;

use crate::ui::calendar::{
    CalendarEventData, CalendarMessage, CalendarWorkflow, EditorSession, EventField,
    EventTextField, ViewingSurface,
};
use crate::{Message, ReadyApp};

impl ReadyApp {
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
                let mut event = CalendarEventData::new_at(date, hour);
                self.pre_assign_calendar_if_unambiguous(&mut event, None);
                let account_id = event.account_id.clone();
                let session = EditorSession::new(event);
                self.calendar.workflow = CalendarWorkflow::CreatingEvent {
                    account_id,
                    session,
                };
                self.calendar.sync_surfaces();
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
                        let attendees = db
                            .get_event_attendees(ev.account_id.clone(), ev.id.clone())
                            .await
                            .unwrap_or_default();
                        let reminders = db
                            .get_event_reminders(ev.account_id.clone(), ev.id.clone())
                            .await
                            .unwrap_or_default();
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
                        self.calendar.workflow = CalendarWorkflow::ViewingEvent {
                            event_data: data,
                            surface: ViewingSurface::Popover,
                        };
                        self.calendar.sync_surfaces();
                    }
                    Err(e) => {
                        log::error!("Failed to load calendar event: {e}");
                        self.status = format!("Failed to load event: {e}");
                    }
                }
                Task::none()
            }
            CalendarMessage::Noop => Task::none(),
            CalendarMessage::ClosePopover => {
                self.calendar.workflow = CalendarWorkflow::Idle;
                self.calendar.sync_surfaces();
                Task::none()
            }
            CalendarMessage::CloseModal => {
                // If we're in the discard confirmation, "Close" means cancel
                // the discard and return to the editor.
                if let CalendarWorkflow::ConfirmingDiscard {
                    was_creating,
                    event_id,
                    account_id,
                    ..
                } = &self.calendar.workflow
                {
                    let was_creating = *was_creating;
                    let event_id = event_id.clone();
                    let account_id = account_id.clone();
                    // Take ownership of the session from ConfirmingDiscard.
                    let session = match std::mem::replace(
                        &mut self.calendar.workflow,
                        CalendarWorkflow::Idle,
                    ) {
                        CalendarWorkflow::ConfirmingDiscard { session, .. } => session,
                        _ => unreachable!(),
                    };
                    if was_creating {
                        self.calendar.workflow = CalendarWorkflow::CreatingEvent {
                            account_id,
                            session,
                        };
                    } else {
                        self.calendar.workflow = CalendarWorkflow::EditingEvent {
                            event_id: event_id.unwrap_or_default(),
                            account_id: account_id.unwrap_or_default(),
                            session,
                        };
                    }
                    self.calendar.sync_surfaces();
                    return Task::none();
                }

                // Check for unsaved changes in the editor.
                let is_dirty = match &self.calendar.workflow {
                    CalendarWorkflow::CreatingEvent { session, .. }
                    | CalendarWorkflow::EditingEvent { session, .. } => session.is_dirty(),
                    _ => false,
                };
                if is_dirty {
                    // Preserve session for cancel-discard.
                    let (was_creating, event_id, account_id) = match &self.calendar.workflow {
                        CalendarWorkflow::CreatingEvent { account_id, .. } => {
                            (true, None, account_id.clone())
                        }
                        CalendarWorkflow::EditingEvent {
                            event_id,
                            account_id,
                            ..
                        } => (false, Some(event_id.clone()), Some(account_id.clone())),
                        _ => unreachable!(),
                    };
                    let session = match std::mem::replace(
                        &mut self.calendar.workflow,
                        CalendarWorkflow::Idle,
                    ) {
                        CalendarWorkflow::CreatingEvent { session, .. }
                        | CalendarWorkflow::EditingEvent { session, .. } => session,
                        _ => unreachable!(),
                    };
                    self.calendar.workflow = CalendarWorkflow::ConfirmingDiscard {
                        was_creating,
                        event_id,
                        account_id,
                        session,
                    };
                    self.calendar.sync_surfaces();
                    return Task::none();
                }

                self.calendar.workflow = CalendarWorkflow::Idle;
                self.calendar.sync_surfaces();
                Task::none()
            }
            CalendarMessage::ExpandPopoverToModal => {
                // Workflow identity stays the same - only the surface changes.
                if let CalendarWorkflow::ViewingEvent { surface, .. } =
                    &mut self.calendar.workflow
                {
                    *surface = ViewingSurface::FullModal;
                    self.calendar.sync_surfaces();
                }
                Task::none()
            }
            CalendarMessage::OpenEventEditor(data) => {
                let mut event = match data {
                    Some(e) => e,
                    None => {
                        let date = self.calendar.selected_date;
                        let hour = self.calendar.selected_hour.unwrap_or(9);
                        CalendarEventData::new_at(date, hour)
                    }
                };
                // Create-vs-edit derived from event.id presence.
                let is_editing = event.id.is_some();
                if !is_editing {
                    self.pre_assign_calendar_if_unambiguous(&mut event, None);
                }
                let event_id = event.id.clone().unwrap_or_default();
                let account_id = event.account_id.clone();
                let session = EditorSession::new(event);
                if is_editing {
                    self.calendar.workflow = CalendarWorkflow::EditingEvent {
                        event_id,
                        account_id: account_id.unwrap_or_default(),
                        session,
                    };
                } else {
                    self.calendar.workflow = CalendarWorkflow::CreatingEvent {
                        account_id,
                        session,
                    };
                }
                self.calendar.sync_surfaces();
                Task::none()
            }
            CalendarMessage::CreateEvent => {
                let date = self.calendar.selected_date;
                let hour = self.calendar.selected_hour.unwrap_or(9);
                let mut event = CalendarEventData::new_at(date, hour);
                // Pre-assign calendar when unambiguous.
                self.pre_assign_calendar_if_unambiguous(&mut event, None);
                let account_id = event.account_id.clone();
                let session = EditorSession::new(event);
                // Workflow first, then surface.
                self.calendar.workflow = CalendarWorkflow::CreatingEvent {
                    account_id,
                    session,
                };
                self.calendar.sync_surfaces();
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
                        self.calendar.workflow = CalendarWorkflow::Idle;
                        self.calendar.sync_surfaces();
                        return self.reload_calendar_events();
                    }
                    Err(e) => {
                        log::error!("Failed to save calendar event: {e}");
                        self.status = format!("Save failed: {e}");
                    }
                }
                Task::none()
            }
            CalendarMessage::ConfirmDeleteEvent {
                event_id,
                title,
                account_id,
            } => {
                // Workflow first, then surface.
                self.calendar.workflow = CalendarWorkflow::ConfirmingDelete {
                    event_id: event_id.clone(),
                    account_id: account_id.clone().unwrap_or_default(),
                    title: title.clone(),
                };
                self.calendar.sync_surfaces();
                Task::none()
            }
            CalendarMessage::DeleteEvent => {
                // Read identity from workflow state.
                let CalendarWorkflow::ConfirmingDelete {
                    event_id,
                    account_id,
                    ..
                } = &self.calendar.workflow
                else {
                    log::warn!("DeleteEvent received outside ConfirmingDelete workflow");
                    return Task::none();
                };
                let _event_id = event_id.clone();
                let _account_id = account_id.clone();

                // Always close the confirmation modal first - the Delete click
                // must produce immediate visual feedback regardless of whether
                // the async dispatch can run.
                self.calendar.workflow = CalendarWorkflow::Idle;
                self.calendar.sync_surfaces();

                // Phase 6c-5 flipped cal::actions::* to take a
                // `CalendarActionContext` (writer-half scoped to the
                // Service crate); the UI no longer has a way to
                // construct one. Phase 6c-8 routes this through the
                // `cal_action.execute_plan` IPC. Until then we surface
                // a clear status to the user rather than silently
                // dropping the click.
                Task::done(Message::Calendar(Box::new(CalendarMessage::EventDeleted(
                    Err("Calendar delete is being relocated Service-side (Phase 6c-8)"
                        .to_string()),
                ))))
            }
            CalendarMessage::DiscardChanges => {
                self.calendar.workflow = CalendarWorkflow::Idle;
                self.calendar.sync_surfaces();
                Task::none()
            }
            CalendarMessage::EventDeleted(result) => {
                match result {
                    Ok(()) => {
                        log::info!("Calendar event deleted");
                        self.status_bar
                            .show_confirmation("Event deleted".to_string());
                        return self.reload_calendar_events();
                    }
                    Err(e) => {
                        log::error!("Failed to delete calendar event: {e}");
                        self.status_bar
                            .show_confirmation(format!("Delete failed: {e}"));
                    }
                }
                Task::none()
            }
            CalendarMessage::SwitchToMail => {
                self.update(Message::SetAppMode(crate::AppMode::Mail))
            }
            CalendarMessage::PopOutCalendar => {
                // Check if a calendar pop-out already exists.
                let existing = self
                    .pop_out_windows
                    .values()
                    .any(|w| matches!(w, crate::pop_out::PopOutWindow::Calendar(_)));
                if existing {
                    // Bring existing pop-out to foreground.
                    // (iced doesn't have a bring-to-front API, so this is a no-op for now)
                    return Task::none();
                }
                // Open new calendar pop-out window.
                let initial = iced::Size::new(1024.0, 768.0);
                let settings = iced::window::Settings {
                    size: initial,
                    ..Default::default()
                };
                let (id, open_task) = iced::window::open(settings);
                self.pop_out_windows.insert(
                    id,
                    crate::pop_out::PopOutWindow::Calendar(crate::pop_out::CalendarPopOutGeometry {
                        width: initial.width,
                        height: initial.height,
                        x: None,
                        y: None,
                    }),
                );
                // Switch main window back to mail mode.
                self.app_mode = crate::AppMode::Mail;
                open_task.discard()
            }
            CalendarMessage::EventsLoaded(load_generation, result) => {
                if !self.calendar.load_generation.is_current(load_generation) {
                    // Stale result from a previous load - discard.
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
                // If the Service is not yet attached, do not apply the eager
                // flip - the write would be lost on next reload. Phase 6a
                // sized this window narrowly (post-`boot.ready` only); if it
                // shows up in practice we surface a status-bar message
                // instead of silently swallowing the click.
                let Some(client) = self.service_client.as_ref().cloned() else {
                    log::warn!(
                        "calendar.set_visibility: no ServiceClient yet; ignoring toggle"
                    );
                    self.status_bar.show_confirmation(
                        "Service not ready - try again in a moment".to_string(),
                    );
                    return Task::none();
                };
                // Eager UI flip for responsiveness, with the prior value
                // captured so the Err arm can snap back without overwriting
                // a newer click.
                if let Some(cal) = self
                    .calendar
                    .calendars
                    .iter_mut()
                    .find(|c| c.id == calendar_id)
                {
                    cal.is_visible = visible;
                }
                let cid = calendar_id.clone();
                Task::perform(
                    async move { client.set_calendar_visibility(cid, visible).await },
                    move |result| {
                        Message::Calendar(Box::new(CalendarMessage::VisibilityToggled {
                            calendar_id: calendar_id.clone(),
                            requested_value: visible,
                            result: result.map_err(|e| e.to_string()),
                        }))
                    },
                )
            }
            CalendarMessage::VisibilityToggled {
                calendar_id,
                requested_value,
                result,
            } => match result {
                Ok(()) => {
                    // Persistence confirmed; reload events so the SQL view
                    // filter (`is_visible = 1`) reflects the new state.
                    self.reload_calendar_events()
                }
                Err(e) => {
                    // Roll back the eager flip iff the local value still
                    // matches the failed request - if the user clicked again
                    // mid-flight, leave their newer intent alone.
                    if let Some(cal) = self
                        .calendar
                        .calendars
                        .iter_mut()
                        .find(|c| c.id == calendar_id)
                        && cal.is_visible == requested_value
                    {
                        cal.is_visible = !requested_value;
                    }
                    log::warn!(
                        "calendar.set_visibility failed for {calendar_id}: {e}"
                    );
                    self.status_bar
                        .show_confirmation(format!("Could not update calendar: {e}"));
                    Task::none()
                }
            },
            CalendarMessage::CalendarsLoaded(load_generation, result) => {
                if !self.calendar.load_generation.is_current(load_generation) {
                    // Stale result from a previous load - discard.
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
        // CalendarSelected needs to update both draft and workflow account_id,
        // which requires two mutable accesses to self.calendar.workflow.
        // Handle it separately to avoid borrow conflicts.
        if let EventField::CalendarSelected {
            calendar_id,
            account_id,
        } = field
        {
            match &mut self.calendar.workflow {
                CalendarWorkflow::CreatingEvent {
                    account_id: wf_account,
                    session,
                } => {
                    session.draft.calendar_id = calendar_id;
                    session.draft.account_id = account_id.clone();
                    *wf_account = account_id;
                }
                CalendarWorkflow::EditingEvent { session, .. } => {
                    // Picker is disabled for edit mode, but handle defensively.
                    session.draft.calendar_id = calendar_id;
                }
                _ => {}
            }
            return;
        }

        let session = match &mut self.calendar.workflow {
            CalendarWorkflow::CreatingEvent { session, .. }
            | CalendarWorkflow::EditingEvent { session, .. } => session,
            _ => return,
        };
        match field {
            EventField::Title(s) => {
                session.undo_title.set_text(s.clone());
                session.draft.title = s;
            }
            EventField::Location(s) => {
                session.undo_location.set_text(s.clone());
                session.draft.location = s;
            }
            EventField::Description(s) => {
                session.undo_description.set_text(s.clone());
                session.draft.description = s;
            }
            EventField::StartHour(s) => session.draft.start_hour = s,
            EventField::StartMinute(s) => session.draft.start_minute = s,
            EventField::EndHour(s) => session.draft.end_hour = s,
            EventField::EndMinute(s) => session.draft.end_minute = s,
            EventField::AllDay(b) => session.draft.all_day = b,
            EventField::CalendarSelected { .. } => unreachable!("handled above"),
            EventField::Timezone(tz) => session.draft.timezone = tz,
            EventField::Availability(a) => session.draft.availability = a,
            EventField::Visibility(v) => session.draft.visibility = v,
            EventField::RecurrenceRule(r) => session.draft.recurrence_rule = r,
        }
    }

    fn handle_event_field_undo(&mut self, text_field: EventTextField) {
        let session = match &mut self.calendar.workflow {
            CalendarWorkflow::CreatingEvent { session, .. }
            | CalendarWorkflow::EditingEvent { session, .. } => session,
            _ => return,
        };
        match text_field {
            EventTextField::Title => {
                if let Some(t) = session.undo_title.undo() {
                    session.draft.title = t.to_owned();
                }
            }
            EventTextField::Location => {
                if let Some(t) = session.undo_location.undo() {
                    session.draft.location = t.to_owned();
                }
            }
            EventTextField::Description => {
                if let Some(t) = session.undo_description.undo() {
                    session.draft.description = t.to_owned();
                }
            }
        }
    }

    fn handle_event_field_redo(&mut self, text_field: EventTextField) {
        let session = match &mut self.calendar.workflow {
            CalendarWorkflow::CreatingEvent { session, .. }
            | CalendarWorkflow::EditingEvent { session, .. } => session,
            _ => return,
        };
        match text_field {
            EventTextField::Title => {
                if let Some(t) = session.undo_title.redo() {
                    session.draft.title = t.to_owned();
                }
            }
            EventTextField::Location => {
                if let Some(t) = session.undo_location.redo() {
                    session.draft.location = t.to_owned();
                }
            }
            EventTextField::Description => {
                if let Some(t) = session.undo_description.redo() {
                    session.draft.description = t.to_owned();
                }
            }
        }
    }

    fn handle_save_event(&mut self) -> Task<Message> {
        // Phase 6c-5 flipped cal::actions::* to take a
        // `CalendarActionContext` (writer-half scoped to the Service
        // crate); the UI no longer has a way to construct one. Phase
        // 6c-8 routes this through the `cal_action.execute_plan` IPC.
        // Until then we surface a clear status to the user rather than
        // silently dropping the save click.
        match &self.calendar.workflow {
            CalendarWorkflow::EditingEvent { .. } | CalendarWorkflow::CreatingEvent { .. } => {
                Task::done(Message::Calendar(Box::new(CalendarMessage::EventSaved(
                    Err("Calendar save is being relocated Service-side (Phase 6c-8)"
                        .to_string()),
                ))))
            }
            _ => {
                log::warn!("SaveEvent received outside editing/creating workflow");
                Task::none()
            }
        }
    }

    /// Pre-assign calendar (and account) ownership on a new event when
    /// unambiguous.
    ///
    /// If `for_account` is `Some`, only calendars on that account are
    /// considered. If `None`, all calendars are considered.
    ///
    /// **Assumption:** all entries in `self.calendar.calendars` are treated
    /// as eligible create targets.
    fn pre_assign_calendar_if_unambiguous(
        &self,
        event: &mut CalendarEventData,
        for_account: Option<&str>,
    ) {
        if event.calendar_id.is_some() {
            return;
        }
        let eligible: Vec<_> = self
            .calendar
            .calendars
            .iter()
            .filter(|c| {
                for_account.is_none_or(|acct| c.account_id == acct)
            })
            .collect();
        if eligible.len() == 1 {
            event.calendar_id = Some(eligible[0].id.clone());
            event.account_id = Some(eligible[0].account_id.clone());
        }
    }

    /// Reload calendar events from DB and rebuild views.
    ///
    /// Increments the load generation counter so that results from
    /// previously-dispatched (now stale) loads are discarded. The
    /// (start, end) window is computed from the currently-visible
    /// mini-month so the SQL filter actually bounds the result and the
    /// connection mutex isn't held while the recurrence expansion runs.
    pub(crate) fn reload_calendar_events(&mut self) -> Task<Message> {
        let load_generation = self.calendar.load_generation.next();
        let db = Arc::clone(&self.db);
        let db2 = Arc::clone(&self.db);
        let (window_start, window_end) = self.calendar.current_view_window();
        Task::batch([
            Task::perform(
                async move {
                    db.load_calendar_events_for_view(window_start, window_end)
                        .await
                },
                move |r| {
                    Message::Calendar(Box::new(CalendarMessage::EventsLoaded(load_generation, r)))
                },
            ),
            Task::perform(
                async move { db2.load_calendars_for_sidebar().await },
                move |r| {
                    Message::Calendar(Box::new(CalendarMessage::CalendarsLoaded(
                        load_generation,
                        r,
                    )))
                },
            ),
        ])
    }
}

/// Convert a CalendarEvent from the DB to CalendarEventData for the UI.
fn db_event_to_calendar_data(ev: &crate::db::CalendarEvent) -> CalendarEventData {
    use chrono::TimeZone;
    let start_dt = chrono::Local.timestamp_opt(ev.start_time, 0).single();
    let end_dt = chrono::Local.timestamp_opt(ev.end_time, 0).single();

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

