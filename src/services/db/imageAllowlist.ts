import { invoke } from "@tauri-apps/api/core";
import { normalizeEmail } from "@/utils/emailUtils";

export async function isAllowlisted(
  accountId: string,
  senderAddress: string,
): Promise<boolean> {
  return invoke<boolean>("db_is_allowlisted", {
    accountId,
    senderAddress: normalizeEmail(senderAddress),
  });
}

/**
 * Batch-check which senders are allowlisted in a single query.
 */
export async function getAllowlistedSenders(
  accountId: string,
  senderAddresses: string[],
): Promise<Set<string>> {
  if (senderAddresses.length === 0) return new Set();
  const normalized = senderAddresses.map(normalizeEmail);
  const results = await invoke<string[]>("db_get_allowlisted_senders", {
    accountId,
    senderAddresses: normalized,
  });
  return new Set(results);
}

export async function addToAllowlist(
  accountId: string,
  senderAddress: string,
): Promise<void> {
  await invoke("db_add_to_allowlist", {
    id: crypto.randomUUID(),
    accountId,
    senderAddress: normalizeEmail(senderAddress),
  });
}

export async function removeFromAllowlist(
  accountId: string,
  senderAddress: string,
): Promise<void> {
  await invoke("db_remove_from_allowlist", {
    accountId,
    senderAddress: normalizeEmail(senderAddress),
  });
}

export interface AllowlistEntry {
  id: string;
  account_id: string;
  sender_address: string;
  created_at: number;
}

export async function getAllowlistForAccount(
  accountId: string,
): Promise<AllowlistEntry[]> {
  return invoke<AllowlistEntry[]>("db_get_allowlist_for_account", {
    accountId,
  });
}
