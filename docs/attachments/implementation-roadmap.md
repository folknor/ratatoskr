# Attachments - Implementation Roadmap

Companion to `problem-statement.md`. Each phase below is intended as a **separate `EnterPlanMode` session** that produces a focused implementation plan, lands as one or a small handful of commits, and unblocks the next phase. Nothing here is a complete plan in itself - the goal is to chart the order of attack and keep us from accidentally building things in the wrong sequence.

This document is a sketch. Phase scope, interfaces, and risks will firm up when each phase enters its own planning session.

**Cross-document dependency.** Several phases depend on **the
Service** (`docs/architecture.md` under "Service process model").
Phase 2 onward runs inside the Service, and Phase 3's cold path needs
the `attachment.fetch` IPC. Phases 1a and 1b are pure library work and
can land before any Service work begins.

## How to read this

- **Goal** - one-sentence outcome.
- **Entry criteria** - what must already exist for this phase to start cleanly.
- **In scope / Out of scope** - hard boundaries for the phase.
- **Touchpoints** - files / modules likely to change. Indicative, not exhaustive.
- **Exit criteria** - observable evidence the phase is complete.
- **Risks / open questions** - unknowns to resolve during the planning session.

---

## Phase 1a - Pack store

**Goal.** A self-contained pack-file blob store under `crates/stores/src/attachment_pack.rs` supporting content-addressed put / get / unref / GC, with a SQLite index, full crash safety, and a `recover` path that rebuilds the index from pack tails.

**Entry criteria.**
- Problem statement approved.
- Format spec finalized (frame layout, tail layout, version byte semantics, tombstone log format).
- Decision made on chunking-or-not (current proposal: no chunking in v1; revisit after squeeze measurement lands in Phase 7).

**In scope.**
- `crates/stores/src/attachment_pack.rs` with `PackStore::open`, `put`, `get`, `unref`, `gc`, `recover`.
- New SQLite migration: drop unused `attachments.local_path / cached_at / cache_size` columns, add `attachment_blobs` table with `(content_hash PK, pack_file_id, offset, length, refcount, written_at, last_read_at)`.
- Frame writer: 4-byte magic + 4-byte length + 8-byte xxh3_64 of payload + payload, batched fsync, atomic index update via SQLite transaction.
- Pack tail writer with version + frame_count + crc.
- Tombstone log format and read-side enforcement (refuse to return tombstoned data even on stale index hit).
- Pack rotation when `PACK_TARGET_SIZE` (256 MB default) exceeded.
- One-writer-per-pack mutex; lock-free positional reads.
- Unit tests: round-trip, dedup (two `put`s of identical bytes append once), refcount lifecycle, tombstone honored on read, recover from missing index, recover from torn last frame, GC on mixed live/dead pack.

**Out of scope.**
- Encryption (deferred - tracked under "Mail content stores not encrypted at rest" in TODO.md).
- Restic format compatibility (deferred to v2).
- Chunking large blobs into smaller frames (out of v1 unless squeeze measurement says otherwise).
- Anyone calling the store. This phase is just the storage layer.

**Touchpoints.**
- New: `crates/stores/src/attachment_pack.rs`, `crates/stores/src/lib.rs` (re-export).
- New SQL migration touching `crates/db/src/db/schema/02_mail.sql`.
- Old `crates/stores/src/attachment_cache.rs` - kept for now but no longer called by the new path; deleted in a follow-up commit once nothing references it.

**Exit criteria.**
- `brokkr check` clean.
- Test suite covers the 8 scenarios above.
- A small benchmark exists (insert 10k 4 KB blobs + 1k 1 MB blobs, time per op, total disk usage, inode count) for future regression checks.

**Risks / open questions.**
- Whether to put the open pack in its own file or always rotate on startup. Trade-off: in-place open-pack means recovery has to scan; always-rotate means more pack files. Probably in-place; rotation only on size cap.
- fsync batching cadence. Too aggressive = slow writes; too lax = larger crash recovery scan. Probably "every 16 frames or 100 ms, whichever first."
- xxh3_64 collision risk at our scale. Birthday-paradox math: at 10M blobs, P(any collision) ≈ 0.000027. Acceptable. If we want stronger guarantees, upgrade to xxh3_128 (still fast, 128 bits = effectively zero collision risk forever).

---

## Phase 1b - Core orchestration

