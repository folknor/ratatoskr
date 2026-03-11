/**
 * Core calendar facade — re-exports every calendar-related function/type used by UI components.
 * UI code should import from here instead of reaching into @/services/ directly.
 */

// CalDAV auto-discovery
// biome-ignore lint/performance/noBarrelFile: this core facade is the intended UI import boundary.
export {
  type CalDavDiscoveryResult,
  discoverCalDavSettings,
  testCalDavConnection,
} from "@/services/calendar/autoDiscovery";
export {
  applyCalendarSyncResult,
  deleteProviderEvent,
  upsertDiscoveredCalendars,
  upsertProviderEvents,
} from "@/services/calendar/persistence";
// Calendar provider
export {
  clearAllCalendarProviders,
  getCalendarProvider,
  hasCalendarSupport,
  removeCalendarProvider,
} from "@/services/calendar/providerFactory";

// Calendar types
export type {
  CalendarEventData,
  CreateEventInput,
} from "@/services/calendar/types";

// Calendar events DB
export {
  type DbCalendarEvent,
  deleteCalendarEvent,
  getCalendarEventsInRangeMulti,
} from "@/services/db/calendarEvents";

// Calendars DB
export {
  type DbCalendar,
  getCalendarsForAccount,
  getVisibleCalendars,
} from "@/services/db/calendars";
