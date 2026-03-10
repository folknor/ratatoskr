import { invoke } from "@tauri-apps/api/core";

export async function getCachedScanResult(
  accountId: string,
  messageId: string,
): Promise<string | null> {
  return invoke<string | null>("db_get_cached_scan_result", {
    accountId,
    messageId,
  });
}

export async function cacheScanResult(
  accountId: string,
  messageId: string,
  resultJson: string,
): Promise<void> {
  await invoke("db_cache_scan_result", {
    accountId,
    messageId,
    resultJson,
  });
}

export async function deleteScanResults(accountId: string): Promise<void> {
  await invoke("db_delete_scan_results", { accountId });
}
