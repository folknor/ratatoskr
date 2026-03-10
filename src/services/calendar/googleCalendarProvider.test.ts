import { vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { GoogleCalendarProvider } from "./googleCalendarProvider";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

const CALENDAR_API_BASE = "https://www.googleapis.com/calendar/v3";

function mockFetchResponse(data: unknown, status = 200) {
  return {
    ok: status >= 200 && status < 300,
    status,
    headers: new Headers(),
    json: () => Promise.resolve(data),
    text: () => Promise.resolve(JSON.stringify(data)),
  } as unknown as Response;
}

describe("GoogleCalendarProvider", () => {
  const accountId = "test-account-1";
  let provider: GoogleCalendarProvider;

  beforeEach(() => {
    vi.mocked(invoke).mockResolvedValue("mock-access-token");
    provider = new GoogleCalendarProvider(accountId);
  });

  afterEach(() => {
    vi.restoreAllMocks();
  });

  describe("listCalendars", () => {
    it("maps Google API response to CalendarInfo array", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({
          items: [
            {
              id: "primary",
              summary: "My Calendar",
              backgroundColor: "#0000ff",
              primary: true,
            },
            {
              id: "work@example.com",
              summary: "Work",
              accessRole: "owner",
            },
          ],
        }),
      );

      const result = await provider.listCalendars();

      expect(fetchSpy).toHaveBeenCalledTimes(1);
      const calledUrl = fetchSpy.mock.calls[0][0] as string;
      expect(calledUrl).toBe(
        `${CALENDAR_API_BASE}/users/me/calendarList`,
      );
      expect(result).toEqual([
        {
          remoteId: "primary",
          displayName: "My Calendar",
          color: "#0000ff",
          isPrimary: true,
        },
        {
          remoteId: "work@example.com",
          displayName: "Work",
          color: null,
          isPrimary: false,
        },
      ]);
    });

    it("returns empty array when no items", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({}),
      );

      const result = await provider.listCalendars();

      expect(result).toEqual([]);
    });
  });

  describe("fetchEvents", () => {
    it("passes correct URL params and maps events", async () => {
      const googleEvent = {
        id: "evt-1",
        summary: "Meeting",
        description: "Discuss plans",
        location: "Room A",
        start: { dateTime: "2025-06-15T10:00:00Z" },
        end: { dateTime: "2025-06-15T11:00:00Z" },
        status: "confirmed",
        organizer: { email: "org@example.com" },
        attendees: [{ email: "a@example.com", responseStatus: "accepted" }],
        htmlLink: "https://calendar.google.com/event/evt-1",
        iCalUID: "uid-1@google.com",
        etag: '"etag-1"',
      };

      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({ items: [googleEvent] }),
      );

      const result = await provider.fetchEvents(
        "cal-id",
        "2025-06-01T00:00:00Z",
        "2025-06-30T23:59:59Z",
      );

      const calledUrl = fetchSpy.mock.calls[0][0] as string;
      expect(calledUrl).toContain("/calendars/cal-id/events?");
      expect(calledUrl).toContain("timeMin=2025-06-01T00%3A00%3A00Z");
      expect(calledUrl).toContain("timeMax=2025-06-30T23%3A59%3A59Z");
      expect(calledUrl).toContain("singleEvents=true");
      expect(calledUrl).toContain("orderBy=startTime");
      expect(calledUrl).toContain("maxResults=250");

      expect(result).toHaveLength(1);
      expect(result[0]).toMatchObject({
        remoteEventId: "evt-1",
        summary: "Meeting",
        description: "Discuss plans",
        location: "Room A",
        isAllDay: false,
        status: "confirmed",
        organizerEmail: "org@example.com",
        htmlLink: "https://calendar.google.com/event/evt-1",
        uid: "uid-1@google.com",
        etag: '"etag-1"',
      });
      expect(result[0].startTime).toBe(
        Math.floor(new Date("2025-06-15T10:00:00Z").getTime() / 1000),
      );
      expect(result[0].endTime).toBe(
        Math.floor(new Date("2025-06-15T11:00:00Z").getTime() / 1000),
      );
    });

    it("encodes calendar ID in URL", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({ items: [] }),
      );

      await provider.fetchEvents(
        "user@example.com",
        "2025-01-01T00:00:00Z",
        "2025-01-31T23:59:59Z",
      );

      const calledUrl = fetchSpy.mock.calls[0][0] as string;
      expect(calledUrl).toContain("/calendars/user%40example.com/events?");
    });
  });

  describe("createEvent", () => {
    it("sends POST with correct body and returns mapped event", async () => {
      const createdEvent = {
        id: "new-evt",
        summary: "Lunch",
        description: "Team lunch",
        location: "Cafe",
        start: { dateTime: "2025-06-20T12:00:00Z" },
        end: { dateTime: "2025-06-20T13:00:00Z" },
        status: "confirmed",
      };

      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse(createdEvent),
      );

      const result = await provider.createEvent("cal-1", {
        summary: "Lunch",
        description: "Team lunch",
        location: "Cafe",
        startTime: "2025-06-20T12:00:00Z",
        endTime: "2025-06-20T13:00:00Z",
      });

      const options = fetchSpy.mock.calls[0][1] as RequestInit;
      expect(options.method).toBe("POST");

      const body = JSON.parse(options.body as string);
      expect(body.summary).toBe("Lunch");
      expect(body.description).toBe("Team lunch");
      expect(body.location).toBe("Cafe");
      expect(body.start.dateTime).toBeDefined();
      expect(body.end.dateTime).toBeDefined();

      expect(result.remoteEventId).toBe("new-evt");
      expect(result.summary).toBe("Lunch");
    });

    it("creates all-day event with date-only start/end", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({
          id: "allday-evt",
          summary: "Holiday",
          start: { date: "2025-12-25" },
          end: { date: "2025-12-26" },
        }),
      );

      await provider.createEvent("cal-1", {
        summary: "Holiday",
        startTime: "2025-12-25T00:00:00Z",
        endTime: "2025-12-26T00:00:00Z",
        isAllDay: true,
      });

      const body = JSON.parse(
        (fetchSpy.mock.calls[0][1] as RequestInit).body as string,
      );
      expect(body.start).toEqual({ date: "2025-12-25" });
      expect(body.end).toEqual({ date: "2025-12-26" });
    });

    it("includes attendees when provided", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({
          id: "evt-att",
          summary: "Sync",
          start: { dateTime: "2025-06-20T14:00:00Z" },
          end: { dateTime: "2025-06-20T15:00:00Z" },
        }),
      );

      await provider.createEvent("cal-1", {
        summary: "Sync",
        startTime: "2025-06-20T14:00:00Z",
        endTime: "2025-06-20T15:00:00Z",
        attendees: [{ email: "bob@example.com" }],
      });

      const body = JSON.parse(
        (fetchSpy.mock.calls[0][1] as RequestInit).body as string,
      );
      expect(body.attendees).toEqual([{ email: "bob@example.com" }]);
    });
  });

  describe("updateEvent", () => {
    it("sends PATCH with partial body", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({
          id: "evt-1",
          summary: "Updated Title",
          start: { dateTime: "2025-06-20T12:00:00Z" },
          end: { dateTime: "2025-06-20T13:00:00Z" },
        }),
      );

      const result = await provider.updateEvent("cal-1", "evt-1", {
        summary: "Updated Title",
      });

      const calledUrl = fetchSpy.mock.calls[0][0] as string;
      expect(calledUrl).toBe(
        `${CALENDAR_API_BASE}/calendars/cal-1/events/evt-1`,
      );
      const options = fetchSpy.mock.calls[0][1] as RequestInit;
      expect(options.method).toBe("PATCH");

      const body = JSON.parse(options.body as string);
      expect(body.summary).toBe("Updated Title");
      expect(body.description).toBeUndefined();
      expect(body.start).toBeUndefined();

      expect(result.remoteEventId).toBe("evt-1");
      expect(result.summary).toBe("Updated Title");
    });

    it("includes time fields when both startTime and endTime are provided", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({
          id: "evt-1",
          summary: "Rescheduled",
          start: { dateTime: "2025-06-21T09:00:00Z" },
          end: { dateTime: "2025-06-21T10:00:00Z" },
        }),
      );

      await provider.updateEvent("cal-1", "evt-1", {
        startTime: "2025-06-21T09:00:00Z",
        endTime: "2025-06-21T10:00:00Z",
      });

      const body = JSON.parse(
        (fetchSpy.mock.calls[0][1] as RequestInit).body as string,
      );
      expect(body.start.dateTime).toBeDefined();
      expect(body.end.dateTime).toBeDefined();
    });
  });

  describe("deleteEvent", () => {
    it("sends DELETE request with correct URL", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse(undefined, 204),
      );

      await provider.deleteEvent("cal-1", "evt-1");

      const calledUrl = fetchSpy.mock.calls[0][0] as string;
      expect(calledUrl).toBe(
        `${CALENDAR_API_BASE}/calendars/cal-1/events/evt-1`,
      );
      const options = fetchSpy.mock.calls[0][1] as RequestInit;
      expect(options.method).toBe("DELETE");
    });

    it("encodes calendar and event IDs", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse(undefined, 204),
      );

      await provider.deleteEvent("user@example.com", "evt/special");

      const calledUrl = fetchSpy.mock.calls[0][0] as string;
      expect(calledUrl).toContain(
        "/calendars/user%40example.com/events/evt%2Fspecial",
      );
    });
  });

  describe("syncEvents", () => {
    it("uses syncToken for incremental sync and handles cancelled events as deletions", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({
          items: [
            {
              id: "evt-updated",
              summary: "Updated Event",
              start: { dateTime: "2025-06-15T10:00:00Z" },
              end: { dateTime: "2025-06-15T11:00:00Z" },
              status: "confirmed",
            },
            {
              id: "evt-deleted",
              summary: undefined,
              start: { dateTime: "2025-06-15T10:00:00Z" },
              end: { dateTime: "2025-06-15T11:00:00Z" },
              status: "cancelled",
            },
          ],
          nextSyncToken: "new-sync-token-123",
        }),
      );

      const result = await provider.syncEvents("cal-1", "old-sync-token");

      const calledUrl = fetchSpy.mock.calls[0][0] as string;
      expect(calledUrl).toContain("syncToken=old-sync-token");
      expect(calledUrl).not.toContain("timeMin");
      expect(calledUrl).not.toContain("singleEvents");

      expect(result.created).toHaveLength(1);
      expect(result.created[0].remoteEventId).toBe("evt-updated");
      expect(result.deletedRemoteIds).toEqual(["evt-deleted"]);
      expect(result.newSyncToken).toBe("new-sync-token-123");
      expect(result.newCtag).toBeNull();
    });

    it("sets time range for initial sync without syncToken", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({
          items: [],
          nextSyncToken: "initial-token",
        }),
      );

      const result = await provider.syncEvents("cal-1");

      const calledUrl = vi.mocked(fetch).mock.calls[0][0] as string;
      expect(calledUrl).toContain("timeMin=");
      expect(calledUrl).toContain("timeMax=");
      expect(calledUrl).toContain("singleEvents=true");
      expect(calledUrl).not.toContain("syncToken");

      expect(result.newSyncToken).toBe("initial-token");
    });

    it("handles 410 error (expired sync token) gracefully", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({ error: "Gone" }, 410),
      );

      const result = await provider.syncEvents("cal-1", "expired-token");

      expect(result).toEqual({
        created: [],
        updated: [],
        deletedRemoteIds: [],
        newSyncToken: null,
        newCtag: null,
      });
    });

    it("rethrows non-sync-token errors", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({ error: "Server Error" }, 500),
      );

      await expect(provider.syncEvents("cal-1", "token")).rejects.toThrow(
        "Calendar API error: 500",
      );
    });

    it("follows pagination with nextPageToken", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch")
        .mockResolvedValueOnce(
          mockFetchResponse({
            items: [
              {
                id: "evt-1",
                summary: "Page 1",
                start: { dateTime: "2025-06-15T10:00:00Z" },
                end: { dateTime: "2025-06-15T11:00:00Z" },
              },
            ],
            nextPageToken: "page-2-token",
          }),
        )
        .mockResolvedValueOnce(
          mockFetchResponse({
            items: [
              {
                id: "evt-2",
                summary: "Page 2",
                start: { dateTime: "2025-06-16T10:00:00Z" },
                end: { dateTime: "2025-06-16T11:00:00Z" },
              },
            ],
            nextSyncToken: "final-sync-token",
          }),
        );

      const result = await provider.syncEvents("cal-1", "token");

      expect(fetchSpy).toHaveBeenCalledTimes(2);
      const secondUrl = fetchSpy.mock.calls[1][0] as string;
      expect(secondUrl).toContain("pageToken=page-2-token");

      expect(result.created).toHaveLength(2);
      expect(result.created[0].remoteEventId).toBe("evt-1");
      expect(result.created[1].remoteEventId).toBe("evt-2");
      expect(result.newSyncToken).toBe("final-sync-token");
    });
  });

  describe("testConnection", () => {
    it("returns success when listCalendars succeeds", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({ items: [] }),
      );

      const result = await provider.testConnection();

      expect(result).toEqual({
        success: true,
        message: "Connected to Google Calendar",
      });
    });

    it("returns failure with error message on error", async () => {
      vi.spyOn(globalThis, "fetch").mockResolvedValue(
        mockFetchResponse({ error: "Unauthorized" }, 401),
      );
      // 401 triggers force refresh, which also needs to return a token
      vi.mocked(invoke)
        .mockResolvedValueOnce("mock-access-token") // initial token
        .mockResolvedValueOnce("refreshed-token"); // force refresh
      // After force refresh, retry also fails
      vi.spyOn(globalThis, "fetch")
        .mockResolvedValueOnce(mockFetchResponse({ error: "Unauthorized" }, 401))
        .mockResolvedValueOnce(mockFetchResponse({ error: "Unauthorized" }, 401));

      const result = await provider.testConnection();

      expect(result).toEqual({
        success: false,
        message: expect.stringContaining("Calendar API error: 401"),
      });
    });

    it("returns generic failure message for non-Error throws", async () => {
      vi.spyOn(globalThis, "fetch").mockRejectedValue("something went wrong");

      const result = await provider.testConnection();

      expect(result).toEqual({
        success: false,
        message: "Connection failed",
      });
    });
  });

  describe("token refresh on 401", () => {
    it("retries with refreshed token after 401", async () => {
      const fetchSpy = vi.spyOn(globalThis, "fetch")
        .mockResolvedValueOnce(mockFetchResponse({ error: "Unauthorized" }, 401))
        .mockResolvedValueOnce(mockFetchResponse({ items: [] }));

      vi.mocked(invoke)
        .mockResolvedValueOnce("initial-token") // gmail_get_access_token
        .mockResolvedValueOnce("refreshed-token"); // gmail_force_refresh_token

      const result = await provider.listCalendars();

      expect(invoke).toHaveBeenCalledWith("gmail_get_access_token", {
        accountId,
      });
      expect(invoke).toHaveBeenCalledWith("gmail_force_refresh_token", {
        accountId,
      });
      expect(fetchSpy).toHaveBeenCalledTimes(2);
      expect(result).toEqual([]);
    });
  });
});
