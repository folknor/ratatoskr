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

// ── Gravatar ────────────────────────────────────────────────
export {
  fetchAndCacheGravatarUrl,
  getGravatarUrl,
} from "@/services/contacts/gravatar";

// ── Types (canonical TS definitions, still from service files) ──
export type { DbAttachment } from "@/services/db/attachments";
export type { DbBundleRule } from "@/services/db/bundleRules";
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
// ── Thread Categories (constant) ────────────────────────────
export { ALL_CATEGORIES } from "@/services/db/threadCategories";
// ── Auth Results (email authentication) ─────────────────────
export {
  type AuthResult,
  type AuthVerdict,
  parseAuthenticationResults,
} from "@/services/gmail/authParser";
// ── Quick Step Types ────────────────────────────────────────
export {
  ACTION_TYPE_METADATA,
  type QuickStep,
  type QuickStepAction,
  type QuickStepActionType,
  type QuickStepExecutionResult,
} from "@/services/quickSteps/types";
// ── Smart Folder Query helpers (from search/) ────────────────
export {
  getSmartFolderSearchQuery,
  getSmartFolderUnreadCount,
  mapSmartFolderRows,
  type SmartFolderRow,
} from "@/services/search/smartFolderQuery";
// ── Rust-backed queries (invoke → Rust commands) ────────────
// ── Bundle Rules (Rust-backed) ───────────────────────────────
// ── Body Store (Phase 2 — compressed body storage) ───────────
// ── Search (tantivy full-text search — Phase 3) ─────────────
export {
  // Image Allowlist
  addToAllowlist,
  // Notification VIPs
  addVipSender,
  type BodyStoreStats,
  bodyStoreDelete,
  bodyStoreGet,
  bodyStoreGetBatch,
  bodyStoreMigrate,
  bodyStorePut,
  bodyStorePutBatch,
  bodyStoreStats,
  // Follow-Up Reminders
  cancelFollowUpForThread,
  // Contacts (Phase 1.5)
  deleteContact,
  // Filters
  deleteFilter,
  // Quick Steps
  deleteQuickStep,
  deleteSearchDocument,
  // Smart Folders
  deleteSmartFolder,
  // Smart Label Rules
  deleteSmartLabelRule,
  // Threads / Messages / Labels / Settings (Phase 1)
  deleteThread,
  getActiveFollowUpThreadIds,
  getAllContacts,
  getAllowlistedSenders,
  getAllSettings,
  getAttachmentsFromContact,
  getBundleRules,
  getBundleSummaries,
  // Categories
  getCategoriesForThreads,
  getCategoryUnreadCounts,
  getContactByEmail,
  getContactStats,
  getContactsFromSameDomain,
  getEnabledQuickStepsForAccount,
  getFiltersForAccount,
  getFollowUpForThread,
  getHeldThreadIds,
  getLabelsForAccount,
  getLatestAuthResult,
  getMessagesForThread,
  getQuickStepsForAccount,
  getRecentThreadsWithContact,
  getSetting,
  getSmartFolderById,
  getSmartFolders,
  getSmartLabelRulesForAccount,
  getThreadById,
  getThreadCount,
  getThreadLabelIds,
  getThreadsForAccount,
  getThreadsForCategory,
  getUnreadCount,
  indexMessage,
  indexMessagesBatch,
  insertFilter,
  insertFollowUpReminder,
  insertQuickStep,
  insertSmartFolder,
  insertSmartLabelRule,
  isVipSender,
  type MessageBody,
  rebuildSearchIndex,
  removeVipSender,
  type SearchDocument,
  type SearchResult,
  searchContacts,
  searchMessages,
  setSetting,
  updateContact,
  updateContactNotes,
  updateFilter,
  updateQuickStep,
  updateSmartFolder,
  updateSmartFolderSortOrder,
  updateSmartLabelRule,
  upsertContact,
} from "./rustDb";

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
