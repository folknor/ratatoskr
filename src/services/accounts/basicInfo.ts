import { invoke } from "@tauri-apps/api/core";
import type { Account } from "@/stores/accountStore";

export interface AccountBasicInfo {
  id: string;
  email: string;
  displayName: string | null;
  avatarUrl: string | null;
  provider: string;
  isActive: boolean;
}

export interface AccountCaldavSettingsInfo {
  id: string;
  email: string;
  caldavUrl: string | null;
  caldavUsername: string | null;
  caldavPassword: string | null;
  calendarProvider: string | null;
}

export interface AccountOAuthCredentials {
  clientId: string;
  clientSecret: string | null;
}

export async function getAccountBasicInfo(
  accountId: string,
): Promise<AccountBasicInfo | null> {
  return invoke<AccountBasicInfo | null>("account_get_basic_info", {
    accountId,
  });
}

export async function listAccountBasicInfo(): Promise<AccountBasicInfo[]> {
  return invoke<AccountBasicInfo[]>("account_list_basic_info");
}

export function mapAccountBasicInfos(accounts: AccountBasicInfo[]): Account[] {
  return accounts.map((account) => ({
    id: account.id,
    email: account.email,
    displayName: account.displayName,
    avatarUrl: account.avatarUrl,
    isActive: account.isActive,
    provider: account.provider,
  }));
}

export async function getAccountCaldavSettingsInfo(
  accountId: string,
): Promise<AccountCaldavSettingsInfo | null> {
  return invoke<AccountCaldavSettingsInfo | null>(
    "account_get_caldav_settings_info",
    {
      accountId,
    },
  );
}

export async function getAccountOAuthCredentials(
  accountId: string,
): Promise<AccountOAuthCredentials | null> {
  return invoke<AccountOAuthCredentials | null>(
    "account_get_oauth_credentials",
    {
      accountId,
    },
  );
}
