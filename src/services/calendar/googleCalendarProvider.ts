import { invoke } from "@tauri-apps/api/core";
import type {
  CalendarEventData,
  CalendarInfo,
  CalendarProvider,
  CalendarProviderType,
  CalendarSyncResult,
  CreateEventInput,
  UpdateEventInput,
} from "./types";

export class GoogleCalendarProvider implements CalendarProvider {
  readonly type: CalendarProviderType = "google_api";
  readonly accountId: string;

  constructor(accountId: string) {
    this.accountId = accountId;
  }

  async listCalendars(): Promise<CalendarInfo[]> {
    return invoke<CalendarInfo[]>("google_calendar_list_calendars", {
      accountId: this.accountId,
    });
  }

  async fetchEvents(
    calendarRemoteId: string,
    timeMin: string,
    timeMax: string,
  ): Promise<CalendarEventData[]> {
    return invoke<CalendarEventData[]>("google_calendar_fetch_events", {
      accountId: this.accountId,
      calendarRemoteId,
      timeMin,
      timeMax,
    });
  }

  async createEvent(
    calendarRemoteId: string,
    event: CreateEventInput,
  ): Promise<CalendarEventData> {
    return invoke<CalendarEventData>("google_calendar_create_event", {
      accountId: this.accountId,
      calendarRemoteId,
      event: buildGoogleCalendarEventPayload(event),
    });
  }

  async updateEvent(
    calendarRemoteId: string,
    remoteEventId: string,
    event: UpdateEventInput,
  ): Promise<CalendarEventData> {
    return invoke<CalendarEventData>("google_calendar_update_event", {
      accountId: this.accountId,
      calendarRemoteId,
      remoteEventId,
      event: buildGoogleCalendarEventPayload(event),
    });
  }

  async deleteEvent(
    calendarRemoteId: string,
    remoteEventId: string,
  ): Promise<void> {
    await invoke("google_calendar_delete_event", {
      accountId: this.accountId,
      calendarRemoteId,
      remoteEventId,
    });
  }

  async syncEvents(
    calendarRemoteId: string,
    syncToken?: string,
  ): Promise<CalendarSyncResult> {
    return invoke<CalendarSyncResult>("google_calendar_sync_events", {
      accountId: this.accountId,
      calendarRemoteId,
      syncToken: syncToken ?? null,
    });
  }

  async testConnection(): Promise<{ success: boolean; message: string }> {
    try {
      await this.listCalendars();
      return { success: true, message: "Connected to Google Calendar" };
    } catch (err) {
      return {
        success: false,
        message: err instanceof Error ? err.message : "Connection failed",
      };
    }
  }
}

function buildGoogleCalendarEventPayload(
  event: CreateEventInput | UpdateEventInput,
): Record<string, unknown> {
  const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
  const body: Record<string, unknown> = {};

  if ("summary" in event && event.summary !== undefined) {
    body.summary = event.summary;
  }
  if ("description" in event && event.description !== undefined) {
    body.description = event.description;
  }
  if ("location" in event && event.location !== undefined) {
    body.location = event.location;
  }

  if (event.startTime && event.endTime) {
    if (event.isAllDay) {
      body.start = { date: event.startTime.split("T")[0] };
      body.end = { date: event.endTime.split("T")[0] };
    } else {
      body.start = {
        dateTime: new Date(event.startTime).toISOString(),
        timeZone: tz,
      };
      body.end = {
        dateTime: new Date(event.endTime).toISOString(),
        timeZone: tz,
      };
    }
  }

  if ("attendees" in event && event.attendees !== undefined) {
    body.attendees = event.attendees;
  }

  return body;
}
