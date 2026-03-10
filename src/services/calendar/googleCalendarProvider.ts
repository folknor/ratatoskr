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

const CALENDAR_API_BASE = "https://www.googleapis.com/calendar/v3";
const MAX_RETRY_ATTEMPTS = 3;
const INITIAL_BACKOFF_MS = 1000;

interface GoogleCalendarListItem {
  id: string;
  summary: string;
  backgroundColor?: string;
  primary?: boolean;
  accessRole?: string;
}

interface GoogleCalendarListResponse {
  items?: GoogleCalendarListItem[];
}

interface GoogleCalendarEvent {
  id: string;
  summary?: string;
  description?: string;
  location?: string;
  start: { dateTime?: string; date?: string; timeZone?: string };
  end: { dateTime?: string; date?: string; timeZone?: string };
  status?: string;
  organizer?: { email: string; displayName?: string };
  attendees?: {
    email: string;
    displayName?: string;
    responseStatus?: string;
  }[];
  htmlLink?: string;
  iCalUID?: string;
  etag?: string;
}

interface GoogleEventListResponse {
  items?: GoogleCalendarEvent[];
  nextPageToken?: string;
  nextSyncToken?: string;
}

/**
 * Make an authenticated request to the Google Calendar API.
 * Handles 401 (token refresh + retry) and 429 (rate limit with exponential backoff).
 */
async function calendarRequest<T>(
  accountId: string,
  url: string,
  options: RequestInit = {},
): Promise<T> {
  let token = await invoke<string>("gmail_get_access_token", { accountId });

  const doFetch = async (accessToken: string): Promise<Response> => {
    let lastResponse: Response | undefined;
    for (let attempt = 0; attempt < MAX_RETRY_ATTEMPTS; attempt++) {
      const response = await fetch(url, {
        ...options,
        headers: {
          Authorization: `Bearer ${accessToken}`,
          "Content-Type": "application/json",
          ...options.headers,
        },
      });
      if (response.status !== 429) return response;

      lastResponse = response;
      if (attempt === MAX_RETRY_ATTEMPTS - 1) break;

      const backoffMs = INITIAL_BACKOFF_MS * 2 ** attempt;
      const retryAfter = response.headers.get("Retry-After");
      let delayMs = backoffMs;
      if (retryAfter) {
        const seconds = parseInt(retryAfter, 10);
        delayMs = !isNaN(seconds) ? seconds * 1000 : backoffMs;
      }
      await new Promise((resolve) => setTimeout(resolve, delayMs));
    }
    // biome-ignore lint/style/noNonNullAssertion: lastResponse is always assigned after at least one loop iteration
    return lastResponse!;
  };

  const response = await doFetch(token);

  if (response.status === 401) {
    // Token was rejected — force-refresh via Rust and retry once
    token = await invoke<string>("gmail_force_refresh_token", { accountId });
    const retry = await doFetch(token);
    if (!retry.ok) {
      throw new Error(
        `Calendar API error: ${retry.status} ${await retry.text()}`,
      );
    }
    if (retry.status === 204) return undefined as T;
    return retry.json();
  }

  if (!response.ok) {
    throw new Error(
      `Calendar API error: ${response.status} ${await response.text()}`,
    );
  }

  if (response.status === 204) return undefined as T;
  return response.json();
}

export class GoogleCalendarProvider implements CalendarProvider {
  readonly type: CalendarProviderType = "google_api";
  readonly accountId: string;

  constructor(accountId: string) {
    this.accountId = accountId;
  }

  async listCalendars(): Promise<CalendarInfo[]> {
    const response = await calendarRequest<GoogleCalendarListResponse>(
      this.accountId,
      `${CALENDAR_API_BASE}/users/me/calendarList`,
    );
    return (response.items ?? []).map((cal) => ({
      remoteId: cal.id,
      displayName: cal.summary,
      color: cal.backgroundColor ?? null,
      isPrimary: Boolean(cal.primary),
    }));
  }

  async fetchEvents(
    calendarRemoteId: string,
    timeMin: string,
    timeMax: string,
  ): Promise<CalendarEventData[]> {
    const params = new URLSearchParams({
      timeMin,
      timeMax,
      singleEvents: "true",
      orderBy: "startTime",
      maxResults: "250",
    });

    const encodedId = encodeURIComponent(calendarRemoteId);
    const url = `${CALENDAR_API_BASE}/calendars/${encodedId}/events?${params}`;
    const response = await calendarRequest<GoogleEventListResponse>(
      this.accountId,
      url,
    );
    return (response.items ?? []).map(mapGoogleEvent);
  }

  async createEvent(
    calendarRemoteId: string,
    event: CreateEventInput,
  ): Promise<CalendarEventData> {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    const encodedId = encodeURIComponent(calendarRemoteId);
    const url = `${CALENDAR_API_BASE}/calendars/${encodedId}/events`;

    const body: Record<string, unknown> = {
      summary: event.summary,
      description: event.description,
      location: event.location,
    };

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

    if (event.attendees) {
      body.attendees = event.attendees;
    }

    const created = await calendarRequest<GoogleCalendarEvent>(
      this.accountId,
      url,
      {
        method: "POST",
        body: JSON.stringify(body),
      },
    );
    return mapGoogleEvent(created);
  }

