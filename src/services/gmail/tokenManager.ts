import { invoke } from "@tauri-apps/api/core";
import { normalizeEmail } from "@/utils/emailUtils";
import { getCurrentUnixTimestamp } from "@/utils/timestamp";
import {
  getAccount,
  getAllAccounts,
  updateAccountAllTokens,
} from "../db/accounts";
import { getSecureSetting, getSetting } from "../db/settings";
import { startOAuthFlow } from "./auth";

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
 * Get the Google OAuth client ID from settings.
 */
export async function getClientId(): Promise<string> {
  const clientId = await getSetting("google_client_id");
  if (!clientId) {
    throw new Error(
      "Google Client ID not configured. Go to Settings to set it up.",
    );
  }
  return clientId;
}

/**
 * Get the Google OAuth client secret from settings (optional, for Web app clients).
 */
export async function getClientSecret(): Promise<string | undefined> {
  const clientSecret = await getSecureSetting("google_client_secret");
  return clientSecret ?? undefined;
}

/**
 * Initialize Rust-side Gmail clients for all active Gmail API accounts on app startup.
 */
export async function initializeClients(): Promise<void> {
  const accounts = await getAllAccounts();
  const clientId = await getSetting("google_client_id");
  if (!clientId) return;

  for (const account of accounts) {
    if (account.is_active && account.access_token && account.refresh_token) {
      if (account.provider === "gmail_api") {
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
}

/**
 * Re-authorize an existing account to obtain new tokens (e.g., after scope changes).
 * Preserves all local data — only replaces tokens.
 */
export async function reauthorizeAccount(
  accountId: string,
  expectedEmail: string,
): Promise<void> {
  const account = await getAccount(accountId);
  if (!account) throw new Error(`Account ${accountId} not found`);

  const clientId = await getClientId();
  const clientSecret = await getClientSecret();

  const { tokens, userInfo } = await startOAuthFlow(clientId, clientSecret);

  if (normalizeEmail(userInfo.email) !== normalizeEmail(expectedEmail)) {
    throw new Error(
      `Signed in as ${userInfo.email}, but expected ${expectedEmail}. Please sign in with the correct account.`,
    );
  }

  if (!tokens.refresh_token) {
    throw new Error(
      "Google did not return a refresh token. Please revoke app access at https://myaccount.google.com/permissions and try again.",
    );
  }

  const expiresAt = getCurrentUnixTimestamp() + tokens.expires_in;
  await updateAccountAllTokens(
    accountId,
    tokens.access_token,
    tokens.refresh_token,
    expiresAt,
  );

  // Re-init Rust-side client (reads fresh tokens from DB)
  try {
    await invoke<void>("gmail_remove_client", { accountId });
  } catch {
    // May not have been initialized yet
  }
  try {
    await invoke<void>("gmail_init_client", { accountId });
  } catch (err) {
    console.error(`Failed to re-init Rust Gmail client for ${accountId}:`, err);
  }
}
