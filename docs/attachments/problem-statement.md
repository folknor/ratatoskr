# Attachments (Cached Storage and File Handling)

## Overview

Email and calendar attachments need to behave like Outlook in Cached Exchange Mode and Thunderbird with "Synchronize messages locally" enabled: **bytes are downloaded during sync and stored on disk so the user can Open, Save, or forward them with no live server connection**. Today, Ratatoskr stores only attachment *metadata* during sync (filename, size, mime type, provider blob id) and never fetches the bytes - meaning offline access doesn't work, and the Open / Save / Save All buttons in the reading pane and pop-out viewer are wired to `log::info!("not yet implemented")` stubs.

This document defines the storage architecture, pre-fetch policy, and file-handling behavior for the unified attachment layer that mail (today) and calendar (later) both depend on.

## Why this matters

The project's stated audience is "enterprise users currently locked into Outlook/Microsoft 365" with "150+ GB cached mailboxes" as a hard requirement. Outlook Cached Exchange Mode pre-fetches and locally caches the entire mailbox including attachments by default - that is the behavior these users expect. Fetch-on-click (the Gmail-web model) is the wrong default for this audience: it breaks offline workflows, it's slow over high-latency links, and it's surprising to users moving from Outlook.

Beyond offline use, a populated local attachment cache also unlocks future work: full-text search inside attachments, attachment dedup across messages (already content-hash-keyed in the schema), and faster Compose forwards (no round-trip to re-fetch the bytes).

## Relationship to the Service

The orchestration described here (pre-fetch, blob-store writes,
eviction, GC, attachment text extraction) all run inside **the
Service** - the subprocess worker described in
`docs/architecture.md` under "Service process model". The `BlobStore`
trait and its implementations are libraries (in `crates/stores/`) and
don't care which process instantiates them; the writer instance lives
in the Service, while UI-side reads are positional reads against the
immutable on-disk pack files (concurrent-reader-safe, no IPC).
Cache-miss attachment fetches from the UI cross the IPC boundary;
cache hits do not. The split is detailed in `File operations` below
and in the architecture doc.

## Current state

What's on disk today:

- **Schema is ready.** `attachments` (in `crates/db/src/db/schema/02_mail.sql`) has `local_path TEXT`, `cached_at INTEGER`, `cache_size INTEGER`, `content_hash TEXT`, with an index on `content_hash` for dedup queries. The schema was designed for caching - it just isn't populated.
- **Storage primitives are ready.** `crates/stores/src/attachment_cache.rs` has the full set of building blocks: `read_cached`, `write_cached`, `find_cache_info`, `update_cache_fields`, `try_cache_hit`, `try_inline_image_hit`, `cache_after_fetch`. None of them are called from the mail-fetch path - only the inline-image store gets seeded during sync (small CID images).
- **Eviction is ready.** `enforce_cache_limit` does LRU eviction by `cached_at`, is dedup-aware (only deletes the file if no other row references the same content hash), and reads its bound from a `attachment_cache_max_mb` setting that defaults to 500 MB. There's no UI to change it yet.
- **Provider fetch is ready.** All four providers implement `ProviderOps::fetch_attachment(message_id, attachment_id) -> AttachmentData`. The function exists everywhere and is called from nowhere.
- **Squeeze is ready.** `squeeze::compress(bytes, mime_type, &Config)` returns a `CompressResult` (or `Unchanged` if not worthwhile). It is currently used as a CLI and in inline-image storage; it is not used in the attachment-cache path.

What's missing:

- **No orchestration.** Nothing ties cache lookup + provider fetch + decode + squeeze + write-back + DB update together. Each step exists in isolation.
- **No sync-time pre-fetch.** Sync writes attachment metadata rows with `content_hash: None, local_path: NULL` and stops. Bytes never get fetched.
- **Reading pane and pop-out viewer Open / Save / Save All are stubs.** `crates/app/src/ui/reading_pane.rs:370/374` and `crates/app/src/handlers/pop_out/message_view.rs:69/73/77` log "not yet implemented". The buttons are clickable but inert.
- **No per-account policy.** There is no setting for "cache attachments for this account", and no per-account size threshold below which to fetch.
- **Calendar event attachments aren't modeled at all.** CalDAV `ATTACH` properties, Graph `event.attachments`, ICS file attachments - none are captured. The `attachments` table is keyed `(account_id, message_id)` and assumes mail.

