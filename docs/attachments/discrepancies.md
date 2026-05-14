# Attachments - Discrepancies

Companion to `problem-statement.md` and `implementation-roadmap.md`. Tracks every piece of work that the attachments project deferred, left optional, or noted as a separate follow-up after Phases 1-9 and the Phase 7/8/9 review batch landed.

Anything not listed here is in the codebase. Anything listed here is not.

## Deferred from landed phases

### Phase 7 follow-up: Gmail batch attachment endpoint

Roadmap-noted under Phase 7 "Still deferred". The per-attachment `users.messages.attachments.get` round-trip is cheap enough that no measurement has shown a need for `messages.batchGet`-style batching. Pick up when sync latency on attachment-heavy Gmail accounts becomes a complaint, or when prefetch backfill of a long retention window measurably stalls.

### Phase 9: measurement and data-driven tuning

Squeeze integration landed at the PackStore-write boundary in Phase 9; everything that turns log lines into a decision is deferred until a real mailbox has been running on the build long enough to produce data.

- Aggregate per-mime measurement table. The `log::info` line emitted per successful compress is the substrate; the aggregate report has no consumer yet.
- CLI report (under `brokkr` or standalone) showing `original_bytes -> compressed_bytes` per mime, savings percent, time spent.
- `Unchanged`-rate logging, broken out by mime. `crates/squeeze/README.md` documents passthrough for files already small or without compressible content; a per-mime count calibrates whether the heuristic matches what the mailbox actually contains.
- Default tuning decisions:
  - Should `allow_lossy_compression` default-on?
  - Should some mimes be skipped entirely (e.g. already-compressed Office docs)?
  - Should squeeze move off the hot path entirely on fast disks?
- Batched fsync (originally a Phase 2 deferral). PackStore currently fsyncs per frame. The "every N frames or M ms" batching pattern from the original problem statement only matters if measurements show fsync is a meaningful chunk of write-path cost; re-evaluate alongside the squeeze measurement work.

## Optional phase

### Phase 10: Linux ErofsStore backend

Entirely optional. Only ships if PackStore proves insufficient on Linux under real cache-pressure data, and if a second backend's maintenance cost is judged worth the disk-usage win.

Work it would entail:

- Extract a `BlobStore` trait. Phase 2 deliberately ducked trait extraction (design-by-future-use with one impl); Phase 10 is the future use. Rename `&PackStore` references in `attachment_materialize.rs`, the boot path, and the account-delete tombstone loop to `&dyn BlobStore` or a generic.
- New `crates/stores/src/attachment_erofs.rs` implementing the trait.
- Rolling-image storage under `<app_data>/attachment_packs/data-NNNNNN.erofs`, ~256 MB each, never modified after bake.
- Staging area for in-flight writes (flat-file directory or in-memory queue with periodic durability sync) until the next bake.
- Bake trigger: staging exceeds threshold (size or time-based), shell out to `mkfs.erofs` or link a library equivalent, drop the resulting image, clear staging.
- New `attachment_blobs_erofs` SQLite index, distinct from `PackStore`'s because location semantics differ (`image_id`, `path_within_image` rather than `pack_file_id`, `offset`, `length`).
- Eviction: tombstone individual blobs (refuse on read); whole-image delete only when *all* blobs in an image are tombstoned. No partial repack.
- Migration policy decision: move existing `PackStore` blobs into `ErofsStore` images, or leave PackStore blobs in place and route only new writes through ErofsStore until eviction naturally drains the old store.
- Cargo feature flag `linux-erofs`, runtime backend selector via `cfg(target_os = "linux")` plus a settings escape hatch to force `PackStore`.
- macOS and Windows stay on PackStore unconditionally.

## Test gaps deferred from the Phase 7/8/9 review batch

- **IMAP folder-batch "one LOGIN + one SELECT per folder" assertion.** Verifies the Phase 7 IMAP session-reuse optimization end-to-end. Blocked on saehrimnir: its cross-protocol `RequestLog` records each IMAP command but has no `connection_id` / `session_id` field, so the harness can count LOGIN / SELECT / UID-FETCH events globally but cannot distinguish "5 sessions x 4 SELECTs" from "1 session x 20 SELECTs". Saehrimnir-side fix: either add a per-connection id (allocated in `serve_connection`) into every IMAP `RequestEntry.detail`, or add a dedicated session-stats accumulator + `GET /test/imap/session-stats`. The behavior under test is an efficiency optimization, not a correctness invariant, so the gap is acceptable until one of those lands.
- **Cross-account shared-blob deletion harness.** Verifies Phase 4's `AccountDeletionStep::AttachmentCache` only tombstones unshared blobs when one account is deleted and a second account still references the same `content_hash`. **Not blocked on saehrimnir** - the fixture model already supports multi-account (every resource carries `account_id`; per-protocol scoping routes by authenticated credential against a single endpoint, not by distinct base URL). The harness can declare two `[[account]]` entries with the same blob bytes and issue different credentials against the same `RATATOSKR_TEST_*_ENDPOINT`. Behavior remains verified by code review only; the script is unbuilt, not blocked.

## Out of phases (separate problem statements)

Each of these is its own piece of work, not a phase of the attachments project. Listed here so they aren't forgotten.

- **Calendar event attachments.** The orchestration layer is calendar-ready (the `ParentRef` enum was shaped with `Message { account_id, message_id }` and `Event { account_id, event_id }` from the start), but capturing attachments during calendar sync, persisting Graph `event.attachments[]`, and surfacing them on event detail are unbuilt. The attachments table is currently keyed `(account_id, message_id)` and assumes mail.
- **Search inside attachment text** (PDF / OOXML text extraction, FTS index). Owned by `docs/architecture.md` § "Text extraction pipeline". The cache being populated is a precondition; the extraction and indexing pipeline is the bulk of the work.
- **Attachment encryption at rest.** Tracked in `TODO.md` under "Mail content stores not encrypted at rest". Applies uniformly to the body store, inline image store, and attachment cache; solve once across all three rather than per-store. Convergent encryption (key derived from the plaintext content hash) is the design that preserves cross-account dedup, at the cost of semantic security against known-plaintext attackers. PackStore's frame format already reserves the `nonce | ciphertext | tag` payload region so the on-disk wire shape is forward-compatible.

## UI work (user's separate work)

Backend wired, frontend not. The Phase 8c note "UI is the user's separate work" applies to all three.

- **Clear-cache button in Storage settings.** `attachment.clear_cache` IPC, `PackStore::tombstone_all_live`, the GC chain, and the `GcTrigger::ClearCache` notification variant all landed. No UI affordance triggers them yet.
- **Attachment chip widget unification.** The reading pane and pop-out viewer have separate attachment-card widgets. Unifying them, and folding in the future cloud-link chips from `cloud-attachments.md`, is a UI consolidation problem; the attachment-storage subsystem doesn't block it.
- **Backfill UI.** "Cache all attachments for this account now" button. Lazy fill plus eager pre-fetch covers the steady-state need; a user-visible one-shot backfill is nice-to-have. The PrefetchRuntime backfill driver already exists; the work is exposing it.
