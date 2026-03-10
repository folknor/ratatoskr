/**
 * Rust DB backend — wraps Tauri invoke() calls for DB queries.
 *
 * Each function mirrors a corresponding TS service function but delegates
 * to a Rust command instead of using the SQLite plugin directly.
 * The facade in queries.ts can switch to these implementations.
 */

import { invoke } from "@tauri-apps/api/core";

import type { DbBundleRule } from "@/services/db/bundleRules";
import type {
  ContactAttachment,
  ContactStats,
  DbContact,
  SameDomainContact,
} from "@/services/db/contacts";
import type { DbFilterRule } from "@/services/db/filters";
import type { DbFollowUpReminder } from "@/services/db/followUpReminders";
import type { DbLabel } from "@/services/db/labels";
import type { DbMessage } from "@/services/db/messages";
import type { DbQuickStep } from "@/services/db/quickSteps";
import type { DbSmartFolder } from "@/services/db/smartFolders";
import type { DbSmartLabelRule } from "@/services/db/smartLabelRules";
import type { DbThread } from "@/services/db/threads";

// ── Rust-specific row types (flat structs returned by Rust) ─────────

/** Row returned by `db_get_all_settings` */
interface SettingRow {
  key: string;
  value: string;
}

/** Row returned by `db_get_category_unread_counts` */
interface CategoryCountRow {
  category: string | null;
  count: number;
}

/** Row returned by `db_get_categories_for_threads` */
interface ThreadCategoryRow {
  thread_id: string;
  category: string;
}

// ── Threads ─────────────────────────────────────────────────────────

export async function getThreadsForAccount(
  accountId: string,
  labelId?: string,
  limit: number = 50,
  offset: number = 0,
): Promise<DbThread[]> {
  return invoke<DbThread[]>("db_get_threads", {
    accountId,
    labelId,
    limit,
    offset,
  });
}

export async function getThreadsForCategory(
  accountId: string,
  category: string,
  limit: number = 50,
  offset: number = 0,
): Promise<DbThread[]> {
  return invoke<DbThread[]>("db_get_threads_for_category", {
    accountId,
    category,
    limit,
    offset,
  });
}

export async function getThreadById(
  accountId: string,
  threadId: string,
): Promise<DbThread | undefined> {
  const row = await invoke<DbThread | null>("db_get_thread_by_id", {
    accountId,
    threadId,
  });
  return row ?? undefined;
}

export async function getThreadLabelIds(
  accountId: string,
  threadId: string,
): Promise<string[]> {
  return invoke<string[]>("db_get_thread_label_ids", {
    accountId,
    threadId,
  });
}

export async function getThreadCount(
  accountId: string,
  labelId?: string,
): Promise<number> {
  return invoke<number>("db_get_thread_count", {
    accountId,
    labelId,
  });
}

// ── Messages ────────────────────────────────────────────────────────

export async function getMessagesForThread(
  accountId: string,
  threadId: string,
): Promise<DbMessage[]> {
  return invoke<DbMessage[]>("db_get_messages_for_thread", {
    accountId,
    threadId,
  });
}

// ── Labels ──────────────────────────────────────────────────────────

export async function getLabelsForAccount(
  accountId: string,
): Promise<DbLabel[]> {
  return invoke<DbLabel[]>("db_get_labels", { accountId });
}

// ── Settings ────────────────────────────────────────────────────────

export async function getSetting(key: string): Promise<string | null> {
  return invoke<string | null>("db_get_setting", { key });
}

export async function getAllSettings(): Promise<Record<string, string>> {
  const rows = await invoke<SettingRow[]>("db_get_all_settings");
  return Object.fromEntries(rows.map((r) => [r.key, r.value]));
}

export async function setSetting(
  key: string,
  value: string,
): Promise<void> {
  return invoke<void>("db_set_setting", { key, value });
}

// ── Thread Categories ───────────────────────────────────────────────