## Cross-provider attachment fetch

| Provider | Endpoint | Returns | Notes |
|----------|----------|---------|-------|
| Gmail | `users.messages.attachments.get` | base64url-encoded bytes | One round-trip per attachment. Cheap. |
| Graph | `GET /me/messages/{id}/attachments/{id}/$value` | raw bytes | One round-trip per attachment. Reference attachments are URLs - skip. |
| JMAP | `Blob/get` (or download URL from session) | raw bytes | Single blob endpoint per attachment. |
| IMAP | `FETCH BODY[part]` with BODYSTRUCTURE-derived part id | raw bytes (possibly base64-encoded inside MIME) | Per-folder session needed; reuse the session that fetched the message. |

All four return roughly the same thing: bytes for one attachment, identified by `(account_id, message_id, attachment_id)`. The orchestration layer can present a uniform `fetch_one(account_id, message_id, attachment_id) -> Vec<u8>` API on top.

## Storage architecture

The unified pipeline for fetching an attachment looks like:

```
                        +---- inline-image-store hit (small images, SQLite blob)
                        |
fetch_or_load(att_id) --+---- BlobStore::get(content_hash)
                        |
                        +---- provider fetch_attachment
                                    |
                                    v
                              decode base64 if needed
                                    |
                                    v
                              squeeze::compress (lossless only by default)
                                    |
                                    v
                              BlobStore::put(bytes) -> Hash
                                    |
                                    v
                              UPDATE attachments.content_hash
                                    |
                                    v
                              return bytes to caller
```

### `BlobStore` trait

The pipeline operates on a `BlobStore` trait, not a concrete storage type. This deliberately leaves room for platform-specific implementations - notably an EROFS-backed store on Linux as a follow-up phase (see `Considered alternatives`). v1 ships exactly one impl (`PackStore`) on all three platforms; the trait exists so that adding a second backend later is an additive change with no impact on the orchestration layer.

```rust
// Sketch; final shape settles in the Phase 1a planning session.
pub trait BlobStore: Send + Sync {
    fn put(&self, bytes: &[u8]) -> Result<BlobHash, Error>;
    fn get(&self, hash: &BlobHash) -> Result<Option<Vec<u8>>, Error>;
    fn unref(&self, hash: &BlobHash) -> Result<(), Error>;
    fn evict_lru(&self, target_free_bytes: u64) -> Result<u64, Error>;
    fn gc(&self) -> Result<GcStats, Error>;
    fn recover(&self) -> Result<(), Error>;
}
```

`put` is content-addressed and idempotent - two `put`s of identical bytes return the same hash and increment a refcount rather than re-storing. The orchestration layer never sees the storage details.

### Storage tiers

Three storage tiers, with the middle tier accessed through `BlobStore`:

1. **Inline image store** (`crates/stores/src/inline_image_store.rs`) - SQLite blob store for images <= `MAX_INLINE_SIZE` (currently 64 KB). Already populated during sync for inline CID images. Stays as-is - SQLite blob storage is fine at this size class and the existing infrastructure works. Not a `BlobStore` impl; lives parallel to it for the small-image case it already serves.

2. **Attachment blob store** (new, `BlobStore` impl). v1 implementation: `PackStore` (`crates/stores/src/attachment_pack.rs`) - **content-addressed pack files**, ~256 MB segments under `<app-data>/attachment_packs/data-NNNNNN.pack`. Each blob is appended once and referenced by `(pack_file_id, offset, length)` from a SQLite index table. Multiple attachment rows that hash to the same content reference the same `(pack_file_id, offset, length)` triple. Cross-platform; this is what ships in v1 everywhere.

3. **Provider** - source of truth, fetched on cache miss. Network round-trip.

