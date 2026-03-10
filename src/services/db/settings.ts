import { invoke } from "@tauri-apps/api/core";
import { decryptValue, encryptValue, isEncrypted } from "@/utils/crypto";

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
 * Get a setting that is stored encrypted. Transparently decrypts the value.
 * Falls back to returning the raw value if decryption fails (e.g. not yet encrypted).
 */
export async function getSecureSetting(key: string): Promise<string | null> {
  const raw = await getSetting(key);
  if (!raw) return null;

  if (isEncrypted(raw)) {
    try {
      return await decryptValue(raw);
    } catch {
      // If decryption fails, the value may be plaintext (pre-encryption migration)
      return raw;
    }
  }
  return raw;
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
