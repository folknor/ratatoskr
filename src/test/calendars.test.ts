import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import type { DbCalendar } from "./calendars";
import {
  deleteCalendarsForAccount,
  getCalendarById,
  getCalendarsForAccount,
  getVisibleCalendars,
  setCalendarVisibility,
  updateCalendarSyncToken,
  upsertCalendar,
} from "./calendars";

const mockInvoke = vi.mocked(invoke);

describe("calendars service", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  describe("upsertCalendar", () => {
    it("invokes the Rust command and returns the id", async () => {
      mockInvoke.mockResolvedValueOnce("cal-returned-id");

      const id = await upsertCalendar({
        accountId: "acc-1",
        provider: "google",
        remoteId: "remote-cal-1",
        displayName: "My Calendar",
        color: "#4285f4",
        isPrimary: true,
      });

      expect(id).toBe("cal-returned-id");
      expect(mockInvoke).toHaveBeenCalledWith("db_upsert_calendar", {
        accountId: "acc-1",
        provider: "google",
        remoteId: "remote-cal-1",
        displayName: "My Calendar",
        color: "#4285f4",
        isPrimary: true,
      });
    });
  });

  describe("getCalendarsForAccount", () => {
    it("returns calendars for the given account", async () => {
      const calendars: DbCalendar[] = [
        makeCal({
          id: "cal-1",
          account_id: "acc-1",
          is_primary: 1,
          display_name: "Primary",
        }),
        makeCal({
          id: "cal-2",
          account_id: "acc-1",
          is_primary: 0,
          display_name: "Work",
        }),
      ];
      mockInvoke.mockResolvedValueOnce(calendars);

      const result = await getCalendarsForAccount("acc-1");

      expect(result).toEqual(calendars);
      expect(mockInvoke).toHaveBeenCalledWith("db_get_calendars_for_account", {
        accountId: "acc-1",
      });
    });

    it("returns empty array when no calendars exist", async () => {
      mockInvoke.mockResolvedValueOnce([]);

      const result = await getCalendarsForAccount("acc-none");

      expect(result).toEqual([]);
    });
  });

  describe("getVisibleCalendars", () => {
    it("only returns visible calendars", async () => {
      const visible = [makeCal({ id: "cal-1", is_visible: 1 })];
      mockInvoke.mockResolvedValueOnce(visible);

      const result = await getVisibleCalendars("acc-1");

      expect(result).toEqual(visible);
      expect(mockInvoke).toHaveBeenCalledWith("db_get_visible_calendars", {
        accountId: "acc-1",
      });
    });
  });

  describe("setCalendarVisibility", () => {
    it("sets visibility to true", async () => {
      await setCalendarVisibility("cal-1", true);

      expect(mockInvoke).toHaveBeenCalledWith("db_set_calendar_visibility", {
        calendarId: "cal-1",
        visible: true,
      });
    });

    it("sets visibility to false", async () => {
      await setCalendarVisibility("cal-1", false);

      expect(mockInvoke).toHaveBeenCalledWith("db_set_calendar_visibility", {
        calendarId: "cal-1",
        visible: false,
      });
    });
  });

  describe("updateCalendarSyncToken", () => {
    it("updates sync_token and ctag", async () => {
      await updateCalendarSyncToken("cal-1", "sync-abc", "ctag-xyz");

      expect(mockInvoke).toHaveBeenCalledWith("db_update_calendar_sync_token", {
        calendarId: "cal-1",
        syncToken: "sync-abc",
        ctag: "ctag-xyz",
      });
    });

    it("sets ctag to null when not provided", async () => {
      await updateCalendarSyncToken("cal-1", "sync-abc");

      expect(mockInvoke).toHaveBeenCalledWith("db_update_calendar_sync_token", {
        calendarId: "cal-1",
        syncToken: "sync-abc",
        ctag: null,
      });
    });

    it("allows null sync_token", async () => {
      await updateCalendarSyncToken("cal-1", null, "ctag-xyz");

      expect(mockInvoke).toHaveBeenCalledWith("db_update_calendar_sync_token", {
        calendarId: "cal-1",
        syncToken: null,
        ctag: "ctag-xyz",
      });
    });
  });

  describe("deleteCalendarsForAccount", () => {
    it("deletes all calendars for the given account", async () => {
      await deleteCalendarsForAccount("acc-1");

      expect(mockInvoke).toHaveBeenCalledWith(
        "db_delete_calendars_for_account",
        { accountId: "acc-1" },
      );
    });
  });

  describe("getCalendarById", () => {
    it("returns the calendar when found", async () => {
      const cal = makeCal({ id: "cal-1" });
      mockInvoke.mockResolvedValueOnce(cal);

      const result = await getCalendarById("cal-1");

      expect(result).toEqual(cal);
      expect(mockInvoke).toHaveBeenCalledWith("db_get_calendar_by_id", {
        calendarId: "cal-1",
      });
    });

    it("returns null when calendar not found", async () => {
      mockInvoke.mockResolvedValueOnce(null);

      const result = await getCalendarById("nonexistent");

      expect(result).toBeNull();
    });
  });
});

function makeCal(overrides: Partial<DbCalendar> = {}): DbCalendar {
  return {
    id: "cal-default",
    account_id: "acc-1",
    provider: "google",
    remote_id: "remote-default",
    display_name: "Default Calendar",
    color: "#4285f4",
    is_primary: 0,
    is_visible: 1,
    sync_token: null,
    ctag: null,
    created_at: 1700000000,
    updated_at: 1700000000,
    ...overrides,
  };
}