export async function getCategoryUnreadCounts(
  accountId: string,
): Promise<Map<string, number>> {
  const rows = await invoke<CategoryCountRow[]>(
    "db_get_category_unread_counts",
    { accountId },
  );
  const map = new Map<string, number>();
  for (const row of rows) {
    const cat = row.category ?? "Primary";
    map.set(cat, (map.get(cat) ?? 0) + row.count);
  }
  return map;
}

export async function getCategoriesForThreads(
  accountId: string,
  threadIds: string[],
): Promise<Map<string, string>> {
  if (threadIds.length === 0) return new Map();
  const rows = await invoke<ThreadCategoryRow[]>(
    "db_get_categories_for_threads",
    { accountId, threadIds },
  );
  const map = new Map<string, string>();
  for (const row of rows) {
    map.set(row.thread_id, row.category);
  }
  return map;
}

// ── Contacts ────────────────────────────────────────────────────────

export async function searchContacts(
  query: string,
  limit: number = 10,
): Promise<DbContact[]> {
  return invoke<DbContact[]>("db_search_contacts", { query, limit });
}

export async function getContactByEmail(
  email: string,
): Promise<DbContact | null> {
  return invoke<DbContact | null>("db_get_contact_by_email", { email });
}

// ── Thread mutations ────────────────────────────────────────────────

export async function deleteThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("db_delete_thread", { accountId, threadId });
}

// ── Unread Count ────────────────────────────────────────────────────

export async function getUnreadCount(accountId: string): Promise<number> {
  return invoke<number>("db_get_unread_count", { accountId });
}

// ═══════════════════════════════════════════════════════════════
// CONTACTS — remaining
// ═══════════════════════════════════════════════════════════════

export async function getAllContacts(
  limit: number = 500,
  offset: number = 0,
): Promise<DbContact[]> {
  return invoke<DbContact[]>("db_get_all_contacts", { limit, offset });
}

export async function upsertContact(
  email: string,
  displayName: string | null,
): Promise<void> {
  const id = crypto.randomUUID();
  return invoke<void>("db_upsert_contact", { id, email, displayName });
}

export async function updateContact(
  id: string,
  displayName: string | null,
): Promise<void> {
  return invoke<void>("db_update_contact", { id, displayName });
}

export async function updateContactNotes(
  email: string,
  notes: string | null,
): Promise<void> {
  return invoke<void>("db_update_contact_notes", { email, notes });
}

export async function deleteContact(id: string): Promise<void> {
  return invoke<void>("db_delete_contact", { id });
}

export async function getContactStats(
  email: string,
): Promise<ContactStats> {
  return invoke<ContactStats>("db_get_contact_stats", { email });
}

export async function getContactsFromSameDomain(
  email: string,
  limit: number = 5,
): Promise<SameDomainContact[]> {
  return invoke<SameDomainContact[]>("db_get_contacts_from_same_domain", {
    email,
    limit,
  });
}

export async function getLatestAuthResult(
  email: string,
): Promise<string | null> {
  return invoke<string | null>("db_get_latest_auth_result", { email });
}

export async function getRecentThreadsWithContact(
  email: string,
  limit: number = 5,
): Promise<
  { thread_id: string; subject: string | null; last_message_at: string | null }[]
> {
  return invoke("db_get_recent_threads_with_contact", { email, limit });
}

export async function getAttachmentsFromContact(
  email: string,
  limit: number = 5,
): Promise<ContactAttachment[]> {
  return invoke<ContactAttachment[]>("db_get_attachments_from_contact", {
    email,
    limit,
  });
}

// ═══════════════════════════════════════════════════════════════
// FILTERS
// ═══════════════════════════════════════════════════════════════

export async function getFiltersForAccount(
  accountId: string,
): Promise<DbFilterRule[]> {
  return invoke<DbFilterRule[]>("db_get_filters_for_account", { accountId });
}

