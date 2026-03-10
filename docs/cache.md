# Attachment Cache

Content-addressed cache for email attachments. Identical blobs (signatures, logos, etc.) store one copy on disk regardless of how many messages reference them.

## How it works

Cache files live at `{AppData}/attachment_cache/{content_hash}` where `content_hash` is a 16-char hex xxh3_64 of the raw bytes.

### Fetch path (`provider_fetch_attachment`)

```
1. DB lookup: find attachment row by (account_id, message_id, gmail_attachment_id)
2. If content_hash is set AND cache file exists on disk → return from cache
3. Otherwise → fetch from provider API
4. Background task: decode base64 → xxh3 hash → write file → update DB
```

The cache-on-miss is fire-and-forget so it doesn't delay the response.

### IMAP sync-time hashing

IMAP has access to raw part bytes during `parse_message()`. The xxh3 hash is computed there and stored in the `content_hash` column at insert time. This means IMAP attachments have their hash before the first fetch — the fetch path just needs to write the file.

Gmail/JMAP/Graph don't have bytes at sync time (only a reference ID), so `content_hash` stays NULL until first fetch.

### Per-message MIME dedup

Before content hits the DB, inline MIME parts are deduplicated within each message:

- **IMAP**: Parts with identical xxh3 hashes are collapsed. Metadata is merged (prefer real filename over "attachment", preserve content_id, union inline flag).
- **Gmail**: Parts with the same `gmail_attachment_id` are collapsed (same blob = same API ID).
- **Graph/JMAP**: Server-side APIs already return deduplicated attachment lists.

### Frontend dedup

`AttachmentList` and `InlineAttachmentPreview` dedup by `content_id ?? filename:size` to catch any remaining duplicates at the UI layer.

## Schema

```sql
-- Migration v25
ALTER TABLE attachments ADD COLUMN content_hash TEXT;
CREATE INDEX idx_attachments_content_hash ON attachments(content_hash);
```

Cache-related columns on `attachments`:

| Column | Purpose |
|--------|---------|
| `content_hash` | xxh3_64 hex string, set on first fetch (or at IMAP sync) |
| `local_path` | Relative path to cached file (`attachment_cache/{hash}`) |
| `cached_at` | Unix timestamp of when the file was cached |
| `cache_size` | Size in bytes of the cached file |

## Eviction

User configures max cache size in settings (100MB–2GB, default 500MB).

`evictOldestCached()` in `cacheManager.ts`:
1. Sum all `cache_size` where `cached_at IS NOT NULL`
2. If over limit, get oldest cached rows ordered by `cached_at ASC`
3. For each: clear DB cache fields, then check `db_count_cached_by_hash` — only delete the file if no other cached attachments share the hash
4. Stop once enough bytes freed

`clearAllCache()` deletes the entire `attachment_cache/` directory and nulls all cache fields.

## Pre-caching

`preCacheManager.ts` runs every 15 minutes. Finds uncached attachments from the last 7 days under 5MB, calls `provider_fetch_attachment` which handles caching automatically.

## Key files

| File | What |
|------|------|
| `src-tauri/src/attachment_cache.rs` | Hash, read, write, DB helpers |
| `src-tauri/src/provider/commands.rs` | Cache check + cache-on-miss in `provider_fetch_attachment` |
| `src-tauri/src/imap/parse.rs` | Sync-time hashing + per-message dedup |
| `src-tauri/src/gmail/parse.rs` | Per-message dedup by attachment_id |
| `src/services/attachments/cacheManager.ts` | Eviction, clear |
| `src/services/attachments/preCacheManager.ts` | Background pre-caching |

## Sync INSERT preservation

All four provider sync paths use `ON CONFLICT(id) DO UPDATE SET` instead of `INSERT OR REPLACE`. This prevents resyncs from wiping `content_hash`, `cached_at`, `local_path`, and `cache_size`.
