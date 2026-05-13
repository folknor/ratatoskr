# Attachments - Implementation Roadmap

Companion to `problem-statement.md`. Each phase below is intended as a **separate `EnterPlanMode` session** that produces a focused implementation plan, lands as one or a small handful of commits, and unblocks the next phase. Nothing here is a complete plan in itself - the goal is to chart the order of attack and keep us from accidentally building things in the wrong sequence.

This document is a sketch. Phase scope, interfaces, and risks will firm up when each phase enters its own planning session.

**Cross-document dependency.** Phases 3 onward run inside **the Service** (`docs/architecture.md` § "Service process model"); Phase 3 owns the `attachment.fetch` IPC defined in `crates/service-api/src/attachment.rs`. Phases 1 and 2 are pure library + schema work with no Service lifecycle wiring.

## How to read this

- **Goal** - one-sentence outcome.
- **Entry criteria** - what must already exist for this phase to start cleanly.
- **In scope / Out of scope** - hard boundaries for the phase.
- **Touchpoints** - files / modules likely to change. Indicative, not exhaustive.
- **Exit criteria** - observable evidence the phase is complete.
- **Risks / open questions** - unknowns to resolve during the planning session.

---

## Phase 1 - Hash and schema cleanup

**Goal.** Land the type plumbing PackStore will be born using: a single `BlobHash` newtype with one algorithm and one SQLite representation, schema columns renamed/retyped for the new world, and provider attachment fetch returning raw bytes. No PackStore yet, no disk-layout change.

**Entry criteria.**
- Problem statement approved.
- BLAKE3-vs-BLOB(32) decision made (current proposal: BLAKE3, raw 32 bytes in SQLite as `BLOB(32)`, hex string for IPC serde).
- `remote_attachment_id` scope decided (rename-only vs. consolidation - see open questions).

**In scope.**

*Type plumbing:*
- New `BlobHash` newtype wrapping `[u8; 32]`. One algorithm (BLAKE3). One SQLite representation (`BLOB(32)` via `ToSql`/`FromSql`). One IPC serde representation (hex string). Methods: `from_bytes`, `from_hex`, `to_hex`, `as_bytes`. `Display`/`Debug` as hex.
- Replace `attachments.content_hash` callsites (currently `Option<String>` xxh3 hex) with `Option<BlobHash>`.
- Replace `attachment_extracted_text.content_hash` similarly.
- Replace `SendAttachmentSource::StagingFile.content_hash: [u8; 32]` (currently SHA-256) with `BlobHash`. Compose's hasher at `crates/app/src/handlers/pop_out/compose_send.rs:259` swaps from `Sha256` to BLAKE3.

*Schema:*
- Per pre-release migration policy (`crates/db/src/db/migrations.rs:65`), edit `crates/db/src/db/schema/02_mail.sql` in place. No new migration entry.
- `attachments.content_hash TEXT` -> `BLOB(32)`.
- `attachment_extracted_text.content_hash` retyped to match.
- `attachments.gmail_attachment_id` -> `remote_attachment_id` (provider-agnostic name). Open question for the phase planning session: collapse `imap_part_id` into the same column with a discriminator, or keep it separate because it carries structural part info?
- Flat-cache columns (`local_path`, `cached_at`, `cache_size`) and `attachment_cache_max_mb` pref stay for now - they still back the live cache and ExtractRuntime; retired in Phase 3.

*Provider interface:*
- `ProviderOps::fetch_attachment` returns raw bytes (`Vec<u8>` or a small `FetchedAttachment { bytes: Vec<u8>, size: u64 }` struct). Gmail decodes its base64-url payload inside the Gmail provider crate; other providers stop carrying the base64 round-trip.
- `store::attachment_cache::AttachmentData` retires its `data: String` field. The `decode_base64` call in `crates/service/src/handlers/attachment.rs:173` goes away.
- Existing flat-cache write path keeps working - it just hashes incoming bytes with BLAKE3 instead of xxh3 and stores them under the new hash. Dev-seed wipes the data dir on every launch so there are no stale xxh3-keyed cache files in the wild.

**Out of scope.**
- PackStore itself (Phase 2).
- Dropping flat-cache columns or files (Phase 3).
- `attachment_blobs` table (Phase 2).
- Any change to disk layout under `attachment_cache/`. The flat cache continues to operate; this phase only retypes columns the cache populates and renames the algorithm under it.

