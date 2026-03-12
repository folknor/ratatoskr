import { invoke } from "@tauri-apps/api/core";
import { normalizeEmail } from "@/utils/emailUtils";
import { listAccountBasicInfo } from "../accounts/basicInfo";

/**
 * Remove a client from cache (e.g., on account removal or re-auth).
 * Evicts the Rust-side Gmail client.
 */
export async function removeClient(accountId: string): Promise<void> {
  try {
    await invoke<void>("gmail_remove_client", { accountId });
  } catch {
    // Rust client may not have been initialized — safe to ignore
  }
}

/**
 * Initialize Rust-side Gmail clients for all active Gmail API accounts on app startup.
 */
export async function initializeClients(): Promise<void> {
  const accounts = await listAccountBasicInfo();

  for (const account of accounts) {
    if (account.isActive && account.provider === "gmail_api") {
      try {
        await invoke<void>("gmail_init_client", { accountId: account.id });
      } catch (err) {
        console.error(
          `Failed to init Rust Gmail client for ${account.id}:`,
          err,
        );
      }
    }
  }
}

/**
 * Re-authorize an existing account to obtain new tokens (e.g., after scope changes).
 * Preserves all local data — only replaces tokens.
 */
export async function reauthorizeAccount(
  accountId: string,
  expectedEmail: string,
): Promise<void> {
  await invoke("account_reauthorize_gmail", {
    accountId,
    expectedEmail: normalizeEmail(expectedEmail),
  });
}
