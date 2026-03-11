import { invoke } from "@tauri-apps/api/core";

export interface UiBootstrapSnapshot {
  theme: string | null;
  sidebarCollapsed: boolean;
  contactSidebarVisible: boolean;
  readingPanePosition: string | null;
  readFilter: string | null;
  emailListWidth: string | null;
  emailDensity: string | null;
  defaultReplyMode: string | null;
  markAsReadBehavior: string | null;
  sendAndArchive: boolean;
  fontSize: string | null;
  colorTheme: string | null;
  inboxViewMode: string | null;
  reduceMotion: boolean;
  showSyncStatus: boolean;
  taskSidebarVisible: boolean;
  sidebarNavConfig: string | null;
}

export async function getUiBootstrapSnapshot(): Promise<UiBootstrapSnapshot> {
  return invoke<UiBootstrapSnapshot>("settings_get_ui_bootstrap_snapshot");
}
