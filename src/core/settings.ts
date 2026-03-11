/**
 * Core facade for settings operations.
 *
 * UI code (components, hooks, stores) should import from here
 * instead of reaching into @/services/db/settings directly.
 */

// biome-ignore lint/performance/noBarrelFile: Intentional app-facing facade for settings APIs.
export {
  getSetting,
  setSecureSetting,
  setSetting,
} from "@/services/db/settings";

// Global keyboard shortcut
export {
  DEFAULT_SHORTCUT,
  getCurrentShortcut,
  initGlobalShortcut,
  registerComposeShortcut,
  unregisterComposeShortcut,
} from "@/services/globalShortcut";
