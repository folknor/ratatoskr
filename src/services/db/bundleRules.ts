import { invoke } from "@tauri-apps/api/core";
import { getCurrentUnixTimestamp } from "@/utils/timestamp";

export interface DeliverySchedule {
  days: number[]; // 0=Sun, 1=Mon, ..., 6=Sat
  hour: number;
  minute: number;
}

export interface DbBundleRule {
  id: string;
  accountId: string;
  category: string;
  isBundled: number;
  deliveryEnabled: number;
  deliverySchedule: string | null;
  lastDeliveredAt: number | null;
  createdAt: number;
}

export interface DbBundledThread {
  account_id: string;
  thread_id: string;
  category: string;
  held_until: number | null;
}

export async function getBundleRules(
  accountId: string,
): Promise<DbBundleRule[]> {
  return invoke<DbBundleRule[]>("db_get_bundle_rules", { accountId });
}

export async function getBundleRule(
  accountId: string,
  category: string,
): Promise<DbBundleRule | null> {
  return invoke<DbBundleRule | null>("db_get_bundle_rule", {
    accountId,
    category,
  });
}

// biome-ignore lint/complexity/useMaxParams: DB operation requires all fields as separate params
export async function setBundleRule(
  accountId: string,
  category: string,
  isBundled: boolean,
  deliveryEnabled: boolean,
  schedule: DeliverySchedule | null,
): Promise<void> {
  await invoke("db_set_bundle_rule", {
    accountId,
    category,
    isBundled,
    deliveryEnabled,
    schedule: schedule ? JSON.stringify(schedule) : null,
  });
}

export async function holdThread(
  accountId: string,
  threadId: string,
  category: string,
  heldUntil: number | null,
): Promise<void> {
  await invoke("db_hold_thread", {
    accountId,
    threadId,
    category,
    heldUntil,
  });
}

export async function isThreadHeld(
  accountId: string,
  threadId: string,
): Promise<boolean> {
  const now = getCurrentUnixTimestamp();
  return invoke<boolean>("db_is_thread_held", { accountId, threadId, now });
}

export async function getHeldThreadIds(
  accountId: string,
): Promise<Set<string>> {
  const ids = await invoke<string[]>("db_get_held_thread_ids", { accountId });
  return new Set(ids);
}

export async function releaseHeldThreads(
  accountId: string,
  category: string,
): Promise<number> {
  return invoke<number>("db_release_held_threads", { accountId, category });
}

export async function updateLastDelivered(
  accountId: string,
  category: string,
): Promise<void> {
  const now = getCurrentUnixTimestamp();
  await invoke("db_update_last_delivered", { accountId, category, now });
}

export async function getBundleSummary(
  accountId: string,
  category: string,
): Promise<{
  count: number;
  latestSubject: string | null;
  latestSender: string | null;
}> {
  return invoke("db_get_bundle_summary", { accountId, category });
}

/**
 * Batch-fetch bundle summaries for multiple categories in 2 queries instead of 2N.
 */
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
  const results = await invoke<
    {
      category: string;
      count: number;
      latestSubject: string | null;
      latestSender: string | null;
    }[]
  >("db_get_bundle_summaries", { accountId, categories });

  const map = new Map<
    string,
    { count: number; latestSubject: string | null; latestSender: string | null }
  >();
  for (const r of results) {
    map.set(r.category, {
      count: r.count,
      latestSubject: r.latestSubject,
      latestSender: r.latestSender,
    });
  }
  // Ensure all requested categories are in the map
  for (const cat of categories) {
    if (!map.has(cat)) {
      map.set(cat, { count: 0, latestSubject: null, latestSender: null });
    }
  }
  return map;
}

/**
 * Calculate the next delivery time for a schedule from now.
 */
export function getNextDeliveryTime(schedule: DeliverySchedule): number {
  const now = new Date();
  const currentDay = now.getDay();
  const currentMinutes = now.getHours() * 60 + now.getMinutes();
  const targetMinutes = schedule.hour * 60 + schedule.minute;

  // Find the next matching day
  for (let offset = 0; offset < 7; offset++) {
    const day = (currentDay + offset) % 7;
    if (schedule.days.includes(day)) {
      // If today and target time hasn't passed, use today
      if (offset === 0 && currentMinutes < targetMinutes) {
        const target = new Date(now);
        target.setHours(schedule.hour, schedule.minute, 0, 0);
        return Math.floor(target.getTime() / 1000);
      }
      // Otherwise use next occurrence
      if (offset > 0) {
        const target = new Date(now);
        target.setDate(target.getDate() + offset);
        target.setHours(schedule.hour, schedule.minute, 0, 0);
        return Math.floor(target.getTime() / 1000);
      }
    }
  }

  // Fallback: next week same day
  const target = new Date(now);
  target.setDate(target.getDate() + 7);
  target.setHours(schedule.hour, schedule.minute, 0, 0);
  return Math.floor(target.getTime() / 1000);
}
