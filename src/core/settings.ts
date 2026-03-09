/**
 * Core facade for settings operations.
 *
 * UI code (components, hooks, stores) should import from here
 * instead of reaching into @/services/db/settings directly.
 */

export {
  getAllSettings,
  getSecureSetting,
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
