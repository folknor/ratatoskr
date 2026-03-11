import {
  getCalendarProvider,
  hasCalendarSupport,
} from "../calendar/providerFactory";
import { clearAccountHistoryId, getAccount } from "../db/accounts";
import {
  deleteEventByRemoteId,
  upsertCalendarEvent,
} from "../db/calendarEvents";
import {
  getVisibleCalendars,
  updateCalendarSyncToken,
  upsertCalendar,
} from "../db/calendars";
import { clearAllFolderSyncStates } from "../db/folderSyncState";
import { deleteAllMessagesForAccount } from "../db/messages";
import { getSetting } from "../db/settings";
import { deleteAllThreadsForAccount } from "../db/threads";
export interface SyncProgress {
  phase: "labels" | "threads" | "messages" | "done";
  current: number;
  total: number;
}

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { categorizeNewThreads } from "@/services/ai/categorizationManager";
import { getMessagesByIds } from "@/services/db/messages";
import { getVipSenders } from "@/services/db/notificationVips";
import { getThreadCategory } from "@/services/db/threadCategories";
import { getMutedThreadIds } from "@/services/db/threads";
import { applyFiltersToNewMessageIds } from "@/services/filters/filterEngine";
import {
  queueNewEmailNotification,
  shouldNotifyForMessage,
} from "@/services/notifications/notificationManager";
import { applySmartLabelsToNewMessageIds } from "@/services/smartLabels/smartLabelManager";

const SYNC_INTERVAL_MS = 60_000; // 60 seconds — delta syncs are lightweight (single API call when idle)

/**
 * Shared post-sync hooks: apply filters, smart labels, notifications, and AI categorization.
 * Called after every successful sync (Gmail, JMAP, Graph, IMAP).
 * Notifications are only dispatched on delta syncs (`isDelta = true`).
 */
