import {
  emailActionAddLabel,
  emailActionArchive,
  emailActionMarkRead,
  emailActionMoveToFolder,
  emailActionMute,
  emailActionPermanentDelete,
  emailActionPin,
  emailActionRemoveLabel,
  emailActionSnooze,
  emailActionSpam,
  emailActionStar,
  emailActionTrash,
  emailActionUnmute,
  emailActionUnpin,
  enqueuePendingOp,
} from "@/core/rustDb";
import { getSelectedThreadId, navigateToThread } from "@/router/navigate";
import { getEmailProvider } from "@/services/email/providerFactory";
import { useThreadStore } from "@/stores/threadStore";
import { useSyncStateStore } from "@/stores/syncStateStore";
import { classifyError } from "@/utils/networkErrors";

// ---------------------------------------------------------------------------
// Action types
// ---------------------------------------------------------------------------

export type EmailAction =
  | { type: "archive"; threadId: string; messageIds: string[] }
  | { type: "trash"; threadId: string; messageIds: string[] }
  | { type: "permanentDelete"; threadId: string; messageIds: string[] }
  | {
      type: "markRead";
      threadId: string;
      messageIds: string[];
      read: boolean;
    }
  | {
      type: "star";
      threadId: string;
      messageIds: string[];
      starred: boolean;
    }
  | {
      type: "spam";
      threadId: string;
      messageIds: string[];
      isSpam: boolean;
    }
  | {
      type: "moveToFolder";
      threadId: string;
      messageIds: string[];
      folderPath: string;
    }
  | { type: "addLabel"; threadId: string; labelId: string }
  | { type: "removeLabel"; threadId: string; labelId: string }
  | {
      type: "sendMessage";
      rawBase64Url: string;
      threadId?: string;
    }
  | {
      type: "createDraft";
      rawBase64Url: string;
      threadId?: string;
    }
  | {
      type: "updateDraft";
      draftId: string;
      rawBase64Url: string;
      threadId?: string;
    }
  | { type: "deleteDraft"; draftId: string }
  | {
      type: "snooze";
      threadId: string;
      messageIds: string[];
      snoozeUntil: number;
    }
  | { type: "pin"; threadId: string }
  | { type: "unpin"; threadId: string }
  | { type: "mute"; threadId: string; messageIds: string[] }
  | { type: "unmute"; threadId: string };

// ---------------------------------------------------------------------------
// Result type
// ---------------------------------------------------------------------------

export interface ActionResult {
  success: boolean;
  queued?: boolean;
  error?: string;
  data?: unknown;
}

// ---------------------------------------------------------------------------
// Optimistic UI helpers
// ---------------------------------------------------------------------------

function getNextThreadId(currentId: string): string | null {
  // Only auto-advance if the removed thread is the one being viewed
  const selectedId = getSelectedThreadId();
  if (selectedId !== currentId) return null;
  const { threads } = useThreadStore.getState();
  const idx = threads.findIndex((t) => t.id === currentId);
  if (idx === -1) return null;
  // Prefer next thread, fall back to previous
  const next = threads[idx + 1];
  if (next) return next.id;
  const prev = threads[idx - 1];
  if (prev) return prev.id;
  return null;
}

function applyOptimisticUpdate(action: EmailAction): void {
  const store = useThreadStore.getState();
  switch (action.type) {
    case "archive":
    case "trash":
    case "permanentDelete":
    case "spam":
    case "moveToFolder":
    case "snooze": {
      const nextId = getNextThreadId(action.threadId);
      store.removeThread(action.threadId);
      if (nextId) {
        navigateToThread(nextId);
      }
      break;
    }
    case "markRead":
      store.updateThread(action.threadId, { isRead: action.read });
      break;
    case "star":
      store.updateThread(action.threadId, { isStarred: action.starred });
      break;
    case "pin":
      store.updateThread(action.threadId, { isPinned: true });
      break;
    case "unpin":
      store.updateThread(action.threadId, { isPinned: false });
      break;
    case "mute": {
      // Mute auto-archives: remove thread from list
      const nextMuteId = getNextThreadId(action.threadId);
      store.removeThread(action.threadId);
      if (nextMuteId) {
        navigateToThread(nextMuteId);
      }
      break;
    }
    case "unmute":
      store.updateThread(action.threadId, { isMuted: false });
      break;
    case "addLabel":
    case "removeLabel":
    case "sendMessage":
    case "createDraft":
    case "updateDraft":
    case "deleteDraft":
      // No universal optimistic update for these
      break;
  }
}

