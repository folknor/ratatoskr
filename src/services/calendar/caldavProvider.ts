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

export class CalDAVProvider implements CalendarProvider {
  readonly type: CalendarProviderType = "caldav";
  readonly accountId: string;

  constructor(accountId: string) {
    this.accountId = accountId;
  }

  async listCalendars(): Promise<CalendarInfo[]> {
    return invoke<CalendarInfo[]>("caldav_list_calendars", {
      accountId: this.accountId,
    });
  }

  async fetchEvents(
    calendarRemoteId: string,
    timeMin: string,
    timeMax: string,
  ): Promise<CalendarEventData[]> {
    return invoke<CalendarEventData[]>("caldav_fetch_events", {
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
    return invoke<CalendarEventData>("caldav_create_event", {
      accountId: this.accountId,
      calendarRemoteId,
      event,
    });
  }

  async updateEvent(
    calendarRemoteId: string,
    remoteEventId: string,
    event: UpdateEventInput,
    etag?: string,
  ): Promise<CalendarEventData> {
    return invoke<CalendarEventData>("caldav_update_event", {
      accountId: this.accountId,
      calendarRemoteId,
      remoteEventId,
      event,
      etag: etag ?? null,
    });
  }

  async deleteEvent(
    calendarRemoteId: string,
    remoteEventId: string,
    etag?: string,
  ): Promise<void> {
    await invoke("caldav_delete_event", {
      accountId: this.accountId,
      calendarRemoteId,
      remoteEventId,
      etag: etag ?? null,
    });
  }

  async syncEvents(
    calendarRemoteId: string,
    _syncToken?: string,
  ): Promise<CalendarSyncResult> {
    return invoke<CalendarSyncResult>("caldav_sync_events", {
      accountId: this.accountId,
      calendarRemoteId,
      syncToken: _syncToken ?? null,
    });
  }

  async testConnection(): Promise<{ success: boolean; message: string }> {
    return invoke<{ success: boolean; message: string }>(
      "caldav_test_connection",
      {
        accountId: this.accountId,
      },
    );
  }
}
