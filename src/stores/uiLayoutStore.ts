import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";
import { persistSetting } from "./uiStoreUtils";

export interface SidebarNavItem {
  id: string;
  visible: boolean;
}

type ReadingPanePosition = "right" | "bottom" | "hidden";
type ReadFilter = "all" | "read" | "unread";

interface UILayoutState {
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
}

export const useUILayoutStore: UseBoundStore<StoreApi<UILayoutState>> =
  create<UILayoutState>((set) => ({
    sidebarCollapsed: false,
    contactSidebarVisible: true,
    readingPanePosition: "right",
    readFilter: "all",
    emailListWidth: 320,
    taskSidebarVisible: false,
    sidebarNavConfig: null,

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
  }));
