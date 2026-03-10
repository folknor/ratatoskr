import { invoke } from "@tauri-apps/api/core";

export interface DbSignature {
  id: string;
  account_id: string;
  name: string;
  body_html: string;
  is_default: number;
  sort_order: number;
}

export async function getSignaturesForAccount(
  accountId: string,
): Promise<DbSignature[]> {
  return invoke<DbSignature[]>("db_get_signatures_for_account", {
    accountId,
  });
}

export async function getDefaultSignature(
  accountId: string,
): Promise<DbSignature | null> {
  return invoke<DbSignature | null>("db_get_default_signature", {
    accountId,
  });
}

export async function insertSignature(sig: {
  accountId: string;
  name: string;
  bodyHtml: string;
  isDefault: boolean;
}): Promise<string> {
  return invoke<string>("db_insert_signature", {
    accountId: sig.accountId,
    name: sig.name,
    bodyHtml: sig.bodyHtml,
    isDefault: sig.isDefault,
  });
}

export async function updateSignature(
  id: string,
  updates: { name?: string; bodyHtml?: string; isDefault?: boolean },
): Promise<void> {
  await invoke("db_update_signature", {
    id,
    name: updates.name ?? null,
    bodyHtml: updates.bodyHtml ?? null,
    isDefault: updates.isDefault ?? null,
  });
}

export async function deleteSignature(id: string): Promise<void> {
  await invoke("db_delete_signature", { id });
}
