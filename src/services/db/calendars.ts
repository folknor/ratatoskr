import { invoke } from "@tauri-apps/api/core";

export interface DbCalendar {
  id: string;
  account_id: string;
  provider: string;
  remote_id: string;
  display_name: string | null;
  color: string | null;
  is_primary: number;
  is_visible: number;
  sync_token: string | null;
  ctag: string | null;
  created_at: number;
  updated_at: number;
}

export async function upsertCalendar(calendar: {
  accountId: string;
  provider: string;
  remoteId: string;
  displayName: string | null;
  color: string | null;
  isPrimary: boolean;
}): Promise<string> {
  return invoke<string>("db_upsert_calendar", {
    accountId: calendar.accountId,
    provider: calendar.provider,
    remoteId: calendar.remoteId,
    displayName: calendar.displayName,
    color: calendar.color,
    isPrimary: calendar.isPrimary,
  });
}

export async function getCalendarsForAccount(
  accountId: string,
): Promise<DbCalendar[]> {
  return invoke<DbCalendar[]>("db_get_calendars_for_account", { accountId });
}

export async function getVisibleCalendars(
  accountId: string,
): Promise<DbCalendar[]> {
  return invoke<DbCalendar[]>("db_get_visible_calendars", { accountId });
}

export async function setCalendarVisibility(
  calendarId: string,
  visible: boolean,
): Promise<void> {
  await invoke("db_set_calendar_visibility", { calendarId, visible });
}

export async function updateCalendarSyncToken(
  calendarId: string,
  syncToken: string | null,
  ctag?: string | null,
): Promise<void> {
  await invoke("db_update_calendar_sync_token", {
    calendarId,
    syncToken,
    ctag: ctag ?? null,
  });
}

export async function deleteCalendarsForAccount(
  accountId: string,
): Promise<void> {
  await invoke("db_delete_calendars_for_account", { accountId });
}

export async function getCalendarById(
  calendarId: string,
): Promise<DbCalendar | null> {
  return invoke<DbCalendar | null>("db_get_calendar_by_id", { calendarId });
}
