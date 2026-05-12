# Attachments - Implementation Roadmap

Companion to `problem-statement.md`. Each phase below is intended as a **separate `EnterPlanMode` session** that produces a focused implementation plan, lands as one or a small handful of commits, and unblocks the next phase. Nothing here is a complete plan in itself - the goal is to chart the order of attack and keep us from accidentally building things in the wrong sequence.

This document is a sketch. Phase scope, interfaces, and risks will firm up when each phase enters its own planning session.

**Cross-document dependency.** Phase 2 onward runs inside **the
Service** (`docs/architecture.md` § "Service process model"); Phase 3's
cold path uses the `attachment.fetch` IPC defined in
`crates/service-api/src/attachment.rs`. Phases 1a and 1b are pure
library work.

## How to read this

- **Goal** - one-sentence outcome.
- **Entry criteria** - what must already exist for this phase to start cleanly.
- **In scope / Out of scope** - hard boundaries for the phase.
- **Touchpoints** - files / modules likely to change. Indicative, not exhaustive.
- **Exit criteria** - observable evidence the phase is complete.
- **Risks / open questions** - unknowns to resolve during the planning session.

---

## Phase 1a - Pack store

**Goal.** A self-contained pack-file blob store under `crates/stores/src/attachment_pack.rs` supporting content-addressed put / get / tombstone / GC, with a SQLite index, full crash safety, and a `recover` path that rebuilds the index from pack tails.

**Entry criteria.**
- Problem statement approved.
- Format spec finalized (frame layout, tail layout, version byte semantics, tombstone log format).
- Decision made on chunking-or-not (current proposal: no chunking in v1; revisit after squeeze measurement lands in Phase 7).

**In scope.**

*Library:*
- `crates/stores/src/attachment_pack.rs` with `PackStore::open`, `put`, `get`, `tombstone`, `gc`, `recover`.
- New SQLite migration: drop the flat-cache addressing columns on `attachments` (`local_path`, `cached_at`, `cache_size`) and add `attachment_blobs` with `(content_hash BLOB(32) PK, pack_file_id, offset, length, written_at, last_read_at, tombstoned_at)`. No `refcount` column - counts are derived from `attachments` at query time. Migrate `attachments.content_hash` to `BLOB(32)` BLAKE3 in lockstep (the existing TEXT-xxh3 column dies in the same migration). **This is an addressing-scheme swap plus a hash-algorithm swap, not a column drop**: see the migration-scope risk below.
- Frame writer: 4-byte magic + 4-byte length + 8-byte xxh3_64 of payload + payload, batched fsync, atomic index update via SQLite transaction.
- Pack tail writer with version + frame_count + crc.
- Tombstone log format and read-side enforcement (refuse to return tombstoned data even on stale index hit; `tombstoned_at IS NOT NULL` in the index is the fast path, tombstone log is the durable record).
- Pack rotation when `PACK_TARGET_SIZE` (256 MB default) exceeded.
- One-writer-per-pack mutex; lock-free positional reads.

*Service integration:*
- Boot-phase placement: `PackStore::open` runs after `BootPhase::SchemaMigrations` (needs the `attachment_blobs` table) and as part of the cross-store invariant pass (since pack-vs-index reconciliation is exactly that). On unclean shutdown (missing sentinel), `PackStore::recover` walks the open pack and reconciles the index *before* `boot.ready`. The recovery path also replays the tombstone log so `tombstoned_at` is rebuilt correctly if the SQLite index was lost.
- Per-fetch transient extraction: the `attachment.fetch` handler reads `(pack_file_id, offset, length)` from `attachment_blobs`, extracts the blob to `<app_data>/attachment_fetch_tmp/<content_hash>` (write-to-tmp + rename, atomic), and returns that path in the `AttachmentFetchAck`. UI sees the same `relative_path` contract as flat cache. Cleanup pass on idle removes `attachment_fetch_tmp/*` older than 10 minutes.
- Drain integration: GC is on-idle and gets cancelled on shutdown signal; no separate slot in the drain order. Tombstone writes flow through whatever subsystem owns the eviction selector (`Prefetch` after the eviction sweep, `Sync` after account-delete cleanup) and inherit that slot's drain ordering.
- Clean-shutdown sentinel: PackStore is a writer, so the sentinel write waits for `PackStore::flush` (final fsync of open pack + index) before recording.

