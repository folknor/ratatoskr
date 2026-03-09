/**
 * Core app facade — re-exports app-level functions used by UI components.
 * UI code should import from here instead of reaching into @/services/ directly.
 */

// Update manager
export {
  checkForUpdateNow,
  getAvailableUpdate,
  installUpdate,
  setUpdateCallback,
  startUpdateChecker,
  stopUpdateChecker,
} from "@/services/updateManager";
