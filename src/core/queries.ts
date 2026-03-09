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
  // Threads / Messages / Labels / Settings (Phase 1)
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
  // Categories
  getCategoriesForThreads,
  getCategoryUnreadCounts,
  // Contacts (Phase 1.5)
  deleteContact,
  getAllContacts,
  getAttachmentsFromContact,
  getContactStats,
  getContactsFromSameDomain,
  getLatestAuthResult,
  getRecentThreadsWithContact,
  updateContact,
  updateContactNotes,
  upsertContact,
  // Filters
  deleteFilter,
  getFiltersForAccount,
  insertFilter,
  updateFilter,
  // Smart Folders
  deleteSmartFolder,
  getSmartFolderById,
  getSmartFolders,
  insertSmartFolder,
  updateSmartFolder,
  updateSmartFolderSortOrder,
  // Smart Label Rules
  deleteSmartLabelRule,
  getSmartLabelRulesForAccount,
  insertSmartLabelRule,
  updateSmartLabelRule,
  // Follow-Up Reminders
  cancelFollowUpForThread,
  getActiveFollowUpThreadIds,
  getFollowUpForThread,
  insertFollowUpReminder,
  // Quick Steps
  deleteQuickStep,
  getEnabledQuickStepsForAccount,
  getQuickStepsForAccount,
  insertQuickStep,
  updateQuickStep,
  // Image Allowlist
  addToAllowlist,
  getAllowlistedSenders,
  // Notification VIPs
  addVipSender,
  isVipSender,
  removeVipSender,
} from "./rustDb";

// ── Types (canonical TS definitions, still from service files) ──
export type { DbAttachment } from "@/services/db/attachments";
export type {
  ContactAttachment,
  ContactStats,
  DbContact,
  SameDomainContact,
} from "@/services/db/contacts";
export type {
  DbFilterRule,
  FilterActions,
  FilterCriteria,
} from "@/services/db/filters";
export type { DbLabel } from "@/services/db/labels";
export type { DbMessage } from "@/services/db/messages";
export type { DbQuickStep } from "@/services/db/quickSteps";
export type { DbSmartFolder } from "@/services/db/smartFolders";
export type { DbSmartLabelRule } from "@/services/db/smartLabelRules";

// ── Bundle Rules (Rust-backed) ───────────────────────────────
export { getBundleRules, getBundleSummaries, getHeldThreadIds } from "./rustDb";
export type { DbBundleRule } from "@/services/db/bundleRules";

// ── Quick Step Types ────────────────────────────────────────
export {
  ACTION_TYPE_METADATA,
  type QuickStep,
  type QuickStepAction,
  type QuickStepActionType,
  type QuickStepExecutionResult,
} from "@/services/quickSteps/types";

// ── Search (still TS — FTS5, Phase 3 will use tantivy) ──────
export { searchMessages } from "@/services/db/search";

// ── Thread Categories (constant) ────────────────────────────
export { ALL_CATEGORIES } from "@/services/db/threadCategories";

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
