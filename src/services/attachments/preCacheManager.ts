import { invoke } from "@tauri-apps/api/core";
import { useSyncStateStore } from "@/stores/syncStateStore";
import {
  type BackgroundChecker,
  createBackgroundChecker,
} from "../backgroundCheckers";
import { getSetting } from "../db/settings";
import { getEmailProvider } from "../email/providerFactory";
import { cacheAttachment } from "./cacheManager";

const MAX_ATTACHMENT_SIZE: number = 5 * 1024 * 1024; // 5MB
const RECENT_DAYS = 7;
const BATCH_LIMIT = 20;

interface UncachedAttachment {
  id: string;
  message_id: string;
  account_id: string;
  size: number;
  gmail_attachment_id: string | null;
  imap_part_id: string | null;
}

let checker: BackgroundChecker | null = null;

async function preCacheRecent(): Promise<void> {
  // Skip if offline
  if (!useSyncStateStore.getState().isOnline) return;

  // Get total cache size
  const currentCacheSize = await invoke<number>(
    "db_attachment_cache_total_size",
  );

  const maxCacheMb = parseInt(
    (await getSetting("attachment_cache_max_mb")) ?? "500",
    10,
  );
  const maxCacheBytes = maxCacheMb * 1024 * 1024;

  if (currentCacheSize >= maxCacheBytes) return;

  // Find uncached small recent attachments
  const cutoff = Math.floor(Date.now() / 1000) - RECENT_DAYS * 24 * 60 * 60;
  const attachments = await invoke<UncachedAttachment[]>(
    "db_uncached_recent_attachments",
    {
      maxSize: MAX_ATTACHMENT_SIZE,
      cutoffEpoch: cutoff,
      limit: BATCH_LIMIT,
    },
  );

  for (const att of attachments) {
    // Check cache limit
    if (currentCacheSize + (att.size ?? 0) > maxCacheBytes) break;

    try {
      const attachmentId = att.gmail_attachment_id ?? att.imap_part_id;
      if (!attachmentId) continue;

      const provider = await getEmailProvider(att.account_id);
      const result = await provider.fetchAttachment(
        att.message_id,
        attachmentId,
      );

      // Decode base64 data
      const binary = Uint8Array.from(atob(result.data), (c) => c.charCodeAt(0));
      await cacheAttachment(att.id, binary);
    } catch (err) {
      console.warn(`[PreCache] Failed to cache attachment ${att.id}:`, err);
    }
  }
}

export function startPreCacheManager(): void {
  if (checker) return;
  checker = createBackgroundChecker(
    "AttachmentPreCache",
    preCacheRecent,
    900_000,
  );
  checker.start();
}

export function stopPreCacheManager(): void {
  checker?.stop();
  checker = null;
}
