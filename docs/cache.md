# Attachment & Inline Image Cache

Two caching layers for email attachments:

1. **Inline image store** (`inline_images.db`) — SQLite blob store for small inline images (≤256KB). Content-addressed by xxh3 hash. One copy per unique image across all messages.
2. **File cache** (`attachment_cache/`) — On-disk file cache for all attachment sizes. Content-addressed by xxh3 hash.

## Fetch path (`provider_fetch_attachment`)

```
1. Check inline image store (SQLite lookup by content_hash)
2. Check file cache (content_hash → disk file)
3. Cache miss → fetch from provider API
4. Background: decode base64 → xxh3 hash → store to inline image store (if small image)
   AND write to file cache → update DB columns
```

Both cache-on-miss writes are fire-and-forget so they don't delay the response.

### IMAP sync-time storage

IMAP has access to raw part bytes during `parse_message()`. Two things happen at parse time:

1. xxh3 hash is computed and stored in the `content_hash` column.
2. For small inline images (≤256KB, `image/*`, `is_inline`), the raw bytes are carried through `ImapAttachment.inline_data` and batch-stored into `inline_images.db` during `store_chunk()`.

This means IMAP inline images are available instantly — no on-demand fetch needed.

Gmail/JMAP/Graph don't have bytes at sync time (only a reference ID), so `content_hash` stays NULL and inline images are only stored reactively on first fetch.

### Per-message MIME dedup

Before content hits the DB, inline MIME parts are deduplicated within each message:

- **IMAP**: Parts with identical xxh3 hashes are collapsed. Metadata is merged (prefer real filename over "attachment", preserve content_id, union inline flag).
- **Gmail**: Parts with the same `gmail_attachment_id` are collapsed (same blob = same API ID).
- **Graph/JMAP**: Server-side APIs already return deduplicated attachment lists.

### Frontend dedup

`AttachmentList` and `InlineAttachmentPreview` dedup by `content_id ?? filename:size` to catch any remaining duplicates at the UI layer.

## Schema

### `inline_images` table (`inline_images.db` — separate SQLite file)

```sql
CREATE TABLE inline_images (
    content_hash TEXT PRIMARY KEY,
    data         BLOB NOT NULL,
    mime_type    TEXT NOT NULL,
    size         INTEGER NOT NULL,
    created_at   INTEGER NOT NULL DEFAULT (unixepoch())
);
```

### `attachments` table columns (`ratatoskr.db`)

```sql
-- Migration v25
ALTER TABLE attachments ADD COLUMN content_hash TEXT;
CREATE INDEX idx_attachments_content_hash ON attachments(content_hash);
```

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

**Not yet implemented**: eviction for `inline_images.db`. See `TODO.md`.

## Pre-caching

`preCacheManager.ts` runs every 15 minutes. Finds uncached attachments from the last 7 days under 5MB, calls `provider_fetch_attachment` which handles caching automatically.

## Key files

| File | What |
|------|------|
| `src-tauri/src/inline_image_store/mod.rs` | Inline image SQLite blob store |
| `src-tauri/src/inline_image_store/commands.rs` | Tauri commands: get, stats, clear |
| `src-tauri/src/attachment_cache.rs` | File cache: hash, read, write, DB helpers |
| `src-tauri/src/provider/commands.rs` | Fetch path: inline check → file cache → provider → cache-on-miss |
| `src-tauri/src/imap/parse.rs` | Sync-time hashing + per-message dedup + inline_data extraction |
| `src-tauri/src/sync/pipeline.rs` | `store_chunk` → `store_inline_images` batch insert |
| `src-tauri/src/gmail/parse.rs` | Per-message dedup by attachment_id |
| `src/services/attachments/cacheManager.ts` | File cache eviction, clear |
| `src/services/attachments/preCacheManager.ts` | Background pre-caching |

## Sync INSERT preservation

All four provider sync paths use `ON CONFLICT(id) DO UPDATE SET` instead of `INSERT OR REPLACE`. This prevents resyncs from wiping `content_hash`, `cached_at`, `local_path`, and `cache_size`.