**Goal.** A self-contained `core::attachments` module that, given `(account_id, message_id, attachment_id)`, returns the bytes - reading from the pack store or the inline image store first, fetching from the provider on miss, squeezing, hashing, writing to the pack store, and updating the DB row.

**Entry criteria.**
- Phase 1a landed. `PackStore` exists and is tested.

**In scope.**
- `crates/core/src/attachments/mod.rs` with `fetch_or_load`, `prefetch_one`, `prefetch_message`, `prefetch_messages`.
- A `ParentRef` enum (`Message { ... } | Event { ... }`) so calendar can plug in later. Only the `Message` variant is wired in this phase.
- Unit tests for: pack-store hit fast path, inline-image-hit fast path, full fetch + squeeze + put + update path (mocked provider).
- Wiring `squeeze::compress` into the write path (lossless defaults).
- Touch-on-read: bump `attachment_blobs.last_read_at` when `PackStore::get` succeeds.

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

## Phase 2 - Sync-time pre-fetch (JMAP first)

**Goal.** New JMAP messages get their attachments cached during sync, respecting per-account policy and a size threshold.

**Entry criteria.**
- Phase 1a + 1b landed.
- `core::attachments::prefetch_messages(...)` exists and is tested.
- The Service hosts the JMAP sync path (Service roadmap phase that relocates sync into the Service has shipped).

**In scope.**
- After `persist_messages` in `crates/jmap/src/sync/storage.rs` writes attachment metadata (now running inside the Service), invoke `prefetch_messages` as a Service-internal task.
- Per-account policy reads from settings (toggle + threshold). Defaults if unset (true / 25 MB).
- Per-message size cap (skip if total exceeds 100 MB).
- Failures logged, never block sync completion.
- Progress events through the existing `ProgressReporter`, surfaced to the UI as Service notifications on the existing sync-progress channel.

**Out of scope.**
- Other providers (phase 5).
- Settings UI (phase 4) - read raw values from the prefs table for now.
- Backfill of existing messages.

**Touchpoints.**
- `crates/jmap/src/sync/storage.rs` - call site.
- `crates/db/src/db/queries.rs` or a dedicated prefs helper - read pre-fetch policy.
- `crates/core/src/progress.rs` - new event variants if needed (`AttachmentCached { message_id, attachment_id, bytes }`).

**Exit criteria.**
- After a JMAP sync, `SELECT COUNT(*) FROM attachment_blobs WHERE refcount > 0` increases.
- Status bar shows progress events during pre-fetch.
- Pre-fetch failures don't break sync; a retried sync re-attempts the same attachments.

**Risks / open questions.**
- Hammering the JMAP server with N-attachment requests in parallel. Need a semaphore (probably 4 concurrent fetches per account).
- Sync completion timing: dispatch is fire-and-forget, but the sync UI today says "sync complete" the moment metadata lands. Decide whether pre-fetch needs its own progress separate from sync state.

---

## Phase 3 - Open / Save / Save All wiring

**Goal.** The three buttons in the reading pane and pop-out viewer actually work, online or offline, on real or stub-cached attachments.

**Entry criteria.**
- Phase 1a + 1b landed (`fetch_or_load` exists, backed by `PackStore`).
- The Service exposes the `attachment.fetch` IPC method (covers the cold path).
- Phase 2 *helpful but not required* - phase 3 works equally well in fetch-on-click mode if no pre-fetch has happened.

**In scope.**
- New shared module `crates/app/src/handlers/attachments.rs` with `handle_open_attachment`, `handle_save_attachment`, `handle_save_all_attachments`.
- Hot-path: UI looks up `attachment_blobs` via direct read, reads positionally from the pack file. No IPC.
- Cold-path: UI sends `attachment.fetch` to the Service; Service runs `fetch_or_load`; UI receives bytes (or just the resulting `content_hash`, then re-reads from disk).
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

**Goal.** Users can configure pre-fetch behavior, cache size cap, and squeeze policy from settings.

**Entry criteria.**
- Phase 1-3 landed. Pre-fetch and Open/Save read their config from prefs - settings UI just wraps the prefs.

**In scope.**
- Per-account section ("Mail" or new "Storage" tab on the Account editor):
  - `Cache attachments for offline use` (toggle).
  - `Pre-fetch threshold` (slider, 1 - 100 MB).
