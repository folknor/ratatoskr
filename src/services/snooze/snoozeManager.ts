import { invoke } from "@tauri-apps/api/core";
import { emailActionUnsnoozeBatch } from "@/core/rustDb";
import { getCurrentUnixTimestamp } from "@/utils/timestamp";
import {
  type BackgroundChecker,
  createBackgroundChecker,
} from "../backgroundCheckers";
import { snoozeThread as snoozeThreadAction } from "../emailActions";

/**
 * Check for snoozed threads that should be un-snoozed (time has passed).
 * Moves them back to INBOX via Rust command.
 */
async function checkSnoozedThreads(): Promise<void> {
  const now = getCurrentUnixTimestamp();

  const snoozed = await invoke<{ id: string; account_id: string }[]>(
    "db_get_snoozed_threads_due",
    { now },
  );

  if (snoozed.length > 0) {
    const threadIds = snoozed.map((t) => t.id);
    await emailActionUnsnoozeBatch(threadIds);

    // Notify the UI to refresh
    window.dispatchEvent(new Event("ratatoskr-sync-done"));
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
  snoozeUntil: number,
): Promise<void> {
  await snoozeThreadAction(accountId, threadId, snoozeUntil);
}

const snoozeChecker: BackgroundChecker = createBackgroundChecker(
  "Snooze",
  checkSnoozedThreads,
);
export const startSnoozeChecker: () => void = snoozeChecker.start;
export const stopSnoozeChecker: () => void = snoozeChecker.stop;
