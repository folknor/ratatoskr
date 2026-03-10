import { invoke } from "@tauri-apps/api/core";

export interface DbLabel {
  id: string;
  account_id: string;
  name: string;
  type: string;
  color_bg: string | null;
  color_fg: string | null;
  visible: number;
  sort_order: number;
  imap_folder_path: string | null;
  imap_special_use: string | null;
}

export async function getLabelsForAccount(
  accountId: string,
): Promise<DbLabel[]> {
  return invoke<DbLabel[]>("db_get_labels", { accountId });
}

export async function upsertLabel(label: {
  id: string;
  accountId: string;
  name: string;
  type: string;
  colorBg?: string | null;
  colorFg?: string | null;
  imapFolderPath?: string | null;
  imapSpecialUse?: string | null;
}): Promise<void> {
  return invoke<void>("db_upsert_label_coalesce", {
    id: label.id,
    accountId: label.accountId,
    name: label.name,
    labelType: label.type,
    colorBg: label.colorBg ?? null,
    colorFg: label.colorFg ?? null,
    imapFolderPath: label.imapFolderPath ?? null,
    imapSpecialUse: label.imapSpecialUse ?? null,
  });
}

export async function deleteLabelsForAccount(accountId: string): Promise<void> {
  return invoke<void>("db_delete_labels_for_account", { accountId });
}

export async function deleteLabel(
  accountId: string,
  labelId: string,
): Promise<void> {
  return invoke<void>("db_delete_label", { accountId, labelId });
}

export async function updateLabelSortOrder(
  accountId: string,
  labelOrders: { id: string; sortOrder: number }[],
): Promise<void> {
  return invoke<void>("db_update_label_sort_order", {
    accountId,
    labelOrders,
  });
}