**Touchpoints.**
- New: `BlobHash` newtype (location TBD - either `crates/common/src/` or a new tiny crate).
- `crates/db/src/db/schema/02_mail.sql` - column retypes, `remote_attachment_id` rename.
- `crates/common/src/types.rs` - `AttachmentData` shape change.
- `crates/common/src/ops.rs` - `ProviderOps::fetch_attachment` signature.
- `crates/gmail/src/ops.rs`, `crates/graph/src/ops/mod.rs`, `crates/imap/src/ops.rs`, `crates/jmap/src/ops.rs` - provider impls return raw bytes.
- `crates/service/src/handlers/attachment.rs` - drop `decode_base64`.
- `crates/service-api/src/action.rs` - `SendAttachmentSource::StagingFile.content_hash` -> `BlobHash`.
- `crates/app/src/handlers/pop_out/compose_send.rs` - swap `Sha256` for BLAKE3.
- `crates/db/src/db/queries_extra/extract_reindex.rs` - row deserialization updated for the new column type.
- `crates/stores/src/attachment_cache.rs` - `hash_bytes` swaps xxh3 for BLAKE3; return type changes to `BlobHash`.

**Exit criteria.**
- `brokkr check` clean.
- One algorithm, one type, one representation across compose, extract, and the (still flat-file) attachment cache.
- The existing extract pipeline still works end-to-end on real synced attachments under the new hash.
- `attachment.fetch` still returns the same `AttachmentFetchAck` wire shape; the only behavioral change is that new hashes are BLAKE3 (encoded the same way over the wire).

**Risks / open questions.**
- `remote_attachment_id` scope: rename-only, or fold `imap_part_id` in too with a discriminator? Decide before touching the schema.
- BlobHash crate location: `common` (already a wide dep) vs. a new tiny `blob-hash` crate to avoid pulling `common`'s deps into compose / staging. Decide based on what consumers actually need.
- Whether the IPC serde repr is hex or base64. Hex matches the current `AttachmentFetchAck.content_hash: String` shape - stick with hex unless there's a reason not to.

---

## Phase 2 - PackStore library

**Goal.** A self-contained pack-file blob store under `crates/stores/src/attachment_pack.rs` with content-addressed `put` / `get` / `tombstone` / `gc` / `recover`, a SQLite index, and full crash safety. **Library only.** No Service lifecycle wiring, no producers, no consumers.

**Entry criteria.**
- Phase 1 landed. `BlobHash` exists; schema uses `BLOB(32)` BLAKE3.
- Pack file format finalized (frame layout, tail layout, version byte semantics, tombstone log format).

**In scope.**
- `crates/stores/src/attachment_pack.rs` with `PackStore::open`, `put(bytes) -> BlobHash`, `get(&BlobHash) -> Option<Vec<u8>>`, `tombstone(&BlobHash)`, `gc(threshold) -> GcStats`, `recover()`.
- New SQLite table `attachment_blobs(content_hash BLOB(32) PRIMARY KEY, pack_file_id INTEGER, offset INTEGER, length INTEGER, written_at INTEGER, last_read_at INTEGER, tombstoned_at INTEGER)` - added in `crates/db/src/db/schema/02_mail.sql`. No `refcount` column - counts derive from `attachments` at query time.
- Frame writer: 4-byte magic + 4-byte length + 8-byte xxh3_64 of payload + payload, batched fsync, atomic index update via SQLite transaction.
- Pack tail writer with version + frame_count + crc.
- Tombstone log (`tombstones-NNNNNN.log`) and read-side enforcement (refuse to return tombstoned data even on a stale index hit).
- Pack rotation when `PACK_TARGET_SIZE` (256 MB default) exceeded.
- One-writer-per-pack mutex; lock-free positional reads.

**Out of scope.**
- Anyone calling the store (Phase 3 is the first consumer).
- Service boot/drain wiring (Phase 3).
- Encryption (deferred - tracked under "Mail content stores not encrypted at rest" in TODO.md).
- Restic format compatibility (deferred to v2).
- Chunking large blobs (out of v1 unless squeeze measurement says otherwise).

**Touchpoints.**
- New: `crates/stores/src/attachment_pack.rs`, `crates/stores/src/lib.rs` (re-export).
- `crates/db/src/db/schema/02_mail.sql` - new `attachment_blobs` table.

**Exit criteria.**
- `brokkr check` clean.
- Unit tests cover: round-trip, dedup (two `put`s of identical bytes append once), tombstone honored on read, recover from missing index, recover from torn last frame, GC on mixed live/dead pack.
- A small library-level benchmark exists (insert 10k 4 KB blobs + 1k 1 MB blobs, time per op, total disk usage, inode count) for future regression checks.
- `crates/stores/src/attachment_cache.rs` is **untouched** - it still backs the live attachment cache.

**Risks / open questions.**
- Whether to put the open pack in its own file or always rotate on startup. Trade-off: in-place open-pack means recovery has to scan; always-rotate means more pack files. Probably in-place; rotation only on size cap.
- fsync batching cadence. Too aggressive = slow writes; too lax = larger crash recovery scan. Probably "every 16 frames or 100 ms, whichever first."

---

## Phase 3 - Service integration and ExtractRuntime migration

**Goal.** `attachment.fetch` reads through PackStore via per-fetch transient extraction. ExtractRuntime reads bytes through the same materialization helper. The flat cache is retired.

