import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import {
  type DbCalendarEvent,
  deleteCalendarEvent,
  deleteEventByRemoteId,
  deleteEventsForCalendar,
  getCalendarEventsInRange,
  getCalendarEventsInRangeMulti,
  getEventByRemoteId,
  upsertCalendarEvent,
} from "./calendarEvents";

const mockInvoke = vi.mocked(invoke);

const makeEvent = (
  overrides: Partial<DbCalendarEvent> = {},
): DbCalendarEvent => ({
  id: "evt-1",
  account_id: "acc-1",
  google_event_id: "gev-1",
  summary: "Team standup",
  description: "Daily sync",
  location: "Room A",
  start_time: 1000,
  end_time: 2000,
  is_all_day: 0,
  status: "confirmed",
  organizer_email: "org@example.com",
  attendees_json: null,
  html_link: "https://calendar.google.com/event/1",
  updated_at: 999,
  calendar_id: null,
  remote_event_id: null,
  etag: null,
  ical_data: null,
  uid: null,
  ...overrides,
});

describe("calendarEvents service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("upsertCalendarEvent", () => {
    it("invokes Rust command with all fields including CalDAV fields", async () => {
      await upsertCalendarEvent({
        accountId: "acc-1",
        googleEventId: "gev-1",
        summary: "Team standup",
        description: "Daily sync",
        location: "Room A",
        startTime: 1000,
        endTime: 2000,
        isAllDay: false,
        status: "confirmed",
        organizerEmail: "org@example.com",
        attendeesJson: '[{"email":"a@b.com"}]',
        htmlLink: "https://calendar.google.com/event/1",
        calendarId: "cal-1",
        remoteEventId: "remote-1",
        etag: '"etag-abc"',
        icalData: "BEGIN:VEVENT\nEND:VEVENT",
        uid: "uid-123@example.com",
      });

      expect(mockInvoke).toHaveBeenCalledWith("db_upsert_calendar_event", {
        accountId: "acc-1",
        googleEventId: "gev-1",
        summary: "Team standup",
        description: "Daily sync",
        location: "Room A",
        startTime: 1000,
        endTime: 2000,
        isAllDay: false,
        status: "confirmed",
        organizerEmail: "org@example.com",
        attendeesJson: '[{"email":"a@b.com"}]',
        htmlLink: "https://calendar.google.com/event/1",
        calendarId: "cal-1",
        remoteEventId: "remote-1",
        etag: '"etag-abc"',
        icalData: "BEGIN:VEVENT\nEND:VEVENT",
        uid: "uid-123@example.com",
      });
    });

    it("defaults optional CalDAV fields to null", async () => {
      await upsertCalendarEvent({
        accountId: "acc-1",
        googleEventId: "gev-3",
        summary: null,
        description: null,
        location: null,
        startTime: 1000,
        endTime: 2000,
        isAllDay: false,
        status: "confirmed",
        organizerEmail: null,
        attendeesJson: null,
        htmlLink: null,
      });

      expect(mockInvoke).toHaveBeenCalledWith("db_upsert_calendar_event", {
        accountId: "acc-1",
        googleEventId: "gev-3",
        summary: null,
        description: null,
        location: null,
        startTime: 1000,
        endTime: 2000,
        isAllDay: false,
        status: "confirmed",
        organizerEmail: null,
        attendeesJson: null,
        htmlLink: null,
        calendarId: null,
        remoteEventId: null,
        etag: null,
        icalData: null,
        uid: null,
      });
    });
  });

  describe("getCalendarEventsInRange", () => {
    it("returns events within the given time range", async () => {
      const events = [
        makeEvent(),
        makeEvent({ id: "evt-2", start_time: 1500 }),
      ];
      mockInvoke.mockResolvedValueOnce(events);

      const result = await getCalendarEventsInRange("acc-1", 500, 2500);

      expect(result).toEqual(events);
      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_calendar_events_in_range",
        { accountId: "acc-1", startTime: 500, endTime: 2500 },
      );
    });

    it("returns empty array when no events match", async () => {
      mockInvoke.mockResolvedValueOnce([]);

      const result = await getCalendarEventsInRange("acc-1", 5000, 6000);

      expect(result).toEqual([]);
    });
  });

  describe("getCalendarEventsInRangeMulti", () => {
    it("filters by calendar IDs via Rust command", async () => {
      const events = [
        makeEvent({ calendar_id: "cal-1" }),
        makeEvent({ id: "evt-2", calendar_id: null }),
      ];
      mockInvoke.mockResolvedValueOnce(events);

      const result = await getCalendarEventsInRangeMulti(
        "acc-1",
        ["cal-1", "cal-2"],
        500,
        2500,
      );

      expect(result).toEqual(events);
      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_calendar_events_in_range_multi",
        {
          accountId: "acc-1",
          calendarIds: ["cal-1", "cal-2"],
          startTime: 500,
          endTime: 2500,
        },
      );
    });

    it("falls back to getCalendarEventsInRange when calendarIds is empty", async () => {
      const events = [makeEvent()];
      mockInvoke.mockResolvedValueOnce(events);

      const result = await getCalendarEventsInRangeMulti(
        "acc-1",
        [],
        500,
        2500,
      );

      expect(result).toEqual(events);
      // Should call the simple range query
      expect(mockInvoke).toHaveBeenCalledWith(
        "db_get_calendar_events_in_range",
        { accountId: "acc-1", startTime: 500, endTime: 2500 },
      );
    });
  });

  describe("deleteEventsForCalendar", () => {
    it("removes all events for a given calendar_id", async () => {
      await deleteEventsForCalendar("cal-1");

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_delete_events_for_calendar",
        { calendarId: "cal-1" },
      );
    });
  });

  describe("getEventByRemoteId", () => {
    it("returns event matching calendar_id and remote_event_id", async () => {
      const event = makeEvent({
        calendar_id: "cal-1",
        remote_event_id: "remote-1",
      });
      mockInvoke.mockResolvedValueOnce(event);

      const result = await getEventByRemoteId("cal-1", "remote-1");

      expect(result).toEqual(event);
      expect(mockInvoke).toHaveBeenCalledWith("db_get_event_by_remote_id", {
        calendarId: "cal-1",
        remoteEventId: "remote-1",
      });
    });

    it("returns null when no event matches", async () => {
      mockInvoke.mockResolvedValueOnce(null);

      const result = await getEventByRemoteId("cal-1", "nonexistent");

      expect(result).toBeNull();
    });
  });

  describe("deleteEventByRemoteId", () => {
    it("removes event matching calendar_id and remote_event_id", async () => {
      await deleteEventByRemoteId("cal-1", "remote-1");

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_delete_event_by_remote_id",
        { calendarId: "cal-1", remoteEventId: "remote-1" },
      );
    });
  });

  describe("deleteCalendarEvent", () => {
    it("removes event by id", async () => {
      await deleteCalendarEvent("evt-1");

      expect(mockInvoke).toHaveBeenCalledWith("db_delete_calendar_event", {
        eventId: "evt-1",
      });
    });
  });
});