function revertOptimisticUpdate(action: EmailAction): void {
  const store = useThreadStore.getState();
  switch (action.type) {
    case "markRead":
      store.updateThread(action.threadId, { isRead: !action.read });
      break;
    case "star":
      store.updateThread(action.threadId, { isStarred: !action.starred });
      break;
    case "pin":
      store.updateThread(action.threadId, { isPinned: false });
      break;
    case "unpin":
      store.updateThread(action.threadId, { isPinned: true });
      break;
    case "unmute":
      store.updateThread(action.threadId, { isMuted: true });
      break;
    // For removes (archive/trash/spam/move/mute), we can't easily restore the thread
    // to the list from here. The next sync will fix it.
    default:
      break;
  }
}

// ---------------------------------------------------------------------------
// Local DB updates (Rust commands — DB only, no queueing)
// ---------------------------------------------------------------------------

async function applyLocalDbUpdate(
  accountId: string,
  action: EmailAction,
): Promise<void> {
  switch (action.type) {
    case "archive":
      await emailActionArchive(accountId, action.threadId);
      break;
    case "trash":
      await emailActionTrash(accountId, action.threadId);
      break;
    case "permanentDelete":
      await emailActionPermanentDelete(accountId, action.threadId);
      break;
    case "markRead":
      await emailActionMarkRead(accountId, action.threadId, action.read);
      break;
    case "star":
      await emailActionStar(accountId, action.threadId, action.starred);
      break;
    case "spam":
      await emailActionSpam(accountId, action.threadId, action.isSpam);
      break;
    case "snooze":
      await emailActionSnooze(
        accountId,
        action.threadId,
        action.snoozeUntil,
      );
      break;
    case "addLabel":
      await emailActionAddLabel(accountId, action.threadId, action.labelId);
      break;
    case "removeLabel":
      await emailActionRemoveLabel(accountId, action.threadId, action.labelId);
      break;
    case "moveToFolder":
      await emailActionMoveToFolder(
        accountId,
        action.threadId,
        action.folderPath,
      );
      break;
    case "pin":
      await emailActionPin(accountId, action.threadId);
      break;
    case "unpin":
      await emailActionUnpin(accountId, action.threadId);
      break;
    case "mute":
      await emailActionMute(accountId, action.threadId);
      break;
    case "unmute":
      await emailActionUnmute(accountId, action.threadId);
      break;
    default:
      // sendMessage, createDraft, updateDraft, deleteDraft — no local DB update
      break;
  }
}

// ---------------------------------------------------------------------------
// Core execution
// ---------------------------------------------------------------------------

function getResourceId(action: EmailAction): string {
  if ("threadId" in action && action.threadId) return action.threadId;
  if ("draftId" in action) return action.draftId;
  return crypto.randomUUID();
}

function actionToParams(action: EmailAction): Record<string, unknown> {
  // Strip the type field — it's stored separately as operation_type
  const { type: _, ...rest } = action;
  return rest;
}

async function executeViaProvider(
  accountId: string,
  action: EmailAction,
): Promise<unknown> {
  const provider = await getEmailProvider(accountId);
  switch (action.type) {
    case "archive":
      return provider.archive(action.threadId, action.messageIds);
    case "trash":
      return provider.trash(action.threadId, action.messageIds);
    case "permanentDelete":
      return provider.permanentDelete(action.threadId, action.messageIds);
    case "markRead":
      return provider.markRead(action.threadId, action.messageIds, action.read);
    case "star":
      return provider.star(action.threadId, action.messageIds, action.starred);
    case "spam":
      return provider.spam(action.threadId, action.messageIds, action.isSpam);
    case "moveToFolder":
      return provider.moveToFolder(
        action.threadId,
        action.messageIds,
        action.folderPath,
      );
    case "addLabel":
      return provider.addLabel(action.threadId, action.labelId);
    case "removeLabel":
      return provider.removeLabel(action.threadId, action.labelId);
    case "sendMessage":
      return provider.sendMessage(action.rawBase64Url, action.threadId);
    case "createDraft":
      return provider.createDraft(action.rawBase64Url, action.threadId);
    case "updateDraft":
      return provider.updateDraft(
        action.draftId,
        action.rawBase64Url,
        action.threadId,
      );
    case "deleteDraft":
      return provider.deleteDraft(action.draftId);
    case "snooze":
      // Snooze is local state; on the provider side we just archive
      return provider.archive(action.threadId, action.messageIds);
    case "pin":
    case "unpin":
    case "unmute":
      // Local-only operations — no IMAP/Gmail concept of pin or unmute
      return;
    case "mute":
      // Mute auto-archives; on the provider side we archive
      return provider.archive(action.threadId, action.messageIds);
  }
}