The pack store replaces the existing `crates/stores/src/attachment_cache.rs` file-per-blob design (currently unused - the `local_path`, `cached_at`, `cache_size`, `content_hash` columns exist on `attachments` but are never populated). At our 150-200 GB target with the long tail of small blobs (avatars, signatures, tracking pixels), a flat-file-per-blob layout produces ~2 million inodes with corresponding metadata pressure, terrible backup-tool throughput, and per-blob SSD-page read overhead even for sub-KB content. Pack files reduce inode count by ~3 orders of magnitude, give us sequential write patterns, and let multiple small blobs share an SSD page on read. See "Considered alternatives" below for the full rationale.

### Pack file format (v1)

Append-only segments, rotated when the current segment exceeds `PACK_TARGET_SIZE` (default 256 MB). Each segment is a sequence of blob frames followed by a tail:

```
+--------------------------------------------------+
| frame 0   : magic | length | xxh3_64 | bytes...  |
| frame 1   : magic | length | xxh3_64 | bytes...  |
| ...                                              |
| frame N-1 : magic | length | xxh3_64 | bytes...  |
+--------------------------------------------------+
| pack tail : magic | version | frame_count | crc  |
+--------------------------------------------------+
```

- **Frame header**: 4-byte magic + 4-byte length + 8-byte xxh3_64 of the payload + payload bytes. Length is the byte count of the payload (post-squeeze). Magic distinguishes data frames from the tail.
- **Pack tail**: written last, identifies the file as complete. Used for crash recovery: if the tail is missing, the file is "open" (current segment) and gets scanned forward at startup to rebuild the trailing index entries.
- **Versioned**: a `version` byte in the tail lets us evolve the format without breaking older segments.
- **No encryption in v1**: framed for it - encryption would replace the payload region of each frame with `nonce | ciphertext | tag`. Tracked separately under "Mail content stores not encrypted at rest" in TODO.md.

A separate "tombstones" file (`tombstones-NNNNNN.log`) records `(pack_id, blob_xxh3)` pairs for blobs that have been logically deleted. Tombstones are consulted during read for safety (refuse to return tombstoned data even if the index has a stale row) and consumed by the GC pass.

### SQLite index (PackStore-specific)

Every `BlobStore` implementation owns its own on-disk index format. For `PackStore` v1, the index lives in a new `attachment_blobs` SQLite table:

```sql
CREATE TABLE attachment_blobs (
    content_hash TEXT PRIMARY KEY,   -- xxh3_64 hex
    pack_file_id INTEGER NOT NULL,   -- which pack-NNNNNN.pack
    offset INTEGER NOT NULL,         -- byte offset within the pack
    length INTEGER NOT NULL,         -- payload length (post-squeeze)
    refcount INTEGER NOT NULL,       -- number of attachments rows pointing here
    written_at INTEGER NOT NULL,
    last_read_at INTEGER             -- updated by `fetch_or_load` on cache hit
);
CREATE INDEX idx_attachment_blobs_lru ON attachment_blobs(last_read_at);
```

The existing `attachments.content_hash` column becomes the join key. `attachments.local_path`, `cached_at`, and `cache_size` are dropped (or migrated and then dropped) - that information now lives on `attachment_blobs`.

The index is **rebuildable** from pack tails on corruption: walk every pack, replay frames, regenerate the table. This is the same recoverability story restic / borg / kopia provide.

### Crash safety

- **Append-only writes.** A torn write only affects the *current* (open) pack and only the trailing partial frame. On startup, the pack-store opens the current pack, walks frames forward from the last known good index entry, and truncates any incomplete tail frame.
- **fsync batching.** Every N appended blobs (or every M ms, whichever first) we fsync the pack and the SQLite index together. The two are kept consistent via a SQLite transaction wrapping the index update; the actual byte append precedes the transaction commit so a crash leaves us with a wasted disk extent (cleaned up by GC) but never an index entry pointing at a missing blob.
- **Index ↔ pack divergence.** Detected at read time: if the index says `(pack=42, offset=X, length=Y)` and the frame's xxh3_64 doesn't match the index's `content_hash`, return an error and log. Repair tool walks the affected pack and rebuilds its index entries.

### Concurrency

