import { invoke } from "@tauri-apps/api/core";

export type ThreadCategory =
  | "Primary"
  | "Updates"
  | "Promotions"
  | "Social"
  | "Newsletters";

export const ALL_CATEGORIES: ThreadCategory[] = [
  "Primary",
  "Updates",
  "Promotions",
  "Social",
  "Newsletters",
];

export async function getThreadCategory(
  accountId: string,
  threadId: string,
): Promise<string | null> {
  return invoke<string | null>("db_get_thread_category", {
    accountId,
    threadId,
  });
}

export async function getThreadCategoryWithManual(
  accountId: string,
  threadId: string,
): Promise<{ category: string; isManual: boolean } | null> {
  return invoke<{ category: string; isManual: boolean } | null>(
    "db_get_thread_category_with_manual",
    { accountId, threadId },
  );
}

export async function getRecentRuleCategorizedThreadIds(
  accountId: string,
  limit: number = 20,
): Promise<
  { id: string; subject: string; snippet: string; fromAddress: string }[]
> {
  return invoke("db_get_recent_rule_categorized_thread_ids", {
    accountId,
    limit,
  });
}

export async function getCategoriesForThreads(
  accountId: string,
  threadIds: string[],
): Promise<Map<string, string>> {
  if (threadIds.length === 0) return new Map();
  const rows = await invoke<{ thread_id: string; category: string }[]>(
    "db_get_categories_for_threads",
    { accountId, threadIds },
  );
  return new Map(rows.map((r) => [r.thread_id, r.category]));
}

export async function setThreadCategory(
  accountId: string,
  threadId: string,
  category: string,
  isManual: boolean = false,
): Promise<void> {
  await invoke("db_set_thread_category", {
    accountId,
    threadId,
    category,
    isManual,
  });
}

export async function setThreadCategoriesBatch(
  accountId: string,
  categories: Map<string, string>,
): Promise<void> {
  const entries = Array.from(categories.entries());
  await invoke("db_set_thread_categories_batch", {
    accountId,
    categories: entries,
  });
}

export async function getCategoryUnreadCounts(
  accountId: string,
): Promise<Map<string, number>> {
  const rows = await invoke<{ category: string | null; count: number }[]>(
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

export async function getUncategorizedInboxThreadIds(
  accountId: string,
  limit: number = 20,
): Promise<
  { id: string; subject: string; snippet: string; fromAddress: string }[]
> {
  return invoke("db_get_uncategorized_inbox_thread_ids", {
    accountId,
    limit,
  });
}
