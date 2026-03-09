/**
 * Rust DB backend — wraps Tauri invoke() calls for DB queries.
 *
 * Each function mirrors a corresponding TS service function but delegates
 * to a Rust command instead of using the SQLite plugin directly.
 * The facade in queries.ts can switch to these implementations.
 */

import { invoke } from "@tauri-apps/api/core";

import type { DbContact } from "@/services/db/contacts";
import type { DbLabel } from "@/services/db/labels";
import type { DbMessage } from "@/services/db/messages";
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