- **One writer per pack at a time.** Multiple sync tasks share a `tokio::sync::Mutex` around the current pack writer. Writes are short (single frame append + index transaction) so contention is bounded.
- **Many readers, lock-free.** Reads are `pread`-style positional I/O against immutable closed packs, or against the open pack at offsets the index has already committed. The OS page cache handles the actual hot-blob caching.
- **Pack rotation.** When a write would push the current pack past `PACK_TARGET_SIZE`, the writer finalizes the tail, fsync's, and starts a new pack file before releasing the lock.

### Compression policy

`squeeze::compress` is mime-aware and runs **before** the bytes hit the pack:

- **Lossless gains**: PNG (oxipng), PDF (lopdf object stream rewrite + dedup), OOXML/ODF (re-pack with better deflate), SVG (whitespace + comment stripping).
- **Lossy gains**: JPEG (mozjpeg). **Off by default** for cached attachments - we don't want to silently re-encode user bytes. Optional setting if users want it.
- **Skip**: zip / 7z / mp4 / already-compressed binaries - `squeeze` returns `Unchanged`. Pack stores the original bytes.

The pack stores the post-squeeze bytes. The DB row records `attachment_blobs.length` (post-squeeze) and `attachments.size` (pre-squeeze, from the provider's metadata), so the user-visible size in the UI stays accurate.

If the user later opens or forwards the attachment, the cached bytes are returned as-is. For lossless squeeze (the default), this is byte-equivalent to the original at the application level (a re-encoded PNG decodes to the same pixels; a re-packed PDF renders identically). Forwarding a squeezed attachment sends the squeezed bytes - this is fine; recipients will receive a smaller file with no semantic loss.

### Content-addressable dedup

The `content_hash` is the primary key on `attachment_blobs`. Insert-or-ignore semantics:

- Hash the bytes after squeeze (xxh3_64 - fast, 64-bit, more than enough for the keyspace we're addressing).
- `INSERT OR IGNORE INTO attachment_blobs(content_hash, ...) VALUES (?, ...)`. If the row already exists, the bytes are not re-appended to a pack - we just `UPDATE attachment_blobs SET refcount = refcount + 1 WHERE content_hash = ?`.
- The new `attachments` row gets `content_hash` populated and is otherwise unchanged.
- Result: a 12 MB company-wide PowerPoint sent to 200 people is appended to a pack exactly once and referenced 200 times.

### Considered alternatives

Why pack files and not one of these:

| Option | Verdict |
|--------|---------|
| **Flat files per blob with `<hash[0:2]>/<hash[2:4]>/<hash>` fanout** | The current design. Simplest to implement (~50 LOC patch on top of existing code). Loses on inode pressure (~2M files at 200 GB target), backup-tool throughput, and small-blob SSD-page locality. Fine for v1 if we accept the cost; we don't. |
| **LMDB via `heed`** | Solves the file-count problem. Mmap-based, fast at this size. **No online compaction** - once the file grows you can't shrink it without dump-and-reload. Fatal for a cache that does eviction. |
| **RocksDB** | Has online compaction. C++ FFI; compile-time and Windows-build cost is real. Write amplification (5-10x for LSM compaction) is wasted work for our write-once workload. Overkill. |
| **`sled`** | Pure Rust. Less battle-tested than the others; has had stability issues over the years. Not the storage layer to bet on for our highest-volume on-disk data. |
| **`rustic_core` (restic format)** | The `blob` module is private; no public put/get-by-hash API. Public surface is snapshot-shaped (Repository::backup/restore), not blob-shaped. Crate is explicitly "subject to change." Spike concluded: wrong shape, wrong contract. The format is well-documented and we *could* implement restic-compatible packs ourselves for free backup-tool compatibility (~200 extra LOC) - flagged as an option for v2. |
| **Container-registry style (Docker registry)** | Flat files with `sha256/<prefix>/<hash>/data` fanout. Works because Docker layers are MB-to-GB-sized. Doesn't help with our small-blob long tail. |
| **Single growing SQLite blob DB** | The Outlook PST/OST cautionary tale. Notorious for corruption at >20 GB; "compact" is a dump-and-reload. No. |
| **Pack files with SQLite index** (chosen for v1, all platforms) | Standard answer used by restic, borg, kopia, bup, Firefox cache2, Chromium disk cache. Solves all of: file-count pressure, backup ergonomics, small-blob locality, online compaction (via tombstones + GC), crash safety (append-only). ~600 LOC. We own the format and can extend it (encryption, restic-compat) when needed. Lives behind the `BlobStore` trait. |
| **EROFS-backed rolling images** (deferred to a follow-up phase, Linux-only) | Read-only kernel filesystem with built-in zstd compression and intra-image dedup. Rolling-image model (~256 MB EROFS images, baked from a small staging area, never modified after) maps cleanly onto our workload. Wins: free compression (no squeeze on the cache side), free intra-image dedup, well-tested format, less per-blob overhead than pack files. Loses: read-only means a bake step instead of streaming appends, granular eviction is less precise (whole-image delete), kernel mount is Linux-only (userspace readers exist but maturity varies on macOS/Windows). Lands as a second `BlobStore` impl after v1 ships and we have real cache-pressure data; `cfg(target_os = "linux")` gates which impl is selected. |

## Pre-fetch policy

Pre-fetch happens in the sync pipeline, after `insert_attachments` writes metadata rows. It runs as a background task spawned per-batch - sync completion does **not** wait for pre-fetch.

**Defaults:**
- **Enabled per account on creation** (`prefetch_attachments: true`).
- **Per-attachment size threshold**: 25 MB. Above this, do not pre-fetch; user can still trigger fetch on Open/Save click.
- **Per-message threshold**: skip pre-fetch entirely for messages whose summed attachment size exceeds 100 MB (a single 200-MB email shouldn't dominate the sync queue).
- **Cache cap**: respects existing `attachment_cache_max_mb` (default 500 MB; bumped to a saner default - probably 5 GB - as part of this work).
- **Backfill**: on first run after the feature lands, do not retroactively pre-fetch existing messages. The cache populates lazily for old messages (fetch-on-click cache-write-back) and eagerly for new ones (sync-time pre-fetch). A future "Cache all attachments now" button can do the backfill on demand.

**Per-account opt-out:** Users can disable pre-fetch per account from settings. With pre-fetch off, attachments only enter the cache when the user clicks Open/Save (fetch-on-click write-back). This matches the Gmail-web model for users who want it.

**Failure behavior:** Pre-fetch errors are logged but never fail the sync. The attachment row stays metadata-only and gets fetched on demand instead.

**Progress reporting:** The existing `ProgressReporter` trait emits "Caching attachments... 42 / 120" events that the status bar already knows how to render.

## Cache eviction and GC

Eviction is a two-phase process because the on-disk pack format is append-only.

**Phase 1 - logical eviction (cheap, frequent):**
- LRU candidates picked from `attachment_blobs ORDER BY last_read_at ASC` until the live byte count would fall under the cap.
- For each candidate: `UPDATE attachment_blobs SET refcount = 0` + append a tombstone to `tombstones-NNNNNN.log`. The bytes are still on disk in the pack file but unreferenced and unreadable (reads consult the tombstone log).
- The space is *logically* freed immediately - new writes won't push us over cap because `SUM(length WHERE refcount > 0)` is what we measure.

**Phase 2 - GC (expensive, rare):**
- Runs on app idle when total tombstoned bytes exceed a threshold (e.g. 25% of any single pack, or 10% of total cache).
- For each pack with high tombstone density: read the pack, copy live frames to a fresh pack at the end of the chain, update `pack_file_id` + `offset` for each surviving blob in the index, atomically delete the old pack file.
- Worst-case cost: read+rewrite of one pack (~256 MB sequential I/O). Runs in the background via the existing `enforce_cache_limit` task path, refactored to drive Phase 2.

**Tweaks for the new pre-fetch volume:**

1. **Default cap raised** from current `attachment_cache_max_mb=500` to **5 GB**. Settings UI exposes the value with a slider (1 GB - 50 GB).
2. **Touch on read**: `fetch_or_load` updates `last_read_at` on a cache hit so frequently-opened files survive Phase 1 eviction. Done in the same SQLite transaction as the read response so we don't add a round-trip.
3. **Eviction trigger**: Phase 1 runs after every sync batch (cheap). Phase 2 runs on app idle every N minutes if the tombstone threshold is met.
4. **Cache-bypass for oversize items**: if pre-fetching one item would force Phase 1 to evict more than X% of the cache, skip the item entirely rather than thrash. Configurable; default X=10%.

## File operations: Open, Save, Save All

The three button operations all sit on top of the same `fetch_or_load` orchestration. UI handlers are shared between the reading pane and the pop-out viewer (currently both surfaces have their own stubbed copies). The orchestration itself runs in the Service; the UI calls into it via IPC for cache-miss fetches and reads cache-hit bytes directly from disk.

**Hot path (cache hit):** UI looks up `attachment_blobs` row by `content_hash`, reads the bytes positionally from the immutable pack file at `(pack_file_id, offset, length)`. No IPC. The blob store's read API is in the `stores` crate and the UI links it directly.

**Cold path (cache miss):** UI sends `attachment.fetch { account_id, message_id, attachment_id }` to the Service. The Service runs the full pipeline (provider fetch -> squeeze -> `BlobStore::put`), then returns either the bytes inline (small) or just the resulting `content_hash` (large; UI re-reads from disk). The pipeline updates `attachment_blobs.last_read_at` regardless of path.

### Open

1. Try the hot path. If it misses, send `attachment.fetch` to the Service and await the response.
2. Write the bytes to `<app-data>/opened_attachments/<safe_filename>` (NOT `/tmp` - CLAUDE.md forbids `/tmp` use).
3. OS-default open via `xdg-open` (Linux) / `open` (macOS) / `cmd /c start` (Windows). Pattern is already established at `reading_pane.rs:917-925` for link-click handling.
4. Files in `opened_attachments/` are not deleted on close (the OS handler may keep the file open or move it). They get reaped by a periodic cleanup (configurable, default: 7 days).

**Filename safety:** Strip path separators, control chars, and shell metacharacters. Reuse `sanitize_filename` from `crates/app/src/handlers/pop_out/save_as.rs:38`.

### Save

1. `rfd::AsyncFileDialog::save_file()` with the original filename pre-filled, mime-derived extension filter.
2. Hot path / cold path as in Open.
3. Write to the chosen path with `std::fs::write`.
4. Remember the chosen folder per thread (see "Last folder per thread" below).

### Save All

1. `rfd::AsyncFileDialog::pick_folder()`.
2. For each attachment on the message: hot path / cold path; write to `<chosen_folder>/<safe_filename>`. Filename collisions get a `(N)` suffix.
3. Aggregate progress reported through `ProgressReporter` (notifications from the Service for cold-path items). Errors collected and shown as a single end-of-operation toast (once the toast system lands - until then, log).

### Last folder per thread

Currently a separate TODO entry. Subsumed into this work since it's natural to wire alongside Save / Save All. Storage: small key-value table `attachment_save_paths(thread_id, last_path, updated_at)`. Pre-fills the file dialog's initial directory.

## Settings

Two new sections under per-account settings ("Mail" / "Storage"), one global ("Storage"):

**Per account:**
- `Cache attachments for offline use` (boolean, default true)
- `Pre-fetch threshold` (slider, 1 - 100 MB, default 25 MB)

**Global:**
- `Attachment cache size cap` (slider, 1 - 50 GB, default 5 GB)
- `Compress cached attachments` (toggle, default on - controls squeeze pipeline)
- `Allow lossy compression (JPEG re-encoding)` (toggle, default off)
- `Cleanup opened-files temp folder after N days` (slider, default 7 days)

## Calendar attachments

Calendar events can carry attachments via:
- **CalDAV** - VEVENT `ATTACH` property (URL or inline base64).
- **Graph** - `event.attachments[]`, same shape as `message.attachments[]`.
- **ICS** - `ATTACH` line in the VEVENT block.

None of these are currently captured by the calendar sync. When they are added (separate work), they should plug into the same orchestration:

- `core::attachments::fetch_or_load(account_id, parent_id, attachment_id)` where `parent_id` is either a message id or an event id.
- A new `event_attachments` table mirroring `attachments` (or a polymorphic `parent_kind TEXT NOT NULL` column on `attachments` - decide at calendar-attachment time, not now).
- Same pack store, same content-hash dedup, same eviction. A meeting invite that carries the same agenda PDF as a forwarded email naturally shares one frame in one pack.
- Same Open / Save handlers, parameterized by parent kind.

The orchestration API should be designed to accept a `ParentRef` enum (`Message { account_id, message_id }` | `Event { account_id, event_id }`) so calendar can be wired in without changing the contract.

## Relationship to `cloud-attachments.md`

`cloud-attachments.md` covers a different axis: **outgoing large files** uploaded to OneDrive / Google Drive and sent as sharing links, plus **incoming detection** of cloud-storage links in received messages.

The two features intersect at:

- **Threshold logic**: cloud-attachments uses 25 MB / 10 MB defaults. Pre-fetch uses 25 MB. Worth surfacing both in the same Attachments settings panel so they read coherently.
- **Attachment chip widget** (future): the chip in the reading pane / pop-out viewer needs to render both real file attachments (with Open / Save) and detected cloud-link attachments (with "Open in browser"). The widget is shared even if the storage is not.
- **Ordering**: this work (pre-fetch + cache + Open/Save) ships first. Cloud-link enrichment and incoming-link detection ride on top once attachment chips exist as a real surface.

The `cloud_attachments` table (migration 39) and the upload paths in `crates/graph/src/onedrive.rs` and `crates/gmail/src/gdrive.rs` are unaffected by this work.

## Out of scope (v1)

- **Search inside attachment content** (PDF text extraction, OOXML text extraction). Possible follow-up once the cache is populated, but a substantial separate piece of work (text extractors, FTS index integration, language detection).
- **Attachment preview rendering inside Ratatoskr** (PDF viewer, image gallery). Open in OS handler is sufficient for v1.
- **Backfill UI** ("Cache all attachments for this account now"). Lazy fill via fetch-on-click + sync-time pre-fetch covers it; a backfill button can come later if users ask.
- **Attachment encryption at rest**. Tracked separately under "Mail content stores not encrypted at rest" in TODO.md - applies to the body store, inline image store, and attachment cache uniformly.
- **Calendar event attachments**. Modeled here so the orchestration is calendar-ready; actual capture in calendar sync is separate work.
- **Attachment retention policy beyond LRU** ("evict everything older than N days regardless of cache size"). LRU on size cap is enough for v1.

## Implementation phases

Each phase should be commit-sized and independently reviewable.
**Note**: Phases 2 and 3 depend on the Service infrastructure being
far enough along to host the orchestration and serve `attachment.fetch`
IPC requests. Phase 1a is fully library work and can land before any
Service work; Phase 1b can land but stays uncalled until the Service
consumes it.

**Phase 1a - Pack store (`stores` crate)** *(no Service dependency)*
- New module `crates/stores/src/attachment_pack.rs` implementing the pack format described above: append-only segments, frame headers with xxh3_64, pack tail with version + crc, tombstone log.
- New SQLite table `attachment_blobs` (and migration that drops the unused `attachments.local_path / cached_at / cache_size` columns).
- Public API: `PackStore::open(dir, db)`, `put(bytes) -> Hash` (insert-or-ignore semantics, returns hash), `get(&Hash) -> Option<Vec<u8>>`, `unref(&Hash)` (decrements refcount, tombstones if zero), `gc(threshold)`, `recover()` (rebuilds index from pack tails).
- Unit tests: round-trip, dedup (two puts of same bytes append once), refcount increment/decrement, tombstone honored on read, recover from missing index, recover from torn last frame, GC with mixed live/dead frames.

**Phase 1b - Core orchestration** *(no Service dependency yet; consumed by the Service when it lands)*
- New module `crates/core/src/attachments/` with `fetch_or_load`, `prefetch_message`, `prefetch_messages`.
- `fetch_or_load` does inline-image hit -> pack-store hit -> provider fetch -> squeeze -> `PackStore::put` -> update `attachments.content_hash`.
- Unit tests for the hit paths; integration test for the full fetch path (mocked provider) writing into a temp `PackStore`.
- No callers yet - just the building block.

**Phase 2 - Sync wiring (one provider end-to-end)** *(requires Service hosting sync)*
- JMAP first (single blob endpoint, simplest).
- After `persist_messages` writes attachment metadata (in the Service), invoke `prefetch_messages` as a Service-internal task.
- Respect the per-account toggle and size threshold (read from settings, default if unset).
- Progress events through the existing `ProgressReporter`, surfaced to the UI as Service notifications.

**Phase 3 - Open / Save / Save All handlers** *(requires Service exposing `attachment.fetch` IPC)*
- New module `crates/app/src/handlers/attachments.rs` shared between reading pane and pop-out.
- Hot-path: UI looks up `attachment_blobs` and reads positionally from the pack file directly. No IPC.
- Cold-path: UI sends `attachment.fetch` to the Service; Service runs `fetch_or_load`; UI receives bytes (or just `content_hash`, then re-reads from disk).
- Wire the existing Message variants (`OpenAttachment`, `SaveAttachment`, `SaveAllAttachments`) in both surfaces to call into the shared module.
- File dialogs via `rfd`, OS-default open via the platform-specific Command pattern.
- Last-folder-per-thread state.

**Phase 4 - Settings UI**
- Per-account: pre-fetch toggle + threshold (in Account editor).
- Global: cache cap + squeeze toggles + opened-files retention (new "Storage" section in global settings).

**Phase 5 - Port pre-fetch to other providers**
- Gmail, Graph, IMAP. Each is a small change once the core orchestration is in place.

**Phase 6 - Cache eviction + GC**
- `last_read_at` touched on `fetch_or_load` cache hits (in the same transaction as the read).
- Phase 1 LRU eviction (tombstones) triggered after sync batches and on cap pressure.
- Phase 2 GC (pack repack) triggered on app idle when tombstone density crosses threshold.
- Size-aware skip for oversize items.

**Phase 7 - Squeeze measurement**
- Instrument the pipeline to log compression ratios per mime type on a real mailbox.
- Decide whether the defaults should change based on observed savings.

**Out of phases**: calendar attachment capture (separate work), attachment-chip widget unification with cloud links (separate work), search inside attachment text (separate work).

## Verification

End-to-end behavior to test once Phase 1-3 land:

1. Add a JMAP account with a mailbox containing attachments. Sync.
2. After sync settles, `<app-data>/attachment_packs/` should contain at least one `data-NNNNNN.pack` file. `SELECT COUNT(*) FROM attachment_blobs WHERE refcount > 0` should be > 0; `SELECT COUNT(*) FROM attachments WHERE content_hash IS NOT NULL` should match.
3. Disable the network. Open a thread with an attachment. Click Open: file opens in the OS default handler.
4. Click Save: file dialog opens, save, file on disk matches the original bytes (modulo squeeze for compressible formats - decoded content is byte-equivalent).
5. Click Save All on a multi-attachment message: folder picker, all files written.
6. Re-enable the network. Send a copy of the message to a different account. Sync that account. Both accounts share one `attachment_blobs` row: `SELECT refcount FROM attachment_blobs WHERE content_hash = ?` returns >1; pack file size hasn't grown.
7. Delete the account. `attachment_blobs.refcount` decrements for every formerly-referenced blob. Tombstones added for blobs whose refcount hits zero. Pack file sizes unchanged (until next GC).
8. Set cache cap to a value below current usage. Trigger sync. Phase 1 eviction tombstones oldest-by-`last_read_at` entries until live bytes are under cap. Manually run GC; pack files shrink, freed bytes returned to the filesystem.
9. Kill the process mid-write (e.g. SIGKILL during a sync that's appending to a pack). Restart. `PackStore::recover` walks the open pack, truncates any partial trailing frame, and the index is consistent with what's on disk.
10. Delete the SQLite index entirely. Restart with `--rebuild-attachment-index` (or whatever the equivalent CLI knob is). Index is rebuilt from pack tails; no data loss.
