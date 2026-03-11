import { invoke } from "@tauri-apps/api/core";
import { encryptValue } from "@/utils/crypto";

interface SettingRow {
  key: string;
  value: string;
}

export async function getSetting(key: string): Promise<string | null> {
  return invoke<string | null>("db_get_setting", { key });
}

export async function setSetting(key: string, value: string): Promise<void> {
  await invoke("db_set_setting", { key, value });
}

export async function getAllSettings(): Promise<Record<string, string>> {
  const rows = await invoke<SettingRow[]>("db_get_all_settings");
  return Object.fromEntries(rows.map((r) => [r.key, r.value]));
}

/**
 * Get a setting that is stored encrypted. Decryption happens in Rust.
 */
export async function getSecureSetting(key: string): Promise<string | null> {
  return invoke<string | null>("db_get_secure_setting", { key });
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
