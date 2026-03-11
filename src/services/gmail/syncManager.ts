import {
  applyCalendarSyncResult,
  upsertDiscoveredCalendars,
} from "../calendar/persistence";
import { getCalendarProvider } from "../calendar/providerFactory";
import { getVisibleCalendars } from "../db/calendars";
export interface SyncProgress {
  phase: "labels" | "threads" | "messages" | "done";
  current: number;
  total: number;
}

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  type CategorizationCandidate,
  categorizeNewThreads,
} from "@/services/ai/categorizationManager";
import { queueNewEmailNotification } from "@/services/notifications/notificationManager";
import { applySmartLabelsToNewMessageIds } from "@/services/smartLabels/smartLabelManager";
import type {
  SmartLabelAIRule,
  SmartLabelAIThread,
} from "@/services/smartLabels/smartLabelService";

/**
 * Shared post-sync hooks: apply filters, smart labels, notifications, and AI categorization.
 * Called after every successful sync (Gmail, JMAP, Graph, IMAP).
 */
interface PostSyncHooksInput {
  accountId: string;
  newInboxEmailIds: string[];
  affectedThreadIds: string[];
  criteriaSmartLabelMatches?: { threadId: string; labelIds: string[] }[];
  notificationsToQueue?: {
    threadId: string;
    fromName?: string | null;
    fromAddress?: string | null;
    subject?: string | null;
  }[];
  aiCategorizationCandidates?: CategorizationCandidate[];
  aiSmartLabelThreads?: SmartLabelAIThread[];
  aiSmartLabelRules?: SmartLabelAIRule[];
}

async function runPostSyncHooks(input: PostSyncHooksInput): Promise<void> {
  const {
    accountId,
    newInboxEmailIds,
    affectedThreadIds,
    criteriaSmartLabelMatches = [],
    notificationsToQueue = [],
    aiCategorizationCandidates = [],
    aiSmartLabelThreads = [],
    aiSmartLabelRules = [],
  } = input;

  if (newInboxEmailIds.length > 0) {
    // Smart labels (fire-and-forget)
    applySmartLabelsToNewMessageIds(
      accountId,
      newInboxEmailIds,
      criteriaSmartLabelMatches,
      { threads: aiSmartLabelThreads, rules: aiSmartLabelRules },
    ).catch((err) => console.error("[syncManager] Smart label error:", err));

    try {
      for (const candidate of notificationsToQueue) {
        queueNewEmailNotification(
          candidate.fromName ?? candidate.fromAddress ?? "Unknown",
          candidate.subject ?? "",
          candidate.threadId,
          accountId,
          candidate.fromAddress ?? undefined,
        );
      }
    } catch (err) {
      console.error("[syncManager] Notification dispatch failed:", err);
    }
  }

  // AI categorization (fire-and-forget)
  if (affectedThreadIds.length > 0 && aiCategorizationCandidates.length > 0) {
    categorizeNewThreads(accountId, aiCategorizationCandidates).catch((err) =>
      console.error("[syncManager] Categorization error:", err),
    );
  }
}

/** Map IMAP sync phases to the SyncProgress phases the UI understands. */
function mapImapPhase(
  phase: string,
): "labels" | "threads" | "messages" | "done" {
  if (phase === "folders") return "labels";
  if (phase === "threading" || phase === "storing_threads") return "threads";
  if (phase === "messages") return "messages";
  if (phase === "done") return "done";
  return phase as "labels" | "threads" | "messages" | "done";
}

