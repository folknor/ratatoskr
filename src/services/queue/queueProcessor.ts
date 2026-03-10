import { useSyncStateStore } from "@/stores/syncStateStore";
import { classifyError } from "@/utils/networkErrors";
import {
  type BackgroundChecker,
  createBackgroundChecker,
} from "../backgroundCheckers";
import {
  compactQueue,
  deleteOperation,
  getPendingOperations,
  getPendingOpsCount,
  incrementRetry,
  updateOperationStatus,
} from "../db/pendingOperations";
import { executeQueuedAction } from "../emailActions";

const BATCH_SIZE = 50;

let checker: BackgroundChecker | null = null;

async function processQueue(): Promise<void> {
  // Skip if offline
  if (!useSyncStateStore.getState().isOnline) return;

  // Compact first to eliminate redundant ops
  await compactQueue();

  // Get pending operations
  const ops = await getPendingOperations(undefined, BATCH_SIZE);
  if (ops.length === 0) {
    await updatePendingCount();
    return;
  }

  for (const op of ops) {
    try {
      // Mark as executing
      await updateOperationStatus(op.id, "executing");

      // Parse params and execute
      const params = JSON.parse(op.params) as Record<string, unknown>;
      await executeQueuedAction(op.account_id, op.operation_type, params);

      // Success — delete from queue
      await deleteOperation(op.id);
    } catch (err) {
      const classified = classifyError(err);
      const originalError = err instanceof Error ? err : null;
      const fullErrorContext = [
        `[${classified.type}] ${classified.message}`,
        originalError?.stack ? `Stack: ${originalError.stack}` : null,
        originalError?.cause
          ? `Cause: ${String(originalError.cause)}`
          : null,
      ]
        .filter(Boolean)
        .join("\n");

      if (classified.isRetryable) {
        // Increment retry with exponential backoff
        await updateOperationStatus(op.id, "pending", fullErrorContext);
        await incrementRetry(op.id);
      } else {
        // Permanent failure — preserve full error context for debugging
        console.error(
          `[QueueProcessor] permanent failure for op ${op.id} (${op.operation_type}):`,
          err,
        );
        await updateOperationStatus(op.id, "failed", fullErrorContext);
      }
    }
  }

  await updatePendingCount();
}

async function updatePendingCount(): Promise<void> {
  const count = await getPendingOpsCount();
  useSyncStateStore.getState().setPendingOpsCount(count);
}

export function startQueueProcessor(): void {
  if (checker) return;
  checker = createBackgroundChecker("QueueProcessor", processQueue, 30_000);
  checker.start();
}

export function stopQueueProcessor(): void {
  checker?.stop();
  checker = null;
}

/**
 * Trigger an immediate queue flush (e.g., when coming back online).
 * Returns a promise that resolves when processing completes.
 */
export async function triggerQueueFlush(): Promise<void> {
  try {
    await processQueue();
  } catch (err) {
    console.error("[QueueProcessor] flush failed:", err);
  }
}
