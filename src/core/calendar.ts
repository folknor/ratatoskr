/**
 * Core calendar facade — re-exports every calendar-related function/type used by UI components.
 * UI code should import from here instead of reaching into @/services/ directly.
 */

// Calendar provider
export {
  clearAllCalendarProviders,
  getCalendarProvider,
  hasCalendarSupport,
  removeCalendarProvider,
} from "@/services/calendar/providerFactory";

// CalDAV auto-discovery
export {
  type CalDavDiscoveryResult,
  discoverCalDavSettings,
  testCalDavConnection,
} from "@/services/calendar/autoDiscovery";

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
  upsertCalendarEvent,
} from "@/services/db/calendarEvents";

// Calendars DB
export {
  type DbCalendar,
  getCalendarsForAccount,
  getVisibleCalendars,
  upsertCalendar,
} from "@/services/db/calendars";
