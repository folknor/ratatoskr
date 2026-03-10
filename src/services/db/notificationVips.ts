import { invoke } from "@tauri-apps/api/core";
import { normalizeEmail } from "@/utils/emailUtils";

export interface NotificationVip {
  id: string;
  account_id: string;
  email_address: string;
  display_name: string | null;
  created_at: number;
}

export async function getVipSenders(accountId: string): Promise<Set<string>> {
  const emails = await invoke<string[]>("db_get_vip_senders", {
    accountId,
  });
  return new Set(emails.map((e) => normalizeEmail(e)));
}

export async function getAllVipSenders(
  accountId: string,
): Promise<NotificationVip[]> {
  return invoke<NotificationVip[]>("db_get_all_vip_senders", {
    accountId,
  });
}

export async function addVipSender(
  accountId: string,
  email: string,
  displayName?: string,
): Promise<void> {
  await invoke("db_add_vip_sender", {
    id: crypto.randomUUID(),
    accountId,
    email: normalizeEmail(email),
    displayName: displayName ?? null,
  });
}

export async function removeVipSender(
  accountId: string,
  email: string,
): Promise<void> {
  await invoke("db_remove_vip_sender", {
    accountId,
    email: normalizeEmail(email),
  });
}

export async function isVipSender(
  accountId: string,
  email: string,
): Promise<boolean> {
  return invoke<boolean>("db_is_vip_sender", {
    accountId,
    email: normalizeEmail(email),
  });
}