export async function insertFilter(filter: {
  accountId: string;
  name: string;
  criteria: unknown;
  actions: unknown;
  isEnabled?: boolean;
}): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_insert_filter", {
    id,
    accountId: filter.accountId,
    name: filter.name,
    criteriaJson: JSON.stringify(filter.criteria),
    actionsJson: JSON.stringify(filter.actions),
    isEnabled: filter.isEnabled,
  });
  return id;
}

export async function updateFilter(
  id: string,
  updates: {
    name?: string;
    criteria?: unknown;
    actions?: unknown;
    isEnabled?: boolean;
  },
): Promise<void> {
  return invoke<void>("db_update_filter", {
    id,
    name: updates.name,
    criteriaJson: updates.criteria ? JSON.stringify(updates.criteria) : undefined,
    actionsJson: updates.actions ? JSON.stringify(updates.actions) : undefined,
    isEnabled: updates.isEnabled,
  });
}

export async function deleteFilter(id: string): Promise<void> {
  return invoke<void>("db_delete_filter", { id });
}

// ═══════════════════════════════════════════════════════════════
// SMART FOLDERS
// ═══════════════════════════════════════════════════════════════

export async function getSmartFolders(
  accountId?: string,
): Promise<DbSmartFolder[]> {
  return invoke<DbSmartFolder[]>("db_get_smart_folders", { accountId });
}

export async function getSmartFolderById(
  id: string,
): Promise<DbSmartFolder | null> {
  return invoke<DbSmartFolder | null>("db_get_smart_folder_by_id", { id });
}

export async function insertSmartFolder(folder: {
  name: string;
  query: string;
  accountId?: string | undefined;
  icon?: string | undefined;
  color?: string | undefined;
}): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_insert_smart_folder", {
    id,
    name: folder.name,
    query: folder.query,
    accountId: folder.accountId,
    icon: folder.icon,
    color: folder.color,
  });
  return id;
}

export async function updateSmartFolder(
  id: string,
  updates: {
    name?: string | undefined;
    query?: string | undefined;
    icon?: string | undefined;
    color?: string | undefined;
  },
): Promise<void> {
  return invoke<void>("db_update_smart_folder", { id, ...updates });
}

export async function deleteSmartFolder(id: string): Promise<void> {
  return invoke<void>("db_delete_smart_folder", { id });
}

export async function updateSmartFolderSortOrder(
  orders: { id: string; sortOrder: number }[],
): Promise<void> {
  return invoke<void>("db_update_smart_folder_sort_order", {
    orders: orders.map((o) => ({ id: o.id, sort_order: o.sortOrder })),
  });
}

// ═══════════════════════════════════════════════════════════════
// SMART LABEL RULES
// ═══════════════════════════════════════════════════════════════

export async function getSmartLabelRulesForAccount(
  accountId: string,
): Promise<DbSmartLabelRule[]> {
  return invoke<DbSmartLabelRule[]>("db_get_smart_label_rules_for_account", {
    accountId,
  });
}

export async function insertSmartLabelRule(rule: {
  accountId: string;
  labelId: string;
  aiDescription: string;
  criteria?: unknown;
  isEnabled?: boolean;
}): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_insert_smart_label_rule", {
    id,
    accountId: rule.accountId,
    labelId: rule.labelId,
    aiDescription: rule.aiDescription,
    criteriaJson: rule.criteria ? JSON.stringify(rule.criteria) : undefined,
    isEnabled: rule.isEnabled,
  });
  return id;
}

export async function updateSmartLabelRule(
  id: string,
  updates: {
    labelId?: string;
    aiDescription?: string;
    criteria?: unknown | null;
    isEnabled?: boolean;
  },
): Promise<void> {
  return invoke<void>("db_update_smart_label_rule", {
    id,
    labelId: updates.labelId,
    aiDescription: updates.aiDescription,
    criteriaJson:
      updates.criteria !== undefined
        ? updates.criteria
          ? JSON.stringify(updates.criteria)
          : null
        : undefined,
    isEnabled: updates.isEnabled,
  });
}

