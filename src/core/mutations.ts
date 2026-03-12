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
  deleteDraftThread,
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
// Sync triggers & draft cleanup
export { deleteDraftsForThread } from "@/services/gmail/draftDeletion";
export { triggerSync } from "@/services/gmail/syncManager";
export { seedDefaultQuickSteps } from "@/services/quickSteps/defaults";
// Quick step execution & defaults
export { executeQuickStep } from "@/services/quickSteps/executor";
// Smart label backfill
export { backfillSmartLabels } from "@/services/smartLabels/backfillService";
// Unsubscribe
export {
  executeUnsubscribe,
  getSubscriptions,
  type ParsedUnsubscribe,
  parseUnsubscribeHeaders,
  type SubscriptionEntry,
} from "@/services/unsubscribe/unsubscribeManager";
// DB writes — routed through Rust invoke() via rustDb
// Thread category writes (Rust-backed)
export {
  addToAllowlist,
  cancelFollowUpForThread,
  deleteQuickStep,
  deleteSmartLabelRule,
  deleteThread,
  insertFollowUpReminder,
  insertQuickStep,
  insertSmartLabelRule,
  setThreadCategory,
  updateQuickStep,
  updateSmartLabelRule,
  upsertContact,
} from "./rustDb";
