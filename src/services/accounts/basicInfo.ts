import { invoke } from "@tauri-apps/api/core";

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