async function runPostSyncHooks(
  accountId: string,
  newInboxEmailIds: string[],
  affectedThreadIds: string[],
  isDelta: boolean,
): Promise<void> {
  if (newInboxEmailIds.length > 0) {
    try {
      await applyFiltersToNewMessageIds(accountId, newInboxEmailIds);
    } catch (err) {
      console.error("[syncManager] Filter application failed:", err);
    }

    // Smart labels (fire-and-forget)
    applySmartLabelsToNewMessageIds(accountId, newInboxEmailIds).catch((err) =>
      console.error("[syncManager] Smart label error:", err),
    );

    // Notifications — only on delta sync (not initial)
    if (isDelta) {
      try {
        const smartNotifSetting = await getSetting("smart_notifications");
        const smartEnabled = smartNotifSetting === "true";
        const notifCatSetting =
          (await getSetting("notify_categories")) ?? "Primary";
        const allowedCategories = new Set(
          notifCatSetting.split(",").map((s) => s.trim()),
        );
        const vipSenders = smartEnabled
          ? await getVipSenders(accountId)
          : new Set<string>();
        const mutedThreadIds = await getMutedThreadIds(accountId);

        const newMsgs = await getMessagesByIds(accountId, newInboxEmailIds);
        for (const msg of newMsgs) {
          if (mutedThreadIds.has(msg.thread_id)) continue;
          const category = await getThreadCategory(accountId, msg.thread_id);
          if (
            shouldNotifyForMessage(
              smartEnabled,
              allowedCategories,
              vipSenders,
              category,
              msg.from_address ?? undefined,
            )
          ) {
            queueNewEmailNotification(
              msg.from_name ?? msg.from_address ?? "Unknown",
              msg.subject ?? "",
              msg.thread_id,
              accountId,
              msg.from_address ?? undefined,
            );
          }
        }
      } catch (err) {
        console.error("[syncManager] Notification dispatch failed:", err);
      }
    }
  }

  // AI categorization (fire-and-forget)
  if (affectedThreadIds.length > 0) {
    categorizeNewThreads(accountId).catch((err) =>
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

function getSyncEventName(provider: string): string | null {
  if (provider === "gmail_api") return "gmail-sync-progress";
  if (provider === "imap") return "imap-sync-progress";
  if (provider === "jmap") return "jmap-sync-progress";
  if (provider === "graph") return "graph-sync-progress";
  return null;
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

interface ProviderAutoSyncResult {
  newInboxMessageIds: string[];
  affectedThreadIds: string[];
  wasDelta: boolean;
  fellBackToInitial: boolean;
}

let syncTimer: ReturnType<typeof setInterval> | null = null;
let syncPromise: Promise<void> | null = null;
let pendingAccountIds: string[] | null = null;

export type SyncStatusCallback = (
  accountId: string,
  status: "syncing" | "done" | "error",
  progress?: SyncProgress,
  error?: string,
) => void;

let statusCallback: SyncStatusCallback | null = null;

export function onSyncStatus(cb: SyncStatusCallback): () => void {
  statusCallback = cb;
  return () => {
    statusCallback = null;
  };
}

async function syncEmailAccount(accountId: string): Promise<void> {
  const account = await getAccount(accountId);
  if (!account) throw new Error("Account not found");

  // Listen for progress events from Rust
  let unlisten: UnlistenFn | null = null;
  const eventName = getSyncEventName(account.provider);
  try {
    if (eventName) {
      unlisten = await listen<Record<string, unknown>>(eventName, (event) => {
        if (event.payload.accountId !== accountId) return;
        statusCallback?.(
          accountId,
          "syncing",
          mapProviderSyncProgress(account.provider, event.payload),
        );
      });
    }
  } catch {
    // Listen failure is non-fatal — sync will work without progress events
  }

  try {
    const result = await invoke<ProviderAutoSyncResult>("provider_sync_auto", {
      accountId,
    });
    await runPostSyncHooks(
      accountId,
      result.newInboxMessageIds,
      result.affectedThreadIds,
      result.wasDelta && !result.fellBackToInitial,
    );
  } finally {
    unlisten?.();
  }
}

/**
 * Sync calendars for a single account via the CalendarProvider abstraction.
 * Discovers calendars, syncs events for each visible calendar, stores results in DB.
 */
async function syncCalendarForAccount(accountId: string): Promise<void> {
  try {
    const supported = await hasCalendarSupport(accountId);
    if (!supported) return;

    const provider = await getCalendarProvider(accountId);

    // Discover/update calendars
    const calendarInfos = await provider.listCalendars();
    for (const cal of calendarInfos) {
      await upsertCalendar({
        accountId,
        provider: provider.type,
        remoteId: cal.remoteId,
        displayName: cal.displayName,
        color: cal.color,
        isPrimary: cal.isPrimary,
      });
    }

    // Sync events for each visible calendar
    const visibleCals = await getVisibleCalendars(accountId);
    for (const cal of visibleCals) {
      try {
        const syncResult = await provider.syncEvents(
          cal.remote_id,
          cal.sync_token ?? undefined,
        );

        // Upsert created/updated events
        for (const event of [...syncResult.created, ...syncResult.updated]) {
          await upsertCalendarEvent({
            accountId,
            googleEventId: event.remoteEventId,
            summary: event.summary,
            description: event.description,
            location: event.location,
            startTime: event.startTime,
            endTime: event.endTime,
            isAllDay: event.isAllDay,
            status: event.status,
            organizerEmail: event.organizerEmail,
            attendeesJson: event.attendeesJson,
            htmlLink: event.htmlLink,
            calendarId: cal.id,
            remoteEventId: event.remoteEventId,
            etag: event.etag,
            icalData: event.icalData,
            uid: event.uid,
          });
        }

        // Delete removed events
        for (const remoteId of syncResult.deletedRemoteIds) {
          await deleteEventByRemoteId(cal.id, remoteId);
        }

        // Update sync token
        if (syncResult.newSyncToken || syncResult.newCtag) {
          await updateCalendarSyncToken(
            cal.id,
            syncResult.newSyncToken,
            syncResult.newCtag,
          );
        }
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

/**
 * Run a sync for a single account (initial or delta).
 */
async function syncAccountInternal(accountId: string): Promise<void> {
  try {
    const account = await getAccount(accountId);

    if (!account) {
      throw new Error("Account not found");
    }

    statusCallback?.(accountId, "syncing");

    console.log(
      `[syncManager] Syncing account ${accountId} (provider=${account.provider}, history_id=${account.history_id ?? "null"})`,
    );

    if (account.provider === "caldav") {
      // CalDAV-only accounts — skip email sync, only sync calendar
      await syncCalendarForAccount(accountId);
      statusCallback?.(accountId, "done");
      return;
    }

    await syncEmailAccount(accountId);

    // Always emit "done" when an initial sync completes (clears the bar).
    // Also emit for delta syncs that fell back to initial (recovery re-sync)
    // since those emit progress via statusCallback inside syncImapAccount.
    statusCallback?.(accountId, "done");

    // Sync calendar alongside email (non-blocking — calendar errors don't affect email sync)
    syncCalendarForAccount(accountId).catch((err) => {
      console.warn(`[syncManager] Calendar sync error for ${accountId}:`, err);
    });
  } catch (err) {
    const message =
      err instanceof Error ? err.message : String(err ?? "Unknown error");
    console.error(
      `[syncManager] Sync failed for account ${accountId}:`,
      message,
    );
    statusCallback?.(accountId, "error", undefined, message);
  }
}

async function runSync(accountIds: string[]): Promise<void> {
  // If a sync is already in progress, merge into the pending set and wait for
  // the active cycle to drain the queue. Using a Set ensures no IDs are lost
  // even under rapid concurrent triggers — all mutations of pendingAccountIds
  // happen synchronously (no await between read and write).
  if (syncPromise !== null) {
    const existing = new Set(pendingAccountIds ?? []);
    for (const id of accountIds) existing.add(id);
    pendingAccountIds = [...existing];
    // Wait for the active sync (and any queued drains) to finish so callers
    // that `await runSync(...)` only resolve once their IDs have been processed.
    await syncPromise;
    return;
  }

  syncPromise = (async () => {
    let toSync: string[] | null = accountIds;

    // Loop: process current batch, then drain anything queued while we were busy.
    while (toSync !== null) {
      for (const id of toSync) {
        await syncAccountInternal(id);
      }

      // Atomically grab and clear the pending queue (synchronous — no race window).
      toSync = pendingAccountIds;
      pendingAccountIds = null;
    }
  })();

  try {
    await syncPromise;
  } finally {
    syncPromise = null;
  }
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

  if (!skipImmediateSync) {
    // Immediate sync
    void runSync(accountIds);
  }

  // Periodic sync
  syncTimer = setInterval(() => {
    void runSync(accountIds);
  }, SYNC_INTERVAL_MS);
}

/**
 * Stop the background sync timer.
 */
export function stopBackgroundSync(): void {
  if (syncTimer) {
    clearInterval(syncTimer);
    syncTimer = null;
  }
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
  for (const id of accountIds) {
    await clearAccountHistoryId(id);
  }
  await runSync(accountIds);
}

/**
 * Delete all local data for a single account and re-sync from scratch.
 * Removes all threads, messages, history ID, and IMAP folder sync states,
 * then runs a fresh initial sync.
 */
export async function resyncAccount(accountId: string): Promise<void> {
  await deleteAllThreadsForAccount(accountId);
  await deleteAllMessagesForAccount(accountId);
  await clearAccountHistoryId(accountId);
  await clearAllFolderSyncStates(accountId);
  await runSync([accountId]);
}
