import { invoke } from "@tauri-apps/api/core";
import type {
  CalendarEventData,
  CalendarInfo,
  CalendarSyncResult,
} from "./types";

function toCalendarEventInput(event: CalendarEventData): {
  remoteEventId: string;
  uid: string | null;
  etag: string | null;
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
  icalData: string | null;
} {
  return {
    remoteEventId: event.remoteEventId,
    uid: event.uid,
    etag: event.etag,
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
    icalData: event.icalData,
  };
}

export async function upsertDiscoveredCalendars(
  accountId: string,
  provider: string,
  calendars: CalendarInfo[],
): Promise<void> {
  await invoke("calendar_upsert_discovered_calendars", {
    accountId,
    provider,
    calendars,
  });
}

export async function upsertProviderEvents(
  accountId: string,
  calendarRemoteId: string,
  events: CalendarEventData[],
): Promise<void> {
  await invoke("calendar_upsert_provider_events", {
    accountId,
    calendarRemoteId,
    events: events.map(toCalendarEventInput),
  });
}

export async function applyCalendarSyncResult(
  accountId: string,
  calendarRemoteId: string,
  result: CalendarSyncResult,
): Promise<void> {
  await invoke("calendar_apply_sync_result", {
    accountId,
    calendarRemoteId,
    created: result.created.map(toCalendarEventInput),
    updated: result.updated.map(toCalendarEventInput),
    deletedRemoteIds: result.deletedRemoteIds,
    newSyncToken: result.newSyncToken,
    newCtag: result.newCtag,
  });
}

export async function deleteProviderEvent(
  accountId: string,
  calendarRemoteId: string,
  remoteEventId: string,
): Promise<void> {
  await invoke("calendar_delete_provider_event", {
    accountId,
    calendarRemoteId,
    remoteEventId,
  });
}
