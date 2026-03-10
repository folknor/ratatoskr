import { checkFollowUpReminders } from "@/core/rustDb";
import {
  type BackgroundChecker,
  createBackgroundChecker,
} from "../backgroundCheckers";
import { notifyFollowUpDue } from "../notifications/notificationManager";

/**
 * Check for follow-up reminders that have fired.
 * Delegates to a single Rust command that, in one transaction:
 *   - Finds all pending reminders that are due
 *   - Cancels those where a reply has arrived
 *   - Triggers those with no reply
 *   - Returns triggered reminders for notification dispatch
 */
async function checkFollowUps(): Promise<void> {
  const triggered = await checkFollowUpReminders();

  for (const reminder of triggered) {
    notifyFollowUpDue(
      reminder.subject,
      reminder.thread_id,
      reminder.account_id,
    );
  }

  if (triggered.length > 0) {
    window.dispatchEvent(new Event("ratatoskr-sync-done"));
  }
}

const followUpChecker: BackgroundChecker = createBackgroundChecker(
  "FollowUp",
  checkFollowUps,
);
export const startFollowUpChecker: () => void = followUpChecker.start;
export const stopFollowUpChecker: () => void = followUpChecker.stop;
