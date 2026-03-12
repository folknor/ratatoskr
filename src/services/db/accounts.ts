import { invoke } from "@tauri-apps/api/core";
import { decryptValue, encryptValue, isEncrypted } from "@/utils/crypto";

export interface DbAccount {
  id: string;
  email: string;
  display_name: string | null;
  avatar_url: string | null;
  access_token: string | null;
  refresh_token: string | null;
  token_expires_at: number | null;
  history_id: string | null;
  last_sync_at: number | null;
  is_active: number;
  created_at: number;
  updated_at: number;
  provider: string;
  imap_host: string | null;
  imap_port: number | null;
  imap_security: string | null;
  smtp_host: string | null;
  smtp_port: number | null;
  smtp_security: string | null;
  auth_method: string;
  imap_password: string | null;
  oauth_provider: string | null;
  oauth_client_id: string | null;
  oauth_client_secret: string | null;
  imap_username: string | null;
  caldav_url: string | null;
  caldav_username: string | null;
  caldav_password: string | null;
  caldav_principal_url: string | null;
  caldav_home_url: string | null;
  calendar_provider: string | null;
  accept_invalid_certs: number;
  jmap_url: string | null;
}

async function decryptAccountTokens(account: DbAccount): Promise<DbAccount> {
  async function decryptField(
    value: string | null,
    fieldName: string,
  ): Promise<string | null> {
    if (!(value && isEncrypted(value))) {
      return value;
    }
    try {
      return await decryptValue(value);
    } catch (err) {
      console.warn(
        `Failed to decrypt ${fieldName}, clearing stored value:`,
        err,
      );
      return null;
    }
  }

  account.access_token = await decryptField(
    account.access_token,
    "access token",
  );
  account.refresh_token = await decryptField(
    account.refresh_token,
    "refresh token",
  );
  account.imap_password = await decryptField(
    account.imap_password,
    "IMAP password",
  );
  account.oauth_client_secret = await decryptField(
    account.oauth_client_secret,
    "OAuth client secret",
  );
  account.caldav_password = await decryptField(
    account.caldav_password,
    "CalDAV password",
  );
  return account;
}

export async function getAllAccounts(): Promise<DbAccount[]> {
  const accounts = await invoke<DbAccount[]>("db_get_all_accounts");
  return Promise.all(accounts.map(decryptAccountTokens));
}

export async function getAccount(id: string): Promise<DbAccount | null> {
  const account = await invoke<DbAccount | null>("db_get_account", { id });
  return account ? decryptAccountTokens(account) : null;
}

export async function getAccountByEmail(
  email: string,
): Promise<DbAccount | null> {
  const account = await invoke<DbAccount | null>("db_get_account_by_email", {
    email,
  });
  return account ? decryptAccountTokens(account) : null;
}

export async function insertAccount(account: {
  id: string;
  email: string;
  displayName: string | null;
  avatarUrl: string | null;
  accessToken: string;
  refreshToken: string;
  tokenExpiresAt: number;
}): Promise<void> {
  const encAccessToken = await encryptValue(account.accessToken);
  const encRefreshToken = await encryptValue(account.refreshToken);
  return invoke("db_insert_account", {
    id: account.id,
    email: account.email,
    displayName: account.displayName,
    avatarUrl: account.avatarUrl,
    accessToken: encAccessToken,
    refreshToken: encRefreshToken,
    tokenExpiresAt: account.tokenExpiresAt,
    provider: "gmail_api",
    authMethod: "oauth2",
  });
}

export async function updateAccountTokens(
  id: string,
  accessToken: string,
  tokenExpiresAt: number,
): Promise<void> {
  const encAccessToken = await encryptValue(accessToken);
  return invoke("db_update_account_tokens", {
    id,
    accessToken: encAccessToken,
    tokenExpiresAt,
  });
}

export async function updateAccountSyncState(
  id: string,
  historyId: string,
): Promise<void> {
  return invoke("db_update_account_sync_state", { id, historyId });
}

export async function clearAccountHistoryId(id: string): Promise<void> {
  return invoke("db_clear_account_history_id", { id });
}

export async function updateAccountAllTokens(
  id: string,
  accessToken: string,
  refreshToken: string,
  tokenExpiresAt: number,
): Promise<void> {
  const encAccessToken = await encryptValue(accessToken);
  const encRefreshToken = await encryptValue(refreshToken);
  return invoke("db_update_account_all_tokens", {
    id,
    accessToken: encAccessToken,
    refreshToken: encRefreshToken,
    tokenExpiresAt,
  });
}

export async function deleteAccount(id: string): Promise<void> {
  return invoke("account_delete", { accountId: id });
}

