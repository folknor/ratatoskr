import { invoke } from "@tauri-apps/api/core";

export interface DbSmartFolder {
  id: string;
  account_id: string | null;
  name: string;
  query: string;
  icon: string;
  color: string | null;
  sort_order: number;
  is_default: boolean;
  created_at: number;
}

/**
 * Return global (account_id IS NULL) + account-specific folders, ordered by sort_order.
 */
export async function getSmartFolders(
  accountId?: string,
): Promise<DbSmartFolder[]> {
  return invoke<DbSmartFolder[]>("db_get_smart_folders", {
    accountId: accountId ?? null,
  });
}

export async function getSmartFolderById(
  id: string,
): Promise<DbSmartFolder | null> {
  return invoke<DbSmartFolder | null>("db_get_smart_folder_by_id", { id });
}

export async function insertSmartFolder(folder: {
  name: string;
  query: string;
  accountId?: string | undefined;
  icon?: string | undefined;
  color?: string | undefined;
}): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_insert_smart_folder", {
    id,
    name: folder.name,
    query: folder.query,
    accountId: folder.accountId ?? null,
    icon: folder.icon ?? null,
    color: folder.color ?? null,
  });
  return id;
}

export async function updateSmartFolder(
  id: string,
  updates: {
    name?: string | undefined;
    query?: string | undefined;
    icon?: string | undefined;
    color?: string | undefined;
  },
): Promise<void> {
  await invoke("db_update_smart_folder", { id, ...updates });
}

export async function deleteSmartFolder(id: string): Promise<void> {
  await invoke("db_delete_smart_folder", { id });
}

export async function updateSmartFolderSortOrder(
  orders: { id: string; sortOrder: number }[],
): Promise<void> {
  await invoke("db_update_smart_folder_sort_order", {
    orders: orders.map((o) => ({ id: o.id, sort_order: o.sortOrder })),
  });
}