*Tests:*
- Unit tests (library): round-trip, dedup (two `put`s of identical bytes append once, no counter mutation), tombstone honored on read, recover from missing index, recover from torn last frame, GC on mixed live/dead pack.
- Harness scripts (Service IO boundary, under `crates/app/tests/service-harness/`): SIGKILL mid-write (partial trailing frame on the open pack, boot recovers cleanly), SIGKILL mid-repack (partial new pack with no tail, boot discards it and old pack stays authoritative), boot with deleted SQLite index (`--rebuild-attachment-index` walks pack tails and regenerates), boot with sentinel missing after clean-looking shutdown (invariant pass runs, no data loss).

**Out of scope.**
- Encryption (deferred - tracked under "Mail content stores not encrypted at rest" in TODO.md).
- Restic format compatibility (deferred to v2).
- Chunking large blobs into smaller frames (out of v1 unless squeeze measurement says otherwise).
- Anyone calling the store *as a producer*. The Service's boot/drain wiring lands in this phase so PackStore is correctly lifecycle-managed; actual put/get callers (sync, `fetch_or_load`, eviction) come in Phase 1b onward.

**Touchpoints.**
- New: `crates/stores/src/attachment_pack.rs`, `crates/stores/src/lib.rs` (re-export).
- New SQL migration touching `crates/db/src/db/schema/02_mail.sql`.
- `crates/service/src/dispatch/init.rs` and `crates/service/src/boot.rs` - PackStore::open call site, invariant-pass integration, drain wiring.
- `crates/service/src/subsystems.rs` - GC scheduler hook.
- `crates/app/tests/service-harness/` - new harness scripts named above.
- Old `crates/stores/src/attachment_cache.rs` - kept for now but no longer called by the new path; deleted in a follow-up commit once nothing references it.

**Exit criteria.**
- `brokkr check` clean.
- Unit tests cover the six scenarios above.
- Harness scripts pass under `brokkr service-suite` and preserve artefact dirs on intentional failure.
- A small benchmark exists (insert 10k 4 KB blobs + 1k 1 MB blobs, time per op, total disk usage, inode count) for future regression checks.

**Risks / open questions.**
- **Migration scope.** `attachments.local_path`, `cached_at`, and `cache_size` are not unused - they're the current flat-cache addressing scheme. Phase 1a is an addressing-scheme swap, not a column drop. Callsites that change in lockstep: `crates/stores/src/attachment_cache.rs` (read/write paths through `find_cache_info`, `update_cache_fields`, `enforce_cache_limit`, `try_cache_hit`, `cache_after_fetch`), the Service's `attachment.fetch` handler (today reads `local_path` to produce the `relative_path` ack), and the startup cross-store invariant pass. Any in-flight cached bytes need a one-shot migration into pack files at first boot under the new scheme, or a tombstoning-and-refetch fallback if migration complexity isn't worth it.
- **GC-during-shutdown ordering.** GC reads from immutable closed packs and writes to a fresh pack at the chain tail. A shutdown signal during an in-flight repack must either complete the repack (slow shutdown) or discard the partial new pack and leave the old one authoritative (fast shutdown, GC re-runs next idle). Pick the policy as part of the drain integration.
- Whether to put the open pack in its own file or always rotate on startup. Trade-off: in-place open-pack means recovery has to scan; always-rotate means more pack files. Probably in-place; rotation only on size cap.
- fsync batching cadence. Too aggressive = slow writes; too lax = larger crash recovery scan. Probably "every 16 frames or 100 ms, whichever first."
- Hash choice is BLAKE3, not xxh3. xxh3 is non-cryptographic and email attachments are adversarial input (a sender controls the bytes); a second-preimage attack against xxh3_64 is trivial and would let a sender substitute cached bytes via `INSERT OR IGNORE` collision. BLAKE3 is collision-resistant and within a constant factor of xxh3 on throughput. xxh3 stays as the frame-payload checksum *inside* pack files (corruption detection only).

---

## Phase 1b - Core orchestration

**Goal.** A self-contained `core::attachments` module that, given `(account_id, message_id, attachment_id)`, returns the bytes - reading from the pack store or the inline image store first, fetching from the provider on miss, squeezing, hashing, writing to the pack store, and updating the DB row.

