import { invoke } from "@tauri-apps/api/core";

export interface FolderSyncState {
  account_id: string;
  folder_path: string;
  uidvalidity: number | null;
  last_uid: number;
  modseq: number | null;
  last_sync_at: number | null;
}

export async function getFolderSyncState(
  accountId: string,
  folderPath: string,
): Promise<FolderSyncState | null> {
  return invoke<FolderSyncState | null>("db_get_folder_sync_state", {
    accountId,
    folderPath,
  });
}

export async function upsertFolderSyncState(
  state: FolderSyncState,
): Promise<void> {
  await invoke("db_upsert_folder_sync_state", {
    accountId: state.account_id,
    folderPath: state.folder_path,
    uidvalidity: state.uidvalidity,
    lastUid: state.last_uid,
    modseq: state.modseq,
    lastSyncAt: state.last_sync_at,
  });
}

export async function deleteFolderSyncState(
  accountId: string,
  folderPath: string,
): Promise<void> {
  await invoke("db_delete_folder_sync_state", { accountId, folderPath });
}

export async function clearAllFolderSyncStates(
  accountId: string,
): Promise<void> {
  await invoke("db_clear_all_folder_sync_states", { accountId });
}

export async function getAllFolderSyncStates(
  accountId: string,
): Promise<FolderSyncState[]> {
  return invoke<FolderSyncState[]>("db_get_all_folder_sync_states", {
    accountId,
  });
}
