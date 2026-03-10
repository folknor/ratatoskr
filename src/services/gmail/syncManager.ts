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
import { ensureFreshToken } from "../oauth/oauthTokenManager";
export interface SyncProgress {
  phase: "labels" | "threads" | "messages" | "done";
  current: number;
  total: number;
}

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import {
  type ImapSyncResult,
  syncGmailDelta,
  syncGmailInitial,
  syncImapDelta,
  syncImapInitial,
} from "@/core/rustDb";
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

/**
 * Run a sync for a single Gmail API account (initial or delta).
 *
 * The entire sync pipeline runs in Rust: Gmail API calls → message parsing →
 * DB writes → body store → tantivy indexing → label sync.
 * TS only calls the Rust command and runs post-sync hooks on the result.
 */
async function syncGmailAccount(accountId: string): Promise<void> {
  const account = await getAccount(accountId);
  if (!account) {
    throw new Error("Account not found");
  }

  const syncPeriodStr = await getSetting("sync_period_days");
  const syncDays = parseInt(syncPeriodStr ?? "365", 10) || 365;

  // Listen for progress events from Rust
  let unlisten: UnlistenFn | null = null;
  try {
    unlisten = await listen<{
      accountId: string;
      phase: string;
      current: number;
      total: number;
    }>("gmail-sync-progress", (event) => {
      if (event.payload.accountId !== accountId) return;
      statusCallback?.(accountId, "syncing", {
        phase: mapImapPhase(event.payload.phase),
        current: event.payload.current,
        total: event.payload.total,
      });
    });
  } catch {
    // Listen failure is non-fatal — sync will work without progress events
  }

  try {
    if (account.history_id) {
      // Delta sync
      try {
        const result = await syncGmailDelta(accountId);
        await runPostSyncHooks(
          accountId,
          result.newInboxMessageIds,
          result.affectedThreadIds,
          true,
        );
      } catch (err) {
        const message = String(err ?? "");
        if (
          message === "HISTORY_EXPIRED" ||
          message.includes("HISTORY_EXPIRED")
        ) {
          // Fallback to full sync — still run categorization (not notifications).
          // Initial sync doesn't return message IDs, but we pass a synthetic
          // marker so categorization runs for the re-synced account.
          await syncGmailInitial(accountId, syncDays);
          await runPostSyncHooks(accountId, [], ["_resync"], false);
        } else {
          throw err;
        }
      }
    } else {
      // First time — full initial sync
      await syncGmailInitial(accountId, syncDays);
    }
  } finally {
    unlisten?.();
  }
}

/**
 * Run a sync for a single JMAP account (initial or delta).
 *
 * The entire sync pipeline runs in Rust: JMAP API calls → message parsing →
 * DB writes → body store → tantivy indexing.
 * TS only calls the Rust command and runs post-sync hooks on the result.
 */
async function syncJmapAccount(accountId: string): Promise<void> {
  // Check if this account has been synced before (has a state token)
  const account = await getAccount(accountId);
  const isDelta = !!account?.history_id;

  try {
    const result: { newInboxEmailIds: string[]; affectedThreadIds: string[] } =
      await invoke("jmap_sync_delta", { accountId });

    await runPostSyncHooks(
      accountId,
      result.newInboxEmailIds,
      result.affectedThreadIds,
      isDelta,
    );
  } catch (err) {
    const message = String(err ?? "");
    if (
      message === "JMAP_STATE_EXPIRED" ||
      message === "JMAP_NO_STATE" ||
      message.includes("JMAP_STATE_EXPIRED") ||
      message.includes("JMAP_NO_STATE")
    ) {
      // Fallback to full initial sync — still run categorization
      await invoke("jmap_sync_initial", { accountId });
      await runPostSyncHooks(accountId, [], ["_resync"], false);
    } else {
      throw err;
    }
  }
}

/**
 * Run a sync for a single Graph account (initial or delta).
 *
 * The entire sync pipeline runs in Rust: Graph API calls → message parsing →
 * DB writes → body store → tantivy indexing.
 * TS only calls the Rust command and runs post-sync hooks on the result.
 */