export async function deleteSmartLabelRule(id: string): Promise<void> {
  return invoke<void>("db_delete_smart_label_rule", { id });
}

// ═══════════════════════════════════════════════════════════════
// FOLLOW-UP REMINDERS
// ═══════════════════════════════════════════════════════════════

export async function insertFollowUpReminder(
  accountId: string,
  threadId: string,
  messageId: string,
  remindAt: number,
): Promise<void> {
  const id = crypto.randomUUID();
  return invoke<void>("db_insert_follow_up_reminder", {
    id,
    accountId,
    threadId,
    messageId,
    remindAt,
  });
}

export async function getFollowUpForThread(
  accountId: string,
  threadId: string,
): Promise<DbFollowUpReminder | null> {
  return invoke<DbFollowUpReminder | null>("db_get_follow_up_for_thread", {
    accountId,
    threadId,
  });
}

export async function cancelFollowUpForThread(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("db_cancel_follow_up_for_thread", {
    accountId,
    threadId,
  });
}

export async function getActiveFollowUpThreadIds(
  accountId: string,
  threadIds: string[],
): Promise<Set<string>> {
  const ids = await invoke<string[]>("db_get_active_follow_up_thread_ids", {
    accountId,
    threadIds,
  });
  return new Set(ids);
}

/** Result of a triggered follow-up reminder (returned by batch check). */
export interface TriggeredFollowUp {
  id: string;
  account_id: string;
  thread_id: string;
  subject: string;
}

/**
 * Batch-check all pending follow-up reminders in a single transaction.
 * Cancels reminders where a reply exists, triggers those that are due,
 * and returns the triggered ones for notification dispatch.
 */
export async function checkFollowUpReminders(): Promise<TriggeredFollowUp[]> {
  return invoke<TriggeredFollowUp[]>("db_check_follow_up_reminders");
}

// ═══════════════════════════════════════════════════════════════
// QUICK STEPS
// ═══════════════════════════════════════════════════════════════

export async function getQuickStepsForAccount(
  accountId: string,
): Promise<DbQuickStep[]> {
  return invoke<DbQuickStep[]>("db_get_quick_steps_for_account", {
    accountId,
  });
}

export async function getEnabledQuickStepsForAccount(
  accountId: string,
): Promise<DbQuickStep[]> {
  return invoke<DbQuickStep[]>("db_get_enabled_quick_steps_for_account", {
    accountId,
  });
}

export async function insertQuickStep(step: {
  accountId: string;
  name: string;
  actions: unknown[];
  description?: string | undefined;
  shortcut?: string | undefined;
  icon?: string | undefined;
  isEnabled?: boolean | undefined;
  continueOnError?: boolean | undefined;
}): Promise<string> {
  const id = crypto.randomUUID();
  await invoke("db_insert_quick_step", {
    step: {
      id,
      account_id: step.accountId,
      name: step.name,
      description: step.description ?? null,
      shortcut: step.shortcut ?? null,
      actions_json: JSON.stringify(step.actions),
      icon: step.icon ?? null,
      is_enabled: step.isEnabled ?? true,
      continue_on_error: step.continueOnError ?? false,
      sort_order: 0,
      created_at: 0,
    },
  });
  return id;
}

export async function updateQuickStep(
  id: string,
  updates: {
    name?: string | undefined;
    description?: string | undefined;
    shortcut?: string | null | undefined;
    actions?: unknown[] | undefined;
    icon?: string | undefined;
    isEnabled?: boolean | undefined;
    continueOnError?: boolean | undefined;
  },
): Promise<void> {
  // Full update — fetch current then merge (Rust takes full struct)
  // For simplicity, pass a struct with the updated fields
  return invoke<void>("db_update_quick_step", {
    step: {
      id,
      account_id: "",
      name: updates.name ?? "",
      description: updates.description ?? null,
      shortcut: updates.shortcut ?? null,
      actions_json: updates.actions
        ? JSON.stringify(updates.actions)
        : "[]",
      icon: updates.icon ?? null,
      is_enabled: updates.isEnabled ?? true,
      continue_on_error: updates.continueOnError ?? false,
      sort_order: 0,
      created_at: 0,
    },
  });
}

