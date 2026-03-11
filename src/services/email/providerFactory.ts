import { getAccountBasicInfo } from "../accounts/basicInfo";
import { GmailApiProvider } from "./gmailProvider";
import { GraphProvider } from "./graphProvider";
import { ImapSmtpProvider } from "./imapSmtpProvider";
import { JmapProvider } from "./jmapProvider";
import { RustBackedProviderBase } from "./rustBackedProvider";
import type { EmailProvider } from "./types";

const providers: Map<string, EmailProvider> = new Map<string, EmailProvider>();
const providerConstructors = {
  gmail_api: GmailApiProvider,
  imap: ImapSmtpProvider,
  jmap: JmapProvider,
  graph: GraphProvider,
} as const;

/**
 * Get or create the appropriate EmailProvider for the given account.
 * Providers are cached in memory by account ID.
 */
export async function getEmailProvider(
  accountId: string,
): Promise<EmailProvider> {
  const existing = providers.get(accountId);
  if (existing) return existing;

  const account = await getAccountBasicInfo(accountId);
  if (!account) throw new Error(`Account ${accountId} not found`);
  const ProviderCtor =
    providerConstructors[
      account.provider as keyof typeof providerConstructors
    ] ?? GmailApiProvider;
  const provider = new ProviderCtor(accountId);

  providers.set(accountId, provider);
  return provider;
}

/**
 * Remove a provider from cache (e.g., on account removal or re-auth).
 */
export function removeProvider(accountId: string): void {
  providers.delete(accountId);
}

/**
 * Invalidate the cached IMAP/SMTP config for a provider without removing
 * the provider itself. Call this after updating account credentials so the
 * next sync picks up the new password/host settings.
 */
export function invalidateProviderConfig(accountId: string): void {
  const existing = providers.get(accountId);
  if (existing instanceof RustBackedProviderBase) {
    existing.clearConfigCache();
  }
}

/**
 * Clear all cached providers.
 */
export function clearAllProviders(): void {
  providers.clear();
}
