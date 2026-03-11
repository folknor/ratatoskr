/**
 * Core facade for account operations.
 *
 * UI code (components, hooks, stores) should import from here
 * instead of reaching into @/services/db/accounts directly.
 */

// biome-ignore lint/performance/noBarrelFile: Intentional app-facing facade for account APIs.
export {
  deleteAccount,
  insertCalDavAccount,
  insertImapAccount,
  insertJmapAccount,
  updateAccountCalDav,
} from "@/services/db/accounts";
// IMAP auto-discovery (backed by Rust discover_email_config command)
export {
  type AuthMethod,
  discoverSettings,
  extractDomain,
  getDefaultImapPort,
  getDefaultSmtpPort,
  guessServerSettings,
  type SecurityType,
  type ServerSettings,
  type WellKnownProviderResult,
} from "@/services/imap/autoDiscovery";
// OAuth (generic provider flow for IMAP accounts)
export { getOAuthProvider } from "@/services/oauth/providers";