export async function deleteQuickStep(id: string): Promise<void> {
  return invoke<void>("db_delete_quick_step", { id });
}

// ═══════════════════════════════════════════════════════════════
// IMAGE ALLOWLIST
// ═══════════════════════════════════════════════════════════════

export async function addToAllowlist(
  accountId: string,
  senderAddress: string,
): Promise<void> {
  const id = crypto.randomUUID();
  return invoke<void>("db_add_to_allowlist", { id, accountId, senderAddress });
}

export async function getAllowlistedSenders(
  accountId: string,
  senderAddresses: string[],
): Promise<Set<string>> {
  const addrs = await invoke<string[]>("db_get_allowlisted_senders", {
    accountId,
    senderAddresses,
  });
  return new Set(addrs);
}

// ═══════════════════════════════════════════════════════════════
// NOTIFICATION VIPS
// ═══════════════════════════════════════════════════════════════

export async function addVipSender(
  accountId: string,
  email: string,
  displayName?: string,
): Promise<void> {
  const id = crypto.randomUUID();
  return invoke<void>("db_add_vip_sender", {
    id,
    accountId,
    email,
    displayName,
  });
}

export async function removeVipSender(
  accountId: string,
  email: string,
): Promise<void> {
  return invoke<void>("db_remove_vip_sender", { accountId, email });
}

export async function isVipSender(
  accountId: string,
  email: string,
): Promise<boolean> {
  return invoke<boolean>("db_is_vip_sender", { accountId, email });
}

// ═══════════════════════════════════════════════════════════════
// THREAD CATEGORIES — set
// ═══════════════════════════════════════════════════════════════

export async function setThreadCategory(
  accountId: string,
  threadId: string,
  category: string,
  isManual: boolean,
): Promise<void> {
  return invoke<void>("db_set_thread_category", {
    accountId,
    threadId,
    category,
    isManual,
  });
}

// ═══════════════════════════════════════════════════════════════
// BUNDLE RULES
// ═══════════════════════════════════════════════════════════════

/** Row returned by `db_get_bundle_summaries` */
interface BundleSummaryRow {
  category: string;
  count: number;
  latestSubject: string | null;
  latestSender: string | null;
}

export async function getBundleRules(
  accountId: string,
): Promise<DbBundleRule[]> {
  return invoke<DbBundleRule[]>("db_get_bundle_rules", { accountId });
}

export async function getBundleSummaries(
  accountId: string,
  categories: string[],
): Promise<
  Map<
    string,
    { count: number; latestSubject: string | null; latestSender: string | null }
  >
> {
  if (categories.length === 0) return new Map();
  const rows = await invoke<BundleSummaryRow[]>("db_get_bundle_summaries", {
    accountId,
    categories,
  });
  const map = new Map<
    string,
    { count: number; latestSubject: string | null; latestSender: string | null }
  >();
  for (const row of rows) {
    map.set(row.category, {
      count: row.count,
      latestSubject: row.latestSubject,
      latestSender: row.latestSender,
    });
  }
  return map;
}

export async function getHeldThreadIds(
  accountId: string,
): Promise<Set<string>> {
  const ids = await invoke<string[]>("db_get_held_thread_ids", { accountId });
  return new Set(ids);
}

// ═══════════════════════════════════════════════════════════════
// EMAIL ACTIONS (Phase 5 — local DB updates only, no queueing)
// ═══════════════════════════════════════════════════════════════

export async function emailActionArchive(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("email_action_archive", { accountId, threadId });
}

export async function emailActionTrash(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("email_action_trash", { accountId, threadId });
}

export async function emailActionPermanentDelete(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("email_action_permanent_delete", {
    accountId,
    threadId,
  });
}

