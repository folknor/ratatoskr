export interface SyncProgress {
  phase: "labels" | "threads" | "messages" | "fallback" | "done";
  current: number;
  total: number;
}

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

/** Map IMAP sync phases to the SyncProgress phases the UI understands. */
function mapImapPhase(
  phase: string,
): "labels" | "threads" | "messages" | "fallback" | "done" {
  if (phase === "folders") return "labels";
  if (phase === "threading" || phase === "storing_threads") return "threads";
  if (phase === "messages") return "messages";
  if (phase === "fallback") return "fallback";
  if (phase === "done") return "done";
  return phase as "labels" | "threads" | "messages" | "fallback" | "done";
}

function mapProviderSyncProgress(
  payload: Record<string, unknown>,
): SyncProgress {
  return {
    phase: mapImapPhase(String(payload.phase ?? "")),
    current: Number(payload.current ?? 0),
    total: Number(payload.total ?? 0),
  };
}

interface SyncStatusEvent {
  accountId: string;
  provider: string;
  status: "syncing" | "done" | "error";
  error?: string | null;
  result?: {
    newInboxMessageIds: string[];
    affectedThreadIds: string[];
    criteriaSmartLabelMatches: { threadId: string; labelIds: string[] }[];
  } | null;
}

let syncListenersPromise: Promise<void> | null = null;

export type SyncStatusCallback = (
  accountId: string,
  status: "syncing" | "done" | "error",
  progress?: SyncProgress,
  error?: string,
) => void;

let statusCallback: SyncStatusCallback | null = null;

export function onSyncStatus(cb: SyncStatusCallback): () => void {
  void ensureSyncListeners();
  statusCallback = cb;
  return () => {
    statusCallback = null;
  };
}

async function ensureSyncListeners(): Promise<void> {
  if (syncListenersPromise !== null) {
    await syncListenersPromise;
    return;
  }

  syncListenersPromise = (async () => {
    const unlisteners: UnlistenFn[] = [];

    const progressEvents = [
      { eventName: "gmail-sync-progress" },
      { eventName: "imap-sync-progress" },
      { eventName: "jmap-sync-progress" },
      { eventName: "graph-sync-progress" },
    ] as const;

    for (const { eventName } of progressEvents) {
      try {
        const unlisten = await listen<Record<string, unknown>>(
          eventName,
          (event) => {
            const accountId = String(event.payload.accountId ?? "");
            if (accountId.length === 0) return;
            statusCallback?.(
              accountId,
              "syncing",
              mapProviderSyncProgress(event.payload),
            );
          },
        );
        unlisteners.push(unlisten);
      } catch {
        // Listen failure is non-fatal — sync will still complete without progress events
      }
    }

    try {
      const unlisten = await listen<SyncStatusEvent>("sync-status", (event) => {
        void handleSyncStatusEvent(event.payload);
      });
      unlisteners.push(unlisten);
    } catch {
      // The queue invoke still returns errors to callers; lack of events mainly
      // means the UI won't get progressive status updates.
    }

    void unlisteners;
  })();

  await syncListenersPromise;
}

async function handleSyncStatusEvent(event: SyncStatusEvent): Promise<void> {
  if (event.status === "syncing") {
    statusCallback?.(event.accountId, "syncing");
    return;
  }

  if (event.status === "error") {
    const message = event.error ?? "Unknown error";
    console.error(
      `[syncManager] Sync failed for account ${event.accountId}:`,
      message,
    );
    statusCallback?.(event.accountId, "error", undefined, message);
    return;
  }

  statusCallback?.(event.accountId, "done");
}

async function runSync(accountIds: string[]): Promise<void> {
  await ensureSyncListeners();
  await invoke("sync_run_accounts", { accountIds });
}

/**
 * Run sync for a single account, queuing if already running.
 */
export async function syncAccount(accountId: string): Promise<void> {
  return runSync([accountId]);
}

/**
 * Start the background sync timer for all accounts.
 * When `skipImmediateSync` is true the first periodic sync is deferred to the
 * next interval tick — useful when the caller already triggered a sync for a
 * newly-added account and doesn't want existing accounts to block it.
 */
export function startBackgroundSync(
  accountIds: string[],
  skipImmediateSync: boolean = false,
): void {
  stopBackgroundSync();
  void Promise.resolve(ensureSyncListeners()).catch((error) => {
    console.warn("[syncManager] Failed to initialize sync listeners:", error);
  });
  void Promise.resolve(
    invoke("sync_start_background", {
      accountIds,
      skipImmediateSync,
    }),
  ).catch((error) => {
    console.warn("[syncManager] Failed to start background sync:", error);
  });
}

/**
 * Stop the background sync timer.
 */
export function stopBackgroundSync(): void {
  void invoke("sync_stop_background");
}

/**
 * Trigger an immediate sync for all provided accounts.
 * Waits for completion even if a background sync is in progress.
 */
export async function triggerSync(accountIds: string[]): Promise<void> {
  await runSync(accountIds);
}

/**
 * Clear history IDs and perform a full re-sync for all provided accounts.
 * This re-downloads all threads from scratch.
 */
export async function forceFullSync(accountIds: string[]): Promise<void> {
  await invoke("provider_prepare_full_sync", { accountIds });
  await runSync(accountIds);
}

/**
 * Delete all local data for a single account and re-sync from scratch.
 * Removes all threads, messages, history ID, and IMAP folder sync states,
 * then runs a fresh initial sync.
 */
export async function resyncAccount(accountId: string): Promise<void> {
  await invoke("provider_prepare_account_resync", { accountId });
  await runSync([accountId]);
}
