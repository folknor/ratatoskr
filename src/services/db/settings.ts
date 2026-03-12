import { invoke } from "@tauri-apps/api/core";
import { encryptValue } from "@/utils/crypto";

export async function getSetting(key: string): Promise<string | null> {
  return invoke<string | null>("db_get_setting", { key });
}

export async function setSetting(key: string, value: string): Promise<void> {
  await invoke("db_set_setting", { key, value });
}

/**
 * Set a setting with encryption. The value is encrypted before storing.
 */
export async function setSecureSetting(
  key: string,
  value: string,
): Promise<void> {
  const encrypted = await encryptValue(value);
  await setSetting(key, encrypted);
}