- Global settings, new "Storage" section (sibling to "Notifications", "Composing"):
  - `Attachment cache size cap` (slider, 1 - 50 GB; default raised from current 500 MB to 5 GB as part of this work).
  - `Compress cached attachments` (toggle, default on).
  - `Allow lossy compression (JPEG re-encoding)` (toggle, default off).
  - `Cleanup opened-files temp folder after N days` (slider, default 7).
  - Live "Cache currently using X.Y GB of Z GB" readout below the slider.

**Out of scope.**
- A "Cache all attachments now" backfill button (deferred indefinitely - lazy fill plus eager pre-fetch covers it).
- A "Clear cache now" button (probably wanted but defer to a follow-up phase).

**Touchpoints.**
- `crates/app/src/ui/settings/tabs/...` - new section.
- `crates/app/src/ui/settings/types/...` - new pref keys + bootstrap snapshots.
- `crates/db/src/db/queries.rs` - getters/setters for the new prefs.
- `crates/stores/src/attachment_pack.rs` - cap is already read from `attachment_cache_max_mb`; default raised here.

**Exit criteria.**
- Settings persist across restarts.
- Changing the cache cap triggers Phase 1 eviction in the pack store.
- Disabling pre-fetch on an account stops new attachments from being downloaded by sync (fetch-on-click still works).

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
- After a sync on each provider type, `local_path IS NOT NULL` rows appear for that account.
- IMAP attachment fetches reuse the existing folder session (no extra LOGIN/SELECT round-trips).

**Risks / open questions.**
- Gmail's batch endpoint vs N individual fetches - probably worth using batch when available for messages with multiple attachments.
- IMAP servers with strict concurrency limits (some only allow one connection per user). Keep concurrency conservative.

---

## Phase 6 - Eviction policy + GC + retention

**Goal.** Cache stays bounded under realistic usage. Phase 1 (logical eviction via tombstones) and Phase 2 (GC pack repack) are wired and tuned. Opened-files temp folder gets reaped.

**Entry criteria.**
- Phase 1-5 landed. Real cache pressure exists from observed usage (or a synthetic stress test).

**In scope.**
- LRU candidate selection by `attachment_blobs.last_read_at` after sync batches (Phase 1 logical eviction).
- GC pass (Phase 2 pack repack) triggered on app idle when tombstone density crosses threshold (default: 25% of any single pack, or 10% of total cache bytes).
- Size-aware skip in pre-fetch: if pre-fetching one item would force more than X% of the cache to be evicted in one go, skip the item rather than thrash.
- Periodic cleanup of `<app-data>/opened_attachments/` based on the configured retention window.
- A "Clear attachment cache now" button in the Storage settings panel - tombstones every blob, then runs GC.

**Out of scope.**
- Per-account cache quotas. The cap is global; per-account enforcement would need substantial rework of the eviction selector.

**Touchpoints.**
- `crates/stores/src/attachment_pack.rs` - `evict_lru` and `gc` methods.
- `crates/app/src/subscription.rs` - new periodic timer for GC.
- `crates/app/src/handlers/attachments.rs` - opened-files cleanup.

**Exit criteria.**
- A 24-hour idle test with synthetic cache churn shows cache size stays at or below cap; GC reclaims space when triggered.
- `opened_attachments/` files older than the retention window are removed on startup.

**Risks / open questions.**
- Whether eviction during pre-fetch is acceptable (could thrash if we're constantly evicting one item to make room for the next). Probably need a "headroom" buffer (e.g. evict to cap minus 10%).
- GC during active sync writes - need a clear ordering so GC doesn't read a frame from a pack that's being rotated.

---

## Phase 7 - Squeeze measurement and tuning

**Goal.** Validate that squeeze is paying its way and tune defaults based on observed savings.

**Entry criteria.**
- Phase 1-5 landed and have been running on a real mailbox for a week or two.

**In scope.**
- Instrument the squeeze path to log compression ratios per mime type.
- Aggregate report (CLI tool or settings panel section) showing `original_bytes -> compressed_bytes` per type, savings percent, time spent.
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
- Index in SQLite: `attachment_blobs_erofs(content_hash PK, image_id, path_within_image, refcount, written_at, last_read_at)`. Distinct from the `PackStore` index since the location semantics differ (path-within-image vs offset-in-pack).
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