function mapProviderSyncProgress(
  provider: string,
  payload: Record<string, unknown>,
): SyncProgress {
  if (provider === "graph") {
    return {
      phase: "messages",
      current: Number(payload.messagesProcessed ?? 0),
      total: Number(payload.totalFolders ?? 0),
    };
  }

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
  shouldSyncCalendar?: boolean | null;
  newInboxMessageIds?: string[] | null;
  affectedThreadIds?: string[] | null;
  criteriaSmartLabelMatches?: { threadId: string; labelIds: string[] }[] | null;
  notificationsToQueue?:
    | {
        threadId: string;
        fromName?: string | null;
        fromAddress?: string | null;
        subject?: string | null;
      }[]
    | null;
  aiCategorizationCandidates?: CategorizationCandidate[] | null;
  aiSmartLabelThreads?: SmartLabelAIThread[] | null;
  aiSmartLabelRules?: SmartLabelAIRule[] | null;
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
      { provider: "gmail_api", eventName: "gmail-sync-progress" },
      { provider: "imap", eventName: "imap-sync-progress" },
      { provider: "jmap", eventName: "jmap-sync-progress" },
      { provider: "graph", eventName: "graph-sync-progress" },
    ] as const;

    for (const { provider, eventName } of progressEvents) {
      try {
        const unlisten = await listen<Record<string, unknown>>(
          eventName,
          (event) => {
            const accountId = String(event.payload.accountId ?? "");
            if (accountId.length === 0) return;
            statusCallback?.(
              accountId,
              "syncing",
              mapProviderSyncProgress(provider, event.payload),
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

  if (event.provider === "caldav") {
    statusCallback?.(event.accountId, "done");
    if (event.shouldSyncCalendar === true) {
      await syncCalendarForAccount(event.accountId);
    }
    return;
  }

  await runPostSyncHooks({
    accountId: event.accountId,
    newInboxEmailIds: event.newInboxMessageIds ?? [],
    affectedThreadIds: event.affectedThreadIds ?? [],
    criteriaSmartLabelMatches: event.criteriaSmartLabelMatches ?? [],
    notificationsToQueue: event.notificationsToQueue ?? [],
    aiCategorizationCandidates: event.aiCategorizationCandidates ?? [],
    aiSmartLabelThreads: event.aiSmartLabelThreads ?? [],
    aiSmartLabelRules: event.aiSmartLabelRules ?? [],
  });

  statusCallback?.(event.accountId, "done");

  if (event.shouldSyncCalendar === true) {
    syncCalendarForAccount(event.accountId).catch((err) => {
      console.warn(
        `[syncManager] Calendar sync error for ${event.accountId}:`,
        err,
      );
    });
  }
}

/**
 * Sync calendars for a single account via the CalendarProvider abstraction.
 * Discovers calendars, syncs events for each visible calendar, stores results in DB.
 */
async function syncCalendarForAccount(accountId: string): Promise<void> {
  try {
    const provider = await getCalendarProvider(accountId);

    // Discover/update calendars
    const calendarInfos = await provider.listCalendars();
    await upsertDiscoveredCalendars(accountId, provider.type, calendarInfos);

    // Sync events for each visible calendar
    const visibleCals = await getVisibleCalendars(accountId);
    for (const cal of visibleCals) {
      try {
        const syncResult = await provider.syncEvents(
          cal.remote_id,
          cal.sync_token ?? undefined,
        );
        await applyCalendarSyncResult(accountId, cal.remote_id, syncResult);
      } catch (err) {
        console.warn(
          `[syncManager] Calendar sync failed for ${cal.display_name ?? cal.remote_id}:`,
          err,
        );
      }
    }

    // Emit event for UI update
    window.dispatchEvent(new CustomEvent("ratatoskr-calendar-sync-done"));
  } catch (err) {
    console.warn(
      `[syncManager] Calendar sync failed for account ${accountId}:`,
      err,
    );
  }
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
  void ensureSyncListeners();
  void invoke("sync_start_background", { accountIds, skipImmediateSync });
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
  await invoke("sync_prepare_full_sync", { accountIds });
  await runSync(accountIds);
}

/**
 * Delete all local data for a single account and re-sync from scratch.
 * Removes all threads, messages, history ID, and IMAP folder sync states,
 * then runs a fresh initial sync.
 */
export async function resyncAccount(accountId: string): Promise<void> {
  await invoke("sync_prepare_account_resync", { accountId });
  await runSync([accountId]);
}
