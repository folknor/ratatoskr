use std::sync::Arc;

use chrono::{Datelike, NaiveDate, Timelike};
use iced::Task;

use crate::ui::calendar::{
    CalendarEventData, CalendarMessage, CalendarOverlay, EventField,
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
            CalendarMessage::EventClicked(event_id) => {
                let db = Arc::clone(&self.db);
                let eid = event_id.clone();
                Task::perform(
                    async move { db.get_calendar_event(eid).await },
                    move |result| {
                        let mapped = match result {
                            Ok(Some(ev)) => Ok(db_event_to_calendar_data(&ev)),
                            Ok(None) => Err(format!("Event not found: {event_id}")),
                            Err(e) => Err(e),
                        };
                        Message::Calendar(CalendarMessage::EventLoaded(mapped))
                    },
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
                self.calendar.overlay = CalendarOverlay::None;
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
                self.calendar.overlay =
                    CalendarOverlay::EventEditor { event, is_new };
                Task::none()
            }
            CalendarMessage::CreateEvent => {
                let date = self.calendar.selected_date;
                let hour = self.calendar.selected_hour.unwrap_or(9);
                self.calendar.overlay = CalendarOverlay::EventEditor {
                    event: CalendarEventData::new_at(date, hour),
                    is_new: true,
                };
                Task::none()
            }
            CalendarMessage::EventFieldChanged(field) => {
                self.handle_event_field_changed(field);
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
                let db = Arc::clone(&self.db);
                self.calendar.overlay = CalendarOverlay::None;
                Task::perform(
                    async move { db.delete_calendar_event(event_id).await },
                    |r| Message::Calendar(CalendarMessage::EventDeleted(r)),
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
            CalendarMessage::EventsLoaded(result) => {
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
        }
    }

    fn handle_event_field_changed(&mut self, field: EventField) {
        if let CalendarOverlay::EventEditor { ref mut event, .. } = self.calendar.overlay {
            match field {
                EventField::Title(s) => event.title = s,
                EventField::Location(s) => event.location = s,
                EventField::Description(s) => event.description = s,
                EventField::StartHour(s) => event.start_hour = s,
                EventField::StartMinute(s) => event.start_minute = s,
                EventField::EndHour(s) => event.end_hour = s,
                EventField::EndMinute(s) => event.end_minute = s,
                EventField::AllDay(b) => event.all_day = b,
            }
        }
    }

    fn handle_save_event(&mut self) -> Task<Message> {
        let event = match &self.calendar.overlay {
            CalendarOverlay::EventEditor { event, .. } => event.clone(),
            _ => return Task::none(),
        };

        let db = Arc::clone(&self.db);
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

        if let Some(id) = event.id.clone() {
            Task::perform(
                async move {
                    db.update_calendar_event(
                        id,
                        event.title,
                        event.description,
                        event.location,
                        start_ts,
                        end_ts,
                        event.all_day,
                        event.calendar_id,
                    )
                    .await
                },
                |r| Message::Calendar(CalendarMessage::EventSaved(r)),
            )
        } else {
            let account_id = self
                .sidebar
                .accounts
                .first()
                .map(|a| a.id.clone())
                .unwrap_or_default();
            Task::perform(
                async move {
                    db.create_calendar_event(
                        account_id,
                        event.title,
                        event.description,
                        event.location,
                        start_ts,
                        end_ts,
                        event.all_day,
                        event.calendar_id,
                    )
                    .await
                    .map(|_id| ())
                },
                |r| Message::Calendar(CalendarMessage::EventSaved(r)),
            )
        }
    }

    /// Reload calendar events from DB and rebuild views.
    pub(crate) fn reload_calendar_events(&self) -> Task<Message> {
        let db = Arc::clone(&self.db);
        Task::perform(
            async move { db.load_calendar_events_for_view().await },
            |r| Message::Calendar(CalendarMessage::EventsLoaded(r)),
        )
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
