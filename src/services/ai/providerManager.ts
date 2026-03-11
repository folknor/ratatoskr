import { invoke } from "@tauri-apps/api/core";
import { completeAi, testAiConnection } from "./client";
import type { AiProvider, AiProviderClient } from "./types";

const rustBackedProvider: AiProviderClient = {
  complete: completeAi,
  testConnection: testAiConnection,
};

export async function getActiveProviderName(): Promise<AiProvider> {
  return invoke<AiProvider>("ai_get_provider_name");
}

export async function getActiveProvider(): Promise<AiProviderClient> {
  return rustBackedProvider;
}

export async function isAiAvailable(): Promise<boolean> {
  try {
    return await invoke<boolean>("ai_is_available");
  } catch {
    return false;
  }
}

export function clearProviderClients(): void {}
