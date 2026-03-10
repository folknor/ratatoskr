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
import { GmailClient } from "./client";

// In-memory cache of active GmailClient instances per account.
// @deprecated — kept only for callers that still use `getGmailClient()`.
// New code should use Tauri `gmail_*` commands or the EmailProvider abstraction.
const clients: Map<string, GmailClient> = new Map<string, GmailClient>();

/**
 * Get or create a GmailClient for the given account.
 *
 * @deprecated Use the Rust Gmail client via Tauri `gmail_*` commands instead.
 * This function is retained only for callers that have not yet been migrated
 * (e.g. sync.ts, calendar). It will be removed in a future phase.
 */
export async function getGmailClient(accountId: string): Promise<GmailClient> {
  const existing = clients.get(accountId);
  if (existing) return existing;

  const clientId = await getClientId();
  const clientSecret = await getClientSecret();
  const accounts = await getAllAccounts();
  const account = accounts.find((a) => a.id === accountId);

  if (!account) throw new Error(`Account ${accountId} not found`);
  if (!(account.access_token && account.refresh_token)) {
    throw new Error(`Account ${accountId} has no tokens`);
  }

  const client = new GmailClient(
    accountId,
    clientId,
    {
      accessToken: account.access_token,
      refreshToken: account.refresh_token,
      expiresAt: account.token_expires_at ?? 0,
    },
    clientSecret,
  );

  clients.set(accountId, client);
  return client;
}

/**
 * Remove a client from cache (e.g., on account removal or re-auth).
 * Also evicts the Rust-side Gmail client.
 */
export async function removeClient(accountId: string): Promise<void> {
  clients.delete(accountId);
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
 * Initialize clients for all active accounts on app startup.
 * For Gmail API accounts, initializes the Rust-side Gmail client via Tauri command.
 * Also creates legacy TS GmailClient instances for callers not yet migrated.
 */
export async function initializeClients(): Promise<void> {
  const accounts = await getAllAccounts();
  const clientId = await getSetting("google_client_id");
  if (!clientId) return;
  const clientSecret =
    (await getSecureSetting("google_client_secret")) ?? undefined;

  for (const account of accounts) {
    if (account.is_active && account.access_token && account.refresh_token) {
      // Initialize Rust-side Gmail client (canonical path)
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

      // Create legacy TS GmailClient for callers not yet migrated
      const client = new GmailClient(
        account.id,
        clientId,
        {
          accessToken: account.access_token,
          refreshToken: account.refresh_token,
          expiresAt: account.token_expires_at ?? 0,
        },
        clientSecret,
      );
      clients.set(account.id, client);
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

  // Evict stale clients and re-initialize
  clients.delete(accountId);

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

  // Create legacy TS client for callers not yet migrated
  const client = new GmailClient(
    accountId,
    clientId,
    {
      accessToken: tokens.access_token,
      refreshToken: tokens.refresh_token,
      expiresAt,
    },
    clientSecret,
  );
  clients.set(accountId, client);
}
