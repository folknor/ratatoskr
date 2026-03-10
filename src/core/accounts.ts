/**
 * Core facade for account operations.
 *
 * UI code (components, hooks, stores) should import from here
 * instead of reaching into @/services/db/accounts directly.
 */

export {
  type DbAccount,
  deleteAccount,
  getAllAccounts,
  insertAccount,
  insertCalDavAccount,
  insertGraphAccount,
  insertImapAccount,
  insertOAuthImapAccount,
  updateAccountCalDav,
} from "@/services/db/accounts";
// Gmail OAuth
export { startOAuthFlow } from "@/services/gmail/auth";
export {
  getClientId,
  getClientSecret,
} from "@/services/gmail/tokenManager";
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
export {
  type ProviderUserInfo,
  refreshProviderToken,
  startProviderOAuthFlow,
  type TokenResponse as OAuthTokenResponse,
} from "@/services/oauth/oauthFlow";
export {
  getAllOAuthProviders,
  getOAuthProvider,
  type OAuthProviderConfig,
} from "@/services/oauth/providers";
