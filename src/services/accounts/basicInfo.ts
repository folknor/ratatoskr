import { invoke } from "@tauri-apps/api/core";

export interface AccountBasicInfo {
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
