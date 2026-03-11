import { invoke } from "@tauri-apps/api/core";

export interface AccountBasicInfo {
  id: string;
  email: string;
  provider: string;
  isActive: boolean;
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