**Entry criteria.**
- Phase 2 landed. `PackStore` is library-tested and dormant.

**In scope.**

*Materialization helper:*
- Service-internal helper `materialize_blob(content_hash) -> path`. Extracts the blob from PackStore to `<app_data>/attachment_fetch_tmp/<content_hash>-<request_id>` (write-to-tmp + rename, atomic). The unique suffix lets duplicate concurrent fetches produce independent tmp files without colliding on rename. Returns the relative path.
- Idle cleanup pass removes `attachment_fetch_tmp/*` older than 10 minutes. UI's open fd is the lifetime pin: `unlink` is fd-safe on Linux; the Service opens with `FILE_SHARE_DELETE` on Windows.

*`attachment.fetch` rewrite:*
- Handler routes through `materialize_blob` for both hit and miss paths. On miss, the full pipeline runs: provider `fetch_attachment` -> squeeze (with signed-content bypass) -> `PackStore::put` -> `materialize_blob` -> ack.
- Wire shape: `AttachmentFetchAck { content_hash, size_bytes, relative_path }` unchanged.
- The lease-design forward references in `crates/service-api/src/attachment.rs` and `docs/architecture.md` § "Forward reference: pack-aware attachment reads" are retracted: per-fetch transient extraction replaces leases entirely.

*Boot lifecycle:*
- `PackStore::open` runs after `BootPhase::SchemaMigrations` (before the cross-store invariant pass; the pack store's own open-time recovery covers what the invariant pass would have done for the old flat cache).
- On unclean shutdown (missing sentinel), `PackStore::open` walks the unique `.open` pack and truncates any torn trailing frame *before* `boot.ready`. Tombstone-log replay is deferred to Phase 8's `--rebuild-attachment-index` tool - the SQLite index is the runtime authority for `tombstoned_at`.
- No explicit `PackStore::flush` in the clean-shutdown drain. `PackStore::put` fsyncs every frame + commits the matching index row in one SQLite transaction, so any blob whose `put` returned is already durable. The drain's subsystem teardown stops every `put` caller before the sentinel write, leaving nothing to flush.

*Drain wiring:*
- GC is on-idle; cancels on shutdown signal. No separate drain slot (the on-idle scheduler hosts it).
- Tombstone writes flow through whatever subsystem owns the eviction selector and inherit that slot's drain ordering.

*ExtractRuntime migration:*
- `crates/service/src/extract.rs:564-567` currently reads `app_data.join("attachment_cache").join(&work.content_hash)` directly. Replace with a call into `materialize_blob`; read bytes from the returned tmp path.
- `crates/db/src/db/queries_extra/extract_reindex.rs:198 find_unindexed_cached_attachments` is rewritten: the predicate moves from `WHERE cached_at IS NOT NULL AND text_indexed_at IS NULL` to a join against `attachment_blobs` (`attachment_blobs.tombstoned_at IS NULL AND attachments.text_indexed_at IS NULL`).
- The partial index `idx_attachments_text_indexed_at` in `02_mail.sql:166` is rewritten without the `WHERE cached_at IS NOT NULL` predicate (replace with a condition that fits the new schema, or move the index onto a join expression).
- Extract harness scripts updated to seed pack-store entries instead of flat-cache files.

*Flat-cache retirement:*
- `crates/stores/src/attachment_cache.rs` deleted (or reduced to whatever the inline-image-store path still needs; the flat hash-keyed file cache goes).
- Schema columns `attachments.local_path`, `cached_at`, `cache_size` dropped in `crates/db/src/db/schema/02_mail.sql`.
- The `attachment_cache_max_mb` preference and `enforce_cache_limit` LRU sweep retire here - the new time-windowed policy lands in Phase 8.

*Harness:*
- New scripts under `crates/app/tests/service-harness/`: SIGKILL mid-write (partial trailing frame, boot recovers), SIGKILL mid-repack (partial new pack, boot discards), boot with deleted SQLite index (`--rebuild-attachment-index` walks pack tails and regenerates), boot with sentinel missing (invariant pass runs, no data loss).

**Out of scope.**
- Pre-fetch (Phase 4).
- UI buttons (Phase 5).
- New eviction policy (Phase 8) - this phase only retires the old LRU code; eviction is dormant until Phase 8.

**Touchpoints.**
- `crates/service/src/handlers/attachment.rs` - rewritten around `materialize_blob`.
- `crates/service/src/extract.rs` - reads through `materialize_blob`.
- `crates/db/src/db/queries_extra/extract_reindex.rs` - predicate rewrite.
- `crates/db/src/db/schema/02_mail.sql` - drop flat-cache columns, adjust partial index.
- `crates/stores/src/attachment_cache.rs` - retired.
- `crates/service-api/src/attachment.rs` - module docs: retract lease forward reference.
- `docs/architecture.md` § "Forward reference: pack-aware attachment reads" - rewrite for transient extraction.
- `crates/service/src/dispatch/init.rs` and `crates/service/src/boot.rs` - `PackStore::open` call site, invariant pass, drain wiring.
- `crates/app/tests/service-harness/` - new harness scripts named above.

**Exit criteria.**
- `brokkr check` clean.
- `attachment.fetch` returns paths under `attachment_fetch_tmp/`, not `attachment_cache/`.
- ExtractRuntime processes attachments by going through `materialize_blob`. No code path under `crates/` reads `attachment_cache/<hash>` directly.
- `attachments.local_path` is gone from the schema and the build still compiles.
- Harness scripts pass under `brokkr service-suite` and preserve artefact dirs on intentional failure.

**Risks / open questions.**
- ExtractRuntime backfill query: `cached_at IS NOT NULL` was a cheap "bytes exist" proxy. The replacement needs a query against the same shape that doesn't blow up on initial boot when most `attachments` rows have no `content_hash` yet. Probably an inner-join against `attachment_blobs` with `tombstoned_at IS NULL`.
- Per-fetch tmp-file cost: every cache hit is now a tmp-file write. If profiling shows this matters, an in-process zero-copy path (`memfd_create` on Linux, similar on Windows) can replace tmp files without changing the wire contract - track as a phase-after-3 optimization.
- GC-during-shutdown ordering. GC reads from immutable closed packs and writes to a fresh pack at the chain tail. A shutdown signal during an in-flight repack must either complete (slow shutdown) or discard the partial new pack (fast shutdown, GC re-runs next idle). Pick the policy as part of drain integration.

---

## Phase 4 - PrefetchRuntime and JMAP trigger

**Goal.** JMAP attachments inside the configured retention window are cached, via sync-time pre-fetch for new messages and first-launch / account-add / window-extend backfill for historical ones.

**Entry criteria.**
- Phase 3 landed. PackStore is the live backing store; `materialize_blob` is the only path to bytes.

**In scope.**

*PrefetchRuntime:*
- New `crates/service/src/prefetch.rs`, sibling of `crates/service/src/extract.rs`.
- Two priority queues (sync-time capacity 64, backfill capacity 256, sync drained first so backfill can't starve live work), per-account semaphore at 4, bounded `Arc<Mutex<HashSet>>` enqueue dedupe (cap 10K, oldest-drop policy), `CancellationToken` + stored `JoinHandle`, 5 min per-fetch wallclock timeout, per-provider circuit breaker (K=5 consecutive timeouts in W=60s opens the circuit, exponential backoff 30s -> 5 min cap), ENOSPC safety backstop (skip the write + log warning if `statvfs` reports free below `min_disk_free_gb`, default 5 GB).

*Sync hook (crate boundary):*
- `provider-sync` cannot depend on `service` (cycle). Add a `PrefetchSink` trait carried on `SyncProviderCtx` in `crates/provider-sync/src/lib.rs:50`. The Service-side `sync_dispatch.rs` installs a sink that enqueues onto `PrefetchRuntime`; tests install a no-op sink. Providers call `ctx.prefetch.enqueue(...)` after persisting attachment metadata for in-window messages.
- Alternative shape: providers don't get a sink; they return persisted attachment IDs from `sync_initial` / `sync_delta` and `sync_dispatch` enqueues. Decide in the planning session - the trait shape keeps the enqueue close to the data; return-and-dispatch keeps providers ignorant of prefetch entirely.

*Retention-depth coupling:*
- `crates/service/src/sync_dispatch.rs:72` currently hardcodes `sync_initial(&ctx, 365)`. Replace with a read from prefs (`retention_window_days`, default 365). Without this, the slider in Phase 6 is a no-op above 1 year - prefetch backfill can only operate on metadata that exists, and the slider would silently bound caching to whatever days `sync_initial` walked back.
- The 1-month / 3-month / 6-month / 1-year / 2-year / "All" slider buckets resolve to day budgets (or `i64::MAX` for "All"). The window value drives both `sync_initial` depth and the prefetch backfill query.

*Backfill driver:*
- Service-side driver (likely folded into `prefetch.rs`) walks historical messages whose date falls inside the window and enqueues missing attachments. Triggers: first launch, account-add, window-extend. Same fetch path as sync-time; lower-priority queue.

*Account deletion:*
- Synchronous tombstoning. When an account is deleted, walk `attachments WHERE account_id = ?` in one transaction, drop those rows, then tombstone every `attachment_blobs` row whose `content_hash` no longer has any surviving `attachments` references. Privacy contract met before account-delete returns; disk reclamation is async (Phase 8 GC).

*Boot recovery kick:*
- On Service boot, walk `attachments WHERE content_hash IS NULL AND message.date >= window_start` per account and re-enqueue. Idempotent.

*Drain order:*
- `PrefetchRuntime` joins the Service's fixed drain order between `Sync` and `Extract`. Update `docs/architecture.md` § "Service process model" in lockstep.

**Out of scope.**
- Other providers (Phase 7).
- Settings UI (Phase 6) - this phase reads raw values from the prefs table with defaults if unset.

**Touchpoints.**
- New: `crates/service/src/prefetch.rs`.
- `crates/provider-sync/src/lib.rs` - `SyncProviderCtx::prefetch: &dyn PrefetchSink` (or the return-and-dispatch shape).
- `crates/jmap/src/sync/...` - call `ctx.prefetch.enqueue(...)` after attachment metadata persists (or return the persisted IDs).
- `crates/service/src/sync_dispatch.rs` - install the real sink; read retention window from prefs; replace the hardcoded 365.
- `crates/service/src/dispatch/...` - boot wiring + drain-order insertion.
- `docs/architecture.md` § "Service process model" - drain-order update.
- `crates/db/src/db/queries.rs` or a dedicated prefs helper - read pre-fetch policy.

**Exit criteria.**
- After a JMAP sync, `SELECT COUNT(*) FROM attachment_blobs WHERE tombstoned_at IS NULL` grows for in-window messages.
- After account-add or window-extend, backfill runs and the count grows over historical in-window messages.
- A retention setting of "2 years" causes `sync_initial` to walk back 2 years of metadata, and prefetch covers those messages.
- Service shutdown during active prefetch drains cleanly; the next boot's recovery kick resumes from `content_hash IS NULL`.
- Pre-fetch failures don't break sync; a retried sync re-attempts the same attachments.

**Risks / open questions.**
- Sink trait vs return-and-dispatch shape. The trait keeps enqueue at the data site; return-and-dispatch keeps providers ignorant of prefetch. Pick in planning.
- Sync completion timing: the existing "sync complete" UI fires when metadata lands. Decide whether `PrefetchRuntime` queue depth should surface as its own progress separate from sync state.
- A multi-year retention window on a heavy mailbox is a real metadata-sync expansion, not just an attachment-bytes expansion. The planning session should size this and decide whether "All" needs a confirmation or progress-aware UX.
- Circuit-breaker calibration (K=5, W=60s, backoff 30s -> 5 min) is a starting point. Real provider misbehavior may want different thresholds; Phase 7 cross-provider parity is the natural place to revisit.

---

## Phase 5 - UI: Open, Save, Save All

**Goal.** The three buttons in the reading pane and pop-out viewer actually work, online or offline.

**Entry criteria.**
- Phase 3 landed. `attachment.fetch` works end-to-end through PackStore.
- Phase 4 *helpful but not required* - this phase works in fetch-on-click mode if no pre-fetch has happened.

**In scope.**

*Event hoist (reading pane):*
- The stubs at `crates/app/src/ui/reading_pane.rs:368-376` live inside the component's `update` fn, which has no `ServiceClient` or `rfd` context. Add `ReadingPaneEvent::{OpenAttachment, SaveAttachment, SaveAllAttachments}` variants and emit them from the component; handle them in `crates/app/src/handlers/core.rs:186 handle_reading_pane_event` where the dispatch surface lives.
- Pop-out (`crates/app/src/handlers/pop_out/message_view.rs:69/73/77`) is structurally easier - its dispatcher already has the handles. Wire directly.

*Shared handler module:*
- New `crates/app/src/handlers/attachments.rs` with `handle_open_attachment`, `handle_save_attachment`, `handle_save_all_attachments`. Both surfaces dispatch into it.

*Behavior:*
- All reads go through `attachment.fetch`. Ack `AttachmentFetchAck { content_hash, size_bytes, relative_path }` is reopened positionally; bytes never cross JSON.
- Open: read bytes from `relative_path`, write to `<app_data>/opened_attachments/<safe_filename>` (not `/tmp`), shell out to OS handler via the platform Command pattern at `reading_pane.rs:917`.
- Save: `rfd::AsyncFileDialog::save_file()` with original filename pre-filled, write via `std::fs::write`.
- Save All: `rfd::AsyncFileDialog::pick_folder()`, write each attachment with `(N)` collision suffix.
- Filename sanitization reuses `crates/app/src/handlers/pop_out/save_as.rs::sanitize_filename`.
- Last-folder-per-thread persisted in a new `attachment_save_paths(thread_id, last_path, updated_at)` table.

**Out of scope.**
- Periodic cleanup of `opened_attachments/` (Phase 8).
- Toast-based error reporting (blocked on the toast system in TODO.md). Errors logged for v1.

**Touchpoints.**
- New: `crates/app/src/handlers/attachments.rs`, `attachment_save_paths` table in `crates/db/src/db/schema/02_mail.sql`, related queries in `crates/db/src/db/queries_extra/...`.
- `crates/app/src/ui/reading_pane.rs` - replace stub branches with `ReadingPaneEvent` emissions.
- `crates/app/src/handlers/core.rs` - new `handle_reading_pane_event` arms dispatching into the shared handler module.
- `crates/app/src/handlers/pop_out/message_view.rs` - dispatch into the shared handler module.

**Exit criteria.**
- Click Open -> file opens in the OS default handler.
- Click Save -> file dialog -> file written, byte-equivalent at the application level (signed content stays byte-identical via the squeeze bypass).
- Click Save All -> folder picker -> all files written; collisions get `(N)` suffix.
- After Save, the next Save on the same thread pre-fills the previous folder.
- All three work with network disabled if the cache is populated.

**Risks / open questions.**
- Save All on a multi-attachment offline message with mixed cached/uncached state. Probably: report which ones failed, write the ones that succeeded.
- Windows / macOS OS-default-open quirks (UAC, quarantine attribute, GateKeeper).

---

## Phase 6 - Settings UI

**Goal.** Users can configure caching, retention window, and squeeze policy from settings.

**Entry criteria.**
- Phases 1-5 landed. Pre-fetch and Open/Save read their config from prefs - settings UI just wraps the prefs.

**In scope.**

Per-account section ("Storage" tab on the Account editor):
- `Cache attachments for offline use` (toggle, default true).
- `Mail to keep offline` (Outlook-style slider: 1 month / 3 months / 6 months / 1 year / 2 years / All; default 1 year). Writes to the `retention_window_days` pref that Phase 4 already reads.

Global settings, new "Storage" section:
- `Compress cached attachments` (toggle, default on).
- `Allow lossy compression (JPEG re-encoding)` (toggle, default off).
- `Cleanup opened-files temp folder after N days` (slider, default 7).
- Live "Cache currently using X.Y GB" readout (informational).

**Out of scope.**
- A "Clear cache now" button (Phase 8).

**Touchpoints.**
- `crates/app/src/ui/settings/tabs/...` - new section.
- `crates/app/src/ui/settings/types/...` - new pref keys + bootstrap snapshots; **retire the dead `attachment_cache_max_mb` field on `PreferencesState`** (Phase 3 left it in place to keep the wire stable - this phase replaces it with the retention slider).
- `crates/db/src/db/queries.rs` - getters/setters for the new prefs.

**Exit criteria.**
- Settings persist across restarts.
- Shortening the retention window triggers a Phase 8 eviction sweep; extending it triggers a Phase 4 backfill (including the metadata-depth extend if applicable).
- Disabling caching on an account stops new attachments from being downloaded by sync or backfill (fetch-on-click still works).

**Risks / open questions.**
- Where does the per-account toggle live? On the Account editor sheet, matching how other per-account settings work.
- "All" on a heavy mailbox is a real metadata-sync expansion. Surface a progress-aware UX or a confirmation step.

---

## Phase 7 - Provider parity (Gmail, Graph, IMAP)

**Goal.** Pre-fetch parity across all four mail providers.

**Entry criteria.**
- Phase 4 landed for JMAP. The PrefetchSink (or return-and-dispatch) pattern is proven.

**In scope.**
- Wire the same enqueue mechanism in:
  - `crates/gmail/src/sync/...`
  - `crates/graph/src/sync/...`
  - `crates/imap/src/sync_pipeline.rs` (or wherever the post-persist hook is)
- IMAP-specific: respect per-folder session reuse so we don't open a new connection per attachment.
- Per-provider concurrency limits (4 for Gmail/Graph, 1 per folder for IMAP).

**Out of scope.**
- IMAP partial-fetch optimization (`BODY[part]<offset.length>`).
- Reference-attachment handling on Graph - URL-only, not bytes; surface as cloud links via the cloud-attachments path.

**Touchpoints.**
- The three sync paths above.
- Possibly `crates/imap/src/client/sessions.rs` for session reuse on attachment fetches inside the same folder.

**Exit criteria.**
- After a sync on each provider type, `attachment_blobs` rows with `tombstoned_at IS NULL` appear for that account.
- IMAP attachment fetches reuse the existing folder session (no extra LOGIN/SELECT round-trips).

**Risks / open questions.**
- Gmail batch endpoint vs N individual fetches.
- IMAP servers with strict concurrency limits.

---

## Phase 8 - Eviction, GC, opened-files cleanup

**Goal.** Cache stays within the configured retention window. Logical eviction (tombstones) and physical GC (pack repack) are wired. The opened-files temp folder gets reaped.

**Entry criteria.**
- Phases 1-7 landed. Real cache pressure exists (or a synthetic stress test).

**In scope.**

*Logical eviction (cheap, frequent):*
- Date-based candidate selection: `attachment_blobs JOIN attachments ON content_hash WHERE message.date < window_start` (plus orphans from account deletion that the synchronous tombstoning in Phase 4 already covers as the privacy contract; this is a backstop for any rows missed).
- For each candidate, in one SQLite transaction: `UPDATE attachment_blobs SET tombstoned_at = ?now` + append to `tombstones-NNNNNN.log`.
- Triggered at startup, after sync batches, and on window shrink.

*Physical GC (expensive, rare):*
- Runs on app idle when tombstoned bytes exceed a threshold (default: 25% of any single pack, or 10% of total cache bytes).
- For each pack with high tombstone density: read the pack, copy live frames to a fresh pack at the chain tail, update `pack_file_id` + `offset` for each surviving blob in the index, atomically delete the old pack.
- Worst-case cost: read + rewrite of one pack (~256 MB sequential I/O).

*Opened-files cleanup:*
- Periodic cleanup of `<app_data>/opened_attachments/` based on the configured window (Phase 6 setting).

*Clear cache button:*
- "Clear attachment cache now" in Storage settings - tombstones every live blob, then runs GC.

*Deferred from earlier phases:*
- **Three crash-recovery harness scenarios** the Phase 3 plan deferred: SIGKILL mid-repack (partial new pack at the chain tail, boot discards), boot with deleted SQLite index (walk pack tails, regenerate `attachment_blobs` rows), and boot with sentinel missing (cross-store invariant pass over PackStore). The first scenario only becomes meaningful here because Phase 8 is the first phase that *runs* a GC repack. Phase 3 landed the single SIGKILL-mid-sync smoke script that covers the open-pack recovery path; this phase fills out the matrix.
- **`--rebuild-attachment-index` tool** the Phase 2 plan stubbed. The recover() entry point handles the open-pack case; a standalone path that walks every sealed pack and regenerates the SQLite index from scratch is the pathological-corruption recovery primitive. Wires up here alongside the deleted-index harness scenario.

**Out of scope.**
- Heuristics for auto-adjusting the window. The window is user-controlled.

**Touchpoints.**
- `crates/stores/src/attachment_pack.rs` - date-based eviction + `gc` methods (the API stubs from Phase 2 grow real bodies).
- `crates/service/src/subsystems.rs` - GC scheduler hook.
- `crates/app/src/handlers/attachments.rs` - opened-files cleanup.

**Exit criteria.**
- After a sync that crosses the window edge, blobs older than `window_start` are tombstoned within one sync cycle.
- After GC fires on high-tombstone-density packs, disk usage drops.
- `opened_attachments/` files older than the cleanup window are removed on startup.

**Risks / open questions.**
- GC during active sync writes - ordering so GC doesn't read a frame from a pack being rotated.
- A window shrink from "All" to "1 month" on a multi-GB cache could tombstone tens of thousands of blobs in one operation. Chunk the sweep so it doesn't block the Service write path during the SQL update.

---

## Phase 9 - Squeeze measurement and tuning

**Goal.** Wire `squeeze::compress` + signed-content bypass into the PackStore write path, validate that squeeze is paying its way, and tune defaults based on observed savings.

**Entry criteria.**
- Phases 1-7 landed and have been running on a real mailbox for a week or two.

**In scope.**

*Squeeze integration (deferred from Phase 3):*
- Wire `squeeze::compress(bytes, mime_type, &Config)` into `attachment.fetch`'s cache-miss path (provider fetch -> squeeze -> `PackStore::put`) and the prefetch path. Phase 3 explicitly deferred this so the signed-content detection corpus and Phase 9 measurement land together.
- Signed-content bypass: build the corpus of real signed PDFs / OOXML / ODF samples + the unit-test surface for the detector. Phase 3 plan called this out as the gating concern.

*Measurement + tuning:*
- Instrument the squeeze path to log compression ratios per mime type.
- Aggregate report (CLI tool or settings panel section) showing `original_bytes -> compressed_bytes` per type, savings percent, time spent.
- Bypass-rate calibration: log how often the signed-content bypass fires, broken out by mime. Zero hits across a populated mailbox suggests detection is broken; an unexpectedly high rate suggests overly aggressive sniffing.
- Decide whether to:
  - Adjust the default per-mime squeeze policy (e.g. always squeeze PDFs, never bother with already-compressed Office docs).
  - Default-on the lossy-JPEG toggle if the win is large enough.
  - Skip squeeze on the hot path entirely if the savings are marginal.

*Batched fsync (deferred from Phase 2):*
- Phase 2 ships per-frame fsync. If the measurement here shows fsync is a meaningful chunk of write-path cost, revisit the "every N frames or M ms" batching pattern the original problem-statement sketched.

**Out of scope.**
- New compressors. Squeeze already covers the formats that matter.

**Touchpoints.**
- `crates/squeeze/src/lib.rs` - metrics callback or enriched results.
- Service-side fetch pipeline - thread measurements through.
- Small CLI subcommand under `brokkr` or standalone binary.

**Exit criteria.**
- A real-mailbox report exists showing per-mime savings.
- Defaults updated based on the data.

**Risks / open questions.**
- May reveal squeeze is net-negative on the sync hot path (CPU > storage savings on fast disks). If so, defer squeeze to a background compaction pass instead of running it inline.

---

## Phase 10 - Linux-specific `ErofsStore` backend (optional)

**Goal.** A second `BlobStore` impl on Linux backed by EROFS rolling images. Selected at runtime via `cfg(target_os = "linux")` (with a settings escape hatch to force `PackStore`). macOS and Windows continue on `PackStore`.

**Entry criteria.**
- Phases 1-9 landed; `PackStore` is in production with measured behavior.
- Real cache-pressure data exists from a Linux user (probably the project owner) running the v1 build long enough to know what we're optimizing.
- A decision has been made that the EROFS win is worth a second backend's maintenance cost. *Optional - if `PackStore` is good enough on Linux, we don't ship it.*

**In scope.**
- **Extract the `BlobStore` trait first.** Phase 2 deferred trait extraction (design-by-future-use with one impl). This phase is the future use. The trait's `pub` surface already matches the sketch in `problem-statement.md` so `PackStore` slides under it mechanically; the work is renaming `&PackStore` references in `attachment_materialize.rs`, the boot path, and the account-delete tombstone loop to `&dyn BlobStore` (or a generic).
- New module `crates/stores/src/attachment_erofs.rs` implementing `BlobStore`.
- Rolling-image storage: `<app_data>/attachment_packs/data-NNNNNN.erofs`, ~256 MB each, never modified after bake.
- Staging area for in-flight writes (small flat-file directory or in-memory queue with periodic durability sync) until the next bake.
- Bake trigger: staging exceeds threshold (size or time-based), shell out to `mkfs.erofs` (or a library equivalent), drop the resulting image, clear staging.
- Index in SQLite: `attachment_blobs_erofs(content_hash PK, image_id, path_within_image, written_at, last_read_at, tombstoned_at)`. Distinct from the `PackStore` index since the location semantics differ.
- Eviction: tombstone individual blobs (refuse on read); whole-image delete only when *all* blobs in an image are tombstoned. No partial repack.
- Migration tool to move existing `PackStore` blobs into `ErofsStore` images on first run with the new backend (or, simpler: leave `PackStore` blobs in place and only put new writes through `ErofsStore` until eviction naturally drains the old store).

**Out of scope.**
- macOS and Windows. They stay on `PackStore`. Cross-platform parity comes from the trait, not from a single backend.
- Encryption.
- Restic-format compatibility (orthogonal axis).

**Touchpoints.**
- New: `crates/stores/src/attachment_erofs.rs`, `crates/stores/src/lib.rs` (re-export), new schema entry for `attachment_blobs_erofs`.
- `crates/stores/src/lib.rs` or wherever `BlobStore` is selected - runtime backend selector via `cfg(target_os)` + settings override.
- `Cargo.toml` - new optional dep on a Rust EROFS reader / writer crate, gated behind a `linux-erofs` feature flag.

**Exit criteria.**
- On Linux: cache writes route to `ErofsStore`; total disk usage measurably lower than equivalent `PackStore` workload (target: 20-40% reduction for typical mail attachment mixes).
- Reads from EROFS images measure within ~10% of `PackStore` reads.
- macOS / Windows builds untouched and continue using `PackStore`.

**Risks / open questions.**
- `mkfs.erofs` invocation: shell out (simple, depends on `erofs-utils` being installed) vs link a library (cleaner, harder to maintain). Probably shell out for v1.
- Staging durability vs bake cadence trade-off.
- Image format compatibility across kernel versions. EROFS has been stable since ~5.4 mainline but feature additions are ongoing. Pin a minimum format-compat level.
- Whether to do a `PackStore` -> `ErofsStore` migration at all, or just let the old store age out via eviction.

---

## Out of phases (deliberately deferred)

These are real follow-ups, but each is a separate problem statement, not a phase of this work:

- **Calendar event attachments**. The orchestration is calendar-ready (the `ParentRef` enum hinted in the orchestration layer), but capturing attachments in calendar sync is its own piece of work.
- **Attachment chip widget unification**. The reading pane and pop-out viewer have separate attachment-card widgets. Unifying them with the future cloud-link chips is a UI consolidation problem, not an attachment-storage problem.
- **Search inside attachment text** (PDF / OOXML extraction, FTS index). Owned by `docs/architecture.md` § "Text extraction pipeline" - the cache being populated is a precondition but not the bulk of it.
- **Attachment encryption at rest**. Tracked under "Mail content stores not encrypted at rest" in TODO.md. Applies to body store, inline image store, and attachment cache uniformly - solve once across all three.
- **Backfill UI**. "Cache all attachments for this account now" button. Lazy fill + eager pre-fetch covers the steady-state need; a one-shot backfill is nice-to-have.
