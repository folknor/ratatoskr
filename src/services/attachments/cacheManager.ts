import { invoke } from "@tauri-apps/api/core";
import { getAttachmentCacheMaxMb } from "@/services/settings/runtimeFlags";

const CACHE_DIR = "attachment_cache";

export async function getCacheSize(): Promise<number> {
  return invoke("db_get_attachment_cache_size");
}

export async function evictOldestCached(): Promise<void> {
  const maxBytes = (await getAttachmentCacheMaxMb()) * 1024 * 1024;
  const currentSize = await getCacheSize();

  if (currentSize <= maxBytes) return;

  const excess = currentSize - maxBytes;
  let freed = 0;

  const rows = await invoke<
    {
      id: string;
      local_path: string;
      cache_size: number;
      content_hash: string | null;
    }[]
  >("db_get_oldest_cached_attachments", { limit: 100 });

  for (const row of rows) {
    if (freed >= excess) break;

    // Clear this attachment's cache entry in DB
    await invoke("db_clear_attachment_cache_entry", {
      attachmentId: row.id,
    });

    // Only delete the file if no other cached attachments share this content hash
    if (row.content_hash) {
      const refCount = await invoke<number>("db_count_cached_by_hash", {
        contentHash: row.content_hash,
      });
      if (refCount > 0) {
        // Other attachments still reference this file — don't delete it
        freed += row.cache_size;
        continue;
      }
    }

    try {
      const { remove, BaseDirectory } = await import("@tauri-apps/plugin-fs");
      await remove(row.local_path, { baseDir: BaseDirectory.AppData });
    } catch {
      // file may not exist
    }

    freed += row.cache_size;
  }
}

export async function clearAllCache(): Promise<void> {
  try {
    const { remove, BaseDirectory } = await import("@tauri-apps/plugin-fs");
    try {
      await remove(CACHE_DIR, {
        baseDir: BaseDirectory.AppData,
        recursive: true,
      });
    } catch {
      // directory may not exist
    }
  } catch {
    // ignore
  }

  await invoke("db_clear_all_attachment_cache");
}