  async updateEvent(
    calendarRemoteId: string,
    remoteEventId: string,
    event: UpdateEventInput,
  ): Promise<CalendarEventData> {
    const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
    const encodedCalId = encodeURIComponent(calendarRemoteId);
    const encodedEventId = encodeURIComponent(remoteEventId);
    const url = `${CALENDAR_API_BASE}/calendars/${encodedCalId}/events/${encodedEventId}`;

    const body: Record<string, unknown> = {};
    if (event.summary !== undefined) body.summary = event.summary;
    if (event.description !== undefined) body.description = event.description;
    if (event.location !== undefined) body.location = event.location;

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

    const updated = await calendarRequest<GoogleCalendarEvent>(
      this.accountId,
      url,
      {
        method: "PATCH",
        body: JSON.stringify(body),
      },
    );
    return mapGoogleEvent(updated);
  }

  async deleteEvent(
    calendarRemoteId: string,
    remoteEventId: string,
  ): Promise<void> {
    const encodedCalId = encodeURIComponent(calendarRemoteId);
    const encodedEventId = encodeURIComponent(remoteEventId);
    const url = `${CALENDAR_API_BASE}/calendars/${encodedCalId}/events/${encodedEventId}`;
    await calendarRequest(this.accountId, url, { method: "DELETE" });
  }

  async syncEvents(
    calendarRemoteId: string,
    syncToken?: string,
  ): Promise<CalendarSyncResult> {
    const encodedId = encodeURIComponent(calendarRemoteId);
    const created: CalendarEventData[] = [];
    const updated: CalendarEventData[] = [];
    const deletedRemoteIds: string[] = [];

    let pageToken: string | undefined;
    let nextSyncToken: string | null = null;

    do {
      const params = new URLSearchParams({ maxResults: "250" });
      if (syncToken) {
        params.set("syncToken", syncToken);
      } else {
        // Initial sync: fetch last 90 days to 365 days forward
        const timeMin = new Date();
        timeMin.setDate(timeMin.getDate() - 90);
        params.set("timeMin", timeMin.toISOString());
        const timeMax = new Date();
        timeMax.setFullYear(timeMax.getFullYear() + 1);
        params.set("timeMax", timeMax.toISOString());
        params.set("singleEvents", "true");
      }
      if (pageToken) params.set("pageToken", pageToken);

      const url = `${CALENDAR_API_BASE}/calendars/${encodedId}/events?${params}`;

      let response: GoogleEventListResponse;
      try {
        response = await calendarRequest<GoogleEventListResponse>(
          this.accountId,
          url,
        );
      } catch (err) {
        const message = err instanceof Error ? err.message : "";
        if (message.includes("410") || message.includes("sync token")) {
          // Sync token expired — caller should do full sync
          return {
            created: [],
            updated: [],
            deletedRemoteIds: [],
            newSyncToken: null,
            newCtag: null,
          };
        }
        throw err;
      }

      for (const item of response.items ?? []) {
        if (item.status === "cancelled") {
          deletedRemoteIds.push(item.id);
        } else {
          const eventData = mapGoogleEvent(item);
          // For sync, we put everything in "created" (upsert logic handles deduplication)
          created.push(eventData);
        }
      }

      pageToken = response.nextPageToken;
      if (response.nextSyncToken) {
        nextSyncToken = response.nextSyncToken;
      }
    } while (pageToken);

    return {
      created,
      updated,
      deletedRemoteIds,
      newSyncToken: nextSyncToken,
      newCtag: null,
    };
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

function mapGoogleEvent(event: GoogleCalendarEvent): CalendarEventData {
  const isAllDay = Boolean(event.start.date);
  const startTime = event.start.dateTime
    ? Math.floor(new Date(event.start.dateTime).getTime() / 1000)
    : Math.floor(new Date(`${event.start.date}T00:00:00`).getTime() / 1000);
  const endTime = event.end.dateTime
    ? Math.floor(new Date(event.end.dateTime).getTime() / 1000)
    : Math.floor(new Date(`${event.end.date}T23:59:59`).getTime() / 1000);

  return {
    remoteEventId: event.id,
    uid: event.iCalUID ?? null,
    etag: event.etag ?? null,
    summary: event.summary ?? null,
    description: event.description ?? null,
    location: event.location ?? null,
    startTime,
    endTime,
    isAllDay,
    status: event.status ?? "confirmed",
    organizerEmail: event.organizer?.email ?? null,
    attendeesJson: event.attendees ? JSON.stringify(event.attendees) : null,
    htmlLink: event.htmlLink ?? null,
    icalData: null,
  };
}