async function syncGraphAccount(accountId: string): Promise<void> {
  // Check if this account has been synced before (has a delta state token)
  const account = await getAccount(accountId);
  const isDelta = !!account?.history_id;

  try {
    const result: {
      newInboxMessageIds: string[];
      affectedThreadIds: string[];
    } = await invoke("provider_sync_delta", { accountId });

    await runPostSyncHooks(
      accountId,
      result.newInboxMessageIds,
      result.affectedThreadIds,
      isDelta,
    );
  } catch (err) {
    const message = String(err ?? "");
    if (
      message === "GRAPH_NO_DELTA_STATE" ||
      message.includes("GRAPH_NO_DELTA_STATE")
    ) {
      // Fallback to full initial sync — still run categorization
      const syncPeriodStr = await getSetting("sync_period_days");
      const syncDays = parseInt(syncPeriodStr ?? "365", 10) || 365;
      await invoke("provider_sync_initial", { accountId, daysBack: syncDays });
      await runPostSyncHooks(accountId, [], ["_resync"], false);
    } else {
      throw err;
    }
  }
}

/**
 * Run IMAP sync via the Rust sync engine (Phase 4).
 *
 * The entire pipeline runs in Rust: IMAP fetch → parse → DB write →
 * body store → tantivy index → JWZ threading → thread storage.
 * Zero IPC during the pipeline — only one invoke() call.
 *
 * Post-sync hooks (filters, notifications, AI categorization) run in TS
 * using the returned new message IDs.
 */
async function syncImapAccountRust(accountId: string): Promise<void> {
  const account = await getAccount(accountId);
  if (!account) throw new Error("Account not found");

  // Refresh OAuth2 token before syncing (if applicable).
  // The Rust engine reads the password from DB, so we need the fresh token
  // written to DB before invoking.
  if (account.auth_method === "oauth2") {
    await ensureFreshToken(account);
  }

  // Listen for progress events from Rust
  let unlisten: UnlistenFn | null = null;
  try {
    unlisten = await listen<{
      accountId: string;
      phase: string;
      current: number;
      total: number;
      folder: string | null;
    }>("imap-sync-progress", (event) => {
      if (event.payload.accountId !== accountId) return;
      statusCallback?.(accountId, "syncing", {
        phase: mapImapPhase(event.payload.phase),
        current: event.payload.current,
        total: event.payload.total,
      });
    });
  } catch {
    // Listen failure is non-fatal — sync will work without progress events
  }

  try {
    let result: ImapSyncResult;
    const isDelta = !!account.history_id;

    if (isDelta) {
      // Recovery (0 messages + 0 threads → force initial) is handled
      // inside the Rust sync_imap_delta command — no extra IPC needed.
      result = await syncImapDelta(accountId);
    } else {
      result = await syncImapInitial(accountId);
    }

    // IMAP uses storedCount as a proxy for affected threads (no thread IDs returned).
    // Build a synthetic affectedThreadIds array to gate AI categorization.
    const affectedThreadIds = result.storedCount > 0 ? ["_imap_stored"] : [];
    await runPostSyncHooks(
      accountId,
      result.newInboxMessageIds,
      affectedThreadIds,
      isDelta,
    );
  } finally {
    unlisten?.();
  }
}

/**
 * Run a sync for a single IMAP account (initial or delta).
 * Delegates entirely to the Rust sync engine.
 */
async function syncImapAccount(accountId: string): Promise<void> {
  return syncImapAccountRust(accountId);
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
 * Routes to Gmail or IMAP sync based on account provider.
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

    if (account.provider === "jmap") {
      await syncJmapAccount(accountId);
    } else if (account.provider === "graph") {
      await syncGraphAccount(accountId);
    } else if (account.provider === "imap") {
      await syncImapAccount(accountId);
    } else {
      await syncGmailAccount(accountId);
    }

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
  if (syncPromise) {
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
    while (toSync) {
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
