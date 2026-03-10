import { invoke } from "@tauri-apps/api/core";

export async function getAiCache(
  accountId: string,
  threadId: string,
  type: string,
): Promise<string | null> {
  return invoke<string | null>("db_get_ai_cache", {
    accountId,
    threadId,
    cacheType: type,
  });
}

export async function setAiCache(
  accountId: string,
  threadId: string,
  type: string,
  content: string,
): Promise<void> {
  await invoke("db_set_ai_cache", {
    accountId,
    threadId,
    cacheType: type,
    content,
  });
}

export async function deleteAiCache(
  accountId: string,
  threadId: string,
  type: string,
): Promise<void> {
  await invoke("db_delete_ai_cache", {
    accountId,
    threadId,
    cacheType: type,
  });
}
