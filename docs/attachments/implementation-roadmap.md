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

**Status: landed.** JMAP attachments inside the configured retention window are cached via sync-time pre-fetch for new messages and first-launch backfill for historical ones. Account-add and window-extend reuse the same machinery (see "Deferrals" below).

**What shipped.**

*PrefetchRuntime* (`crates/service/src/prefetch.rs`):
- Two priority queues (sync-time capacity 64, backfill capacity 256), biased select drains sync first so backfill can't starve live work.
- Per-account `Semaphore` (4 permits), capped FIFO in-flight dedupe (10K entries, oldest-drop on overflow), `CancellationToken` + `JoinHandle` shutdown, `JoinSet`-tracked per-item tasks, 5 min per-fetch wallclock timeout.
- Per-account circuit breaker (K=5 consecutive timeouts within W=60s trips open; backoff doubles from 30s with a 5 min cap; reset on success). Per-provider would be equivalent for Phase 4 (JMAP only) and revisit in Phase 7.
- Unix ENOSPC backstop via `libc::statvfs` against `MIN_DISK_FREE_BYTES` (5 GB). Windows is permissive (returns None - the cache fills until the OS surfaces ENOSPC as a `PackStore::put` error).
- Dedupe key is `(account_id, attachment_id)` - `message_id` was redundant once `(account_id, attachment_id)` already uniquely identifies a row.

*Sync hook* (`crates/service/src/sync.rs::run_sync`):
- **Post-sync sweep, not a sink trait.** After `sync_for_account` returns `Ok`, `run_sync` calls `prefetch.enqueue_window_for_account(account, window_start, Sync, Some(64))`. The SELECT joins `attachments` against `messages` on `content_hash IS NULL AND m.date >= window_start AND is_inline = 0` and bounds via `SYNC_SWEEP_LIMIT`. Provider-sync sees zero diff - no `PrefetchSink` trait, no return-value change.
- The "sink trait vs return-and-dispatch" question dissolved: a third option, querying the DB after sync returns, is simpler than either. The prior `(account_id, message_id, attachment_id)` enqueue plumbing collapses into a SQL query.

*Retention-depth coupling* (`crates/service/src/sync_dispatch.rs`):
- The hardcoded `sync_initial(&ctx, 365)` now reads from the existing `sync_period_days` setting (already had a reader in `crates/sync/src/config.rs`). Phase 4 added no new pref key. Phase 6's slider writes the same key.
- Day budgets clamp to `>= 1` to keep an absurd pref from underflowing `i64`.

*Backfill driver* (`crates/service/src/prefetch.rs::kick_backfill_account`):
- Walks `attachments` for an account inside the retention window, paginating in 256-row batches, enqueuing each row on the Backfill priority lane. Fire-and-forget; the worker drains asynchronously.

*Boot recovery kick* (`crates/service/src/dispatch/post_ready.rs::spawn_post_ready_prefetch_startup`):
- After `boot.ready`, build `PrefetchRuntime`, install via `BootSharedState::install_prefetch_runtime`, then enumerate every active non-deleting JMAP account and call `kick_backfill_account(account, window_start)`. Idempotent across restarts: the in-flight set is fresh per incarnation, the row-level `content_hash IS NULL` check is the authority.

*Account deletion* (`crates/service/src/handlers/account.rs::handle_delete`):
- Phase 4 adds `prefetch.cancel_account(account_id)` before `delete_with_marker` runs, so a disappearing account doesn't keep issuing provider fetches.
- **The synchronous orphan-blob tombstoning was already wired in Phase 3** via `AccountDeletionStep::AttachmentCache` (`crates/service/src/accounts/delete.rs:226`). Phase 4 inherits it; nothing to add.

*Drain order* (`crates/service/src/subsystems.rs`):
- `drain_prefetch` slot wired between `drain_sync` and `drain_extract`. `Subsystems::drain_runtimes` now visits `Push -> Calendar -> Sync -> Prefetch -> Extract -> Rebuild -> search writer`.

