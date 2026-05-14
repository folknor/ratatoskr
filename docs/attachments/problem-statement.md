# Attachments (Cached Storage and File Handling)

## Overview

Email and calendar attachments need to behave like Outlook in Cached Exchange Mode and Thunderbird with "Synchronize messages locally" enabled: **bytes are downloaded during sync and stored on disk so the user can Open, Save, or forward them with no live server connection**. Today, Ratatoskr stores only attachment *metadata* during sync (filename, size, mime type, provider blob id) and never fetches the bytes - meaning offline access doesn't work, and the Open / Save / Save All buttons in the reading pane and pop-out viewer are wired to `log::info!("not yet implemented")` stubs.

This document defines the storage architecture, pre-fetch policy, and file-handling behavior for the unified attachment layer that mail (today) and calendar (later) both depend on.

## Why this matters

The project's stated audience is "enterprise users currently locked into Outlook/Microsoft 365" with "150+ GB cached mailboxes" as a hard requirement. Outlook Cached Exchange Mode pre-fetches and locally caches the entire mailbox including attachments by default - that is the behavior these users expect. Fetch-on-click (the Gmail-web model) is the wrong default for this audience: it breaks offline workflows, it's slow over high-latency links, and it's surprising to users moving from Outlook.

Beyond offline use, a populated local attachment cache also unlocks future work: full-text search inside attachments, attachment dedup across messages (already content-hash-keyed in the schema), and faster Compose forwards (no round-trip to re-fetch the bytes).

## Relationship to the Service

The orchestration described here (pre-fetch, blob-store writes,
eviction, GC) runs inside **the Service** - the subprocess worker
described in `docs/architecture.md` § "Service process model". The
`BlobStore` trait and its implementations are Service-internal; the
UI never reads storage layout directly. Every attachment read - hit
or miss - goes through the `attachment.fetch` IPC, and the Service
is the sole owner of the on-disk format. This keeps storage layout
swappable (flat cache today, `PackStore` tomorrow, anything else
later) without coordinated UI changes. The wire contract is detailed
in `File operations` below.

## Current state

What's on disk today (the state the cleanup phases migrate *away from*, not the v1 target):

- **Schema.** `attachments` (in `crates/db/src/db/schema/02_mail.sql`) has `content_hash BLOB` (BLAKE3 raw bytes via `BlobHash`, landed Phase 1) and joins to `attachment_blobs (content_hash BLOB PK, pack_file_id, offset, length, written_at, last_read_at, tombstoned_at)` (landed Phase 2). The legacy `local_path` / `cached_at` / `cache_size` columns retired with Phase 3.
- **`PackStore` is the live backing store.** `crates/stores/src/attachment_pack.rs` owns content-addressed pack files under `<app_data>/attachment_packs/data-NNNNNN.pack[.open]` + a tombstone log. The Service constructs it during `OpeningBodyAndInlineStores` and stashes it on `BootSharedState`. `crates/service/src/attachment_materialize.rs::materialize_blob` materializes pack frames to `<app_data>/attachment_fetch_tmp/<hash>-<uuid>` per fetch; an idle cleanup kick reaps tmp entries older than 10 minutes. The flat hash-keyed file cache (`crates/stores/src/attachment_cache.rs`) and the LRU `attachment.eviction_kick` retired with Phase 3; the notification variant is retained for Phase 8's date-windowed eviction kick.
- **Provider fetch is ready.** All four providers implement `ProviderOps::fetch_attachment(message_id, attachment_id) -> FetchedAttachment` (raw bytes since Phase 1). Cache miss in `attachment.fetch` runs provider fetch → `BlobHash::hash` → `PackStore::put` → `materialize_blob` → ack. Sync-time pre-fetch is still missing - Phase 4 lands `PrefetchRuntime`.
- **Squeeze is ready.** `squeeze::compress(bytes, mime_type, &Config)` returns a `CompressResult` (or `Unchanged` if not worthwhile - files already small or without compressible content pass through as-is per `crates/squeeze/README.md`). Currently used as a CLI and in inline-image storage. **Not yet wired into the pack-store write path** - Phase 3 deferred the integration to Phase 9 so it lands alongside measurement.

What's missing:

