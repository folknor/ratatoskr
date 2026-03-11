import { invoke } from "@tauri-apps/api/core";
import { CalDAVProvider } from "./caldavProvider";
import { GoogleCalendarProvider } from "./googleCalendarProvider";
import type { CalendarProvider } from "./types";

type CalendarProviderKind = "google_api" | "caldav";

interface CalendarProviderInfo {
  provider: CalendarProviderKind;
}

const providerCache: Map<string, CalendarProvider> = new Map<
  string,
  CalendarProvider
>();

async function getCalendarProviderInfo(
  accountId: string,
): Promise<CalendarProviderInfo | null> {
  return invoke<CalendarProviderInfo | null>(
    "account_get_calendar_provider_info",
    {
      accountId,
    },
  );
}

/**
 * Get a CalendarProvider for the given account.
 * Routes based on `account.calendar_provider` or `account.provider` for standalone CalDAV accounts.
 */
export async function getCalendarProvider(
  accountId: string,
): Promise<CalendarProvider> {
  const cached = providerCache.get(accountId);
  if (cached) return cached;

  let provider: CalendarProvider;
  const info = await getCalendarProviderInfo(accountId);

  if (info?.provider === "caldav") {
    provider = new CalDAVProvider(accountId);
  } else if (info?.provider === "google_api") {
    provider = new GoogleCalendarProvider(accountId);
  } else {
    throw new Error(`No calendar provider configured for account ${accountId}`);
  }

  providerCache.set(accountId, provider);
  return provider;
}

/**
 * Check if an account has calendar support configured.
 */
export async function hasCalendarSupport(accountId: string): Promise<boolean> {
  const info = await getCalendarProviderInfo(accountId);
  return info !== null;
}

export function removeCalendarProvider(accountId: string): void {
  providerCache.delete(accountId);
}

export function clearAllCalendarProviders(): void {
  providerCache.clear();
}