export async function emailActionSpam(
  accountId: string,
  threadId: string,
  isSpam: boolean,
): Promise<void> {
  return invoke<void>("email_action_spam", { accountId, threadId, isSpam });
}

export async function emailActionMarkRead(
  accountId: string,
  threadId: string,
  isRead: boolean,
): Promise<void> {
  return invoke<void>("email_action_mark_read", {
    accountId,
    threadId,
    isRead,
  });
}

export async function emailActionStar(
  accountId: string,
  threadId: string,
  isStarred: boolean,
): Promise<void> {
  return invoke<void>("email_action_star", {
    accountId,
    threadId,
    isStarred,
  });
}

export async function emailActionSnooze(
  accountId: string,
  threadId: string,
  snoozeUntil: number,
): Promise<void> {
  return invoke<void>("email_action_snooze", {
    accountId,
    threadId,
    snoozeUntil,
  });
}

export async function emailActionUnsnooze(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("email_action_unsnooze", { accountId, threadId });
}

export function emailActionUnsnoozeBatch(
  threadIds: string[],
): Promise<number> {
  return invoke<number>("email_action_unsnooze_batch", { threadIds });
}

export async function emailActionPin(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("email_action_pin", { accountId, threadId });
}

export async function emailActionUnpin(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("email_action_unpin", { accountId, threadId });
}

export async function emailActionMute(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("email_action_mute", { accountId, threadId });
}

export async function emailActionUnmute(
  accountId: string,
  threadId: string,
): Promise<void> {
  return invoke<void>("email_action_unmute", { accountId, threadId });
}

export async function emailActionAddLabel(
  accountId: string,
  threadId: string,
  labelId: string,
): Promise<void> {
  return invoke<void>("email_action_add_label", {
    accountId,
    threadId,
    labelId,
  });
}

export async function emailActionRemoveLabel(
  accountId: string,
  threadId: string,
  labelId: string,
): Promise<void> {
  return invoke<void>("email_action_remove_label", {
    accountId,
    threadId,
    labelId,
  });
}

export async function emailActionMoveToFolder(
  accountId: string,
  threadId: string,
  folderLabelId: string,
): Promise<void> {
  return invoke<void>("email_action_move_to_folder", {
    accountId,
    threadId,
    folderLabelId,
  });
}


// ═══════════════════════════════════════════════════════════════
// BODY STORE (Phase 2 — compressed body storage)
// ═══════════════════════════════════════════════════════════════

export interface MessageBody {
  messageId: string;
  bodyHtml: string | null;
  bodyText: string | null;
}

export interface BodyStoreStats {
  messageCount: number;
  compressedHtmlBytes: number;
  compressedTextBytes: number;
}

/** Store a single message body (zstd-compressed in Rust). */
export async function bodyStorePut(
  messageId: string,
  bodyHtml: string | null,
  bodyText: string | null,
): Promise<void> {
  return invoke<void>("body_store_put", { messageId, bodyHtml, bodyText });
}

/** Store multiple message bodies in a single transaction. */
export async function bodyStorePutBatch(
  bodies: MessageBody[],
): Promise<void> {
  return invoke<void>("body_store_put_batch", { bodies });
}

/** Retrieve a single message body (decompressed). */
export async function bodyStoreGet(
  messageId: string,
): Promise<MessageBody | null> {
  return invoke<MessageBody | null>("body_store_get", { messageId });
}

/** Retrieve multiple message bodies (decompressed). */
export async function bodyStoreGetBatch(
  messageIds: string[],
): Promise<MessageBody[]> {
  return invoke<MessageBody[]>("body_store_get_batch", { messageIds });
}

/** Delete bodies for given message IDs. */
export async function bodyStoreDelete(
  messageIds: string[],
): Promise<number> {
  return invoke<number>("body_store_delete", { messageIds });
}

/** Get body store statistics. */
export async function bodyStoreStats(): Promise<BodyStoreStats> {
  return invoke<BodyStoreStats>("body_store_stats");
}