export async function insertImapAccount(account: {
  id: string;
  email: string;
  displayName: string | null;
  avatarUrl: string | null;
  imapHost: string;
  imapPort: number;
  imapSecurity: string;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: string;
  authMethod: string;
  password: string;
  imapUsername?: string | null;
  acceptInvalidCerts?: boolean;
}): Promise<void> {
  const encPassword = await encryptValue(account.password);
  return invoke("db_insert_account", {
    id: account.id,
    email: account.email,
    displayName: account.displayName,
    avatarUrl: account.avatarUrl,
    provider: "imap",
    authMethod: account.authMethod,
    imapHost: account.imapHost,
    imapPort: account.imapPort,
    imapSecurity: account.imapSecurity,
    smtpHost: account.smtpHost,
    smtpPort: account.smtpPort,
    smtpSecurity: account.smtpSecurity,
    imapPassword: encPassword,
    imapUsername: account.imapUsername || null,
    acceptInvalidCerts: account.acceptInvalidCerts ? 1 : 0,
  });
}

export async function insertCalDavAccount(account: {
  id: string;
  email: string;
  displayName: string | null;
  caldavUrl: string;
  caldavUsername: string;
  caldavPassword: string;
  caldavPrincipalUrl?: string | null;
  caldavHomeUrl?: string | null;
}): Promise<void> {
  const encPassword = await encryptValue(account.caldavPassword);
  return invoke("db_insert_account", {
    id: account.id,
    email: account.email,
    displayName: account.displayName,
    provider: "caldav",
    authMethod: "password",
    calendarProvider: "caldav",
    caldavUrl: account.caldavUrl,
    caldavUsername: account.caldavUsername,
    caldavPassword: encPassword,
    caldavPrincipalUrl: account.caldavPrincipalUrl ?? null,
    caldavHomeUrl: account.caldavHomeUrl ?? null,
  });
}

export async function insertJmapAccount(account: {
  id: string;
  email: string;
  displayName: string | null;
  jmapUrl: string;
  password: string;
  username?: string | null;
}): Promise<void> {
  const encPassword = await encryptValue(account.password);
  return invoke("db_insert_account", {
    id: account.id,
    email: account.email,
    displayName: account.displayName,
    provider: "jmap",
    authMethod: "password",
    jmapUrl: account.jmapUrl,
    imapPassword: encPassword,
    imapUsername: account.username || null,
  });
}

export async function insertGraphAccount(account: {
  id: string;
  email: string;
  displayName: string | null;
  avatarUrl?: string | null;
  accessToken: string;
  refreshToken: string;
  tokenExpiresAt: number;
}): Promise<void> {
  const encAccessToken = await encryptValue(account.accessToken);
  const encRefreshToken = await encryptValue(account.refreshToken);
  return invoke("db_insert_account", {
    id: account.id,
    email: account.email,
    displayName: account.displayName,
    avatarUrl: account.avatarUrl ?? null,
    provider: "graph",
    authMethod: "oauth2",
    accessToken: encAccessToken,
    refreshToken: encRefreshToken,
    tokenExpiresAt: account.tokenExpiresAt,
  });
}

export async function updateAccountCalDav(
  accountId: string,
  fields: {
    caldavUrl: string;
    caldavUsername: string;
    caldavPassword: string;
    caldavPrincipalUrl?: string | null;
    caldavHomeUrl?: string | null;
    calendarProvider: string;
  },
): Promise<void> {
  const encPassword = await encryptValue(fields.caldavPassword);
  return invoke("db_update_account_caldav", {
    id: accountId,
    caldavUrl: fields.caldavUrl,
    caldavUsername: fields.caldavUsername,
    caldavPassword: encPassword,
    caldavPrincipalUrl: fields.caldavPrincipalUrl ?? null,
    caldavHomeUrl: fields.caldavHomeUrl ?? null,
    calendarProvider: fields.calendarProvider,
  });
}

export async function insertOAuthImapAccount(account: {
  id: string;
  email: string;
  displayName: string | null;
  avatarUrl: string | null;
  imapHost: string;
  imapPort: number;
  imapSecurity: string;
  smtpHost: string;
  smtpPort: number;
  smtpSecurity: string;
  accessToken: string;
  refreshToken: string;
  tokenExpiresAt: number;
  oauthProvider: string;
  oauthClientId: string;
  oauthClientSecret: string | null;
  imapUsername?: string | null;
  acceptInvalidCerts?: boolean;
}): Promise<void> {
  const encAccessToken = await encryptValue(account.accessToken);
  const encRefreshToken = await encryptValue(account.refreshToken);
  const encClientSecret = account.oauthClientSecret
    ? await encryptValue(account.oauthClientSecret)
    : null;
  return invoke("db_insert_account", {
    id: account.id,
    email: account.email,
    displayName: account.displayName,
    avatarUrl: account.avatarUrl,
    accessToken: encAccessToken,
    refreshToken: encRefreshToken,
    tokenExpiresAt: account.tokenExpiresAt,
    provider: "imap",
    authMethod: "oauth2",
    imapHost: account.imapHost,
    imapPort: account.imapPort,
    imapSecurity: account.imapSecurity,
    smtpHost: account.smtpHost,
    smtpPort: account.smtpPort,
    smtpSecurity: account.smtpSecurity,
    oauthProvider: account.oauthProvider,
    oauthClientId: account.oauthClientId,
    oauthClientSecret: encClientSecret,
    imapUsername: account.imapUsername || null,
    acceptInvalidCerts: account.acceptInvalidCerts ? 1 : 0,
  });
}
