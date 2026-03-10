/**
 * Barrel re-export for backwards compatibility.
 *
 * The original monolithic UIStore has been split into three focused stores:
 *   - uiLayoutStore     — sidebar, reading pane, email list, task sidebar, nav config
 *   - uiPreferencesStore — theme, density, font, reply mode, color theme, etc.
 *   - syncStateStore     — online/offline, pending ops, syncing folder
 *
 * New code should import from the specific store directly.
 * This file provides `useUIStore` as a combined facade so existing consumers
 * continue to work without changes during migration.
 */

import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";
import type { ColorThemeId } from "@/constants/themes";
import { useSyncStateStore } from "./syncStateStore";
import { useUILayoutStore } from "./uiLayoutStore";
import { useUIPreferencesStore } from "./uiPreferencesStore";

export { useSyncStateStore } from "./syncStateStore";
// Re-export types from the new stores
export type { SidebarNavItem } from "./uiLayoutStore";

// Re-export the individual stores for direct access
export { useUILayoutStore } from "./uiLayoutStore";
export type {
  DefaultReplyMode,
  EmailDensity,
  FontScale,
  InboxViewMode,
  MarkAsReadBehavior,
} from "./uiPreferencesStore";
export { useUIPreferencesStore } from "./uiPreferencesStore";

type Theme = "light" | "dark" | "system";
type ReadingPanePosition = "right" | "bottom" | "hidden";
type ReadFilter = "all" | "read" | "unread";

interface SidebarNavItem {
  id: string;
  visible: boolean;
}

/** Combined UI state — mirrors the original monolithic store interface. */
interface UIState {
  // Layout
  sidebarCollapsed: boolean;
  contactSidebarVisible: boolean;
  readingPanePosition: ReadingPanePosition;
  readFilter: ReadFilter;
  emailListWidth: number;
  taskSidebarVisible: boolean;
  sidebarNavConfig: SidebarNavItem[] | null;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  toggleContactSidebar: () => void;
  setContactSidebarVisible: (visible: boolean) => void;
  setReadingPanePosition: (position: ReadingPanePosition) => void;
  setReadFilter: (filter: ReadFilter) => void;
  setEmailListWidth: (width: number) => void;
  toggleTaskSidebar: () => void;
  setTaskSidebarVisible: (visible: boolean) => void;
  setSidebarNavConfig: (config: SidebarNavItem[]) => void;
  restoreSidebarNavConfig: (config: SidebarNavItem[]) => void;
  // Preferences
  theme: Theme;
  colorTheme: ColorThemeId;
  emailDensity: "compact" | "default" | "spacious";
  fontScale: "small" | "default" | "large" | "xlarge";
  defaultReplyMode: "reply" | "replyAll";
  markAsReadBehavior: "instant" | "2s" | "manual";
  sendAndArchive: boolean;
  inboxViewMode: "unified" | "split";
  reduceMotion: boolean;
  showSyncStatusBar: boolean;
  setTheme: (theme: Theme) => void;
  setColorTheme: (theme: ColorThemeId) => void;
  setEmailDensity: (density: "compact" | "default" | "spacious") => void;
  setFontScale: (scale: "small" | "default" | "large" | "xlarge") => void;
  setDefaultReplyMode: (mode: "reply" | "replyAll") => void;
  setMarkAsReadBehavior: (behavior: "instant" | "2s" | "manual") => void;
  setSendAndArchive: (enabled: boolean) => void;
  setInboxViewMode: (mode: "unified" | "split") => void;
  setReduceMotion: (reduce: boolean) => void;
  setShowSyncStatusBar: (show: boolean) => void;
  // Sync state
  isOnline: boolean;
  pendingOpsCount: number;
  isSyncingFolder: string | null;
  setOnline: (online: boolean) => void;
  setPendingOpsCount: (count: number) => void;
  setSyncingFolder: (folder: string | null) => void;
}

/**
 * Combined facade store that delegates to the three focused stores.
 *
 * `useUIStore(selector)` works exactly as before — it subscribes to all
 * three underlying stores and recomputes whenever any of them change.
 * For reduced re-render scope, import the specific store instead.
 */
function getCombinedState(): UIState {
  const layout = useUILayoutStore.getState();
  const prefs = useUIPreferencesStore.getState();
  const sync = useSyncStateStore.getState();
  return { ...layout, ...prefs, ...sync };
}

// Create a thin proxy store that merges all three stores.
// It subscribes to the underlying stores and pushes merged snapshots.
export const useUIStore: UseBoundStore<StoreApi<UIState>> = create<UIState>(
  () => getCombinedState(),
);

// Keep the facade in sync with the underlying stores
useUILayoutStore.subscribe(() => {
  useUIStore.setState(getCombinedState());
});
useUIPreferencesStore.subscribe(() => {
  useUIStore.setState(getCombinedState());
});
useSyncStateStore.subscribe(() => {
  useUIStore.setState(getCombinedState());
});
