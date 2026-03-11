import { invoke } from "@tauri-apps/api/core";
import type { AiProvider } from "./types";

export async function getActiveProviderName(): Promise<AiProvider> {
  return invoke<AiProvider>("ai_get_provider_name");
}

export async function isAiAvailable(): Promise<boolean> {
  try {
    return await invoke<boolean>("ai_is_available");
  } catch {
    return false;
  }
}

export function clearProviderClients(): void {}
