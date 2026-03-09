/**
 * Core facade for all DB read/write operations used by UI code.
 *
 * UI code (components, hooks, stores) should import from here
 * instead of reaching into @/services/db/* directly.
 *
 * Functions backed by Rust commands are imported from ./rustDb;
 * everything else still routes through the TS service layer.
 */

import { getDb } from "@/services/db/connection";

// ── Rust-backed queries (invoke → Rust commands) ────────────
export {
  deleteThread,
  getAllSettings,
  getContactByEmail,
  getLabelsForAccount,
  getMessagesForThread,
  getSetting,
  getThreadById,
  getThreadCount,
  getThreadLabelIds,
  getThreadsForAccount,
  getThreadsForCategory,
  getUnreadCount,
  searchContacts,
  setSetting,
} from "./rustDb";

// Re-export Rust-backed category helpers (return Map, same API)
export {
  getCategoriesForThreads,
  getCategoryUnreadCounts,
} from "./rustDb";

// ── Types (canonical TS definitions) ────────────────────────
export type { DbAttachment } from "@/services/db/attachments";
export type { DbContact } from "@/services/db/contacts";
export type { DbLabel } from "@/services/db/labels";
export type { DbMessage } from "@/services/db/messages";

// ── Bundle Rules ─────────────────────────────────────────────
export {
  type DbBundleRule,
  getBundleRules,
  getBundleSummaries,
  getHeldThreadIds,
} from "@/services/db/bundleRules";

// ── Contacts (remaining TS-only functions) ───────────────────
export {
  type ContactAttachment,
  type ContactStats,
  deleteContact,
  getAllContacts,
  getAttachmentsFromContact,
  getContactStats,
  getContactsFromSameDomain,
  getLatestAuthResult,
  getRecentThreadsWithContact,
  type SameDomainContact,
  updateContact,
  updateContactNotes,
  upsertContact,
} from "@/services/db/contacts";

// ── Filters ──────────────────────────────────────────────────
export {
  type DbFilterRule,
  deleteFilter,
  type FilterActions,
  type FilterCriteria,
  getFiltersForAccount,
  insertFilter,
  updateFilter,
} from "@/services/db/filters";

// ── Follow-Up Reminders ──────────────────────────────────────
export {
  cancelFollowUpForThread,
  getActiveFollowUpThreadIds,
  getFollowUpForThread,
  insertFollowUpReminder,
} from "@/services/db/followUpReminders";

// ── Image Allowlist ──────────────────────────────────────────
export {
  addToAllowlist,
  getAllowlistedSenders,
} from "@/services/db/imageAllowlist";

// ── Notification VIPs ────────────────────────────────────────
export {
  addVipSender,
  isVipSender,
  removeVipSender,
} from "@/services/db/notificationVips";

// ── Quick Steps ──────────────────────────────────────────────
export {
  type DbQuickStep,
  deleteQuickStep,
  getEnabledQuickStepsForAccount,
  getQuickStepsForAccount,
  insertQuickStep,
  updateQuickStep,
} from "@/services/db/quickSteps";

// ── Quick Step Types ────────────────────────────────────────
export {
  ACTION_TYPE_METADATA,
  type QuickStep,
  type QuickStepAction,
  type QuickStepActionType,
  type QuickStepExecutionResult,
} from "@/services/quickSteps/types";

// ── Search ───────────────────────────────────────────────────
export { searchMessages } from "@/services/db/search";

// ── Smart Folders ────────────────────────────────────────────
export {
  type DbSmartFolder,
  deleteSmartFolder,
  getSmartFolderById,
  getSmartFolders,
  insertSmartFolder,
  updateSmartFolder,
  updateSmartFolderSortOrder,
} from "@/services/db/smartFolders";

// ── Smart Label Rules ────────────────────────────────────────
export {
  type DbSmartLabelRule,
  deleteSmartLabelRule,
  getSmartLabelRulesForAccount,
  insertSmartLabelRule,
  updateSmartLabelRule,
} from "@/services/db/smartLabelRules";

// ── Thread Categories (constant) ────────────────────────────
export { ALL_CATEGORIES } from "@/services/db/threadCategories";

// (deleteThread is now Rust-backed via rustDb)

// ── Auth Results (email authentication) ─────────────────────
export {
  type AuthResult,
  type AuthVerdict,
  parseAuthenticationResults,
} from "@/services/gmail/authParser";

// ── Gravatar ────────────────────────────────────────────────
export {
  fetchAndCacheGravatarUrl,
  getGravatarUrl,
} from "@/services/contacts/gravatar";

// ── Smart Folder Query helpers (from search/) ────────────────
export {
  getSmartFolderSearchQuery,
  getSmartFolderUnreadCount,
  mapSmartFolderRows,
  type SmartFolderRow,
} from "@/services/search/smartFolderQuery";

// ── Raw DB wrappers (for code that previously called getDb() directly) ──

/**
 * Run a smart-folder SQL query and return the raw rows.
 * Wraps the direct getDb() + db.select() pattern.
 */
export async function querySmartFolderThreads<T>(
  sql: string,
  params: unknown[],
): Promise<T[]> {
  const db = await getDb();
  return db.select<T[]>(sql, params);
}

/**
 * Run a smart-folder unread-count SQL query.
 * Wraps the direct getDb() + db.select() pattern used in smartFolderStore.
 */
export async function querySmartFolderUnreadCount(
  sql: string,
  params: unknown[],
): Promise<number> {
  const db = await getDb();
  const rows = await db.select<{ count: number }[]>(sql, params);
  return rows[0]?.count ?? 0;
}
