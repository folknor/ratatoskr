import { getSettingsBootstrapSnapshot } from "./bootstrapSnapshot";
import { getUiBootstrapSnapshot } from "./uiBootstrap";

export async function getUndoSendDelaySeconds(): Promise<number> {
  const snapshot = await getSettingsBootstrapSnapshot();
  return parseInt(snapshot.undoSendDelaySeconds ?? "5", 10);
}

export async function getBlockRemoteImagesEnabled(): Promise<boolean> {
  const snapshot = await getSettingsBootstrapSnapshot();
  return snapshot.blockRemoteImages;
}

export async function getNotificationsEnabled(): Promise<boolean> {
  const snapshot = await getSettingsBootstrapSnapshot();
  return snapshot.notificationsEnabled;
}

export async function getAttachmentCacheMaxMb(): Promise<number> {
  const snapshot = await getSettingsBootstrapSnapshot();
  return parseInt(snapshot.attachmentCacheMaxMb ?? "500", 10);
}

export async function getPhishingSettings(): Promise<{
  enabled: boolean;
  sensitivity: "low" | "default" | "high";
}> {
  const snapshot = await getSettingsBootstrapSnapshot();
  return {
    enabled: snapshot.phishingDetectionEnabled,
    sensitivity:
      snapshot.phishingSensitivity === "low" ||
      snapshot.phishingSensitivity === "high"
        ? snapshot.phishingSensitivity
        : "default",
  };
}

export async function getAiWritingFlags(): Promise<{
  writingStyleEnabled: boolean;
  autoDraftEnabled: boolean;
}> {
  const snapshot = await getSettingsBootstrapSnapshot();
  return {
    writingStyleEnabled: snapshot.aiWritingStyleEnabled,
    autoDraftEnabled: snapshot.aiAutoDraftEnabled,
  };
}

export async function getStoredLanguagePreference(): Promise<string | null> {
  const snapshot = await getUiBootstrapSnapshot();
  return snapshot.language;
}

export async function getGlobalComposeShortcut(): Promise<string | null> {
  const snapshot = await getUiBootstrapSnapshot();
  return snapshot.globalComposeShortcut;
}

export async function getCustomShortcutOverrides(): Promise<string | null> {
  const snapshot = await getUiBootstrapSnapshot();
  return snapshot.customShortcuts;
}

export async function getStartupUiSettings(): Promise<{
  activeAccountId: string | null;
  searchIndexVersion: string | null;
}> {
  const snapshot = await getUiBootstrapSnapshot();
  return {
    activeAccountId: snapshot.activeAccountId,
    searchIndexVersion: snapshot.searchIndexVersion,
  };
}
