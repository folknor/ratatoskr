import { emailActionUnsnooze } from "@/core/rustDb";
import { getCurrentUnixTimestamp } from "@/utils/timestamp";
import {
  type BackgroundChecker,
  createBackgroundChecker,
} from "../backgroundCheckers";
import { getDb } from "../db/connection";
import { snoozeThread as snoozeThreadAction } from "../emailActions";

/**
 * Check for snoozed threads that should be un-snoozed (time has passed).
 * Moves them back to INBOX via Rust command.
 */
async function checkSnoozedThreads(): Promise<void> {
  const db = await getDb();
  const now = getCurrentUnixTimestamp();

  // Find threads where snooze time has passed
  const snoozed = await db.select<{ id: string; account_id: string }[]>(
    "SELECT id, account_id FROM threads WHERE is_snoozed = 1 AND snooze_until <= $1",
    [now],
  );

  if (snoozed.length > 0) {
    for (const thread of snoozed) {
      const opId = crypto.randomUUID();
      await emailActionUnsnooze(thread.account_id, thread.id, opId);
    }

    // Notify the UI to refresh
    window.dispatchEvent(new Event("velo-sync-done"));
  }
}

/**
 * Snooze a thread: remove from INBOX, set snooze time, archive on provider.
 * Delegates to the centralized emailActions system for optimistic UI,
 * local DB updates, offline queueing, and provider sync.
 */
export async function snoozeThread(
  accountId: string,
  threadId: string,
  messageIds: string[],
  snoozeUntil: number,
): Promise<void> {
  await snoozeThreadAction(accountId, threadId, messageIds, snoozeUntil);
}

const snoozeChecker: BackgroundChecker = createBackgroundChecker(
  "Snooze",
  checkSnoozedThreads,
);
export const startSnoozeChecker: () => void = snoozeChecker.start;
export const stopSnoozeChecker: () => void = snoozeChecker.stop;