**Entry criteria.**
- Phase 1a landed. `PackStore` exists and is tested.

**In scope.**
- `crates/core/src/attachments/mod.rs` with `fetch_or_load`, `prefetch_one`, `prefetch_message`, `prefetch_messages`.
- A `ParentRef` enum (`Message { ... } | Event { ... }`) so calendar can plug in later. Only the `Message` variant is wired in this phase.
- Unit tests for: pack-store hit fast path, inline-image-hit fast path, full fetch + squeeze + put + update path (mocked provider).
- Wiring `squeeze::compress` into the write path (lossless defaults, signed-content bypass active from day one - detection lives inside the `squeeze` crate, not in `core::attachments`).
- Touch-on-read: bump `attachment_blobs.last_read_at` when `PackStore::get` succeeds (informational stat).

**Out of scope.**
- Calling the new module from anywhere. Sync, reading pane, pop-out viewer all untouched. This phase just lands the building block.
- New settings.
- Calendar wiring.

**Touchpoints.**
- New: `crates/core/src/attachments/{mod,fetch,prefetch,parent_ref,tests}.rs`.
- `crates/core/src/lib.rs` - re-export the new module.

**Exit criteria.**
- `brokkr check` clean.
- Tests pass for the three paths above.
- Module is documented in `docs/architecture.md` under "Settled Patterns"
  ("Attachment fetch orchestration").

**Risks / open questions.**
- Squeeze cost on the sync hot path (batch hashing of large attachments). May need to spawn the squeeze on a blocking pool. Resolve with a benchmark in the planning session.
- Whether to surface "fetch failed for this attachment" through a proper error type vs swallowing. Likely a typed enum so phase 3 can disambiguate "cache miss + offline" from "cache miss + provider error".

---

## Phase 2 - Cache population (JMAP first)

**Goal.** JMAP attachments inside the configured retention window are cached, via sync-time pre-fetch for new messages and first-launch / account-add / window-extend backfill for historical ones.

**Entry criteria.**
- Phase 1a + 1b landed.
- `core::attachments::prefetch_messages(...)` exists and is tested.