/** Migrate existing bodies from metadata DB to body store. Returns count migrated. */
export async function bodyStoreMigrate(): Promise<number> {
  return invoke<number>("body_store_migrate");
}

// ═══════════════════════════════════════════════════════════════
// TANTIVY FULL-TEXT SEARCH (Phase 3)
// ═══════════════════════════════════════════════════════════════

import {
  hasSearchOperators,
  parseSearchQuery,
} from "@/services/search/searchParser";

export interface SearchResult {
  message_id: string;
  account_id: string;
  thread_id: string;
  subject: string | null;
  from_name: string | null;
  from_address: string | null;
  snippet: string | null;
  date: number;
  rank: number;
}

export interface SearchDocument {
  messageId: string;
  accountId: string;
  threadId: string;
  subject: string | null;
  fromName: string | null;
  fromAddress: string | null;
  toAddresses: string | null;
  bodyText: string | null;
  snippet: string | null;
  date: number;
  isRead: boolean;
  isStarred: boolean;
  hasAttachment: boolean;
}

/**
 * Search messages using tantivy full-text search.
 * Parses the query string for operators (from:, to:, is:unread, etc.)
 * and sends structured params to the Rust search_messages command.
 */
export async function searchMessages(
  query: string,
  accountId?: string,
  limit: number = 50,
): Promise<SearchResult[]> {
  const trimmed = query.trim();
  if (!trimmed) return [];

  // Parse operators in TS (reuse existing parser)
  if (hasSearchOperators(trimmed)) {
    const parsed = parseSearchQuery(trimmed);
    return invoke<SearchResult[]>("search_messages", {
      params: {
        accountId: accountId ?? "",
        freeText: parsed.freeText || null,
        from: parsed.from ?? null,
        to: parsed.to ?? null,
        subject: parsed.subject ?? null,
        hasAttachment: parsed.hasAttachment ?? null,
        isUnread: parsed.isUnread ?? parsed.isRead === true ? false : null,
        isStarred: parsed.isStarred ?? null,
        before: parsed.before ?? null,
        after: parsed.after ?? null,
        label: parsed.label ?? null,
        limit,
      },
    });
  }

  // Plain free-text search
  return invoke<SearchResult[]>("search_messages", {
    params: {
      accountId: accountId ?? "",
      freeText: trimmed,
      from: null,
      to: null,
      subject: null,
      hasAttachment: null,
      isUnread: null,
      isStarred: null,
      before: null,
      after: null,
      label: null,
      limit,
    },
  });
}

/** Index a single message into tantivy. */
export async function indexMessage(doc: SearchDocument): Promise<void> {
  return invoke<void>("index_message", { doc });
}

/** Batch-index multiple messages into tantivy. */
export async function indexMessagesBatch(
  docs: SearchDocument[],
): Promise<void> {
  return invoke<void>("index_messages_batch", { docs });
}

/** Delete a document from the tantivy index by message_id. */
export async function deleteSearchDocument(messageId: string): Promise<void> {
  return invoke<void>("delete_search_document", { messageId });
}

/** Rebuild the entire tantivy index from SQLite messages. Returns count indexed. */
export async function rebuildSearchIndex(): Promise<number> {
  return invoke<number>("rebuild_search_index");
}

// ═══════════════════════════════════════════════════════════════
// FILTER ENGINE (Phase 6 — evaluate filters in Rust)
// ═══════════════════════════════════════════════════════════════

/** Message data needed for filter matching (sent to Rust). */
export interface FilterableMessage {
  thread_id: string;
  from_name: string | null;
  from_address: string | null;
  to_addresses: string | null;
  subject: string | null;
  body_text: string | null;
  body_html: string | null;
  has_attachments: boolean;
}

/** Per-thread filter result returned from Rust. */
export interface RustFilterResult {
  add_label_ids: string[];
  remove_label_ids: string[];
  mark_read: boolean;
  star: boolean;
}

