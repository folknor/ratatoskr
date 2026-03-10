import type { StoreApi, UseBoundStore } from "zustand";
import { create } from "zustand";
import type { ColorThemeId } from "@/constants/themes";
import { persistSetting } from "./uiStoreUtils";

type Theme = "light" | "dark" | "system";
export type EmailDensity = "compact" | "default" | "spacious";
export type DefaultReplyMode = "reply" | "replyAll";
export type MarkAsReadBehavior = "instant" | "2s" | "manual";
export type FontScale = "small" | "default" | "large" | "xlarge";
export type InboxViewMode = "unified" | "split";

interface UIPreferencesState {
  theme: Theme;
  colorTheme: ColorThemeId;
  emailDensity: EmailDensity;
  fontScale: FontScale;
  defaultReplyMode: DefaultReplyMode;
  markAsReadBehavior: MarkAsReadBehavior;
  sendAndArchive: boolean;
  inboxViewMode: InboxViewMode;
  reduceMotion: boolean;
  showSyncStatusBar: boolean;
  setTheme: (theme: Theme) => void;
  setColorTheme: (theme: ColorThemeId) => void;
  setEmailDensity: (density: EmailDensity) => void;
  setFontScale: (scale: FontScale) => void;
  setDefaultReplyMode: (mode: DefaultReplyMode) => void;
  setMarkAsReadBehavior: (behavior: MarkAsReadBehavior) => void;
  setSendAndArchive: (enabled: boolean) => void;
  setInboxViewMode: (mode: InboxViewMode) => void;
  setReduceMotion: (reduce: boolean) => void;
  setShowSyncStatusBar: (show: boolean) => void;
}

export const useUIPreferencesStore: UseBoundStore<
  StoreApi<UIPreferencesState>
> = create<UIPreferencesState>((set) => ({
  theme: "system",
  colorTheme: "indigo",
  emailDensity: "default",
  fontScale: "default",
  defaultReplyMode: "reply",
  markAsReadBehavior: "instant",
  sendAndArchive: false,
  inboxViewMode: "unified",
  reduceMotion: false,
  showSyncStatusBar: true,

  setTheme: (theme: Theme) => set({ theme }),
  setColorTheme: (colorTheme: ColorThemeId) => {
    persistSetting("color_theme", colorTheme);
    set({ colorTheme });
  },
  setEmailDensity: (emailDensity: EmailDensity) => {
    persistSetting("email_density", emailDensity);
    set({ emailDensity });
  },
  setFontScale: (fontScale: FontScale) => {
    persistSetting("font_size", fontScale);
    set({ fontScale });
  },
  setDefaultReplyMode: (defaultReplyMode: DefaultReplyMode) => {
    persistSetting("default_reply_mode", defaultReplyMode);
    set({ defaultReplyMode });
  },
  setMarkAsReadBehavior: (markAsReadBehavior: MarkAsReadBehavior) => {
    persistSetting("mark_as_read_behavior", markAsReadBehavior);
    set({ markAsReadBehavior });
  },
  setSendAndArchive: (sendAndArchive: boolean) => {
    persistSetting("send_and_archive", String(sendAndArchive));
    set({ sendAndArchive });
  },
  setInboxViewMode: (inboxViewMode: InboxViewMode) => {
    persistSetting("inbox_view_mode", inboxViewMode);
    set({ inboxViewMode });
  },
  setReduceMotion: (reduceMotion: boolean) => {
    persistSetting("reduce_motion", String(reduceMotion));
    set({ reduceMotion });
  },
  setShowSyncStatusBar: (showSyncStatusBar: boolean) => {
    persistSetting("show_sync_status", String(showSyncStatusBar));
    set({ showSyncStatusBar });
  },
}));
