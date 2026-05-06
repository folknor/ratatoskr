# The Service - Phase 6b Plan: OAuth two-step + `attachment.fetch` + global write-half lockdown

Companion to `phase-6a-plan.md`. Implements the second half of Phase 6 of `implementation-roadmap.md`.

## Revision history

**2026-05-06 - initial draft.** Authored at the same time as `phase-6a-plan.md` so the 6a / 6b boundary is explicit on day one. Phase 6c (calendar event mutations) is carved out into its own future plan; this document does not address it.

**2026-05-06 - second post-arch-review revision (small).** While reviewing 6c, claude pointed out that the transitive `cargo metadata` lockdown check fails today because `app -> cal -> service-state` is a real path (Phase 5 added `service-state` to `cal/Cargo.toml` for Service-side calendar sync). 6c relocates `cal::actions::*` Service-side and drops `cal` from `app/Cargo.toml`; that's the commit that closes the transitive path. 6b's lockdown task list correspondingly moves the transitive check to 6c, retaining only the direct-dep check + the constructor-visibility test for 6b.

**2026-05-06 - post-arch-review revision.** Two reviewers (claude + codex) flagged eight architectural issues, six of them duplicated independently. Major revisions: (1) OAuth latency design dropped - the dispatch loop already has `RequestParams::bypasses_admission()` (used by HealthPing/BootReady); `oauth.exchange_code` joins it. The "bounded queue depth + Busy" idea was also a duplicate of the existing `ServiceError::Backpressure`. (2) `attachment.fetch` ack reshaped to a read lease: returns `{ content_hash, size, lease_id }`; UI re-resolves through `PackStore::get_with_lease` which atomically pins the read against eviction/GC/repack. (3) `PackRuntime` framing dropped entirely - pack storage is content-addressed and global; there is no per-account dimension. Replaced with a single global kicked sweep mirroring `pinned_search.kick`'s shape, with a single-flight Tokio Mutex guard. (4) Orphan sweep split into two: whole-file orphan (boot pass, keyed on `pack_index`) drops files no row references; frame-orphan GC (kicked, requires repack) handles individual blob reclaim. (5) Pack store dependency made explicit - 6b is gated on attachments-roadmap Phase 1a landing; if it hasn't, 6b's IPC ack collapses to `(content_hash, size)` against the existing flat-file cache. (6) Lockdown enforcement layered on top of 6a's positive grep allow-list rather than replacing it; check is transitive (via `cargo metadata`) and enumerates the public bridge constructors in `service-state`. (7) Marker-file pattern documented as a shared helper (`crates/service/src/markers/`) with explicit step enumeration, CASCADE-last ordering, and idempotency rules; same helper hosts the sync, push, and 6b account-delete markers (and 6a's draft WAL marker if practical). (8) `oauth.exchange_code` and `account.create` reconciled: `oauth.exchange_code` is a thin entry point that runs the OAuth token exchange + userinfo fetch, then delegates to the shared `service::accounts::create_account_inner` helper - one create-path, two entry points. Smaller fixes: `Display` redacting test alongside `Debug`; task-2 IPC-deletion split into its own commit; eviction-on-cold-pull bound to prevent first-fetch stalls on existing 50 GB caches.

## Context

Phase 6a closes the small mechanical UI write surfaces and the encryption-key handle. The genuinely tricky surfaces remain:

- **OAuth coordination.** UI captures the redirect (it is the visible app); the Service handles the code-for-token exchange + persists the token. Today's flow runs entirely UI-side: `core::oauth::exchange_code` calls the provider's token endpoint, and the result is persisted via the same UI-side `with_write_conn` paths Phase 6a relocated. The temporary `oauth.refresh_request` shim from Phase 4 still lives - Service requests a fresh token from the UI when sync needs one. Phase 6b reverses that direction: Service refreshes its own tokens and the UI ships the redirect-captured auth code over IPC.
- **`attachment.fetch` IPC.** Cache-miss reads currently happen UI-side: open a thread, the reading pane reads the pack file directly. The pack-store reader has been unified for some time; what is still UI-side is the *cache-miss orchestration* - if the file is not local, the UI fetches from the provider. After Phase 6b that fetch flows through a Service IPC.
- **Eviction policy + GC.** The blob-store writer was relocated as a Phase 3 sync dependency. Eviction (LRU + size cap) and garbage collection (drop blobs whose `messages` rows have been deleted) need to run Service-side; the policy + cadence design is open.
- **Cross-store invariant pass extension.** Phase 3 introduced a boot-time invariant pass for SQLite + Tantivy + body store + inline image store. Phase 6b extends it to the attachment cache (pack files): an account-deletion that crashed mid-cleanup leaves orphaned packs the boot pass should reconcile.
- **Global write-half lockdown.** With OAuth and `attachment.fetch` migrated, no UI-side write call site remains except the calendar event mutations deferred to Phase 6c. Phase 6b makes the constructors of `WriteDbState`, `BodyStoreWriteState`, `InlineImageStoreWriteState`, and `SearchWriteHandle` unreachable from the `app` crate at compile time. The `cal::actions` write-surface escape stays as an explicit `Current Exception` until 6c.

## Scope

### In scope

- **OAuth code exchange (`oauth.exchange_code`).** UI captures the redirect, ships the auth code + redirect URI + PKCE verifier over IPC. Service exchanges the code for tokens against the provider's token endpoint, fetches userinfo to derive email (Google/Microsoft/JMAP all need a second round-trip for the email field), then delegates account creation to the same internal `service::accounts::create_account_inner` helper that 6a's `account.create` calls. The auth code is a one-shot bearer credential; wire types use the existing redacting wrapper for both `Debug` and `Display`. **Composition, not parallel path:** there is one `create_account_inner`; `oauth.exchange_code` is an OAuth-specific entry point that gathers credentials before calling it, not an alternate creation path.
- **Service-side OAuth refresh.** Service refreshes its own tokens during sync. The temporary `oauth.refresh_request` IPC from Phase 4 deletes itself (Service no longer needs the UI as a token broker). The UI-side `core::oauth::exchange_code` and `core::oauth::refresh_token` call sites get pruned where the Service-side path replaces them.
- **OAuth admission bypass.** `oauth.exchange_code` joins `RequestParams::bypasses_admission()` (today: `HealthPing`, `BootReady`). The original draft proposed a "bounded queue depth + Busy" mechanism; the arch review pointed out (a) the per-handler semaphore + admission cap already provides the moral equivalent, (b) `ServiceError::Busy` would duplicate the existing `ServiceError::Backpressure`, and (c) the actual hazard is post-admission queueing behind a 30 s `ActionSend` or a long `attachment.fetch`, which depth-checks on new arrivals don't fix. Bypass is the existing-shape, no-new-mechanism resolution.
- **`attachment.fetch` IPC for cache-miss reads with read-lease semantics.** Wire ack carries `{ content_hash, size_bytes, lease_id }`. UI uses `PackStore::get_with_lease(content_hash, lease_id)` to atomically resolve the read against eviction/GC/repack: the lease is a short-lived (default: 30 s) reader pin that the kicked GC and the repack path both honor. If the lease expires before the read completes (large attachment + slow disk), `PackStore::get_with_lease` returns `LeaseExpired` and the UI re-issues `attachment.fetch`. **No bytes over the IPC** (`phase-1.5-plan.md` backpressure policy); the lease is the new contract that makes "no bytes over IPC" safe under concurrent eviction.
- **Eviction policy.** LRU + total-size cap (default: 5 GB). Today's flat cache (`crates/stores/src/attachment_cache.rs`) is uncapped and tracks `last_accessed_at` per file; once attachments-roadmap Phase 1a lands the pack-store + `pack_index`, eviction tracks per-blob LRU there. Eviction runs on `pack.eviction_kick` (5-min cadence with 1 h staleness gate) plus on-demand from `attachment.fetch` if a fetch would push the cache past the cap. **Per-kick eviction work is bounded** (default: 200 MB reclaimed per kick) so a first cold-pull post-6b on an existing 50 GB cache does not stall - the cache reduces incrementally over the next hours' kicks rather than in one expensive synchronous burst.
- **Garbage collection.** Periodic sweep that drops blobs whose `messages` rows are gone. Same kick infrastructure as eviction (`pack.gc_kick`); runs on 5-min cadence with 24 h staleness gate. **Frame-orphan reclaim requires repack** in the post-Phase-1a world: a pack file holding 1 orphaned frame + 99 live frames cannot be deleted; the GC sweep marks the orphan as reclaimable in `pack_index` and a separate repack pass (long-running, idempotent) eventually rewrites the pack file without the orphans. Today's flat cache version is simpler: each cache file is one blob, GC deletes the file directly.
- **Cross-store invariant pass extension - whole-file orphan only.** `service::startup_invariants` gains a pack-file pass that drops files in `attachment_packs/` (or the flat cache directory pre-Phase-1a) which are not referenced by any `pack_index` row. **This is distinct from frame-orphan GC** - the boot pass handles "file exists, no DB row says so" (e.g., crashed mid-rotation); the kicked GC handles "DB row says so, but message is gone" (frame-level reclaim). Wording in the original draft conflated the two, which would have deleted live data on the boot pass.
- **Account-deletion crash recovery via shared marker helper.** `crates/service/src/markers/` (new module) hosts a generic marker-file helper - one schema, one drain helper, one Settled Pattern entry in `docs/architecture.md`. The account-deletion marker enumerates cleanup steps as a versioned list: body-store delete -> inline-image delete -> pack-cache unref + maybe-evict -> search index delete -> `accounts` row CASCADE. **CASCADE is always last**: once the row is gone, external stores cannot be reverse-mapped by `account_id`. Resume is idempotent step-by-step. The same helper hosts the existing sync markers + Phase 4 push markers; 6a's draft WAL is similar but is content-bearing (entries to replay, not steps completed) so it stays a separate file format.
- **Global write-half lockdown - layered enforcement.** Three checks together:
  - **Continue 6a's positive grep allow-list** at `crates/app/src/`: no `Db::with_write_conn`, no `Db::write_db_state`, no raw rusqlite write call sites.
  - **Cargo dependency check is transitive**: `cargo metadata` over the resolved graph asserts `app` does not depend on `service-state` *directly or transitively*. Direct-only is bypassed by inserting any intermediate crate.
  - **Service-state public surface enumerated**: `WriteDbState::from_arc`, `WriteDbState::from_db_state`, `WriteDbState::to_read_state`, the `BodyStoreWriteState` constructors at `body_store_write.rs`, `InlineImageStoreWriteState` constructors at `inline_image_store_write.rs`, and `SearchWriteHandle` construction at `search_write.rs:72` all flip to `pub(crate)`. Each is documented in the lockdown commit.
- **`cal::actions` exception update.** `docs/architecture.md` Current Exceptions explicitly carries the calendar event mutation escape until Phase 6c lands.

### Entry criteria

- **Phase 5 landed** (calendar/GAL relocation, IMAP cancellation depth).
- **Phase 6a landed** (small UI write-surface relocations + encryption-key handle + `Db::with_write_conn` + `Db::write_db_state` deleted).
- **Attachments roadmap Phase 1a landed** (pack store + `pack_index`). 6b's `attachment.fetch` ack shape (`content_hash, size_bytes, lease_id`) and `pack.eviction_kick` / `pack.gc_kick` plan-task wording assume the pack store exists. **If Phase 1a has not landed**, 6b reduces in scope: the IPC ack stays `{ content_hash, size_bytes, lease_id }` against the existing flat cache (`crates/stores/src/attachment_cache.rs`), the lease semantics still apply (lease pins the cache file against deletion), and the eviction/GC kicks operate against the flat directory. Frame-orphan repack is moot in the flat-cache case (each file is one blob). The plan resumes its full pack-store shape once Phase 1a lands.
- **Phase 4 `oauth.refresh_request` IPC** still in place at the start of 6b (deleted in task 2).

### Out of scope

- **Calendar event mutations.** Phase 6c (`docs/service/phase-6c-plan.md`). Phase 6b leaves `Db::create_calendar_event`, `Db::update_calendar_event`, `Db::delete_calendar_event` UI-side. The `cal::actions` write-surface escape stays in `docs/architecture.md` § Current Exceptions until 6c lands.
- **Settings UI for attachment caching policy.** The eviction parameters (size cap, age cap) are hard-coded defaults in 6b; a settings UI is downstream attachments-roadmap work, not Phase 6.
- **Calendar attachments.** Separate work from the `attachment.fetch` IPC; the existing pack store handles email attachments only.
- **Provider-specific OAuth quirks** (Microsoft tenant routing, Google offline-access scopes, etc.). The IPC is provider-neutral; per-provider handling stays in the provider crates' OAuth helpers.

## Architecture

### OAuth two-step shape

Today's flow:

```
UI redirect handler -> core::oauth::exchange_code (UI) -> provider token endpoint
   -> core::oauth::persist_tokens (UI) -> with_write_conn(...)
```

Phase 4 added `oauth.refresh_request` so Service-side sync could ask the UI for a fresh token (UI-side `refresh_token` runs, Service consumes the result). That direction reverses in 6b.

After Phase 6b:

```
UI redirect handler -> oauth.exchange_code IPC ->
   service::oauth::exchange_code (token-endpoint round-trip + userinfo round-trip)
      -> service::accounts::create_account_inner (shared with `account.create`)
      -> persist tokens

Service-side sync (any provider) -> service::oauth::refresh_for_account ->
   provider token endpoint -> persist
```

The IPC types live in `service-api/src/oauth.rs`:

- `OauthExchangeCodeParams { provider: ProviderKind, code: RedactedString, redirect_uri: String, code_verifier: String, scopes: Vec<String> }`. The `RedactedString` wrapper redacts both `Debug` and `Display` implementations so a stray `format!("{}")` in a log statement does not leak the auth code.
- `OauthExchangeAck { account_id: String, email: String, expires_in_secs: u32 }`. Token bytes never cross the IPC; Service writes them directly to the DB. The `email` field is returned because OAuth derives it from a userinfo round-trip the UI does not have visibility into.

**One create-path, two entry points.** The plan's earlier draft had `oauth.exchange_code` as a separate creation surface alongside `account.create`. The arch review flagged that "what makes a fully-formed account" would have to be maintained at two sites, breaking the make-the-right-thing-the-only-thing rule. Reconciled design: a single internal helper `service::accounts::create_account_inner(provider, email, encrypted_credentials, ...)`. `account.create` (Phase 6a) calls it for `Plaintext`/`Encrypted` envelope variants; `oauth.exchange_code` (this phase) calls it after running the OAuth-specific token + userinfo round-trips. Future "every account needs a default folder set" type changes update the helper, not the entry points.

**Admission bypass instead of priority lane.** OAuth servers expect the redirect-to-token-exchange round-trip in seconds, not minutes. The original draft proposed a "bounded queue depth + Busy" mechanism. The arch review pointed out that the dispatch loop already has the right tool: `RequestParams::bypasses_admission()` (`crates/service-api/src/request.rs:215`) is used today by `HealthPing` and `BootReady` to skip both the per-handler semaphore and the admission cap. `oauth.exchange_code` joins this list. Once admitted, OAuth runs alongside other in-flight handlers without queueing behind a 30 s `ActionSend` or a long `attachment.fetch`. The proposed `ServiceError::Busy` was a duplicate of the existing `ServiceError::Backpressure`; not adding either.

### `attachment.fetch` IPC + read-lease semantics

Wire shape:

```
AttachmentFetchParams { account_id, message_id, attachment_id }
AttachmentFetchAck { content_hash: String, size_bytes: u64, lease_id: u64, lease_expires_at_unix_ms: u64 }
```

The Service ensures the bytes are present (in the pack store post-Phase-1a, or in the flat cache pre-Phase-1a) before returning the ack. The ack carries a `lease_id` rather than a path/offset/length tuple, and the UI reads through `PackStore::get_with_lease(content_hash, lease_id)` which atomically resolves the read against eviction/GC/repack.

**Why a lease, not a path+offset?** The arch review flagged that returning `(pack_path, offset, length)` directly creates an undefined data race: between the ack and the UI's positional read, the eviction kick can drop the blob, the GC kick can mark it reclaimable, or (post-Phase-1a) the repack pass can rewrite the pack file with the blob at a new offset. None of these were specified, and any of them could panic the read. The lease is the explicit contract:

- **Lease lifetime:** 30 s (default). Long enough for a typical attachment open, short enough that a leaked lease does not pin storage indefinitely.
- **Eviction respects active leases:** the kicked GC and repack paths skip blobs whose `pack_index.active_leases > 0` (atomic counter, decremented when the UI calls `PackStore::release_lease(lease_id)` or when the lease expires).
- **Lease expiry returns `LeaseExpired`:** if the UI's read takes longer than 30 s (very large attachment, slow disk), `PackStore::get_with_lease` returns the expired error and the UI re-issues `attachment.fetch`. Fresh ack, fresh lease.
- **No bytes over the IPC** (`phase-1.5-plan.md` backpressure policy stays). The lease is what makes that policy safe under concurrent eviction.

The IPC is request-response; cancel-on-account-delete piggybacks the existing `cancel_and_await` flow (Phase 5 task 9 made the cancel IPC carry per-account run ids).

### Eviction + GC: single global kicked sweep, not a per-account runtime

The original draft framed pack eviction/GC as a `PackRuntime` mirroring `CalendarRuntime`'s per-account map + panic supervisor + cancel/run-id machinery. The arch review rejected that framing: pack storage is content-addressed and global. `attachment_blobs.content_hash` is the primary key with no account scope - cross-account deduplication is the entire point of attachments-roadmap Phase 1a. There is no "per-account run id" because there is no per-account state. Forcing the `CalendarRuntime` shape would import a per-account map, panic supervisor, run-completion correlation, and cancel APIs the runtime doesn't need.

Replaced with a single global kicked sweep, structurally closer to `pinned_search.kick` than `calendar.kick`:

- **Single-flight guard.** Module-level `static PACK_SWEEP_LOCK: Mutex<()>` prevents concurrent eviction/GC sweeps. Two `pack.eviction_kick` notifications back-to-back through `NOTIFY_CAP=4` would otherwise run duplicated reclaim work; same hazard the GAL handler addresses with its own `Mutex`.
- **Two kick handlers, one lock:**
  - `pack.eviction_kick` (5-min cadence with 1 h staleness gate). Reclaim is bounded to 200 MB per sweep so a first cold-pull post-6b on an existing 50 GB cache does not stall the request lane.
  - `pack.gc_kick` (5-min cadence with 24 h staleness gate). Marks frame-orphans as reclaimable in `pack_index`; the actual repack pass runs on its own staleness gate (default: 7 d) since it is expensive and idempotent.
- **Shutdown drain.** The consolidated drain (Phase 4) waits for any in-flight sweep before exiting - no more than 30 s for the bounded eviction work + 60 s for repack. Stale kicks during shutdown are dropped (notification class is `Drop`).
- **No `PackRuntime` type.** The two handlers in `crates/service/src/handlers/pack.rs` plus the static lock are the entire infrastructure. No type, no runtime, no constructor.

**Account-deletion crash recovery via shared marker helper.** The original draft asserted "today's account-deletion writes a marker file" - the arch review pointed out this is not true (`crates/core/src/account/delete.rs` runs orchestrate and returns; no marker). 6b is *introducing* the marker, not extending one. To avoid pattern accretion (sync markers, Phase 4 push markers, 6a draft WAL, now 6b account-delete), 6b lands a shared helper:

- **`crates/service/src/markers/`** - new module with a generic `MarkerFile<T: Serialize + DeserializeOwned>` type, a single drain-on-boot helper, and one Settled Pattern entry in `docs/architecture.md`. Sync markers and Phase 4 push markers migrate to use the helper in the same commit (or the next two), reducing four ad-hoc patterns to one.
- **Account-deletion marker** is a `MarkerFile<AccountDeletionState>` where the state enumerates cleanup steps in the order they execute:
  1. `body_store_delete` (delete body files for this account's messages)
  2. `inline_image_store_delete` (same for inline images, refcount-aware so shared hashes survive)
  3. `pack_cache_unref` (decrement `pack_index.refcount`; later GC reclaims orphaned frames)
  4. `search_index_delete` (drop Tantivy docs for this account)
  5. `accounts_row_cascade` (the SQLite `DELETE FROM accounts WHERE id = ?` that fires the schema CASCADE)

  **Step 5 is always last** because once the row is gone, external stores cannot be reverse-mapped by `account_id`. Steps 1-4 must each be idempotent (re-running drops nothing extra and produces the same end state). Resume on boot reads the marker, identifies the next un-completed step, runs forward.
- **6a's draft WAL is similar but not the same.** The WAL is content-bearing (entries to replay) where the markers are step-completed lists. They share a directory (`<data_dir>/markers/` and `<data_dir>/drafts.wal`) and the boot-drain phase but not the wire format. Plan does not unify them.

### Cross-store invariant pass extension - whole-file orphan only

`service::startup_invariants` already covers SQLite (Phase 3 task 11), body store, inline image store, and Tantivy (Phase 4). Phase 6b adds two passes that the arch review pointed out were conflated in the original draft:

- **Whole-file orphan sweep (boot pass).** Scan the pack directory (`attachment_packs/` post-Phase-1a, or the flat cache directory pre-Phase-1a); drop files whose path is **not referenced by any `pack_index` row**. The keying matters: a pack file holding 1 orphaned frame + 99 live frames does not get deleted on the boot pass - the boot pass deletes files that the index does not know about at all (e.g., a crash between fsync of the file and the index commit). Pre-Phase-1a, each cache file is one blob, so the index-keyed sweep collapses to "file in dir, no row points to it."
- **Pack-index reconciliation (boot pass).** `pack_index` rows whose underlying file is gone get deleted. Cheap; runs in the same boot phase.

**Frame-orphan reclaim is NOT a boot pass** - it is the kicked GC's responsibility. The original draft wording ("scan the pack directory; drop files with no corresponding `attachments` row") would have deleted live data because pack files are shared by many `attachments` rows. The two sweeps are now spelled out as distinct passes.

The boot pass runs inside `BootPhase::ConsistencyPass`, after the body / inline / search passes. Cost is bounded by directory size; expected runtime well under a second on typical caches.

### Global write-half lockdown - layered enforcement

The `service-state` crate today exports a number of public constructors that the arch review enumerated:

- `WriteDbState::from_arc` (`crates/service-state/src/lib.rs`)
- `WriteDbState::from_db_state` (same file, `:52`)
- `WriteDbState::to_read_state` (same file)
- `BodyStoreWriteState` constructors at `body_store_write.rs:41`
- `InlineImageStoreWriteState` constructors at `inline_image_store_write.rs:22`
- `SearchWriteHandle` construction at `search_write.rs:72`

After Phase 6b all of the above flip to `pub(crate)`. The arch review pointed out that the original draft's enforcement plan ("`crates/app/Cargo.toml` drops the `service-state` dependency") is already true today and would not prove the mutation gate - the actual leak today is `Db::write_db_state()` returning the writable connection wrapped as `ReadDbState` (closed in Phase 6a). 6b's enforcement is layered, not replacing 6a's:

1. **Continue 6a's positive grep allow-list** at `crates/app/src/`: no `Db::with_write_conn`, no `Db::write_db_state`, no raw rusqlite write call sites.
2. **Add a transitive Cargo dependency check** (sequenced post-6c, see note below). `cargo metadata --format-version=1 | jq` (or equivalent) over the resolved graph asserts that no path from `app` reaches `service-state`. Direct-only is bypassed by inserting any intermediate crate; transitive is the actual gate.

   **Sequencing note.** `crates/calendar/Cargo.toml` declares `service-state = { path = "../service-state" }` (Phase 5 made `cal::sync` Service-side, legitimate need for `WriteDbState`). `crates/app/Cargo.toml` declares `cal = { path = "../calendar" }`. The transitive path `app -> cal -> service-state` is real today and persists through Phase 6b. **Phase 6c removes it** by relocating `cal::actions::*` Service-side and dropping `cal` from the app's `Cargo.toml` (the only remaining `cal::*` use sites in app are the three action functions plus `CalendarEventInput`, which moves to `service-api` in 6c). The transitive check therefore lands as part of 6c's lockdown task, not 6b's. 6b lands the direct-dep check + the constructor-visibility test; 6c completes the lockdown by closing the transitive path.
3. **Add a constructor-visibility check.** A small Rust test in `crates/service-state/tests/` asserts that the enumerated constructors above are inaccessible from outside the crate. The test compiles a snippet that imports each, expects it to fail, and panics if any succeed. (Equivalent to a `compile_fail` doctest, but as an integration test for clarity.)

All three checks run in CI. A regression has to defeat all three.

### Architecture-doc update

Phase 6a's architecture-doc rewrite captured the post-6a state. Phase 6b's update is smaller:

- **Action service as mutation gate** § "Enforcement": replace the "global lockdown lands at Phase 6" text with "global lockdown landed at Phase 6b. The `app` crate no longer depends on `service-state`."
- **Current Exceptions** § `cal::actions`: keep until Phase 6c lands.
- **Settled Patterns** § "Service kick handlers": add `pack.eviction_kick` and `pack.gc_kick`.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

**0. Inventory + entry-criteria check.** Verify attachments-roadmap Phase 1a status (pack store + `pack_index`). If landed, the `attachment.fetch` ack and pack/index references stay as written; if not, scope reduces to the flat-cache shape. Survey today's UI-side OAuth call sites (`core::oauth::exchange_code`, `core::oauth::refresh_token`, persistence call paths). Confirm the `RedactedString` wrapper redacts both `Debug` and `Display`.

**1. Shared marker helper (`crates/service/src/markers/`).** Generic `MarkerFile<T>` type, drain-on-boot helper. Lift the existing sync markers (Phase 4) and Phase 4 push markers into it in the same commit so the new pattern lands with three real consumers, not one. Settled Pattern entry in `docs/architecture.md` follows in task 11.

**2a. `service::oauth::refresh` per-provider helpers.** `service/src/oauth/refresh.rs` plus per-provider routing. Sync paths consume the new helpers. The Phase 4 `oauth.refresh_request` IPC is still in place and unused after this commit but the UI-side OAuth refresh path is dead.

**2b. Delete the Phase 4 `oauth.refresh_request` IPC.** Separate commit so a regression in 2a can roll back without re-exposing the temporary IPC.

**3. Shared `service::accounts::create_account_inner` helper.** Extract the 6a `account.create` handler's body into the shared helper so 6b's `oauth.exchange_code` can call it. Re-route `account.create` through the helper. No external behavior change.

**4. `oauth.exchange_code` IPC.** Wire types in `service-api/src/oauth.rs` (with `RedactedString` Debug + Display redaction). Service handler in `service/src/handlers/oauth.rs` runs token-endpoint exchange + userinfo round-trip, then calls `create_account_inner`. UI-side `service_client.rs` async wrapper. UI redirect handler routes through the new IPC. **Add `OauthExchangeCode` to `RequestParams::bypasses_admission()`.** Mark the request timeout 30 s (provider token endpoints are slow under load).

**5. `attachment.fetch` IPC + read leases.** Wire types (ack carries `lease_id`). Service handler ensures the file is local + mints a lease before returning the ack. `PackStore::get_with_lease` + `release_lease` API in `crates/stores/`. UI-side cache-miss path replaces `core::attachment_cache::fetch_*` direct calls with the IPC. **Lease expiry path tested.**

**6. Eviction policy + `pack.eviction_kick`.** Single global sweep with `PACK_SWEEP_LOCK` single-flight guard. LRU + 5 GB size cap; per-kick reclaim bounded to 200 MB. UI-side `kick_pack_eviction` joins `Message::SyncTick` fan-out.

**7. Garbage collection + `pack.gc_kick`.** Frame-orphan marker pass (24 h staleness gate). Repack pass on its own staleness gate (7 d) - heavyweight, idempotent.

**8. Cross-store invariant pass extension.** Whole-file orphan sweep + `pack_index` reconciliation in `service::startup_invariants`. Distinct from frame-orphan GC; keyed on `pack_index`, not `attachments`.

**9. Account-deletion marker (uses task 1's helper).** Service handler writes the marker before step 1 of cleanup, updates after each step, removes after step 5. Boot drain replays unfinished cleanups. Each cleanup step is idempotent.

**10. Global lockdown.** Three commits' worth, but landed together:
   - 10a: flip `service-state` constructors to `pub(crate)`. Fix all the compile errors by routing through new IPC methods.
   - 10b: transitive `cargo metadata` check in CI.
   - 10c: constructor-visibility integration test in `crates/service-state/tests/`.

**11. `core::oauth` cleanup.** Remove the now-dead UI-side OAuth call sites and imports.

**12. `docs/architecture.md` Phase 6b delta.** Per § "Architecture-doc update" above. Updates `implementation-roadmap.md` Phase 6 entry to reflect 6b "LANDED" status.

## File-by-file changes

**New files:**
- `crates/service-api/src/oauth.rs` - OAuth wire types (`RedactedString` covers both Debug and Display).
- `crates/service-api/src/attachment.rs` - attachment-fetch wire types including `lease_id`.
- `crates/service-api/src/pack.rs` - `pack.eviction_kick` and `pack.gc_kick` notification declarations.
- `crates/service/src/handlers/oauth.rs` - `oauth.exchange_code` handler.
- `crates/service/src/handlers/attachment.rs` - `attachment.fetch` handler with lease-mint.
- `crates/service/src/handlers/pack.rs` - global eviction + GC kick handlers (with `static PACK_SWEEP_LOCK`).
- `crates/service/src/oauth/refresh.rs` - per-provider refresh helpers.
- `crates/service/src/accounts/create.rs` - shared `create_account_inner` helper (extracted from 6a's `account.create` handler in task 3).
- `crates/service/src/markers/` - shared marker-file helper module.
- `crates/stores/src/pack_lease.rs` (or extension to existing pack store crate) - `PackStore::get_with_lease` + `release_lease` API.
- `scripts/check_app_service_state_dep.sh` (or `.rs` test) - transitive cargo-metadata dep check for the lockdown.

**Modified files:**
- `crates/service-api/src/lib.rs` - module declarations.
- `crates/service-api/src/request.rs` - new `RequestParams` variants. `OauthExchangeCode` joins `bypasses_admission()`. OAuth timeout 30 s (token endpoint + userinfo); attachment.fetch timeout 60 s; pack-kick notifications inherit Drop class.
- `crates/service-api/src/notification.rs` - new `ClientNotification::PackEvictionKick` + `PackGcKick`.
- `crates/service/src/dispatch.rs` - new request + notification arms.
- `crates/service/src/startup_invariants.rs` - whole-file orphan sweep + `pack_index` reconciliation. Distinct from frame-orphan GC.
- `crates/service/src/boot.rs` - install marker-helper drain phase, account-deletion marker drain.
- `crates/service-state/src/lib.rs` - `WriteDbState::from_arc`, `from_db_state`, `to_read_state` flip to `pub(crate)`.
- `crates/service-state/src/body_store_write.rs` - constructors flip to `pub(crate)`.
- `crates/service-state/src/inline_image_store_write.rs` - same.
- `crates/service-state/src/search_write.rs` - same.
- `crates/service-state/tests/lockdown.rs` - new constructor-visibility integration test.
- `crates/service/src/sync_markers.rs` (etc.) - migrate existing marker callers to the new shared helper in task 1.
- `crates/app/src/service_client.rs` - new IPC wrappers.
- `crates/app/src/handlers/provider.rs` - `kick_pack_eviction` + `kick_pack_gc`.
- `crates/app/src/update.rs` - `Message::SyncTick` fan-out.
- `crates/core/src/oauth/` - dead UI-side call site removal in task 11.
- `crates/core/src/attachment_cache.rs` - cache-miss path now calls the IPC + uses `get_with_lease`.
- `docs/architecture.md` - Phase 6b delta + new Settled Pattern for marker helper.
- `docs/service/implementation-roadmap.md` - mark Phase 6b "LANDED".

## Code-comment requirements

1. **`crates/service/src/handlers/oauth.rs::handle_exchange_code`** must contain:
   - "OAuth code is a one-shot bearer credential. The wire-type wrapper redacts both `Debug` and `Display`; logging frameworks reach for both. After Phase 6b the auth code never reaches the UI beyond the redirect handler that captures it; the IPC ships the code straight to Service. The handler runs token-endpoint exchange + userinfo round-trip, then delegates account creation to `service::accounts::create_account_inner` - the shared helper that 6a's `account.create` also calls. One create-path, two entry points."

2. **`crates/service/src/handlers/oauth.rs`** module-level doc-comment:
   - "Phase 6b reverses the Phase 4 `oauth.refresh_request` direction. Phase 4 added a temporary IPC where Service-side sync asked the UI for a fresh token; Phase 6b makes Service refresh its own tokens via the per-provider helpers in `service::oauth::refresh`. The temporary IPC is deleted in a separate commit (task 2b) so a regression in 2a can roll back without re-exposing it. `oauth.exchange_code` joins `RequestParams::bypasses_admission()` - the same admission-bypass list as `health.ping` and `boot.ready` - so the OAuth round-trip is not queued behind heavy traffic."

3. **`crates/service/src/handlers/attachment.rs::handle_fetch`** must contain:
   - "Wire ack carries `{ content_hash, size_bytes, lease_id, lease_expires_at_unix_ms }`. Bytes never cross the IPC (phase-1.5-plan.md backpressure policy). The lease is the contract that makes this safe under concurrent eviction/GC/repack: UI calls `PackStore::get_with_lease(content_hash, lease_id)` which atomically resolves the read against the eviction kick. Lease lifetime defaults to 30 s. Expired leases return `LeaseExpired` and the UI re-issues the IPC."

4. **`crates/service/src/handlers/pack.rs` module-level doc-comment** must contain:
   - "Pack eviction and GC are global, not per-account, because pack storage is content-addressed (`attachment_blobs.content_hash` is the primary key, no account scope). The original Phase 6b draft framed this as a `PackRuntime` mirroring `CalendarRuntime`'s per-account shape; the arch-review revision dropped that framing. Single-flight is enforced by the module-level `PACK_SWEEP_LOCK` Mutex - same pattern the GAL handler uses for the same reason (`NOTIFY_CAP=4` would otherwise duplicate reclaim work). Per-kick reclaim is bounded to 200 MB so a first cold-pull on a 50 GB cache does not stall the request lane."

5. **`crates/service/src/markers/` module-level doc-comment** must contain:
   - "Shared marker-file helper: sync, push, and account-delete recovery markers all live here as `MarkerFile<T>`. Each marker carries a step-completed list serialised as JSON. Recovery on boot: read marker -> identify next un-completed step -> run forward. Each step must be idempotent. Account-delete steps are ordered: body -> inline -> pack-cache-unref -> search -> accounts row CASCADE; CASCADE is always last because external stores cannot be reverse-mapped by `account_id` once the row is gone."

6. **`docs/architecture.md` § "Action service as mutation gate"** new sentence:
   - "Phase 6b closed the global write-half lockdown via three layered checks: 6a's positive grep allow-list at `crates/app/src/`, a transitive `cargo metadata` check that no path from `app` reaches `service-state`, and a constructor-visibility integration test in `crates/service-state/tests/`. A regression has to defeat all three."

7. **`docs/architecture.md` § "Settled Patterns"** new entry:
   - "Service marker files (`crates/service/src/markers/`). Multi-step recovery for crash-safe operations (sync, push, account-delete). Each marker is a versioned `MarkerFile<T>` carrying the step-completed list. Boot drain reads each marker, runs forward from the next un-completed step. All steps must be idempotent. New marker types extend the helper rather than introducing parallel patterns."

## Test plan

### Unit tests

- Wire-type round-trips for `oauth`, `attachment`, `pack` modules.
- Pack-eviction LRU test: seed a pack with 100 MB of blobs, set a 50 MB cap, fire eviction; assert the oldest blobs are dropped first AND that no more than 200 MB is reclaimed per kick.
- Whole-file orphan boot pass: drop a pack file in `attachment_packs/` with no `pack_index` row; run boot pass; assert the file is deleted. Drop a pack file with one orphan frame and 99 live frames; run boot pass; assert the file is NOT deleted.
- Frame-orphan GC: insert a `pack_index` row whose `messages` row is gone; fire `pack.gc_kick`; assert the row is marked reclaimable. Run repack pass; assert the pack is rewritten without the orphan.
- Cross-store invariant pass: simulate a half-finished account deletion (marker file present, some external-store cleanup steps un-completed); boot with the pass enabled; assert the cleanup runs forward from the next un-completed step.
- `bypasses_admission` includes `OauthExchangeCode`: regression test that locks the list.

### Integration tests (in-process)

- `oauth_exchange_round_trips`: stub provider token endpoint + userinfo endpoint; UI-side IPC call returns ack with the derived email; row in `accounts` has the expected encrypted token bytes; the create-path went through `service::accounts::create_account_inner` (assert via test hook or by observing the same default-folder set the `account.create` path produces).
- `service_refreshes_own_token_on_sync`: stub provider token endpoint; trigger sync against an account whose token is about to expire; assert refresh ran Service-side and the IPC `oauth.refresh_request` was not invoked.
- `attachment_fetch_cache_miss`: clear the cache; trigger the IPC; assert ack carries a valid lease and the file is locally present.
- `lockdown_app_dep_check_transitive`: parse `cargo metadata` over the resolved graph; assert no path from `app` reaches `service-state`.
- `lockdown_constructor_visibility`: integration test in `crates/service-state/tests/lockdown.rs` that asserts the enumerated public constructors (`WriteDbState::from_arc`, `from_db_state`, `to_read_state`, plus the body / inline / search write-handle constructors) are inaccessible from outside the crate.
- `oauth_redacted_string_redacts_debug_and_display`: round-trip an `OauthExchangeCodeParams`; assert the auth code does not appear in either `format!("{:?}")` or `format!("{}")`.
- `attachment_fetch_lease_pins_blob_against_eviction`: in-process `PackStore`; mint a lease via `attachment.fetch`; trigger eviction; assert the leased blob survives. Release the lease; assert the blob is now reclaimable.
- `attachment_fetch_lease_expiry_returns_lease_expired`: mint a lease; sleep past expiry; call `get_with_lease`; assert `LeaseExpired`.
- `marker_helper_round_trips_step_list`: write marker -> simulate crash -> read marker -> assert next-unfinished-step matches.
- `account_delete_marker_resumes_after_crash_at_each_step`: parameterised over each step (1-5); kill mid-step; restart; assert cleanup completes idempotently.

### Real-subprocess smoke tests

- `service_subprocess_oauth_full_flow`: spawn Service with an OAuth-enabled stub account; UI ships an auth code via IPC; Service exchanges + persists; UI re-reads the row over the existing read path. Asserts no `oauth.refresh_request` IPC fires during the test, and that `oauth.exchange_code` does not queue behind a concurrent slow handler (validates the `bypasses_admission` wiring).
- `service_subprocess_pack_eviction_bounded`: seed a 50 GB cache; fire eviction kick; assert reclaim reduces the cache by no more than 200 MB per kick (not the entire excess in one sweep).
- `service_subprocess_attachment_fetch_lease`: real subprocess, real pack store; verify lease pinning across an eviction kick.

### Manual matrix updates

- OAuth flow end-to-end (Google, Microsoft) via real provider endpoints. Verify no auth code appears in any log output.
- Cache-miss attachment open in the reading pane (assert the user sees no UI freeze during the IPC round-trip).
- Account deletion mid-flight crash recovery (kill -9 the Service mid-cleanup; restart; assert the orphaned external-store data is reclaimed on the next boot).

## Open questions

- **Eviction size cap default.** 5 GB chosen as a starting point; the cache today is uncapped. Plan picks 5 GB; revisit after the first weeks of dogfooding.
- **Marker helper format.** JSON for readability vs binary cookie for write-once-ness. Plan picks JSON - the markers are small and the boot drain is the only consumer; readability wins.
- **Lease lifetime tuning.** 30 s default. A very large attachment (100 MB) over a slow disk could exceed it. The retry loop is correct (UI re-fetches), but if a hot path emerges we extend the default rather than proliferate per-call lifetimes.
- **Repack pass cadence.** 7 d default. The reclaim model assumes most cleanups are eventually consistent; if dogfooding shows pack files staying bloated for too long after deletes, the cadence drops.

## Verification (end-to-end)

- All three lockdown checks pass: 6a's positive grep allow-list, the transitive dep check, the constructor-visibility test.
- An OAuth login completes end-to-end without the UI invoking `core::oauth::exchange_code` directly. The auth code does not appear in any log output (Debug or Display).
- A token expires mid-sync and Service refreshes it without invoking `oauth.refresh_request`.
- An attachment cache-miss completes via `attachment.fetch`; the lease pins the blob against a concurrent eviction kick.
- A Service crash mid-account-delete is recoverable on next boot - the marker drives cleanup forward from the next un-completed step; the `accounts` row CASCADE only fires after external stores are clean.
- The whole-file orphan boot pass deletes pack files unreferenced by `pack_index` without touching live data; the kicked GC reclaims frame-orphans without deleting whole pack files prematurely.

## Promotion criteria

- All items in `In scope` landed.
- Calendar event mutations are the only remaining UI write surface, tracked in `docs/service/phase-6c-plan.md`.
- `docs/architecture.md` reflects the post-Phase-6b state and includes the new Settled Pattern entry for the marker helper.
- `phase-6b-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry; every code-comment requirement is present in the relevant file.