/**
 * Evaluate enabled filters for an account against a set of messages.
 * Reads filter rules from DB in Rust, runs matching, returns per-thread actions.
 */
export async function filtersEvaluate(
  accountId: string,
  messages: FilterableMessage[],
): Promise<Record<string, RustFilterResult>> {
  if (messages.length === 0) return {};
  return invoke<Record<string, RustFilterResult>>("filters_evaluate", {
    accountId,
    messages,
  });
}

// ═══════════════════════════════════════════════════════════════
// JWZ THREADING (Phase 6 — threading algorithm in Rust)
// ═══════════════════════════════════════════════════════════════

/** Message data for threading (sent to Rust). */
export interface ThreadableMessageInput {
  id: string;
  message_id: string;
  in_reply_to: string | null;
  references: string | null;
  subject: string | null;
  date: number;
}

/** Thread group returned from Rust. */
export interface RustThreadGroup {
  thread_id: string;
  message_ids: string[];
}

/** Build threads from messages using JWZ algorithm (Rust). */
export async function threadingBuildThreads(
  messages: ThreadableMessageInput[],
): Promise<RustThreadGroup[]> {
  if (messages.length === 0) return [];
  return invoke<RustThreadGroup[]>("threading_build_threads", { messages });
}

/** Incrementally update thread assignments (Rust). */
export async function threadingUpdateThreads(
  existingThreads: RustThreadGroup[],
  newMessages: ThreadableMessageInput[],
): Promise<RustThreadGroup[]> {
  if (newMessages.length === 0) return [];
  return invoke<RustThreadGroup[]>("threading_update_threads", {
    existingThreads,
    newMessages,
  });
}

// ═══════════════════════════════════════════════════════════════
// CATEGORIZATION RULE ENGINE (Phase 6 — deterministic rules in Rust)
// ═══════════════════════════════════════════════════════════════

import type { ThreadCategory } from "@/services/db/threadCategories";

/** Input for rule-based categorization (sent to Rust). */
export interface CategorizationInput {
  label_ids: string[];
  from_address: string | null;
  list_unsubscribe: string | null;
}

/**
 * Categorize a single thread using deterministic rules (Rust).
 * Returns "Primary", "Updates", "Promotions", "Social", or "Newsletters".
 */
export async function categorizeThreadByRules(
  input: CategorizationInput,
): Promise<ThreadCategory> {
  return invoke<ThreadCategory>("categorize_thread_by_rules", { input });
}

/**
 * Batch-categorize multiple threads using deterministic rules (Rust).
 * Returns categories in the same order as the inputs.
 */
export async function categorizeThreadsByRules(
  inputs: CategorizationInput[],
): Promise<ThreadCategory[]> {
  if (inputs.length === 0) return [];
  return invoke<ThreadCategory[]>("categorize_threads_by_rules", { inputs });
}

// ── IMAP Sync Engine (Phase 4) ──────────────────────────────────────

/** Result returned by Rust IMAP sync commands. */
export interface ImapSyncResult {
  storedCount: number;
  threadCount: number;
  newInboxMessageIds: string[];
  affectedThreadIds: string[];
}

/**
 * Run initial IMAP sync entirely in Rust.
 * Pipeline: IMAP fetch → parse → DB write → body store → tantivy index → thread.
 * Zero IPC during the sync pipeline — only one invoke() to start it.
 */
export function syncImapInitial(
  accountId: string,
  daysBack?: number,
): Promise<ImapSyncResult> {
  return invoke<ImapSyncResult>("sync_imap_initial", { accountId, daysBack });
}

/**
 * Run delta IMAP sync entirely in Rust.
 * Checks for new UIDs since last sync, fetches and processes only new messages.
 */
export function syncImapDelta(
  accountId: string,
  daysBack?: number,
): Promise<ImapSyncResult> {
  return invoke<ImapSyncResult>("sync_imap_delta", { accountId, daysBack });
}