- **No sync-time pre-fetch.** Sync writes attachment metadata rows with `content_hash: None` and stops. Bytes never get fetched until the user clicks the attachment. Phase 4 lands `PrefetchRuntime` to populate PackStore at sync time.
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
// Sketch; final shape settles in the Phase 2 planning session.
pub trait BlobStore: Send + Sync {
    fn put(&self, bytes: &[u8]) -> Result<BlobHash, Error>;
    fn get(&self, hash: &BlobHash) -> Result<Option<Vec<u8>>, Error>;
    fn tombstone(&self, hash: &BlobHash) -> Result<(), Error>;
    fn gc(&self) -> Result<GcStats, Error>;
    fn recover(&self) -> Result<(), Error>;
}
```

`put` is content-addressed and idempotent - two `put`s of identical bytes return the same hash and the second is a no-op on the blob side; reference accounting is derived from `attachments` rows, not tracked in the store. Selection of *which* blobs to tombstone (date-window predicate, orphan detection) lives in the orchestration layer; `BlobStore::tombstone` is just the primitive. The orchestration layer never sees the storage details.

The `get` shape sketched here returns owned bytes, which works for the small-blob common case but allocates the whole blob into memory for large attachments and means materialization writes those same bytes back to disk. Whether to add a `get_reader` or `extract_to(path)` primitive for streaming pack-to-tmp without buffering is a Phase 2 planning question, settled when the trait's final shape lands.

### Storage tiers

Three storage tiers, with the middle tier accessed through `BlobStore`:

1. **Inline image store** (`crates/stores/src/inline_image_store.rs`) - SQLite blob store for images <= `MAX_INLINE_SIZE` (currently 64 KB). Already populated during sync for inline CID images. Stays as-is - SQLite blob storage is fine at this size class and the existing infrastructure works. Not a `BlobStore` impl; lives parallel to it for the small-image case it already serves. **The two stores do not share a content-hash namespace**, so a 65 KB image in packs and a 63 KB copy of the same image in the inline store will not dedup across the boundary. Accepted trade-off: cross-store dedup would require both stores to compute and store BLAKE3 keys, and the inline store's existing xxh3 keying is good enough for its size class. The dedup miss only fires when the same image arrives at sizes straddling 64 KB, which is rare in practice.

2. **Attachment blob store** (new, `BlobStore` impl). v1 implementation: `PackStore` (`crates/stores/src/attachment_pack.rs`) - **content-addressed pack files**, ~256 MB segments under `<app-data>/attachment_packs/data-NNNNNN.pack`. Each blob is appended once and referenced by `(pack_file_id, offset, length)` from a SQLite index table. Multiple attachment rows that hash to the same content reference the same `(pack_file_id, offset, length)` triple. Cross-platform; this is what ships in v1 everywhere.

3. **Provider** - source of truth, fetched on cache miss. Network round-trip.

Phase 3 retired the previous `crates/stores/src/attachment_cache.rs` file-per-blob design in full: the schema columns (`local_path`, `cached_at`, `cache_size`) are dropped, the LRU sweep is gone, and the ExtractRuntime worker reads bytes through `materialize_blob` against PackStore. At our 150-200 GB target with the long tail of small blobs (avatars, signatures, tracking pixels), a flat-file-per-blob layout produces ~2 million inodes with corresponding metadata pressure, terrible backup-tool throughput, and per-blob SSD-page read overhead even for sub-KB content. Pack files reduce inode count by ~3 orders of magnitude, give us sequential write patterns, and let multiple small blobs share an SSD page on read. See "Considered alternatives" below for the full rationale.

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

A separate "tombstones" file (`tombstones-NNNNNN.log`) records `(pack_id, content_hash)` pairs for blobs that have been logically deleted. The log is the durable record used to rebuild `attachment_blobs.tombstoned_at` after index corruption; the column is the runtime authority for reads (see "Tombstone authority" under "Cache eviction and GC").

### SQLite index (PackStore-specific)

Every `BlobStore` implementation owns its own on-disk index format. For `PackStore` v1, the index lives in a new `attachment_blobs` SQLite table:

```sql
CREATE TABLE attachment_blobs (
    content_hash BLOB(32) PRIMARY KEY, -- BLAKE3, raw 32 bytes
    pack_file_id INTEGER NOT NULL,     -- which pack-NNNNNN.pack
    offset INTEGER NOT NULL,           -- byte offset within the pack
    length INTEGER NOT NULL,           -- payload length (post-squeeze)
    written_at INTEGER NOT NULL,
    last_read_at INTEGER,              -- bumped on cache hit; informational only
    tombstoned_at INTEGER              -- NULL = live; non-NULL = logically evicted
);
CREATE INDEX idx_attachment_blobs_tombstoned ON attachment_blobs(tombstoned_at);
```

**Content hash is BLAKE3, not xxh3.** Email attachments are adversarial input - bytes arrive from the public internet, controlled by senders we don't trust. A non-cryptographic hash like xxh3 is vulnerable to second-preimage attacks: an attacker who knows what's in your cache (e.g. because they planted it via an earlier mail) can craft a different blob with the same hash, mail it to you, and have your `INSERT OR IGNORE` silently serve the cached (malicious) bytes on the new attachment's open. BLAKE3 is collision-resistant against this attack and within a small constant factor of xxh3 on throughput (~1 GB/s vs ~30 GB/s on modern x86) - negligible compared to the network fetch the squeeze pipeline already runs. xxh3_64 stays as the frame-payload checksum *inside* pack files (corruption detection, not identity) - the threat models are different.

**Single hash type across the attachment subsystem.** Phase 1 collapsed the three pre-existing representations (xxh3-hex `TEXT` on `attachments.content_hash`, the same on `attachment_extracted_text.content_hash`, SHA-256 `[u8; 32]` on `SendAttachmentSource::StagingFile.content_hash`) onto a single `BlobHash` newtype wrapping `[u8; 32]` BLAKE3 raw bytes, with a hex serde repr for IPC and a `BLOB` SQLite repr. Every attachment-subsystem callsite (compose's hasher, the provider sync writes, every row deserialization in `extract_reindex.rs` and friends) moved to `BlobHash` in lockstep so PackStore was born using the canonical type. The inline image store keeps its own xxh3 keying as a scoped exception (see "Storage tiers" above for rationale) - "single hash type" is scoped to the attachment subsystem, not the whole codebase.

Compose's existing `Sha256` predates this design; whether anything external (logs, IPC consumers, headers) depends on the SHA-256 value is a Phase 1 planning question. This will probably be settled when Phase 1 enters its planning session.

`attachments.content_hash` is the join key (Phase 1 retyped it from `TEXT` xxh3 hex to `BLOB` BLAKE3). Phase 3 dropped `attachments.local_path`, `cached_at`, and `cache_size` - that information now lives on `attachment_blobs`. Per the pre-release migration policy at `crates/db/src/db/migrations.rs:65`, both changes were in-place edits to `schema/02_mail.sql`, not new migration entries.

**Reference counts are derived, not stored.** `attachments` is the source of truth for which blobs are referenced. "How many messages point at this blob?" is `SELECT COUNT(*) FROM attachments WHERE content_hash = ?`; "which blobs are orphans?" is `attachment_blobs LEFT JOIN attachments ON content_hash WHERE attachments.attachment_id IS NULL`. Eviction selection (Finding #1's date predicate) already needs `attachments` joined in for the date check, so the derivation cost is amortized into the same query. Carrying a separate `refcount` column would create a second source of truth that has to track every callsite touching `attachments.content_hash` - account deletion, stale-row recovery, future calendar-attachment plumbing - and a missed increment silently tombstones a still-referenced blob. The derived approach can't drift.

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

If the user later opens or forwards the attachment, the cached bytes are returned as-is. For squeezed content this is application-level equivalent (decodes / renders identically) but not byte-equivalent.

**Signed-content bypass.** Squeeze's PDF and OOXML/ODF paths are byte-changing (lopdf rewrites object streams; ZIP re-pack changes deflate output), so re-packing an e-signed contract or a regulated-industry document would invalidate the signature. `squeeze::compress` sniffs for signature markers before dispatching to a backend and returns `Unchanged` on match. See `crates/squeeze/README.md` § "Signed-content bypass" for the marker list. Detection is biased toward false positives. S/MIME envelopes already pass through as unsupported. Detached signatures (`.asc`, GPG `.sig`) and XAdES-signed SVG / XML are documented limitations.

**Risk: inline squeeze may be net-negative on fast disks.** Phase 9 measures per-mime savings on real mailboxes. If squeeze costs more CPU than it saves in storage on the sync hot path, the pipeline shape changes: PackStore stores raw bytes and a background compaction pass rewrites them later. That is not a small refactor of a built PackStore design, so Phase 2 and Phase 3 should plan with this risk in mind before they freeze the orchestration shape. This will probably be revisited when Phase 9 lands, but the structural option needs to stay open through Phases 2 and 3.

### Content-addressable dedup

The `content_hash` is the primary key on `attachment_blobs`. Insert-or-ignore semantics:

- Hash the bytes after squeeze with BLAKE3 (cryptographic; resists second-preimage attacks against adversarial-input keyspaces).
- `INSERT OR IGNORE INTO attachment_blobs(content_hash, ...) VALUES (?, ...)`. If the row already exists, the bytes are not re-appended to a pack and the call is a no-op on the blob side.
- The new `attachments` row gets `content_hash` populated and is otherwise unchanged. That row is the only reference accounting the blob needs; counts are derived on demand (see SQLite index above).
- Result: a 12 MB company-wide PowerPoint sent to 200 people is appended to a pack exactly once. 200 `attachments` rows share the same `content_hash`. No counter is incremented anywhere.

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
| **EROFS-backed rolling images** (deferred to a follow-up phase, Linux-only) | Read-only kernel filesystem with built-in zstd compression and intra-image dedup. Rolling-image model (~256 MB EROFS images, baked from a small staging area, never modified after) maps cleanly onto our workload. Wins: free compression (no squeeze on the cache side), free intra-image dedup, well-tested format, less per-blob overhead than pack files. Loses: read-only means a bake step instead of streaming appends, granular eviction is less precise (whole-image delete), kernel mount is Linux-only (userspace readers exist but maturity varies on macOS/Windows), and a user who copies their data dir to a non-Linux machine loses access to ErofsStore-backed packs (PackStore-backed packs survive). Lands as a second `BlobStore` impl after v1 ships and we have real cache-pressure data; `cfg(target_os = "linux")` gates which impl is selected. Migration and cross-platform constraints settle in Phase 10 planning, if Phase 10 lands at all. |

## Pre-fetch policy

The cache is populated by **time-windowed retention**, mirroring Outlook's "Mail to keep offline" model. Every attachment on a message whose date falls inside the window is pre-fetched; messages outside the window are fetch-on-click. The window is per-account.

Pre-fetch fires from two triggers:
- **Sync-time** - after `persist_messages` writes attachment metadata for newly-synced messages within the window, the Service enqueues the pre-fetch.
- **Backfill** - on first launch, on account-add, and when the window is extended, the Service walks historical messages whose date now falls inside the window and enqueues missing attachments. This is the core mechanism, not a deferred feature.

Both triggers share the same fetch path; only the source of the message set differs.

**Defaults:**
- **`Mail to keep offline` slider:** 1 month / 3 months / 6 months / 1 year / 2 years / All. Default 1 year (matches Outlook's current default).
- **Retention drives metadata sync depth.** The same `sync_period_days` pref that bounds prefetch also drives `sync_initial`'s walk-back distance. Phase 4 wired the coupling at `crates/service/src/sync_dispatch.rs`: the pref is read once per dispatch (clamped to `>= 1`) and passed to `sync_initial`. Without this coupling, picking '2 years' or 'All' on the slider would be a no-op above 1 year - prefetch backfill can only operate on metadata that exists in the local DB, so the metadata window must expand first. The existing `sync_period_days` key is reused; no new `retention_window_days` was added.
- **Enabled per account on creation** (`cache_attachments: true`).
- **No size thresholds.** Everything inside the window is cached, regardless of per-attachment or per-message size. Time-window *is* the policy.
- **No global cache cap.** If the user picks "All" and has 80 GB of email, the cache holds 80 GB. Disk-space protection is the OS's responsibility, not the cache's.

**Per-account opt-out:** Users can disable caching per account from settings. With caching off, no pre-fetch and no backfill; attachments enter the cache only via fetch-on-click. This matches the Gmail-web model for users who want it.

### Runtime shape

Pre-fetch runs inside a **`PrefetchRuntime`**, a sibling of `ExtractRuntime` (`crates/service/src/extract.rs`). Both triggers (sync, backfill) enqueue onto the same runtime; nothing spawns a detached task. The runtime shape:

- **Two priority queues**, sync-time and backfill, drained sync-first. A multi-year backfill on a heavy mailbox can't starve live sync's prefetch of newly-arriving messages. Each queue is a bounded mpsc (sync capacity 64, backfill capacity 256). Enqueuers `send.await`; backpressure throttles producers when the runtime is saturated.
- **Bounded enqueue dedupe** via a capped FIFO of `(account_id, attachment_id)` pairs at 10K entries (`HashSet` + `VecDeque` kept under one mutex so the cap is enforced without a stop-the-world scan). On overflow the oldest entry is evicted; a re-enqueue then passes - acceptable because `PackStore::put` is content-hash idempotent. The row-level `content_hash IS NULL` check at fetch time is the correctness contract; the set is a perf hint. `message_id` was dropped from the key because `(account_id, attachment_id)` already uniquely identifies the row.
- **Bounded concurrency**: per-account semaphore at 4. No separate global cap. Phase 4 also gates the per-account circuit-breaker check + disk check inside the same permit so outbound load shapes consistently.
- **Per-fetch timeout**: 5 min wallclock per work item.
- **Circuit breaker, per account.** K consecutive timeouts within window W (K=5, W=60s default) open the circuit for that account: queued items skip with `SkipReason::CircuitOpen` rather than fetching; the circuit half-opens after a backoff (start 30s, doubling to a 5 min cap). Reset on any successful fetch. Phase 4 keys on account because JMAP is the only wired provider; Phase 7 may promote the keyspace to provider when cross-provider behaviour differs.
- **ENOSPC safety backstop.** On Unix, before every pack write the runtime checks free disk space via `libc::statvfs`; if free space is below `MIN_DISK_FREE_BYTES` (default 5 GB), the fetch is dropped with a `SkipReason::DiskLow` and the attachment row stays `content_hash IS NULL`. Windows is permissive in Phase 4 (`statvfs_free_bytes` returns None, the cache fills until the OS surfaces ENOSPC as a `PackStore::put` error). `GetDiskFreeSpaceExW` parity is a small follow-up. This is system-protection, not a cache cap - "no global cache cap" means the cache doesn't impose a ceiling on its own size, but it doesn't mean we knowingly fill the user's disk and corrupt SQLite WAL. Whether to surface "cache paused, disk low" or stay silent is a UX call for Phase 6.
- **`CancellationToken` + stored worker `JoinHandle`**, drained on Service shutdown. Per-item tasks are tracked in a `JoinSet` so the worker can abort + await them on cancellation without leaving zombie tokio tasks.
- **Crate-boundary plumbing.** **Resolved as post-sync sweep in Service, not a `PrefetchSink` trait.** `provider-sync` stays prefetch-ignorant: `SyncProviderCtx` got no new field, no new trait was added. `crates/service/src/sync.rs::run_sync` issues a `SELECT attachment_id, message_id, remote_attachment_id, imap_part_id FROM attachments JOIN messages ... WHERE content_hash IS NULL AND date >= ?` after `sync_for_account` returns `Ok`, and enqueues each row on the Sync priority lane (limit 64). The earlier "return-and-dispatch vs sink trait" question (see `implementation-roadmap.md` § Phase 4) dissolved when this third option turned out simpler than both.

**Drain order:** `PrefetchRuntime` joins the Service's fixed drain order between `Sync` and `Extract`. Sync produces prefetch work; prefetch's writes feed extract. The canonical order lives in `docs/architecture.md` § "Service process model".

**Crash recovery:** On Service boot, after `attachments.text_indexed_at`-shaped extract backfill kicks, a parallel **prefetch backfill kick** walks `attachments WHERE content_hash IS NULL AND message.date >= window_start` per JMAP account and re-enqueues on the Backfill priority lane. Idempotent because the row-level `content_hash IS NULL` check rejects already-cached work even if the dedupe set was lost across the restart.

**Account deletion.** Synchronous orphan-blob tombstoning is already wired by Phase 3's `AccountDeletionStep::AttachmentCache` step (`crates/service/src/accounts/delete.rs`): the deletion plan pre-collects every `content_hash` referenced by the account's attachments, identifies which are shared with other accounts, and tombstones the unshared ones via `PackStore::tombstone(hash)` before the CASCADE drops the rows. Phase 4's only addition is `prefetch.cancel_account(account_id)` before `delete_with_marker` runs, so a disappearing account doesn't keep issuing provider fetches. The crash-mid-delete concern from prior drafts is resolved by the resumable `AccountDeletionStep` marker - each step is idempotent and the marker is unlinked only after the final CASCADE succeeds.

**Failure behavior:** Pre-fetch errors are logged but never fail the sync. The attachment row stays `content_hash IS NULL` and gets re-attempted by the next sync, by the boot-time backfill kick, or by user-initiated fetch-on-click.

**Progress reporting:** The existing `ProgressReporter` trait emits "Caching attachments... 42 / 120" events that the status bar already knows how to render. Backfill emits the same shape; the surface is trigger-agnostic.

## Cache eviction and GC

Eviction is driven by the configured retention window, not by access patterns or size pressure. Anything outside the window is eligible for eviction; everything inside is retained.

**Phase 1 - logical eviction (cheap, frequent):**
- Candidates selected by message date: blobs whose every referencing `attachments` row is older than `window_start` (`attachment_blobs LEFT JOIN attachments ON content_hash` filtered by `MAX(date) < window_start`, or blobs with no surviving `attachments` row at all - orphans from account deletion).
- For each candidate, in one SQLite transaction: `UPDATE attachment_blobs SET tombstoned_at = ?now` + append a tombstone to `tombstones-NNNNNN.log`. The bytes are still on disk in the pack file but unreadable.
- Runs at app startup, after every sync batch (to drop newly-aged-out blobs), and whenever the retention window is shortened.

**Tombstone authority.** The `tombstoned_at` column is the runtime authority: every read consults it first via the same SQLite path that resolves the pack location, so there's no read-path cost compared to "is this row present at all?". The `tombstones-NNNNNN.log` file is the durable record used to rebuild the column after index corruption (the `recover()` path replays the log when regenerating `attachment_blobs` from pack tails). On disagreement at runtime, the column wins; on rebuild after corruption, the log wins. The two are always written in the same transaction so divergence implies index damage, which is exactly when the log-wins-on-rebuild rule fires.

**Phase 2 - GC (expensive, rare):**
- Runs on app idle when total tombstoned bytes exceed a threshold (e.g. 25% of any single pack, or 10% of total cache).
- For each pack with high tombstone density: read the pack, copy live frames to a fresh pack at the end of the chain, update `pack_file_id` + `offset` for each surviving blob in the index, atomically delete the old pack file.
- Worst-case cost: read+rewrite of one pack (~256 MB sequential I/O). Phase 8 wires the trigger; the on-idle scheduler hosts it.

**Notes:**

1. **Window shrink** - changing the retention window from 2 years to 6 months triggers a Phase 1 sweep over messages whose dates fall between the old and new edges. Phase 2 follows when tombstone density crosses threshold.
2. **Window extend** - the reverse triggers a backfill (see "Pre-fetch policy"), not an eviction pass.
3. **`last_read_at` is informational.** Not used for eviction (date is the policy); kept as a stat for storage analytics.
4. **GC vs. active writes** - GC reads from immutable closed packs and writes to a fresh pack at the chain tail; it never collides with the open-pack writer.

## File operations: Open, Save, Save All

The three button operations all sit on top of the same `fetch_or_load` orchestration. UI handlers are shared between the reading pane and the pop-out viewer (currently both surfaces have their own stubbed copies). The orchestration runs in the Service; the UI calls into it via IPC.

Every UI read - cache hit or miss - goes through `attachment.fetch { account_id, message_id, attachment_id }`. The Service handler disambiguates hit from miss internally, runs the full pipeline on miss (provider fetch -> squeeze -> `BlobStore::put`), and returns `AttachmentFetchAck { content_hash, size_bytes, relative_path }`. The UI re-opens the file at the returned path and reads positionally. Bytes never cross JSON; on the current flat cache the open fd is the read pin against eviction. The pipeline updates `last_read_at` regardless of path.

The UI does not read `attachment_blobs` directly, link `BlobStore`, or know about pack files or offsets.

**Materialization under `PackStore`.** Today's flat-cache `relative_path` ack points at the on-disk cache file directly. Under `PackStore`, blobs live inside pack files at `(pack_id, offset, length)` - there is no user-readable file at a relative path. The Service bridges this by **per-fetch transient extraction**: on `attachment.fetch`, the handler extracts the requested blob from its pack to `<app_data>/attachment_fetch_tmp/<content_hash>` (write-to-tmp + rename, atomic) and returns that path in the ack. The UI opens the tmp file positionally exactly as it does today. The IPC contract is identical between flat and pack backends; no lease IDs, no `BlobStore::get_with_lease`, no UI-side awareness of which backend is live.

The UI's open fd is the lifetime pin for the tmp file. `unlink` on Linux is fd-safe (the file is removed from the directory but the kernel reclaims its space only when the last fd closes); on Windows the Service opens with `FILE_SHARE_DELETE` so a concurrent unlink doesn't fail the UI's read. The Service runs an on-idle cleanup pass that removes any `attachment_fetch_tmp/*` entry older than 10 minutes - by that point any UI consumer has either finished reading or has the fd open and survives the unlink.

Eviction during read is now race-free without lease counters: tombstoning a blob in `attachment_blobs` and eventual GC of its pack frame is completely independent of any in-flight UI read, because the in-flight read is against a *separate* tmp file. The "open fd is the read pin" story stays true - just against the tmp file, not the pack.

Two consequences:
- The same blob fetched twice in quick succession produces two tmp files. Acceptable: the cleanup pass bounds storage, and tmp-stage dedup would re-introduce the lease lifetime problem we just sidestepped.
- The tmp directory is a real on-disk write per fetch even on cache hit - measurable cost on small frequent fetches. If profiling shows it matters, an in-process zero-copy path (memfd_create on Linux, similar on Windows) can replace the tmp file without changing the UI's contract; that's a post-v1 optimization. Phase 3's ExtractRuntime migration routes every cached-bytes read through this same helper, so the per-fetch tmp cost touches more than just UI reads - the perf budget against pre-PackStore extract is a Phase 3 planning concern.

### Open

1. Send `attachment.fetch` to the Service and await the ack.
2. Read the bytes from the returned `relative_path`, write them to `<app-data>/opened_attachments/<safe_filename>` (NOT `/tmp` - CLAUDE.md forbids `/tmp` use).
3. OS-default open via `xdg-open` (Linux) / `open` (macOS) / `cmd /c start` (Windows). Pattern is already established at `reading_pane.rs:917-925` for link-click handling.
4. Files in `opened_attachments/` are not deleted on close (the OS handler may keep the file open or move it). They get reaped by a periodic cleanup (configurable, default: 7 days).

Filename collisions across messages (two `report.pdf` attachments from different threads opened in sequence) need a uniqueness suffix on Open; Save already has the `(N)` suffix pattern. This will probably be settled when Phase 5 enters its planning session.

**Filename safety:** Strip path separators, control chars, and shell metacharacters. Reuse `sanitize_filename` from `crates/app/src/handlers/pop_out/save_as.rs:38`.

### Save

1. `rfd::AsyncFileDialog::save_file()` with the original filename pre-filled, mime-derived extension filter.
2. Send `attachment.fetch`; read bytes from the returned `relative_path`.
3. Write to the chosen path with `std::fs::write`.
4. Remember the chosen folder per thread (see "Last folder per thread" below).

### Save All

1. `rfd::AsyncFileDialog::pick_folder()`.
2. For each attachment on the message: send `attachment.fetch`, read from the returned path, write to `<chosen_folder>/<safe_filename>`. Filename collisions get a `(N)` suffix.
3. Aggregate progress reported through `ProgressReporter` (notifications from the Service). Errors collected and shown as a single end-of-operation toast (once the toast system lands - until then, log).

### Last folder per thread

Currently a separate TODO entry. Subsumed into this work since it's natural to wire alongside Save / Save All. Storage: small key-value table `attachment_save_paths(thread_id, last_path, updated_at)`. Pre-fills the file dialog's initial directory.

## Settings

The backend for these settings landed in Phase 6 (schema, IPC, PrefetchRuntime gating, cache-size readout). **The settings UI itself is being implemented separately from this roadmap** - the existing settings surface doesn't have the host structure ("Storage" section / Account editor tabs) the original plan assumed, so the widget code lands as a sibling effort once the user has decided where the new section lives.

Backend persistence keys (already wired):

**Per account** (`accounts.cache_attachments_enabled`, default 1):
- `Cache attachments for offline use` (boolean). Drives `PrefetchRuntime`'s per-account gate, the boot recovery kick's account enumeration, and the post-sync sweep's short-circuit.

**Global** (rows in `settings`):
- `sync_period_days` (default 365) - `Mail to keep offline`. Drives both `sync_initial`'s walk-back depth and the prefetch backfill kick's `window_start_unix` filter. On `settings.set` extend, fires `prefetch.kick_backfill_account` for every JMAP account.
- `compress_attachments` (default true) - squeeze pipeline master switch. Read by Phase 9.
- `allow_lossy_compression` (default false) - JPEG re-encoding etc. Read by Phase 9.
- `opened_files_cleanup_days` (default 7) - TTL for `<app_data>/opened_attachments/`. Read by Phase 8's periodic reaper.

Plus a `attachment.cache_size` IPC returning `(live_bytes, tombstoned_bytes)` for the live readout the UI will surface.

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

## Relationship to encryption at rest

Content-addressed dedup and per-account encryption at rest are in tension. The dedup win ("12 MB company-wide PowerPoint sent to 200 people stored once") depends on identical plaintext producing identical bytes on disk. Per-account symmetric encryption with distinct keys produces 200 different ciphertexts from the same plaintext, and dedup goes to zero.

This doc's position: **rely on full-disk encryption** (FileVault, LUKS, BitLocker) for at-rest protection of cached attachment bytes. The "Mail content stores not encrypted at rest" TODO.md item applies uniformly to body store, inline image store, and attachment cache; addressing it for *some* stores via FDE while leaving the dedup-sensitive store unencrypted at the app layer is a coherent stance for a v1 enterprise client, since the target audience already runs managed laptops with FDE policies enforced.

If app-layer encryption of the attachment cache is later required (regulated workflows where FDE alone isn't sufficient), **convergent encryption** is the option that preserves dedup: derive the encryption key from the plaintext content hash itself, so identical plaintext → identical ciphertext → dedup works. The trade-off is loss of semantic security: an attacker who can guess the plaintext (e.g. a known phishing template) can verify whether you have it cached. For attachments delivered over an untrusted network in the first place, this is a defensible trade. Per-account encryption with rotating keys is incompatible with cross-account dedup and would have to be scoped per-account (defeating the headline storage win).

The frame format's "encryption hook" (`nonce | ciphertext | tag` payload region) is preserved as a future option, but the design's expectation is that the attachment cache uses FDE for confidentiality and BLAKE3 content hashes for integrity. Frame xxh3 checksums catch on-disk corruption; BLAKE3 content hashes catch substitution attacks at the index layer.

## Relationship to text extraction

Attachment text extraction and Tantivy indexing ship as a sibling pipeline owned by `docs/architecture.md` § "Text extraction pipeline" - not a phase of this work. Extraction is content-hash-keyed (persisted in `attachment_extracted_text`), so the indexed text survives attachment-cache eviction and carries over unchanged when Phase 3 swaps the flat-file cache for `PackStore`.

## Out of scope (v1)

- **Attachment preview rendering inside Ratatoskr** (PDF viewer, image gallery). Open in OS handler is sufficient for v1.
- **Attachment encryption at rest**. Tracked separately under "Mail content stores not encrypted at rest" in TODO.md - applies to the body store, inline image store, and attachment cache uniformly.
- **Calendar event attachments**. Modeled here so the orchestration is calendar-ready; actual capture in calendar sync is separate work.
## Implementation phases

Detailed plan with entry/exit criteria for each phase in `implementation-roadmap.md`. Each phase is intended as a separate `EnterPlanMode` session.

**Phase 1 - Hash and schema cleanup.** `BlobHash` newtype unifies the hash representations across compose, extract, and the attachment cache. Schema retypes: `attachments.content_hash` and `attachment_extracted_text.content_hash` to `BLOB(32)` BLAKE3. `gmail_attachment_id` becomes `remote_attachment_id`. `ProviderOps::fetch_attachment` returns raw bytes; Gmail decodes its base64 internally. No PackStore, no disk-layout change.

**Phase 2 - PackStore library.** `crates/stores/src/attachment_pack.rs` with put/get/tombstone/gc/recover, plus the new `attachment_blobs` table. Library-only - no Service lifecycle, no producers, no consumers. Unit tests + library benchmark.

**Phase 3 - Service integration and ExtractRuntime migration.** `attachment.fetch` rewired through a Service-internal `materialize_blob` helper (per-fetch transient extraction to `attachment_fetch_tmp/<content_hash>-<request_id>`). ExtractRuntime reads bytes via the same helper. Flat cache retired: `attachment_cache.rs` deleted, `local_path/cached_at/cache_size` dropped, lease forward-references in `service-api` + `architecture.md` retracted. Boot lifecycle, drain wiring, and crash recovery land here.

**Phase 4 - PrefetchRuntime and JMAP trigger.** New `crates/service/src/prefetch.rs` sibling of `extract.rs`. Provider-sync stays prefetch-ignorant: instead of a `PrefetchSink` trait, `run_sync` issues a post-sync DB sweep that enqueues NULL-hash rows on the Sync priority lane. `sync_initial`'s hardcoded 365-day window replaced by reading the existing `sync_period_days` pref so the slider is meaningful above 1 year. Backfill driver for first-launch (boot recovery kick covers it); account-add and window-extend covered by the post-sync sweep and the next boot's recovery kick respectively (explicit Phase-6 kick on slider write). JMAP only. See `implementation-roadmap.md` § Phase 4 for the deferral ledger.

**Phase 5 - UI: Open, Save, Save All.** Reading-pane stubs hoisted from `ReadingPaneMessage` into `ReadingPaneEvent` variants handled in `handlers/core.rs` where the dispatch surface lives. Pop-out wires directly. Shared `crates/app/src/handlers/attachments.rs` module. `rfd` dialogs, OS-default open, last-folder-per-thread persistence.

**Phase 6 - Settings (backend slice landed; UI is the user's separate work).** Backend plumbing every setting will eventually invoke: `accounts.cache_attachments_enabled` column, `sync_period_days` / `compress_attachments` / `allow_lossy_compression` / `opened_files_cleanup_days` setting keys, `AccountUpdateParams` patch, `attachment.cache_size` IPC, `PrefetchRuntime` per-account gate, `settings.set` window-extend kick. The settings UI lands as a sibling effort once the user has decided where the new section lives in the existing settings surface (the original "Storage tab on Account editor" plan didn't fit the actual UI). See `implementation-roadmap.md` § Phase 6 for the deferral ledger.

**Phase 7 - Provider parity.** Gmail / Graph / IMAP wired into the same enqueue mechanism. IMAP session reuse.

**Phase 8 - Eviction, GC, opened-files cleanup.** Date-based tombstoning at startup / post-sync / window-shrink. Pack-repack GC on app idle when tombstone density crosses threshold. Periodic reap of `opened_attachments/`. "Clear cache now" button.

**Phase 9 - Squeeze measurement.** Instrument the pipeline; per-mime savings report; calibrate defaults.

**Phase 10 - Linux `ErofsStore` backend (optional).** Second `BlobStore` impl. Lands only if `PackStore` measurements warrant it.

**Out of phases**: calendar attachment capture, attachment-chip widget unification with cloud links, search inside attachment text, at-rest encryption, backfill UI (each its own separate problem statement).

## Verification

End-to-end behavior to test once Phases 1-5 land (retention shrink in scenario 8 requires Phase 8):

1. Add a JMAP account with a mailbox containing attachments. Sync.
2. After sync settles, `<app-data>/attachment_packs/` should contain at least one `data-NNNNNN.pack` file. `SELECT COUNT(*) FROM attachment_blobs WHERE tombstoned_at IS NULL` should be > 0; `SELECT COUNT(*) FROM attachments WHERE content_hash IS NOT NULL` should match.
3. Disable the network. Open a thread with an attachment. Click Open: file opens in the OS default handler.
4. Click Save: file dialog opens, save, file on disk matches the original bytes (modulo squeeze for compressible formats - decoded content is byte-equivalent; signed PDFs / OOXML / ODF are byte-identical thanks to the signed-content bypass in squeeze).
5. Click Save All on a multi-attachment message: folder picker, all files written.
6. Re-enable the network. Send a copy of the message to a different account. Sync that account. Both accounts share one `attachment_blobs` row: `SELECT COUNT(*) FROM attachments WHERE content_hash = ?` returns >1 and the pack file size hasn't grown.
7. Delete the account. The corresponding `attachments` rows are removed; `attachment_blobs` rows that no longer have any matching `attachments` row become orphans and are tombstoned by the next eviction sweep. Pack file sizes unchanged (until next GC).
8. Shorten the retention window from 1 year to 1 month. Phase 1 eviction tombstones blobs whose every referencing message's date is older than the new edge. Manually run GC; pack files shrink, freed bytes returned to the filesystem.
9. Kill the process mid-write (e.g. SIGKILL during a sync that's appending to a pack). Restart. `PackStore::recover` walks the open pack, truncates any partial trailing frame, and the index is consistent with what's on disk.
10. Drop the `attachment_blobs` table (e.g. via the dev-seed wipe path or a deliberate corruption test). Restart with `--rebuild-attachment-index` (or whatever the equivalent CLI knob is). Index is rebuilt from pack tails; tombstone log is replayed; `attachments.content_hash` survives in the main DB so the references still resolve. No data loss.