*Notifications* (`crates/service-api/src/extract.rs`, `crates/service-api/src/notification.rs`):
- `PrefetchProgress { service_generation, remaining, fetched_in_session }` - `Coalesce { key: PrefetchProgress }`, latest-wins. Mirrors `ExtractProgress`.
- `PrefetchCompleted { service_generation, fetched, skipped, failed }` - `MustDeliver`. Mirrors `ExtractCompleted`.
- App-side: `update.rs` logs both; `app.rs` accepts them in the drop-list.

**Deferred (not blocked, not in this phase's commits).**

- **Account-add explicit `kick_backfill_account` call.** The plan called for an explicit kick from `handle_create`. The post-sync sweep already covers the account's first sync (after `sync_for_account` returns), and the boot recovery kick covers subsequent app restarts. A direct kick in `handle_create` adds no coverage that the existing two triggers don't already give; leaving it out keeps `handle_create` simple.
- **Window-extend trigger.** Belongs to the slider's pref-write site, which lands in Phase 6. Until then, the next boot's recovery kick covers any retention expansion. Phase 6 will add the immediate kick.
- **Windows ENOSPC backstop** -> **Phase 7**. `statvfs_free_bytes` returns `None` on non-Unix; the cache fills until `PackStore::put` raises an ENOSPC-shaped error. `GetDiskFreeSpaceExW` lands with the cross-provider / cross-platform polish in Phase 7.
- **`SkipReason::ProviderPermanent` vs `Transient` split** -> **Phase 7**. Provider errors are folded into one transient lane for now; splitting them requires a provider-error taxonomy that the cross-provider parity phase is the natural place to add.
- **Per-provider circuit breaker** -> **Phase 7**. Implemented per-account, equivalent for Phase 4 (JMAP only). Phase 7 promotes the keyspace if cross-provider behaviour differs.
- **Shutdown-mid-prefetch harness script** -> **Phase 7**. Blocked on a slow-provider hook for attachment fetch (analogous to `harness-slow-sync`) which doesn't exist yet; lands with Phase 7's cross-provider harness expansion. Phase 4 lands one end-to-end script - `crates/app/tests/sync-harness/jmap-attachment-prefetch.lua` - that asserts a JMAP sync against `saehrimnir` produces a populated `content_hash` and a pack file on disk, and that `prefetch.completed` fires with `fetched >= 1`.
- **Account-delete `prefetch.cancel_account` harness coverage** -> **Phase 8**. Account-delete tombstoning is Phase 8's natural test territory since Phase 8 owns the eviction policy. Synchronous orphan-blob tombstoning is already covered indirectly by the Phase 3 `AccountDeletionStep::AttachmentCache` path; explicit harness coverage for the prefetch-cancel side lands once Phase 7's slow-provider hook is available.

**Open questions resolved during implementation.**

- *"Sink trait vs return-and-dispatch"* (`docs/attachments/problem-statement.md` § runtime shape): resolved as **post-sync DB sweep in Service**. Neither planned option ships.
- *"Pref name `retention_window_days`"* (Phase 4 plan): resolved as **reuse existing `sync_period_days`**. Same semantic, no migration.
- *"`SyncRuntime` access to `PrefetchRuntime`"* (not in the plan, surfaced during wiring): `SyncRuntimeInner` now holds `Arc<BootSharedState>` and dereferences `boot_state.prefetch_runtime()` per call. Mirrors `ExtractRuntime`'s pattern. The Arc cycle breaks at drain time when `take_sync_runtime` removes SyncRuntime from BootSharedState.
- *"Crash mid-delete leaves orphan blobs visible"* (problem-statement.md § account deletion): resolved at Phase 3 time by the resumable `AccountDeletionStep` marker; Phase 4 inherits the resolution.

---

## Phase 5 - UI: Open, Save, Save All

**Status: landed.** The reading-pane and pop-out attachment chips drive real Open / Save / Save All flows. All three route through `attachment.fetch` IPC; the UI re-opens `relative_path` positionally and bytes never cross JSON.

**What shipped.**

*Shared handler module* (`crates/app/src/handlers/attachments.rs`):
- `AttachmentRef { account_id, message_id, attachment_id, filename, mime_type }` is the common payload.
- `OpenAttachmentParams { item }`, `SaveAttachmentParams { thread_id, item }`, `SaveAllAttachmentsParams { thread_id, items }` are the three input types.
- `impl ReadyApp { handle_open_attachment, handle_save_attachment, handle_save_all_attachments }` are the call sites. Each wraps a private `async fn ..._worker` in `Task::perform`.
- A new `sanitize_attachment_filename` lives in the module rather than reusing `save_as.rs::sanitize_filename` (that one strips dots and would mangle extensions). Preserves alphanumerics + dots + spaces + parens; strips path separators, Windows-reserved chars, and control bytes; fallback to `"attachment"` if the result is empty.
- `mime_to_ext` covers the common types (PDF, OOXML, images, text, zip, json); `save_dialog_filter` prefers the filename's own extension and falls back to mime.
- `pick_collision_free_path` does the `(N)` suffix preserving the extension - `report.pdf` -> `report (1).pdf`.
- `open_file_with_os_default` is the cross-platform `xdg-open` / `open` / `cmd /c start "" <path>` shell-out.
- Unit tests cover sanitizer edge cases (path separators, Windows-reserved chars, empty input, all-dots), the stem/ext split (`report.pdf`, `README`, `.hidden`), and the mime mapping with `text/plain; charset=utf-8` style suffixes.

*Reading-pane event hoist* (`crates/app/src/ui/reading_pane.rs`):
- New `ReadingPaneEvent::{OpenAttachment, SaveAttachment, SaveAllAttachments}` variants carrying the param types from the shared module.
- The three stub arms in `ReadingPane::update` build the events from `current_thread` and `thread_attachments` and emit them.
- `crates/app/src/handlers/core.rs::handle_reading_pane_event` dispatches each into the matching `ReadyApp` method.

*Pop-out direct wiring* (`crates/app/src/handlers/pop_out/dispatcher.rs`):
- The dispatcher intercepts `MessageViewMessage::{OpenAttachment, SaveAttachment, SaveAllAttachments}` BEFORE they reach `handle_message_view_update`, builds the params from `MessageViewState` via three private `build_*_params` helpers, and calls the matching `self.handle_*_attachment` method directly. No new events; the dispatcher has `&mut self` access.
- The leftover arms in `handle_message_view_update` log a "dispatcher routing regression" warning if they're ever reached.

*Last-folder cache:*
- `ReadyApp::attachment_last_folders: HashMap<(account_id, thread_id), PathBuf>`, populated when a Save / Save All dialog returns a chosen path. Used to `set_directory` the next dialog inside the same thread. In-memory only.

*Boilerplate:*
- New `Message::AttachmentSaveFolderRemembered((account_id, thread_id), PathBuf)` variant + dispatch arm in `update.rs` that just inserts into the cache.

**Deferred.**

- **Cross-session save-path persistence.** The plan called for an `attachment_save_paths` table cascading off `threads`. The write side would require a new IPC (the app is read-only on DB by architecture), which is ~100 LOC across schema + service-api + service handler + service_client wrapper for a modest UX gain. Phase 5 ships the in-memory cache; durable persistence is a small follow-up if real users want it.
- **Toast-based error reporting.** Still blocked on the toast surface in `TODO.md`. Phase 5 logs at `warn!` on all user-facing failures; the user sees nothing beyond the action having no effect.
- **`opened_attachments/` periodic cleanup.** Owned by Phase 8.

**Touchpoints.**
- New: `crates/app/src/handlers/attachments.rs`.
- `crates/app/src/handlers/mod.rs` - re-export.
- `crates/app/src/ui/reading_pane.rs` - new event variants, replaced stub arms, `AttachmentAction` enum and `build_attachment_event` helper.
- `crates/app/src/handlers/core.rs` - three new dispatch arms.
- `crates/app/src/handlers/pop_out/dispatcher.rs` - three new intercepting arms + three `build_*_params` helpers.
- `crates/app/src/handlers/pop_out/message_view.rs` - leftover stubs degraded to routing-regression warnings.
- `crates/app/src/app.rs` - `attachment_last_folders` field + initializer.
- `crates/app/src/message.rs` - `Message::AttachmentSaveFolderRemembered` variant.
- `crates/app/src/update.rs` - dispatch arm for the new Message.

**Exit criteria (manual verification).**
- Click Open in either surface -> file lands in `<app_data>/opened_attachments/<safe_filename>` and the OS default handler launches.
- Click Save -> dialog opens (pre-filled with sanitized filename, mime-derived extension filter), chosen file written, second Save inside the same thread opens the dialog at the previously-chosen folder.
- Click Save All -> folder picker, every attachment written, name collisions get `(N)` suffix preserving the extension.
- All three work with network disabled if the cache is populated (Phase 4 prefetch covers this).

**Risks / open questions** (carry-over to follow-up work, not Phase 5-blocking).
- Save All on a multi-attachment message with mixed cached / uncached state surfaces only as a summary log line; per-attachment failure UI waits for the toast surface.
- Windows / macOS OS-default-open quirks (UAC, quarantine attribute, GateKeeper). Untested on those platforms in this phase.

---

## Phase 6 - Settings (backend slice landed; UI is the user's separate work)

**Status: backend slice landed.** The plumbing every Phase 6 setting will eventually invoke is in place: schema columns, wire types, service handlers, PrefetchRuntime gating, service_client wrappers, and harness coverage. The widget code lives outside this roadmap - the user is implementing the settings UI separately once the existing settings surface is ready to host the new section.

**What shipped (backend).**

*Schema* (`crates/db/src/db/schema/01_core.sql`):
- New column `accounts.cache_attachments_enabled INTEGER NOT NULL DEFAULT 1`. Pre-release policy is edit-in-place; no migration entry.
- New settings seed rows: `compress_attachments=true`, `allow_lossy_compression=false`, `opened_files_cleanup_days=7`. The existing `sync_period_days` key (already plumbed in Phase 4) is the retention slider's target.

*Wire types:*
- `SettingValue` gained `SyncPeriodDays(String)`, `CompressAttachments(bool)`, `AllowLossyCompression(bool)`, `OpenedFilesCleanupDays(String)`. The two string-typed ones match the existing snapshot's `Option<String>` shape; the Service parses to `i64` on read.
- `AccountUpdateParams` gained `cache_attachments_enabled: Option<bool>` as a patch field (None leaves the column untouched).
- `SettingsBootstrapSnapshot` (`crates/core/src/db/queries.rs`) gained `compress_attachments: bool`, `allow_lossy_compression: bool`, `opened_files_cleanup_days: Option<String>`. The UI reads these from the bootstrap; no per-pref `settings.get` round-trips needed.
- New IPC `attachment.cache_size` with `AttachmentCacheSize{Params,Ack}`. The ack is `{ live_bytes, tombstoned_bytes }` from a single SQL aggregate over `attachment_blobs.length`. `tombstoned_bytes` is surfaced separately so a future UI can show both the in-use total and the reclaimable-on-next-cleanup figure.

*Service handlers:*
- `settings.set` (`crates/service/src/handlers/settings.rs::handle_set`) now reads the existing `sync_period_days` before the write transaction and, post-commit, fires `prefetch.kick_backfill_account` for every JMAP account with caching enabled when the new value is strictly larger than the old. Idempotent: re-firing against unchanged rows is a no-op.
- `account.update` (`crates/service/src/handlers/account.rs::handle_update`) patches `cache_attachments_enabled` via the existing `UpdateAccountParams` flow.
- `attachment.cache_size` (`crates/service/src/handlers/attachment.rs::handle_cache_size`) backed by a new `PackStore::size_breakdown()` that returns `(live_bytes, tombstoned_bytes)`.

*PrefetchRuntime gating:*
- `run_pipeline` in `crates/service/src/prefetch.rs` adds a per-account check after the circuit-breaker / disk-headroom checks. Disabled accounts skip with new `SkipReason::AccountDisabled`. The row stays `content_hash IS NULL`; the next re-enable + sync covers it.
- Boot recovery kick (`dispatch/post_ready.rs`) filters `WHERE cache_attachments_enabled = 1` in its account enumeration.
- Post-sync sweep (`crates/service/src/sync.rs::run_sync`) reads the same flag and skips the sweep entirely for disabled accounts, avoiding the enqueue-just-to-drop round-trip the worker check would do.

*App client:*
- `crates/app/src/service_client.rs::attachment_cache_size` wraps the new IPC.
- `crates/app/src/ui/settings/types/mod.rs` extends its default snapshot with the three new fields so the UI struct has them in scope when the user wires widgets.

*Harness coverage:*
- `crates/app/tests/sync-harness/jmap-attachment-cache-disabled.lua` walks the full lifecycle: sync with caching disabled (no prefetch fires, `content_hash IS NULL`), flip the toggle on, sync again (prefetch fires, hash populated), and assert `attachment.cache_size.live_bytes > 0`. Plus an `AccountUpdate` registry arm so the Lua side can drive `account.update` directly.

**Out of scope (user-owned).**
- Where the new section / toggles live in the existing settings surface, what they look like, the per-account toggle's host (account editor sheet or elsewhere).
- "All" retention bucket UX (the backend accepts any positive `i64`; the slider's bucket-to-days mapping is the UI's call).
- "Clear cache now" button (Phase 8).
- Live polling cadence for the cache-size readout (the IPC returns a snapshot; the UI decides when to call).

**Deferrals carried forward.**
- Phase 9 reads `compress_attachments` / `allow_lossy_compression` when wiring inline squeeze into `PackStore::put`.
- Phase 8 reads `opened_files_cleanup_days` when the periodic reaper for `<app_data>/opened_attachments/` lands.
- Cross-session save-path persistence (deferred from Phase 5) stays deferred; no IPC was added for it in this phase either.

**Verification (landed).**
- `brokkr check` workspace-wide clean.
- Phase 4 harness (`jmap-attachment-prefetch.lua`) still passes - no regression in the prefetch happy path.
- New harness (`jmap-attachment-cache-disabled.lua`) passes - per-account toggle gates correctly in both directions, and the cache-size readout reflects post-prefetch state.

---

## Phase 7 - Provider parity (Gmail, Graph, IMAP)

**Status: landed.** Prefetch enqueues for Gmail, Graph, and IMAP accounts alongside JMAP. IMAP-specific folder-batching holds one session across every attachment fetched from the same folder. Error taxonomy splits transient from permanent. Windows gains the disk-headroom backstop. Cross-provider end-to-end harness coverage lands for Gmail and Graph; IMAP and SIGINT-mid-prefetch deferred to Phase 7.5 pending `saehrimnir` knobs.

**What shipped.**

*Error taxonomy* (`crates/common/src/error.rs`, `crates/service/src/prefetch.rs`):
- `ProviderError::kind() -> ProviderErrorKind` classifies `Auth | NotFound | Client` as `Permanent` and `Network | RateLimit | Server | Db` as `Transient`. No new variants, wire shape unchanged.
- `SkipReason::ProviderTransient` split into `ProviderTransient` and `ProviderPermanent`. Both still count as failures and neither feeds the breaker (which remains timeout-only); the split exists so future skip-attempt logic and logs can distinguish "retry me" from "stop trying."

*Schema cleanup* (`crates/db/src/db/schema/02_mail.sql`):
- `attachments.imap_part_id` retired. IMAP sync already writes the part path into `remote_attachment_id` (the provider-agnostic Phase 1 name), and the sweep's `COALESCE(remote, imap)` fallback was dead code. `AttachmentInsertRow`, `UncachedAttachment`, the `attachment.fetch` lookup, and the sweep query all simplify accordingly.

*Filter lift* (`crates/service/src/dispatch/post_ready.rs`, `crates/service/src/sync.rs`):
- `WHERE provider = 'jmap'` dropped from the boot recovery kick and the post-sync sweep gate. Every account with `cache_attachments_enabled = 1` now participates. Provider type is read alongside the account ID so the worker can pick its concurrency cap and breaker key.

*IMAP folder-batching* (`crates/service/src/prefetch.rs`, `crates/imap/src/client/mod.rs`, `crates/imap/src/ops.rs`):
- `PrefetchWork` is now an enum: `Item(PrefetchItem)` for per-attachment work (the existing JMAP/Gmail/Graph path) and `ImapBatch { account_id, folder_id, items }` for IMAP, grouped by message folder at sweep time.
- `process_imap_batch` holds the per-account semaphore once, opens one session, issues one `SELECT`, then drains the batch through `imap::client::fetch_attachment_on_selected` (new public entry point that skips the redundant per-fetch SELECT). LOGOUT runs at the end. Cancellation between items is honored.
- Per-provider semaphore cap (`provider_semaphore_cap`) returns 1 for IMAP (1-per-folder serialization) and the existing 4 for the others.
- `imap::ops::parse_imap_message_id` is now `pub` so the service crate can extract UID + folder without re-parsing.

*Circuit breaker keyspace* (`crates/service/src/prefetch.rs`):
- Breakers are keyed by `(provider, account_id)` rather than `account_id`. Today the two-tuple is practically equivalent (each account has one provider) but encodes the dimension explicitly so a future per-provider promotion is a key-derivation tweak rather than a refactor.

*Windows ENOSPC backstop* (`crates/service/src/prefetch.rs`):
- `statvfs_free_bytes` gained a Windows branch using `GetDiskFreeSpaceExW` against the pack-store directory (walking up to the volume root on failure). The Unix `statvfs` path is unchanged; the `cfg(not(any(unix, windows)))` branch keeps the permissive fallback semantics.

*Harness* (`crates/app/tests/sync-harness/`):
- New `gmail-attachment-prefetch.lua` and `graph-attachment-prefetch.lua` clones of the JMAP script, parameterized by `provider = "gmail_api" | "graph"`. Both assert `prefetch.completed` fires with `fetched >= 1`, `content_hash` populates, and a pack file lands on disk.

**Exit criteria met.**
- After a sync on JMAP, Gmail, and Graph accounts, `attachment_blobs` rows with `tombstoned_at IS NULL` appear for that account (harness-verified).
- IMAP attachment fetches reuse the folder session: the batch worker issues one LOGIN + SELECT per `(account, folder)` and runs `fetch_attachment_on_selected` per item (code-verified; end-to-end harness pending the saehrimnir-side IMAP attachment fixture).

**Deferred to Phase 7.5 (pending external `saehrimnir` work).**
- **`RATATOSKR_TEST_ATTACHMENT_LATENCY_MS` knob in `saehrimnir`** - needed to reliably reproduce mid-prefetch state for SIGINT scripts. `saehrimnir` lives outside this repo; the knob lands there first.
- **`imap-attachment-prefetch.lua`** - the JMAP fixture's IMAP mock doesn't surface the attachment through BODYSTRUCTURE, so an end-to-end IMAP prefetch test needs either a new fixture or saehrimnir-side fixture support.
- **`sigint-mid-prefetch.lua`** - blocked on the latency knob above. The boot recovery kick path is exercised every time the service starts in any harness script that prefetched on the prior run, but a script that explicitly asserts mid-flight resumption needs the latency hook to make the race deterministic.
- **Gmail batch attachment endpoint** - roadmap-noted, no measured need yet.

**Out of scope.**
- IMAP partial-fetch optimization (`BODY[part]<offset.length>`).
- Reference-attachment handling on Graph - URL-only, not bytes; surface as cloud links via the cloud-attachments path.
- Per-message coalescing inside an IMAP folder batch (one full-body fetch yielding N attachments).

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
- **Account-delete `prefetch.cancel_account` harness coverage** (Phase 4 deferral). Tombstoning a freshly-deleted account's blobs is Phase 8's natural test territory because Phase 8 owns the eviction policy. Script seeds a JMAP account with cached attachments, kicks an active prefetch (using Phase 7's slow-provider hook), issues `account.delete`, and asserts (a) no further provider fetches fire for the cancelled account, (b) the `AccountDeletionStep::AttachmentCache` step tombstones every unshared `content_hash`, (c) pack bytes are reclaimed on the next GC repack.

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
