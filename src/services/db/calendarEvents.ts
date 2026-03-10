import { invoke } from "@tauri-apps/api/core";

export interface DbCalendarEvent {
  id: string;
  account_id: string;
  google_event_id: string;
  summary: string | null;
  description: string | null;
  location: string | null;
  start_time: number;
  end_time: number;
  is_all_day: number;
  status: string;
  organizer_email: string | null;
  attendees_json: string | null;
  html_link: string | null;
  updated_at: number;
  // CalDAV fields
  calendar_id: string | null;
  remote_event_id: string | null;
  etag: string | null;
  ical_data: string | null;
  uid: string | null;
}

export async function upsertCalendarEvent(event: {
  accountId: string;
  googleEventId: string;
  summary: string | null;
  description: string | null;
  location: string | null;
  startTime: number;
  endTime: number;
  isAllDay: boolean;
  status: string;
  organizerEmail: string | null;
  attendeesJson: string | null;
  htmlLink: string | null;
  calendarId?: string | null;
  remoteEventId?: string | null;
  etag?: string | null;
  icalData?: string | null;
  uid?: string | null;
}): Promise<void> {
  await invoke("db_upsert_calendar_event", {
    accountId: event.accountId,
    googleEventId: event.googleEventId,
    summary: event.summary,
    description: event.description,
    location: event.location,
    startTime: event.startTime,
    endTime: event.endTime,
    isAllDay: event.isAllDay,
    status: event.status,
    organizerEmail: event.organizerEmail,
    attendeesJson: event.attendeesJson,
    htmlLink: event.htmlLink,
    calendarId: event.calendarId ?? null,
    remoteEventId: event.remoteEventId ?? null,
    etag: event.etag ?? null,
    icalData: event.icalData ?? null,
    uid: event.uid ?? null,
  });
}

export async function getCalendarEventsInRange(
  accountId: string,
  startTime: number,
  endTime: number,
): Promise<DbCalendarEvent[]> {
  return invoke<DbCalendarEvent[]>("db_get_calendar_events_in_range", {
    accountId,
    startTime,
    endTime,
  });
}

export async function getCalendarEventsInRangeMulti(
  accountId: string,
  calendarIds: string[],
  startTime: number,
  endTime: number,
): Promise<DbCalendarEvent[]> {
  if (calendarIds.length === 0) {
    return getCalendarEventsInRange(accountId, startTime, endTime);
  }
  return invoke<DbCalendarEvent[]>(
    "db_get_calendar_events_in_range_multi",
    { accountId, calendarIds, startTime, endTime },
  );
}

export async function deleteEventsForCalendar(
  calendarId: string,
): Promise<void> {
  await invoke("db_delete_events_for_calendar", { calendarId });
}

export async function getEventByRemoteId(
  calendarId: string,
  remoteEventId: string,
): Promise<DbCalendarEvent | null> {
  return invoke<DbCalendarEvent | null>("db_get_event_by_remote_id", {
    calendarId,
    remoteEventId,
  });
}

export async function deleteEventByRemoteId(
  calendarId: string,
  remoteEventId: string,
): Promise<void> {
  await invoke("db_delete_event_by_remote_id", { calendarId, remoteEventId });
}

export async function deleteCalendarEvent(eventId: string): Promise<void> {
  await invoke("db_delete_calendar_event", { eventId });
}