export async function executeEmailAction(
  accountId: string,
  action: EmailAction,
): Promise<ActionResult> {
  // 1. Optimistic UI update
  applyOptimisticUpdate(action);

  // 2. Local DB update via Rust
  try {
    await applyLocalDbUpdate(accountId, action);
  } catch (err) {
    console.warn("Local DB update failed:", err);
  }

  // 3. If offline, queue for later
  if (!useSyncStateStore.getState().isOnline) {
    await enqueuePendingOp(
      accountId,
      action.type,
      getResourceId(action),
      actionToParams(action),
    );
    return { success: true, queued: true };
  }

  // 4. Try online execution
  try {
    const data = await executeViaProvider(accountId, action);
    return { success: true, data };
  } catch (err) {
    const classified = classifyError(err);

    if (classified.isRetryable) {
      await enqueuePendingOp(
        accountId,
        action.type,
        getResourceId(action),
        actionToParams(action),
      );
      return { success: true, queued: true };
    }

    // Permanent error — revert optimistic update
    revertOptimisticUpdate(action);
    console.error(`Email action ${action.type} failed permanently:`, err);
    return { success: false, error: classified.message };
  }
}

// ---------------------------------------------------------------------------
// Execute a queued operation (used by queue processor)
// ---------------------------------------------------------------------------

export async function executeQueuedAction(
  accountId: string,
  operationType: string,
  params: Record<string, unknown>,
): Promise<void> {
  const action = { type: operationType, ...params } as EmailAction;
  await executeViaProvider(accountId, action);
}

// ---------------------------------------------------------------------------
// Convenience wrappers
// ---------------------------------------------------------------------------

export function archiveThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "archive",
    threadId,
    messageIds,
  });
}

export function trashThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "trash",
    threadId,
    messageIds,
  });
}

export function permanentDeleteThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "permanentDelete",
    threadId,
    messageIds,
  });
}

export function markThreadRead(
  accountId: string,
  threadId: string,
  messageIds: string[],
  read: boolean,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "markRead",
    threadId,
    messageIds,
    read,
  });
}

export function starThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
  starred: boolean,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "star",
    threadId,
    messageIds,
    starred,
  });
}

export function spamThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
  isSpam: boolean,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "spam",
    threadId,
    messageIds,
    isSpam,
  });
}

export function moveThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
  folderPath: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "moveToFolder",
    threadId,
    messageIds,
    folderPath,
  });
}

export function addThreadLabel(
  accountId: string,
  threadId: string,
  labelId: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "addLabel",
    threadId,
    labelId,
  });
}

export function removeThreadLabel(
  accountId: string,
  threadId: string,
  labelId: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "removeLabel",
    threadId,
    labelId,
  });
}

export async function sendEmail(
  accountId: string,
  rawBase64Url: string,
  threadId?: string,
): Promise<ActionResult> {
  const result = await executeEmailAction(accountId, {
    type: "sendMessage",
    rawBase64Url,
    ...(threadId != null && { threadId }),
  });

  // Notify the UI to refresh (so sent message appears in Sent folder)
  if (result.success) {
    window.dispatchEvent(new Event("ratatoskr-sync-done"));
  }

  return result;
}

export function createDraft(
  accountId: string,
  rawBase64Url: string,
  threadId?: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "createDraft",
    rawBase64Url,
    ...(threadId != null && { threadId }),
  });
}

export function updateDraft(
  accountId: string,
  draftId: string,
  rawBase64Url: string,
  threadId?: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "updateDraft",
    draftId,
    rawBase64Url,
    ...(threadId != null && { threadId }),
  });
}

export function deleteDraft(
  accountId: string,
  draftId: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, { type: "deleteDraft", draftId });
}

export function pinThread(
  accountId: string,
  threadId: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, { type: "pin", threadId });
}

export function unpinThread(
  accountId: string,
  threadId: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, { type: "unpin", threadId });
}

export function muteThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "mute",
    threadId,
    messageIds,
  });
}

export function unmuteThread(
  accountId: string,
  threadId: string,
): Promise<ActionResult> {
  return executeEmailAction(accountId, { type: "unmute", threadId });
}

export function snoozeThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
  snoozeUntil: number,
): Promise<ActionResult> {
  return executeEmailAction(accountId, {
    type: "snooze",
    threadId,
    messageIds,
    snoozeUntil,
  });
}