**In scope.**
- New `PrefetchRuntime` in `crates/service/src/prefetch.rs`, mirroring `ExtractRuntime` (`crates/service/src/extract.rs`): two priority queues (sync-time capacity 64, backfill capacity 256, sync drained first so backfill can't starve live work), per-account semaphore at 4, bounded `Arc<Mutex<HashSet>>` enqueue dedupe (cap 10K, oldest-drop policy when full), `CancellationToken` + stored `JoinHandle`, 5 min per-fetch wallclock timeout, per-provider circuit breaker (5 consecutive timeouts in 60s opens the circuit, exponential backoff 30s → 5min cap), ENOSPC safety backstop (skip the write + log warning if `statvfs` reports free space below `min_disk_free_gb`, default 5 GB).
- Drain-order insertion: `Push -> Calendar -> Sync -> Prefetch -> Extract -> Rebuild -> search writer`. Edit `docs/architecture.md` § "Service process model" in lockstep.
- Sync trigger: after `persist_messages` in `crates/jmap/src/sync/storage.rs` writes attachment metadata for messages within the retention window, enqueue onto `PrefetchRuntime`.
- Backfill trigger: on first launch, account-add, and retention-window-extended, a backfill driver walks historical messages whose date now falls inside the window and enqueues missing attachments onto the runtime's *backfill* queue. Same fetch path; lower priority than the sync-time queue so live mail doesn't wait behind a multi-GB backfill.
- Account deletion handler: synchronous tombstoning. When an account is deleted, walk `attachments WHERE account_id = ?` in one transaction, drop those rows, then tombstone every `attachment_blobs` row whose `content_hash` no longer has any surviving `attachments` references. Disk reclamation is async (GC), but the privacy contract (bytes no longer reachable through the live store) is met before account-delete returns.
- Boot-time recovery kick: on Service boot, walk `attachments WHERE content_hash IS NULL AND message.date >= window_start` per account and re-enqueue. Idempotent (dedupe set + row-level check).
- Per-account policy reads from settings (toggle + retention window). Defaults if unset (true / 1 year).
- No size thresholds. Everything inside the window is cached regardless of per-attachment or per-message size.
- Failures logged, never block sync completion. Failed rows stay `content_hash IS NULL` and get re-attempted by the next sync or the next boot kick.
- Progress events through the existing `ProgressReporter`, surfaced to the UI as Service notifications on the existing sync-progress channel.

**Out of scope.**
- Other providers (phase 5).
- Settings UI (phase 4) - read raw values from the prefs table for now.

**Touchpoints.**
- `crates/service/src/prefetch.rs` - new `PrefetchRuntime` module (sibling of `extract.rs`).
- `crates/service/src/dispatch/...` - boot wiring + drain-order insertion.
- `docs/architecture.md` § "Service process model" - drain-order update.
- `crates/jmap/src/sync/storage.rs` - sync trigger call site (enqueues onto `PrefetchRuntime`).
- A Service-side backfill driver (likely `crates/service/src/prefetch_backfill.rs` or folded into `prefetch.rs`) that enumerates historical in-window messages and enqueues missing attachments.
- `crates/db/src/db/queries.rs` or a dedicated prefs helper - read pre-fetch policy.
- `crates/core/src/progress.rs` - new event variants if needed (`AttachmentCached { message_id, attachment_id, bytes }`).

**Exit criteria.**
- After a JMAP sync, `SELECT COUNT(*) FROM attachment_blobs WHERE tombstoned_at IS NULL` increases for messages within the window.
- After account-add or window-extend, backfill runs and the same count grows over historical in-window messages.
- Service shutdown during active prefetch drains cleanly (no detached tasks); the next boot's recovery kick resumes from `content_hash IS NULL`.
- Status bar shows progress events during sync-time and backfill caching.
- Pre-fetch failures don't break sync; a retried sync re-attempts the same attachments.

**Risks / open questions.**
- Sync completion timing: enqueue is bounded but not awaited, and the sync UI today says "sync complete" the moment metadata lands. Decide whether `PrefetchRuntime` queue depth should surface as its own progress separate from sync state.
- Backfill of a multi-year window on a heavy mailbox may be many GB of fetches. The boot recovery kick (walks `content_hash IS NULL`) covers crash resumption; the dedupe `HashSet` is *not* meant to survive restarts (it's a perf hint, not a durability contract), and the row-level `content_hash IS NULL` check is what actually prevents double-fetch.
- Circuit breaker calibration: K=5, W=60s, backoff 30s→5min is a starting point. Real provider misbehavior (e.g. transient 429 storms vs persistent auth failure) may want different thresholds; Phase 5 cross-provider parity is the natural place to revisit.
- ENOSPC backstop interaction with backfill: a long backfill that hits the disk-free floor will drop work on the floor and log. Decide whether to surface this to the UI as "Cache paused: low disk space" or stay silent.

---

## Phase 3 - Open / Save / Save All wiring

**Goal.** The three buttons in the reading pane and pop-out viewer actually work, online or offline, on real or stub-cached attachments.

**Entry criteria.**
- Phase 1a + 1b landed (`fetch_or_load` exists, backed by `PackStore`).
- Phase 2 *helpful but not required* - phase 3 works equally well in fetch-on-click mode if no pre-fetch has happened.

**In scope.**
- New shared module `crates/app/src/handlers/attachments.rs` with `handle_open_attachment`, `handle_save_attachment`, `handle_save_all_attachments`.
- All reads go through `attachment.fetch` (hit or miss); the Service handler disambiguates internally. Ack returns `AttachmentFetchAck { content_hash, size_bytes, relative_path }` and the UI re-opens the file positionally. Bytes never cross JSON (see `crates/service-api/src/attachment.rs`).
- UI does not read `attachment_blobs` directly, link `BlobStore`, or otherwise know about the storage layout. Storage stays Service-internal across the trait + backend boundary.
- Wire from `ui/reading_pane.rs:370/374` (currently stubbed).
- Wire from `handlers/pop_out/message_view.rs:69/73/77` (currently stubbed).
- File dialogs via `rfd::AsyncFileDialog`. Folder picker for Save All.
- OS-default open via the existing platform Command pattern (`reading_pane.rs:917`).
- Open writes to `<app-data>/opened_attachments/<safe_filename>` (no `/tmp`).
- Filename sanitization reuses `pop_out/save_as.rs::sanitize_filename`.
- Last-folder-per-thread persisted in a small new table.

**Out of scope.**
- Periodic cleanup of `opened_attachments/` (defer to phase 6 or treat as a low-priority follow-up).
- Toast-based error reporting (blocked on the toast system in TODO.md). Errors logged for v1.

**Touchpoints.**
- New: `crates/app/src/handlers/attachments.rs`, `crates/db/src/db/schema/02_mail.sql` (new `attachment_save_paths` table), `crates/db/src/db/queries_extra/...` (CRUD).
- `crates/app/src/ui/reading_pane.rs` - replace stub branches.
- `crates/app/src/handlers/pop_out/message_view.rs` - same.

**Exit criteria.**
- Click Open on an attachment -> file opens in the OS default handler.
- Click Save -> file dialog -> file written to disk, byte-equivalent to the original at the application level.
- Click Save All -> folder picker -> all files written; collisions get `(N)` suffix.
- After Save, the next Save on the same thread pre-fills the previous folder.
- All three work with the network disabled if the cache is populated.

**Risks / open questions.**
- Save All on a multi-attachment offline message with mixed cached / uncached state. Probably: report which ones failed, write the ones that succeeded.
- Windows / macOS testing - the OS-default-open paths exist for links but the file-handler integration may surface platform-specific issues (UAC prompt? Quarantine attribute? GateKeeper?).

---

## Phase 4 - Settings UI

**Goal.** Users can configure caching, retention window, and squeeze policy from settings.

**Entry criteria.**
- Phase 1-3 landed. Pre-fetch and Open/Save read their config from prefs - settings UI just wraps the prefs.

**In scope.**
- Per-account section ("Storage" tab on the Account editor):
  - `Cache attachments for offline use` (toggle).
  - `Mail to keep offline` (Outlook-style slider: 1 month / 3 months / 6 months / 1 year / 2 years / All; default 1 year).
- Global settings, new "Storage" section (sibling to "Notifications", "Composing"):
  - `Compress cached attachments` (toggle, default on).
  - `Allow lossy compression (JPEG re-encoding)` (toggle, default off).
  - `Cleanup opened-files temp folder after N days` (slider, default 7).
  - Live "Cache currently using X.Y GB" readout (informational; no cap to compare against).

**Out of scope.**
- A "Clear cache now" button (probably wanted but defer to a follow-up phase).

**Touchpoints.**
- `crates/app/src/ui/settings/tabs/...` - new section.
- `crates/app/src/ui/settings/types/...` - new pref keys + bootstrap snapshots.
- `crates/db/src/db/queries.rs` - getters/setters for the new prefs.

**Exit criteria.**
- Settings persist across restarts.
- Shortening the retention window triggers a Phase 1 eviction sweep; extending it triggers a backfill.
- Disabling caching on an account stops new attachments from being downloaded by sync or backfill (fetch-on-click still works).

**Risks / open questions.**
- Where does the per-account toggle actually live - on the Account editor sheet, or on a new global "Storage" tab with a per-account dropdown? Probably the editor sheet, matching how other per-account settings work.

---

## Phase 5 - Pre-fetch in Gmail / Graph / IMAP

**Goal.** Pre-fetch parity across all four mail providers.

**Entry criteria.**
- Phase 2 landed for JMAP. The pattern is proven and the orchestration handles real provider load.

**In scope.**
- Wire `prefetch_messages` after `persist_messages` in:
  - `crates/gmail/src/sync/storage.rs`
  - `crates/graph/src/sync/stores.rs`
  - `crates/imap/src/sync_pipeline.rs` (or wherever the equivalent post-persist hook is)
- IMAP-specific: respect the per-folder session reuse so we don't open a new connection per attachment.
- Per-provider concurrency limits (likely 4 for Gmail/Graph, 1 per folder for IMAP).

**Out of scope.**
- IMAP partial-fetch optimization (BODY[part]<offset.length>) - useful eventually for very large attachments, but not v1.
- Reference-attachment handling on Graph - those are URL-only, not bytes; surface them as cloud links (handled by the cloud-attachments path) rather than pre-fetching.

**Touchpoints.**
- The three sync-storage files above.
- Possibly `crates/imap/src/client/sessions.rs` to add session reuse for attachment fetches inside the same folder.

**Exit criteria.**
- After a sync on each provider type, `attachment_blobs` rows with `tombstoned_at IS NULL` appear for that account (joined back to `attachments` via `content_hash`).
- IMAP attachment fetches reuse the existing folder session (no extra LOGIN/SELECT round-trips).

**Risks / open questions.**
- Gmail's batch endpoint vs N individual fetches - probably worth using batch when available for messages with multiple attachments.
- IMAP servers with strict concurrency limits (some only allow one connection per user). Keep concurrency conservative.

---

## Phase 6 - Eviction policy + GC + retention

**Goal.** Cache stays within the configured retention window. Phase 1 (logical eviction via tombstones, selected by message date) and Phase 2 (GC pack repack) are wired and tuned. Opened-files temp folder gets reaped.

**Entry criteria.**
- Phase 1-5 landed. Real cache pressure exists from observed usage (or a synthetic stress test).

**In scope.**
- Date-based candidate selection (Phase 1): `attachment_blobs JOIN attachments ON content_hash WHERE message.date < window_start`. Triggered at startup, after sync batches, and on window shrink.
- GC pass (Phase 2 pack repack) triggered on app idle when tombstone density crosses threshold (default: 25% of any single pack, or 10% of total cache bytes).
- Periodic cleanup of `<app-data>/opened_attachments/` based on the configured cleanup window.
- A "Clear attachment cache now" button in the Storage settings panel - tombstones every blob, then runs GC.

**Out of scope.**
- Heuristics for when to extend / shrink the window automatically. The window is user-controlled; no auto-adjustment based on disk pressure or access patterns.

**Touchpoints.**
- `crates/stores/src/attachment_pack.rs` - date-based eviction + `gc` methods.
- `crates/app/src/subscription.rs` - new periodic timer for GC.
- `crates/app/src/handlers/attachments.rs` - opened-files cleanup.

**Exit criteria.**
- After a sync that crosses the window edge, blobs older than `window_start` are tombstoned within one sync cycle.
- After GC fires on high-tombstone-density packs, the affected pack files are repacked and disk usage drops.
- `opened_attachments/` files older than the configured cleanup window are removed on startup.

**Risks / open questions.**
- GC during active sync writes - need a clear ordering so GC doesn't read a frame from a pack that's being rotated.
- A retention window shrink from "All" to "1 month" on a multi-GB cache could tombstone tens of thousands of blobs in one operation. The sweep needs to be chunked so it doesn't block the Service write path during the SQL update.

---

## Phase 7 - Squeeze measurement and tuning

**Goal.** Validate that squeeze is paying its way and tune defaults based on observed savings.

**Entry criteria.**
- Phase 1-5 landed and have been running on a real mailbox for a week or two.

**In scope.**
- Instrument the squeeze path to log compression ratios per mime type.
- Aggregate report (CLI tool or settings panel section) showing `original_bytes -> compressed_bytes` per type, savings percent, time spent.
- Bypass-rate calibration: log how often the signed-content bypass fires, broken out by mime. Zero hits across a populated mailbox suggests detection is broken; an unexpectedly high rate suggests overly aggressive sniffing.
- Decide whether to:
  - Adjust the default per-mime squeeze policy (e.g. always squeeze PDFs, never bother with already-compressed Office docs).
  - Make the lossy-JPEG toggle default-on if the win is large enough.
  - Skip squeeze on the hot path entirely if the savings are marginal.

**Out of scope.**
- Adding new compressors. Squeeze already covers the formats that matter; the v1 question is calibrating what we have.

**Touchpoints.**
- `crates/squeeze/src/lib.rs` - add a `metrics` callback or return enriched results.
- `crates/core/src/attachments/...` - thread the measurements through.
- A small CLI subcommand under `brokkr` or a standalone binary to emit the report.

**Exit criteria.**
- A real-mailbox report exists showing per-mime savings.
- Defaults updated based on the data.

**Risks / open questions.**
- May reveal that squeeze is net-negative for the sync hot path (CPU > storage savings on fast disks). If so, defer squeeze to a background "compaction" pass instead of running it inline.

---

## Phase 8 - Linux-specific `ErofsStore` backend (optional)

**Goal.** A second `BlobStore` impl on Linux backed by EROFS rolling images. Selected at runtime via `cfg(target_os = "linux")` (with a settings escape hatch to force `PackStore`). macOS and Windows continue on `PackStore`.

**Entry criteria.**
- Phase 1-7 landed; `PackStore` is in production with measured behavior.
- Real cache-pressure data exists from a Linux user (probably the project owner) running the v1 build long enough to know what we're optimizing.
- A decision has been made that the EROFS win is worth a second backend's maintenance cost. *This phase is optional - if `PackStore` is good enough on Linux, we don't ship it.*

**In scope.**
- New module `crates/stores/src/attachment_erofs.rs` implementing `BlobStore`.
- Rolling-image storage: `<app-data>/attachment_packs/data-NNNNNN.erofs`, ~256 MB each, never modified after bake.
- Staging area for in-flight writes (small flat-file directory or in-memory queue with periodic durability sync) until the next bake.
- Bake trigger: staging exceeds threshold (size or time-based), shell out to `mkfs.erofs` (or a library equivalent), drop the resulting image, clear staging.
- Index in SQLite: `attachment_blobs_erofs(content_hash PK, image_id, path_within_image, written_at, last_read_at, tombstoned_at)`. Distinct from the `PackStore` index since the location semantics differ (path-within-image vs offset-in-pack). Counts derived from `attachments`, same as `PackStore`.
- Eviction: tombstone individual blobs (refuse on read); whole-image delete only when *all* blobs in an image are tombstoned. No partial repack - that would mean rebaking, which is expensive and probably not worth it given the natural turnover.
- A migration tool to move existing v1 `PackStore` blobs into `ErofsStore` images on first run with the new backend (or, simpler: leave `PackStore` blobs in place and only put new writes through `ErofsStore` until eviction naturally drains the old store).

**Out of scope.**
- macOS and Windows. They stay on `PackStore`. Cross-platform parity comes from the trait, not from a single backend.
- Encryption (still tracked separately).
- Restic-format compatibility (orthogonal axis).

**Touchpoints.**
- New: `crates/stores/src/attachment_erofs.rs`, `crates/stores/src/lib.rs` (re-export), new SQL migration for `attachment_blobs_erofs`.
- `crates/stores/src/lib.rs` or wherever `BlobStore` is selected - runtime backend selector via `cfg(target_os)` + settings override.
- `Cargo.toml` - new optional dep on a Rust EROFS reader / writer crate, gated behind a `linux-erofs` feature flag.

**Exit criteria.**
- On Linux: cache writes route to `ErofsStore`; total disk usage measurably lower than equivalent `PackStore` workload (target: 20-40% reduction for typical mail attachment mixes).
- Reads from EROFS images measure within ~10% of `PackStore` reads (mmap'd random access should be as good or better).
- macOS / Windows builds untouched and continue using `PackStore`.

**Risks / open questions.**
- `mkfs.erofs` invocation: shell out (simple, depends on `erofs-utils` being installed) vs link a library (cleaner, harder to maintain, may have weak Rust ecosystem support). Probably shell out for v1, library when something solid exists.
- Staging durability vs bake cadence trade-off: small staging means frequent bakes (CPU + I/O cost); large staging means more bytes at risk if we crash mid-staging. Probably 64 MB staging cap, 5-minute idle timer.
- Image format compatibility across kernel versions. EROFS has been stable since ~5.4 mainline but feature additions are ongoing. Pin a minimum format-compat level.
- Whether to do a `PackStore` -> `ErofsStore` migration at all, or just let the old store age out via eviction. Migration is operationally cleaner; aging-out is less code.

---

## Out of phases (deliberately deferred)

These are real follow-ups, but each is a separate problem statement, not a phase of this work:

- **Calendar event attachments**. The orchestration is calendar-ready (the `ParentRef` enum from phase 1), but capturing attachments in calendar sync is its own piece of work. Will get its own problem statement when calendar attachments are prioritized.
- **Attachment chip widget unification**. Currently the reading pane and pop-out viewer have separate attachment-card widgets. Unifying them with the future cloud-link chips is a UI consolidation problem, not an attachment-storage problem.
- **Search inside attachment text** (PDF / OOXML extraction, FTS index). Substantial separate work; the cache being populated is a precondition but not the bulk of it.
- **Attachment encryption at rest**. Tracked under "Mail content stores not encrypted at rest" in TODO.md. Applies to body store, inline image store, and attachment cache uniformly - solve once across all three.
- **Backfill UI**. "Cache all attachments for this account now" button. Lazy fill + eager pre-fetch covers the steady-state need; a one-shot backfill is nice-to-have.
