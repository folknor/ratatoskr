import { invoke } from "@tauri-apps/api/core";

export async function isPhishingAllowlisted(
  accountId: string,
  senderAddress: string,
): Promise<boolean> {
  return invoke<boolean>("db_is_phishing_allowlisted", {
    accountId,
    senderAddress,
  });
}

export async function addToPhishingAllowlist(
  accountId: string,
  senderAddress: string,
): Promise<void> {
  await invoke("db_add_to_phishing_allowlist", {
    accountId,
    senderAddress,
  });
}

export async function removeFromPhishingAllowlist(
  accountId: string,
  senderAddress: string,
): Promise<void> {
  await invoke("db_remove_from_phishing_allowlist", {
    accountId,
    senderAddress,
  });
}

export async function getPhishingAllowlist(
  accountId: string,
): Promise<{ id: string; sender_address: string; created_at: number }[]> {
  return invoke<{ id: string; sender_address: string; created_at: number }[]>(
    "db_get_phishing_allowlist",
    { accountId },
  );
}
