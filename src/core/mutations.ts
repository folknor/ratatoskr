/**
 * Core mutations facade — re-exports every write/action function used by UI components.
 * UI code should import from here instead of reaching into @/services/ directly.
 */

// Email actions (archive, trash, star, spam, snooze, mute, pin, labels, send, read, move)
export {
  addThreadLabel,
  archiveThread,
  createDraft,
  deleteDraft,
  markThreadRead,
  moveThread,
  muteThread,
  permanentDeleteThread,
  pinThread,
  removeThreadLabel,
  sendEmail,
  snoozeThread,
  spamThread,
  starThread,
  trashThread,
  unmuteThread,
  unpinThread,
  updateDraft,
} from "@/services/emailActions";

// DB writes — routed through Rust invoke() via rustDb
export {
  addToAllowlist,
  cancelFollowUpForThread,
  deleteQuickStep,
  deleteSmartLabelRule,
  deleteThread,
  insertFollowUpReminder,
  insertQuickStep,
  insertSmartLabelRule,
  updateQuickStep,
  updateSmartLabelRule,
  upsertContact,
} from "./rustDb";

// Thread category writes (still TS)
export { setThreadCategory } from "@/services/db/threadCategories";

// Gmail client & sync triggers
export { deleteDraftsForThread } from "@/services/gmail/draftDeletion";
export { triggerSync } from "@/services/gmail/syncManager";
export { getGmailClient } from "@/services/gmail/tokenManager";

// Quick step execution & defaults
export { executeQuickStep } from "@/services/quickSteps/executor";
export { seedDefaultQuickSteps } from "@/services/quickSteps/defaults";

// Smart label backfill
export { backfillSmartLabels } from "@/services/smartLabels/backfillService";

// Unsubscribe
export {
  executeUnsubscribe,
  getSubscriptions,
  parseUnsubscribeHeaders,
  type ParsedUnsubscribe,
  type SubscriptionEntry,
} from "@/services/unsubscribe/unsubscribeManager";
