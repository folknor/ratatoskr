import { invoke } from "@tauri-apps/api/core";

export interface PendingOperation {
  id: string;
  account_id: string;
  operation_type: string;
  resource_id: string;
  params: string;
  status: string;
  retry_count: number;
  max_retries: number;
  next_retry_at: number | null;
  created_at: number;
  error_message: string | null;
}

export async function enqueuePendingOperation(
  accountId: string,
  operationType: string,
  resourceId: string,
  params: Record<string, unknown>,
): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_pending_ops_enqueue", {
    id,
    accountId,
    operationType,
    resourceId,
    paramsJson: JSON.stringify(params),
  });
  return id;
}

export async function getPendingOperations(
  accountId?: string,
  limit: number = 50,
): Promise<PendingOperation[]> {
  return invoke("db_pending_ops_get", {
    accountId: accountId ?? null,
    limit,
  });
}

export async function updateOperationStatus(
  id: string,
  status: string,
  errorMessage?: string,
): Promise<void> {
  await invoke("db_pending_ops_update_status", {
    id,
    status,
    errorMessage: errorMessage ?? null,
  });
}

export async function deleteOperation(id: string): Promise<void> {
  await invoke("db_pending_ops_delete", { id });
}

export async function incrementRetry(id: string): Promise<void> {
  await invoke("db_pending_ops_increment_retry", { id });
}

export async function getPendingOpsCount(accountId?: string): Promise<number> {
  return invoke("db_pending_ops_count", {
    accountId: accountId ?? null,
  });
}

export async function getFailedOpsCount(accountId?: string): Promise<number> {
  return invoke("db_pending_ops_failed_count", {
    accountId: accountId ?? null,
  });
}

export async function getPendingOpsForResource(
  accountId: string,
  resourceId: string,
): Promise<PendingOperation[]> {
  return invoke("db_pending_ops_for_resource", {
    accountId,
    resourceId,
  });
}

export async function compactQueue(accountId?: string): Promise<number> {
  return invoke("db_pending_ops_compact", {
    accountId: accountId ?? null,
  });
}

export async function clearFailedOperations(accountId?: string): Promise<void> {
  await invoke("db_pending_ops_clear_failed", {
    accountId: accountId ?? null,
  });
}

export async function retryFailedOperations(accountId?: string): Promise<void> {
  await invoke("db_pending_ops_retry_failed", {
    accountId: accountId ?? null,
  });
}
