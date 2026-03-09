import { getCurrentUnixTimestamp } from "@/utils/timestamp";
import {
  type BackgroundChecker,
  createBackgroundChecker,
} from "../backgroundCheckers";
import { getDb, withTransaction } from "../db/connection";
import { snoozeThread as snoozeThreadAction } from "../emailActions";

/**
 * Check for snoozed threads that should be un-snoozed (time has passed).
 * Moves them back to INBOX.
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
    await withTransaction(async (txDb) => {
      for (const thread of snoozed) {
        // Un-snooze the thread
        await txDb.execute(
          "UPDATE threads SET is_snoozed = 0, snooze_until = NULL WHERE account_id = $1 AND id = $2",
          [thread.account_id, thread.id],
        );

        // Re-add INBOX label
        await txDb.execute(
          "INSERT OR IGNORE INTO thread_labels (account_id, thread_id, label_id) VALUES ($1, $2, 'INBOX')",
          [thread.account_id, thread.id],
        );
      }
    });

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
