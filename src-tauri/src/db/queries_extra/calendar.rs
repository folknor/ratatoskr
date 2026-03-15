#![allow(clippy::let_underscore_must_use)]

use crate::db::types::{DbCalendar, DbCalendarEvent};

db_command! {
    fn db_upsert_calendar(
        state,
        account_id: String,
        provider: String,
        remote_id: String,
        display_name: Option<String>,
        color: Option<String>,
        is_primary: bool
    ) -> String;
    fn db_get_calendars_for_account(state, account_id: String) -> Vec<DbCalendar>;
    fn db_get_visible_calendars(state, account_id: String) -> Vec<DbCalendar>;
    fn db_set_calendar_visibility(state, calendar_id: String, visible: bool) -> ();
    fn db_update_calendar_sync_token(state, calendar_id: String, sync_token: Option<String>, ctag: Option<String>) -> ();
    fn db_delete_calendars_for_account(state, account_id: String) -> ();
    fn db_get_calendar_by_id(state, calendar_id: String) -> Option<DbCalendar>;
    fn db_upsert_calendar_event(
        state,
        account_id: String,
        google_event_id: String,
        summary: Option<String>,
        description: Option<String>,
        location: Option<String>,
        start_time: i64,
        end_time: i64,
        is_all_day: bool,
        status: String,
        organizer_email: Option<String>,
        attendees_json: Option<String>,
        html_link: Option<String>,
        calendar_id: Option<String>,
        remote_event_id: Option<String>,
        etag: Option<String>,
        ical_data: Option<String>,
        uid: Option<String>
    ) -> ();
    fn db_get_calendar_events_in_range(state, account_id: String, start_time: i64, end_time: i64) -> Vec<DbCalendarEvent>;
    fn db_get_calendar_events_in_range_multi(state, account_id: String, calendar_ids: Vec<String>, start_time: i64, end_time: i64) -> Vec<DbCalendarEvent>;
    fn db_delete_events_for_calendar(state, calendar_id: String) -> ();
    fn db_get_event_by_remote_id(state, calendar_id: String, remote_event_id: String) -> Option<DbCalendarEvent>;
    fn db_delete_event_by_remote_id(state, calendar_id: String, remote_event_id: String) -> ();
    fn db_delete_calendar_event(state, event_id: String) -> ();
}
