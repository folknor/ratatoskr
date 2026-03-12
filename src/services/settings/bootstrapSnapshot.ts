import { invoke } from "@tauri-apps/api/core";

export interface SettingsBootstrapSnapshot {
  notificationsEnabled: boolean;
  undoSendDelaySeconds: string | null;
  blockRemoteImages: boolean;
  phishingDetectionEnabled: boolean;
  phishingSensitivity: string | null;
  syncPeriodDays: string | null;
  aiProvider: string | null;
  ollamaServerUrl: string | null;
  ollamaModel: string | null;
  claudeModel: string | null;
  openaiModel: string | null;
  geminiModel: string | null;
  copilotModel: string | null;
  aiEnabled: boolean;
  aiAutoCategorize: boolean;
  aiAutoSummarize: boolean;
  aiAutoDraftEnabled: boolean;
  aiWritingStyleEnabled: boolean;
  autoArchiveCategories: string | null;
  smartNotifications: boolean;
  notifyCategories: string | null;
  attachmentCacheMaxMb: string | null;
}

export interface SettingsSecretsSnapshot {
  claudeApiKey: string | null;
  openaiApiKey: string | null;
  geminiApiKey: string | null;
  copilotApiKey: string | null;
}

export async function getSettingsBootstrapSnapshot(): Promise<SettingsBootstrapSnapshot> {
  return invoke<SettingsBootstrapSnapshot>("settings_get_bootstrap_snapshot");
}

export async function getSettingsSecretsSnapshot(): Promise<SettingsSecretsSnapshot> {
  return invoke<SettingsSecretsSnapshot>("settings_get_secrets_snapshot");
}
