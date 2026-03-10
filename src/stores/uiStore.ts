import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";
import type { ColorThemeId } from "@/constants/themes";
import { setSetting } from "@/core/settings";

/** Fire-and-forget setting persistence with error logging. */
function persistSetting(key: string, value: string): void {
  setSetting(key, value).catch((err: unknown) => {
    console.error(`Failed to persist setting "${key}":`, err);
  });
}

type Theme = "light" | "dark" | "system";
type ReadingPanePosition = "right" | "bottom" | "hidden";
type ReadFilter = "all" | "read" | "unread";
export type EmailDensity = "compact" | "default" | "spacious";
export type DefaultReplyMode = "reply" | "replyAll";
export type MarkAsReadBehavior = "instant" | "2s" | "manual";
export type FontScale = "small" | "default" | "large" | "xlarge";
export type InboxViewMode = "unified" | "split";

export interface SidebarNavItem {
  id: string;
  visible: boolean;
}

interface UIState {
  theme: Theme;
  sidebarCollapsed: boolean;
  contactSidebarVisible: boolean;
  readingPanePosition: ReadingPanePosition;
  readFilter: ReadFilter;
  emailListWidth: number;
  emailDensity: EmailDensity;
  defaultReplyMode: DefaultReplyMode;
  markAsReadBehavior: MarkAsReadBehavior;
  fontScale: FontScale;
  colorTheme: ColorThemeId;
  sendAndArchive: boolean;
  inboxViewMode: InboxViewMode;
  taskSidebarVisible: boolean;
  sidebarNavConfig: SidebarNavItem[] | null;
  reduceMotion: boolean;
  showSyncStatusBar: boolean;
  isOnline: boolean;
  pendingOpsCount: number;
  isSyncingFolder: string | null;
  setTheme: (theme: Theme) => void;
  toggleSidebar: () => void;
  setSidebarCollapsed: (collapsed: boolean) => void;
  toggleContactSidebar: () => void;
  setContactSidebarVisible: (visible: boolean) => void;
  setReadingPanePosition: (position: ReadingPanePosition) => void;
  setReadFilter: (filter: ReadFilter) => void;
  setEmailListWidth: (width: number) => void;
  setEmailDensity: (density: EmailDensity) => void;
  setDefaultReplyMode: (mode: DefaultReplyMode) => void;
  setMarkAsReadBehavior: (behavior: MarkAsReadBehavior) => void;
  setFontScale: (scale: FontScale) => void;
  setColorTheme: (theme: ColorThemeId) => void;
  setSendAndArchive: (enabled: boolean) => void;
  setInboxViewMode: (mode: InboxViewMode) => void;
  toggleTaskSidebar: () => void;
  setTaskSidebarVisible: (visible: boolean) => void;
  setSidebarNavConfig: (config: SidebarNavItem[]) => void;
  restoreSidebarNavConfig: (config: SidebarNavItem[]) => void;
  setReduceMotion: (reduce: boolean) => void;
  setShowSyncStatusBar: (show: boolean) => void;
  setOnline: (online: boolean) => void;
  setPendingOpsCount: (count: number) => void;
  setSyncingFolder: (folder: string | null) => void;
}

export const useUIStore: UseBoundStore<StoreApi<UIState>> = create<UIState>(
  (set) => ({
    theme: "system",
    sidebarCollapsed: false,
    contactSidebarVisible: true,
    readingPanePosition: "right",
    readFilter: "all",
    emailListWidth: 320,
    emailDensity: "default",
    defaultReplyMode: "reply",
    markAsReadBehavior: "instant",
    fontScale: "default",
    colorTheme: "indigo",
    sendAndArchive: false,
    inboxViewMode: "unified",
    taskSidebarVisible: false,
    sidebarNavConfig: null,
    reduceMotion: false,
    showSyncStatusBar: true,
    isOnline: true,
    pendingOpsCount: 0,
    isSyncingFolder: null,

    setTheme: (theme: Theme) => set({ theme }),
    toggleSidebar: () =>
      set((state) => {
        const collapsed = !state.sidebarCollapsed;
        persistSetting("sidebar_collapsed", String(collapsed));
        return { sidebarCollapsed: collapsed };
      }),
    setSidebarCollapsed: (sidebarCollapsed: boolean) =>
      set({ sidebarCollapsed }),
    toggleContactSidebar: () =>
      set((state) => {
        const visible = !state.contactSidebarVisible;
        persistSetting("contact_sidebar_visible", String(visible));
        return { contactSidebarVisible: visible };
      }),
    setContactSidebarVisible: (contactSidebarVisible: boolean) =>
      set({ contactSidebarVisible }),
    setReadingPanePosition: (readingPanePosition: ReadingPanePosition) => {
      persistSetting("reading_pane_position", readingPanePosition);
      set({ readingPanePosition });
    },
    setReadFilter: (readFilter: ReadFilter) => {
      persistSetting("read_filter", readFilter);
      set({ readFilter });
    },
    setEmailListWidth: (emailListWidth: number) => {
      persistSetting("email_list_width", String(emailListWidth));
      set({ emailListWidth });
    },
    setEmailDensity: (emailDensity: EmailDensity) => {
      persistSetting("email_density", emailDensity);
      set({ emailDensity });
    },
    setDefaultReplyMode: (defaultReplyMode: DefaultReplyMode) => {
      persistSetting("default_reply_mode", defaultReplyMode);
      set({ defaultReplyMode });
    },
    setMarkAsReadBehavior: (markAsReadBehavior: MarkAsReadBehavior) => {
      persistSetting("mark_as_read_behavior", markAsReadBehavior);
      set({ markAsReadBehavior });
    },
    setFontScale: (fontScale: FontScale) => {
      persistSetting("font_size", fontScale);
      set({ fontScale });
    },
    setColorTheme: (colorTheme: ColorThemeId) => {
      persistSetting("color_theme", colorTheme);
      set({ colorTheme });
    },
    setSendAndArchive: (sendAndArchive: boolean) => {
      persistSetting("send_and_archive", String(sendAndArchive));
      set({ sendAndArchive });
    },
    setInboxViewMode: (inboxViewMode: InboxViewMode) => {
      persistSetting("inbox_view_mode", inboxViewMode);
      set({ inboxViewMode });
    },
    toggleTaskSidebar: () =>
      set((state) => {
        const visible = !state.taskSidebarVisible;
        persistSetting("task_sidebar_visible", String(visible));
        return { taskSidebarVisible: visible };
      }),
    setTaskSidebarVisible: (taskSidebarVisible: boolean) =>
      set({ taskSidebarVisible }),
    setSidebarNavConfig: (sidebarNavConfig: SidebarNavItem[]) => {
      persistSetting("sidebar_nav_config", JSON.stringify(sidebarNavConfig));
      set({ sidebarNavConfig });
    },
    restoreSidebarNavConfig: (sidebarNavConfig: SidebarNavItem[]) =>
      set({ sidebarNavConfig }),
    setReduceMotion: (reduceMotion: boolean) => {
      persistSetting("reduce_motion", String(reduceMotion));
      set({ reduceMotion });
    },
    setShowSyncStatusBar: (showSyncStatusBar: boolean) => {
      persistSetting("show_sync_status", String(showSyncStatusBar));
      set({ showSyncStatusBar });
    },
    setOnline: (isOnline: boolean) => set({ isOnline }),
    setPendingOpsCount: (pendingOpsCount: number) => set({ pendingOpsCount }),
    setSyncingFolder: (isSyncingFolder: string | null) =>
      set({ isSyncingFolder }),
  }),
);
