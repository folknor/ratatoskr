# The Service - Phase 6b Plan: OAuth two-step + `attachment.fetch` + global write-half lockdown

Companion to `phase-6a-plan.md`. Implements the second half of Phase 6 of `implementation-roadmap.md`.

## Revision history

**2026-05-06 - post-6a-mid-session revision.** Three reality-check findings forced a substantive scope revision:
- **Pack store / Phase 1a is not landed.** `crates/stores/src/attachment_cache.rs` is still the flat hash-keyed file cache (`attachment_cache/<content_hash>`); there is no `attachment_pack.rs`, no `PackStore`, no `attachment_blobs` table, no `pack_index` in code. The previous "if landed... if not, scope reduces" branch resolves to flat-cache only. Pack-aware work (frame-orphan GC, repack, lease semantics tied to in-pack offsets) defers to a future "6b extension" that lands alongside Phase 1a/1b.
- **`oauth.refresh_request` IPC was never actually shipped.** `crates/service/src/push.rs:29` documents that the Phase 4 roadmap entry's IPC was removed during Phase 4 close-out because refresh is purely DB+HTTPS, both Service-internal. The previous plan's tasks 2a/2b (introduce per-provider helpers + delete the IPC) collapse to verification + a single cleanup commit. Sync-time refresh already happens Service-side via per-provider `ensure_valid_token` helpers (jmap/graph/gmail/imap).
- **6a is mid-session, not landed.** `account.delete`, `internal.read_bootstrap_snapshots` + `internal.encrypt_for_storage` + `internal.decrypt_for_storage`, draft WAL, pinned-search per-row CRUD, and the closing commit (`Db::with_write_conn` deletion + CI lockdown script + `docs/architecture.md` rewrite) are explicitly deferred to a 6a-part-2 session. 6b's entry criteria gain that closing commit as a hard prereq; 6b's lockdown task ("Continue 6a's positive grep allow-list") becomes literally true after 6a-part-2 closes.

Other revisions: read-lease design dropped to a simpler "open-fd-survives-unlink" guarantee on the flat cache (no lease IDs, no `PackStore::get_with_lease` API; the read fd is the pin); `PACK_SWEEP_LOCK` renamed `ATTACHMENT_SWEEP_LOCK` to match the flat-cache naming; eviction unit changes from "200 MB reclaimed per kick (frame-level)" to "delete N oldest files until under cap or 200 MB freed, whichever first"; cross-store invariant pass simplifies to "files in `attachment_cache/` dir not referenced by any `attachments.content_hash`"; account-deletion marker work needs 6a-part-2's `account.delete` to land first (so the Service holds the deletion); the eviction/GC distinction collapses to a single sweep on the flat cache (deleting a file frees both the LRU-evicted blob and the GC-reclaimable blob - same operation). Pack-aware sections preserved as forward-references in a new "When Phase 1a lands" sub-section so the future revision pass starts from a clear baseline.

**2026-05-06 - initial draft.** Authored at the same time as `phase-6a-plan.md` so the 6a / 6b boundary is explicit on day one. Phase 6c (calendar event mutations) is carved out into its own future plan; this document does not address it.

**2026-05-06 - second post-arch-review revision (small).** While reviewing 6c, claude pointed out that the transitive `cargo metadata` lockdown check fails today because `app -> cal -> service-state` is a real path (Phase 5 added `service-state` to `cal/Cargo.toml` for Service-side calendar sync). 6c relocates `cal::actions::*` Service-side and drops `cal` from `app/Cargo.toml`; that's the commit that closes the transitive path. 6b's lockdown task list correspondingly moves the transitive check to 6c, retaining only the direct-dep check + the constructor-visibility test for 6b.

**2026-05-06 - post-arch-review revision.** Two reviewers (claude + codex) flagged eight architectural issues, six of them duplicated independently. Major revisions: (1) OAuth latency design dropped - the dispatch loop already has `RequestParams::bypasses_admission()` (used by HealthPing/BootReady); `oauth.exchange_code` joins it. The "bounded queue depth + Busy" idea was also a duplicate of the existing `ServiceError::Backpressure`. (2) `attachment.fetch` ack reshaped to a read lease: returns `{ content_hash, size, lease_id }`; UI re-resolves through `PackStore::get_with_lease` which atomically pins the read against eviction/GC/repack. (3) `PackRuntime` framing dropped entirely - pack storage is content-addressed and global; there is no per-account dimension. Replaced with a single global kicked sweep mirroring `pinned_search.kick`'s shape, with a single-flight Tokio Mutex guard. (4) Orphan sweep split into two: whole-file orphan (boot pass, keyed on `pack_index`) drops files no row references; frame-orphan GC (kicked, requires repack) handles individual blob reclaim. (5) Pack store dependency made explicit - 6b is gated on attachments-roadmap Phase 1a landing; if it hasn't, 6b's IPC ack collapses to `(content_hash, size)` against the existing flat-file cache. (6) Lockdown enforcement layered on top of 6a's positive grep allow-list rather than replacing it; check is transitive (via `cargo metadata`) and enumerates the public bridge constructors in `service-state`. (7) Marker-file pattern documented as a shared helper (`crates/service/src/markers/`) with explicit step enumeration, CASCADE-last ordering, and idempotency rules; same helper hosts the sync, push, and 6b account-delete markers (and 6a's draft WAL marker if practical). (8) `oauth.exchange_code` and `account.create` reconciled: `oauth.exchange_code` is a thin entry point that runs the OAuth token exchange + userinfo fetch, then delegates to the shared `service::accounts::create_account_inner` helper - one create-path, two entry points. Smaller fixes: `Display` redacting test alongside `Debug`; task-2 IPC-deletion split into its own commit; eviction-on-cold-pull bound to prevent first-fetch stalls on existing 50 GB caches.

## Context

Phase 6a's first session shipped 8 small UI write surfaces; a planned 6a-part-2 closes the remaining 5 deferred surfaces (`account.delete`, `internal.read_bootstrap_snapshots` + `internal.encrypt_for_storage` + `internal.decrypt_for_storage`, draft WAL, pinned-search per-row CRUD, OAuth re-auth token persist) plus the closing commit (delete `Db::with_write_conn` / `Db::with_write_conn_sync` / `Db::write_db_state`, land the CI lockdown script, rewrite `docs/architecture.md`). 6b runs after 6a-part-2. The genuinely tricky surfaces remain:

- **OAuth coordination.** UI captures the redirect (it is the visible app); the Service should handle the code-for-token exchange + persist the token. Today's flow runs entirely UI-side: `rtsk::oauth::authorize_with_provider` calls the provider's token endpoint from `add_account/state.rs:648` and `add_account/oauth.rs:106`, and the result is persisted via the `account.create` IPC (6a) or, on re-auth, via the still-UI-side `with_write_conn` path at `add_account/oauth.rs:149` and `add_account/state.rs:514` (slated for 6a-part-2). The previously planned Phase 4 `oauth.refresh_request` IPC was never actually shipped (`crates/service/src/push.rs:29` documents the removal during Phase 4 close-out: refresh is purely DB+HTTPS, both Service-internal). Service-side per-provider refresh helpers already exist via `ensure_valid_token` in jmap/graph/gmail/imap. **6b's residual work is the code-exchange direction, not refresh** - move the token-endpoint round-trip and userinfo round-trip Service-side via `oauth.exchange_code`.
- **`attachment.fetch` IPC.** Cache-miss reads currently happen UI-side: open a thread, the reading pane reads the cache file directly. The current cache (`crates/stores/src/attachment_cache.rs`) is a flat hash-keyed file cache (`attachment_cache/<content_hash>`); each blob is one file. **Phase 1a (pack store + `pack_index` + `attachment_blobs` table) has not landed**, so the cache is still flat. After Phase 6b the cache-miss fetch flows through a Service IPC against the flat cache; lease/frame-orphan/repack semantics defer to a future revision pass that lands alongside Phase 1a.
- **Eviction policy + GC.** The cache today is uncapped. Eviction (LRU + size cap) needs to run Service-side; on the flat cache, eviction is `unlink` of `<content_hash>` files in age order. On Linux, an open fd survives `unlink`, so a UI in-progress read does not need explicit lease pinning (the fd is the pin). GC (drop files whose `attachments.content_hash` references are gone) collapses to the same operation - delete the file - so eviction and GC are one sweep on the flat cache.
- **Cross-store invariant pass extension.** Phase 3 introduced a boot-time invariant pass for SQLite + Tantivy + body store + inline image store. Phase 6b extends it to the attachment cache: files in the cache directory not referenced by any `attachments.content_hash` get reclaimed (e.g., from a crashed mid-write or a half-finished account deletion).
- **Global write-half lockdown.** With OAuth's residual UI surface migrated, no UI-side write call site remains except the calendar event mutations deferred to Phase 6c. Phase 6b makes the constructors of `WriteDbState`, `BodyStoreWriteState`, `InlineImageStoreWriteState`, and `SearchWriteHandle` unreachable from the `app` crate at compile time. The `cal::actions` write-surface escape stays as an explicit `Current Exception` until 6c.

## Scope

### In scope

- **OAuth code exchange (`oauth.exchange_code`).** UI captures the redirect, ships the auth code + redirect URI + PKCE verifier over IPC. Service exchanges the code for tokens against the provider's token endpoint, fetches userinfo to derive email (Google/Microsoft/JMAP all need a second round-trip for the email field), then delegates account creation to the same internal `service::accounts::create_account_inner` helper that 6a's `account.create` calls. The auth code is a one-shot bearer credential; wire types use the existing redacting wrapper for both `Debug` and `Display`. **Composition, not parallel path:** there is one `create_account_inner`; `oauth.exchange_code` is an OAuth-specific entry point that gathers credentials before calling it, not an alternate creation path. The same IPC handles re-auth: when `params.reauth_account_id` is set, Service updates the existing row's tokens via `update_account_tokens_sync` instead of inserting a new row, replacing the UI-side `with_write_conn` calls at `add_account/oauth.rs:149` and `add_account/state.rs:514` (these were the OAuth re-auth deferrals from 6a).
- **Service-side OAuth refresh - verification only.** Per-provider refresh helpers already run Service-side via `ensure_valid_token` in jmap/graph/gmail/imap; the previously planned `oauth.refresh_request` IPC from Phase 4 was never shipped (it was removed during Phase 4 close-out). 6b verifies the refresh path is reachable from every Service-side caller that needs it (sync handlers, push handlers, `oauth.exchange_code` itself when an old account re-auths) and prunes any UI-side `core::oauth::refresh_token` call sites that the Service-side migration leaves dead. **No new IPC method, no deletion of an existing one.**
- **OAuth admission bypass.** `oauth.exchange_code` joins `RequestParams::bypasses_admission()` (today: `HealthPing`, `BootReady`). The original draft proposed a "bounded queue depth + Busy" mechanism; the arch review pointed out (a) the per-handler semaphore + admission cap already provides the moral equivalent, (b) `ServiceError::Busy` would duplicate the existing `ServiceError::Backpressure`, and (c) the actual hazard is post-admission queueing behind a 30 s `ActionSend` or a long `attachment.fetch`, which depth-checks on new arrivals don't fix. Bypass is the existing-shape, no-new-mechanism resolution.
- **`attachment.fetch` IPC for cache-miss reads.** Wire ack carries `{ content_hash, size_bytes, relative_path }` where `relative_path` matches the existing `write_cached` return shape (`attachment_cache/<content_hash>`). UI re-opens the file positionally; the open fd is the pin against concurrent eviction (Linux `unlink` does not invalidate open fds). **No bytes over the IPC** (`phase-1.5-plan.md` backpressure policy); the cache file is the contract. Lease IDs are deferred until pack-aware reads land (a future revision pass alongside Phase 1a) - the flat cache does not need them because each blob is one file and `unlink` is fd-safe.
- **Eviction policy.** LRU + total-size cap (default: 5 GB). The flat cache today (`crates/stores/src/attachment_cache.rs`) is uncapped; the `attachments` table already carries `last_accessed_at` per cached blob via the existing `update_attachment_cache_fields` write path. Eviction sorts cache files by `last_accessed_at` ascending and unlinks the oldest until under cap or 200 MB reclaimed (whichever first). The `attachments.local_path` / `cached_at` / `cache_size` columns are cleared in batch via `clear_attachment_cache_fields_batch`. Eviction runs on `attachment.eviction_kick` (5-min cadence with 1 h staleness gate) plus on-demand from `attachment.fetch` if a fetch would push the cache past the cap. **Per-kick reclaim is bounded** (default: 200 MB) so a first cold-pull post-6b on an existing 50 GB cache does not stall - the cache reduces incrementally over the next hours' kicks rather than in one expensive synchronous burst.
- **Garbage collection collapses into eviction on the flat cache.** A blob whose `messages` row is gone is just an unreferenced file; the eviction sweep already wants to delete it. The "GC" sweep is folded into eviction: each pass drops files in age order, but stale-no-reference files are dropped first regardless of age. No separate `attachment.gc_kick` request type; the kick handler reads `attachments.content_hash` once and any cache file not present in that set is fair game. **The pack-aware split between LRU eviction and frame-orphan GC + repack defers to the post-Phase-1a revision pass.**
- **Cross-store invariant pass extension - whole-file orphan sweep on flat cache.** `service::startup_invariants` gains a cache-dir pass that compares files in `attachment_cache/` against `attachments.content_hash`. Files with no matching row are reclaimed at boot. **This is distinct from steady-state eviction** - the boot pass handles "file exists, no DB row says so" (crashed mid-write, half-finished account deletion); the kicked eviction handles "DB row was there, the cache is past cap." When Phase 1a lands, this becomes the whole-file orphan pass against `pack_index` and a sibling frame-orphan GC + repack pass joins it.
- **Account-deletion crash recovery via shared marker helper.** `crates/service/src/markers/` (new module) hosts a generic marker-file helper - one schema, one drain helper, one Settled Pattern entry in `docs/architecture.md`. The account-deletion marker enumerates cleanup steps as a versioned list: body-store delete -> inline-image delete -> pack-cache unref + maybe-evict -> search index delete -> `accounts` row CASCADE. **CASCADE is always last**: once the row is gone, external stores cannot be reverse-mapped by `account_id`. Resume is idempotent step-by-step. The same helper hosts the existing sync markers + Phase 4 push markers; 6a's draft WAL is similar but is content-bearing (entries to replay, not steps completed) so it stays a separate file format.
- **Global write-half lockdown - layered enforcement.** Three checks together:
  - **Continue 6a's positive grep allow-list** at `crates/app/src/`: no `Db::with_write_conn`, no `Db::write_db_state`, no raw rusqlite write call sites.
  - **Cargo dependency check is transitive**: `cargo metadata` over the resolved graph asserts `app` does not depend on `service-state` *directly or transitively*. Direct-only is bypassed by inserting any intermediate crate.
  - **Service-state public surface enumerated**: `WriteDbState::from_arc`, `WriteDbState::from_db_state`, `WriteDbState::to_read_state`, the `BodyStoreWriteState` constructors at `body_store_write.rs`, `InlineImageStoreWriteState` constructors at `inline_image_store_write.rs`, and `SearchWriteHandle` construction at `search_write.rs:72` all flip to `pub(crate)`. Each is documented in the lockdown commit.
- **`cal::actions` exception update.** `docs/architecture.md` Current Exceptions explicitly carries the calendar event mutation escape until Phase 6c lands.

### Entry criteria

- **Phase 5 landed** (calendar/GAL relocation, IMAP cancellation depth).
- **Phase 6a-part-2 landed** (closing 5 surfaces: `account.delete`, `internal.read_bootstrap_snapshots` + `internal.encrypt_for_storage` + `internal.decrypt_for_storage`, draft WAL, pinned-search per-row CRUD; *plus* the closing commit: `Db::with_write_conn` / `Db::with_write_conn_sync` / `Db::write_db_state` deleted, CI lockdown script live, `docs/architecture.md` rewrite landed). 6b's lockdown task continues 6a-part-2's positive grep allow-list; the script must exist for "continue" to be meaningful. The 6a-part-2 OAuth re-auth deferrals (UI-side `with_write_conn` at `add_account/oauth.rs:149` and `add_account/state.rs:514`) are explicitly carried into 6b instead - the cleanest landing for those is the same `oauth.exchange_code` IPC handling re-auth via a `reauth_account_id` parameter, so they ship in 6b alongside the new entry point rather than 6a-part-2.
- **Attachments roadmap Phase 1a NOT a prerequisite.** 6b ships against the existing flat cache (`crates/stores/src/attachment_cache.rs`); pack-aware design (frame-orphan GC, repack pass, lease IDs tied to in-pack offsets) defers to a future "6b extension" revision pass that lands alongside Phase 1a + 1b. The existing flat cache + `attachments` table cache columns are sufficient for `attachment.fetch` IPC + LRU eviction + whole-file orphan reconciliation.
- **No `oauth.refresh_request` IPC to delete.** Phase 4's review-pass close-out removed it (push refresh is purely DB+HTTPS, both Service-internal). 6b's prior task list had a 2a/2b pair to introduce per-provider helpers + delete the IPC; that work is already done. 6b verifies + prunes any UI-side dead code only.

### Out of scope

- **Calendar event mutations.** Phase 6c (`docs/service/phase-6c-plan.md`). Phase 6b leaves `Db::create_calendar_event`, `Db::update_calendar_event`, `Db::delete_calendar_event` UI-side. The `cal::actions` write-surface escape stays in `docs/architecture.md` § Current Exceptions until 6c lands.
- **Settings UI for attachment caching policy.** The eviction parameters (size cap, age cap) are hard-coded defaults in 6b; a settings UI is downstream attachments-roadmap work, not Phase 6.
- **Calendar attachments.** Separate work from the `attachment.fetch` IPC; the existing pack store handles email attachments only.
- **Provider-specific OAuth quirks** (Microsoft tenant routing, Google offline-access scopes, etc.). The IPC is provider-neutral; per-provider handling stays in the provider crates' OAuth helpers.

## Architecture

### OAuth two-step shape

Today's flow (initial create):

```
UI redirect handler -> rtsk::oauth::authorize_with_provider (UI) ->
   provider token endpoint + userinfo
   -> AddAccountMessage::OAuthComplete (UI) -> account.create IPC (6a) ->
   service::accounts::create_account_inner -> persist
```

Today's flow (re-auth):

```
UI redirect handler -> rtsk::oauth::authorize_with_provider (UI) ->
   provider token endpoint -> AddAccountMessage::OAuthComplete (UI) ->
   db.with_write_conn(update_account_tokens_sync) (UI - 6a-part-2 deferral)
```

Service-side OAuth refresh (already exists, reached during sync): per-provider `ensure_valid_token` in jmap/graph/gmail/imap reads the row, calls the provider's token endpoint, writes the new tokens back. No UI involvement. The previously planned `oauth.refresh_request` IPC was never shipped (Phase 4 close-out documented its removal in `crates/service/src/push.rs:29`).

After Phase 6b:

```
UI redirect handler -> oauth.exchange_code IPC ->
   service::oauth::exchange_code
      (token-endpoint round-trip + userinfo round-trip)
      -> if reauth_account_id: update_account_tokens_sync
         else:                  service::accounts::create_account_inner
      -> persist
```

One IPC handles both initial create and re-auth, distinguished by the `reauth_account_id: Option<String>` parameter. This collapses what would otherwise be two IPCs (`oauth.exchange_code` + `oauth.reauth`) into one handler that already needs to do everything except the final create-vs-update decision.

The IPC types live in `service-api/src/oauth.rs`:

- `OauthExchangeCodeParams { provider: ProviderKind, code: RedactedString, redirect_uri: String, code_verifier: String, scopes: Vec<String>, reauth_account_id: Option<String> }`. The `RedactedString` wrapper redacts both `Debug` and `Display` implementations so a stray `format!("{}")` in a log statement does not leak the auth code.
- `OauthExchangeAck { account_id: String, email: String, expires_in_secs: u32 }`. Token bytes never cross the IPC; Service writes them directly to the DB. The `email` field is returned because OAuth derives it from a userinfo round-trip the UI does not have visibility into.

**One create-path, two entry points.** The plan's earlier draft had `oauth.exchange_code` as a separate creation surface alongside `account.create`. The arch review flagged that "what makes a fully-formed account" would have to be maintained at two sites, breaking the make-the-right-thing-the-only-thing rule. Reconciled design: a single internal helper `service::accounts::create_account_inner(provider, email, encrypted_credentials, ...)`. `account.create` (Phase 6a) calls it for `Plaintext`/`Encrypted` envelope variants; `oauth.exchange_code` (this phase) calls it after running the OAuth-specific token + userinfo round-trips. Future "every account needs a default folder set" type changes update the helper, not the entry points.

**Admission bypass instead of priority lane.** OAuth servers expect the redirect-to-token-exchange round-trip in seconds, not minutes. The original draft proposed a "bounded queue depth + Busy" mechanism. The arch review pointed out that the dispatch loop already has the right tool: `RequestParams::bypasses_admission()` (`crates/service-api/src/request.rs:215`) is used today by `HealthPing` and `BootReady` to skip both the per-handler semaphore and the admission cap. `oauth.exchange_code` joins this list. Once admitted, OAuth runs alongside other in-flight handlers without queueing behind a 30 s `ActionSend` or a long `attachment.fetch`. The proposed `ServiceError::Busy` was a duplicate of the existing `ServiceError::Backpressure`; not adding either.

### `attachment.fetch` IPC on the flat cache

Wire shape:

```
AttachmentFetchParams { account_id, message_id, attachment_id }
AttachmentFetchAck { content_hash: String, size_bytes: u64, relative_path: String }
```

The Service ensures the bytes are present in the flat cache (`attachment_cache/<content_hash>`) before returning the ack. If the file is already present (cache hit), the ack returns immediately. If not, Service issues the provider fetch, writes the file via `attachment_cache::write_cached`, updates the `attachments` row's cache columns, and then acks. The UI re-opens the file positionally using `relative_path` resolved against the app data dir (the existing `attachment_cache::read_cached` path).

**Why path + content_hash, no lease?** On Linux, `unlink` does not invalidate already-open file descriptors. A UI process that has the cache file open survives a concurrent eviction sweep cleanly: the UI keeps reading from its open fd; the file is removed from the directory; the kernel reclaims disk space when the last fd closes. The race that pack-aware reads have (in-pack offset moved by repack, frame-orphan GC marking unreachable) does not exist on the flat cache because each file is one blob and `unlink` is fd-safe. The read pin is the open fd itself.

When Phase 1a lands (pack store + `pack_index`), the lease design returns: pack-aware reads cannot use "open fd" as the pin because eviction may rewrite the *file* and the UI's offset becomes meaningless. The future revision pass adds `lease_id` + `PackStore::get_with_lease` + active-lease counter on `pack_index`. **No bytes over the IPC** (`phase-1.5-plan.md` backpressure policy) is preserved in both designs.

The IPC is request-response; cancel-on-account-delete piggybacks the existing `cancel_and_await` flow (Phase 5 task 9 made the cancel IPC carry per-account run ids).

### Eviction + GC: single global kicked sweep, not a per-account runtime

The original draft framed pack eviction/GC as a `PackRuntime` mirroring `CalendarRuntime`'s per-account map + panic supervisor + cancel/run-id machinery. The arch review rejected that framing: pack storage is content-addressed and global. `attachments.content_hash` is the deduplication key with no account scope - cross-account deduplication is the point of the cache. There is no "per-account run id" because there is no per-account state. Forcing the `CalendarRuntime` shape would import a per-account map, panic supervisor, run-completion correlation, and cancel APIs the sweep doesn't need.

Replaced with a single global kicked sweep, structurally closer to `pinned_search.kick` than `calendar.kick`. **On the flat cache, eviction and GC are one operation** - both reduce to "delete a file from the cache directory." A pack-aware split between LRU eviction (size-driven) and frame-orphan GC (reference-driven) only makes sense when one file holds many blobs; the flat cache has one-file-per-blob, so a GC-eligible blob is also an evict-eligible blob.

- **Single-flight guard.** Module-level `static ATTACHMENT_SWEEP_LOCK: Mutex<()>` prevents concurrent sweeps. Two `attachment.eviction_kick` notifications back-to-back through `NOTIFY_CAP=4` would otherwise run duplicated reclaim work; same hazard the GAL handler addresses with its own `Mutex`.
- **One kick handler, one lock.** `attachment.eviction_kick` (5-min cadence with 1 h staleness gate). The handler reads `attachments.content_hash` once into a `HashSet` and walks the cache directory; files not present in the set are dropped first (no-reference-equivalent), then the remainder are dropped in `last_accessed_at` order until under cap or 200 MB reclaimed (whichever first). Reclaim is bounded so a first cold-pull post-6b on an existing 50 GB cache does not stall the request lane.
- **Shutdown drain.** The consolidated drain (Phase 4) waits for any in-flight sweep before exiting - no more than 30 s for the bounded reclaim work. Stale kicks during shutdown are dropped (notification class is `Drop`).
- **No `PackRuntime`, no `attachment.gc_kick`, no repack handler.** The single handler in `crates/service/src/handlers/attachment.rs` plus the static lock are the entire infrastructure. When Phase 1a lands, the handler splits: `attachment.eviction_kick` retains the LRU role; a sibling `attachment.gc_kick` joins for frame-orphan + repack work; the static lock generalizes to cover both.

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

### Cross-store invariant pass extension - flat-cache orphan sweep

`service::startup_invariants` already covers SQLite (Phase 3 task 11), body store, inline image store, and Tantivy (Phase 4). Phase 6b adds two passes against the flat cache:

- **Whole-file orphan sweep (boot pass).** Read `attachments.content_hash` into a `HashSet`; walk the flat cache directory (`attachment_cache/`); drop files whose name (the content hash) is not in the set. This handles "file exists, no DB row says so" - crashes between `write_cached` and the row update, half-finished account deletions, etc. Each cache file is one blob, so the set-membership check is exact (no per-frame reasoning needed on the flat cache).
- **Stale-row reconciliation (boot pass).** `attachments` rows whose `local_path` points at a missing cache file get their cache columns cleared via `clear_attachment_cache_fields_batch`. Future `attachment.fetch` calls on those rows will re-fetch from the provider. Cheap; runs in the same boot phase.

**When Phase 1a lands**, this pass splits: whole-file orphan keys on `pack_index.pack_file_id` instead of `attachments.content_hash`; a sibling frame-orphan pass keys on `pack_index.refcount` for blobs whose `messages` rows are gone; repack runs on its own kicked schedule. Until Phase 1a lands, the flat-cache version is what 6b ships.

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

Phase 6a-part-2's architecture-doc rewrite captures the post-6a state. Phase 6b's update is smaller:

- **Action service as mutation gate** § "Enforcement": replace the "global lockdown lands at Phase 6" text with "global lockdown landed at Phase 6b for direct deps + constructor visibility; the transitive `app -> cal -> service-state` path closes in Phase 6c when `cal` drops out of `app/Cargo.toml`."
- **Current Exceptions** § `cal::actions`: keep until Phase 6c lands.
- **Settled Patterns** § "Service kick handlers": add `attachment.eviction_kick`.
- **Settled Patterns** § "Marker-file recovery": new entry covering the shared `crates/service/src/markers/` helper, the CASCADE-last ordering rule, and the idempotent step-list pattern.
- **Pack-aware future work**: a forward-reference paragraph noting that `attachment.fetch` and `attachment.eviction_kick` will gain lease semantics + `attachment.gc_kick` + repack handler when attachments-roadmap Phase 1a lands.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

**0. Inventory + entry-criteria check.** Verify 6a-part-2 closing commit landed (positive grep allow-list script in `scripts/`, `Db::with_write_conn` / `with_write_conn_sync` / `write_db_state` deleted from `db/connection.rs`, `docs/architecture.md` rewrite committed). Survey today's UI-side OAuth call sites (`rtsk::oauth::authorize_with_provider` at `add_account/state.rs:648` and `add_account/oauth.rs:106`, `update_account_tokens_sync` at `add_account/oauth.rs:149`, the persist sites at `add_account/state.rs:514`). Confirm the `RedactedString` wrapper redacts both `Debug` and `Display`. Confirm no `oauth.refresh_request` request type lingers in `service-api`.

**1. Shared marker helper (`crates/service/src/markers/`).** Generic `MarkerFile<T>` type, drain-on-boot helper. Lift the existing sync markers (Phase 4) and Phase 4 push markers into it in the same commit so the new pattern lands with three real consumers (sync, push, account-delete coming in task 8), not one. Settled Pattern entry in `docs/architecture.md` follows in task 10.

**2. OAuth refresh path verification + UI-side cleanup.** No new IPC, no IPC deletion. The Service-side `ensure_valid_token` per-provider helpers (jmap/graph/gmail/imap) already exist; this commit walks every Service-side caller of an OAuth-bearing endpoint and confirms the refresh path is reachable. UI-side dead call sites (`core::oauth::refresh_token` if any, dead imports) get pruned in the same commit. Document the verified refresh ownership in `service/src/oauth/mod.rs` (or equivalent). **No `service/src/oauth/refresh.rs` extraction unless duplication is found.**

**3. Shared `service::accounts::create_account_inner` helper.** Extract the 6a `account.create` handler's body into the shared helper so 6b's `oauth.exchange_code` can call it. Re-route `account.create` through the helper. No external behavior change.

**4. `oauth.exchange_code` IPC.** Wire types in `service-api/src/oauth.rs` (with `RedactedString` Debug + Display redaction). Params include `reauth_account_id: Option<String>` so the same IPC handles initial create and re-auth. Service handler in `service/src/handlers/oauth.rs` runs token-endpoint exchange + userinfo round-trip, then either updates the existing row's tokens (re-auth) or calls `create_account_inner` (initial). UI-side `service_client.rs` async wrapper. UI redirect handler in `add_account/state.rs` and `add_account/oauth.rs` routes through the new IPC; the UI-side `rtsk::oauth::authorize_with_provider` call collapses to "open the browser, capture the code, ship to Service" - the token-endpoint round-trip moves Service-side. **Add `OauthExchangeCode` to `RequestParams::bypasses_admission()`.** Mark the request timeout 30 s (provider token endpoints are slow under load).

**5. `attachment.fetch` IPC against the flat cache.** Wire types (ack carries `content_hash`, `size_bytes`, `relative_path`). Service handler ensures the file is local (cache hit -> immediate ack; cache miss -> provider fetch + `write_cached` + cache-column update + ack). UI-side cache-miss path in `core::attachment_cache::fetch_*` (or equivalent) calls the IPC then re-opens the file via `read_cached`. **No leases on the flat cache** - the open fd is the pin against eviction.

**6. Eviction policy + `attachment.eviction_kick`.** Single global sweep with `ATTACHMENT_SWEEP_LOCK` single-flight guard in `crates/service/src/handlers/attachment.rs`. LRU by `last_accessed_at` + 5 GB size cap; per-kick reclaim bounded to 200 MB; orphan files (no matching `attachments.content_hash`) drop first regardless of age. UI-side `kick_attachment_eviction` joins `Message::SyncTick` fan-out.

**7. Cross-store invariant pass extension.** Whole-file orphan sweep + stale-row reconciliation in `service::startup_invariants` against the flat cache. Keyed on `attachments.content_hash` set membership.

**8. Account-deletion marker (uses task 1's helper).** Service handler writes the marker before step 1 of cleanup, updates after each step, removes after step 5. Boot drain replays unfinished cleanups. Each cleanup step is idempotent. **Depends on 6a-part-2's `account.delete` IPC having landed** - the marker is written from inside the Service-side delete handler, which is what 6a-part-2 introduces. If 6a-part-2 lands the deletion handler without marker support, task 8 layers it on; if 6a-part-2's handler is shape-compatible, the marker helper plugs in directly.

**9. Global lockdown.** Two commits, landed together:
   - 9a: flip `service-state` constructors to `pub(crate)` (`WriteDbState::from_arc`, `WriteDbState::from_db_state`, `WriteDbState::to_read_state`, `BodyStoreWriteState`, `InlineImageStoreWriteState`, `SearchWriteHandle`). Fix the compile errors by routing through new IPC methods. Drop `service-state` from `crates/app/Cargo.toml` if any direct dep remains.
   - 9b: constructor-visibility integration test in `crates/service-state/tests/lockdown.rs`.
   - **No transitive `cargo metadata` check in 6b.** The `app -> cal -> service-state` transitive path persists until Phase 6c relocates `cal::actions::*` and drops `cal` from `app/Cargo.toml`. 6c's lockdown task lands the transitive check.

**10. `docs/architecture.md` Phase 6b delta + Settled Pattern for marker helper.** Per § "Architecture-doc update" above. Updates `implementation-roadmap.md` Phase 6 entry to reflect 6b "LANDED" status (with the transitive lockdown note explicitly deferred to 6c).

## File-by-file changes

**New files:**
- `crates/service-api/src/oauth.rs` - OAuth wire types (`RedactedString` covers both Debug and Display).
- `crates/service-api/src/attachment.rs` - attachment-fetch wire types (`content_hash`, `size_bytes`, `relative_path`; no lease fields on the flat-cache shape).
- `crates/service-api/src/attachment_kick.rs` (or fold into `attachment.rs`) - `attachment.eviction_kick` notification declaration.
- `crates/service/src/handlers/oauth.rs` - `oauth.exchange_code` handler.
- `crates/service/src/handlers/attachment.rs` - `attachment.fetch` handler + eviction kick handler (with `static ATTACHMENT_SWEEP_LOCK`).
- `crates/service/src/accounts/create.rs` - shared `create_account_inner` helper (extracted from 6a's `account.create` handler in task 3).
- `crates/service/src/markers/` - shared marker-file helper module.

**Modified files:**
- `crates/service-api/src/lib.rs` - module declarations.
- `crates/service-api/src/request.rs` - new `RequestParams` variants. `OauthExchangeCode` joins `bypasses_admission()`. OAuth timeout 30 s (token endpoint + userinfo); attachment.fetch timeout 60 s; attachment-kick notification inherits Drop class.
- `crates/service-api/src/notification.rs` (or `client_notification.rs`) - new `ClientNotification::AttachmentEvictionKick`.
- `crates/service/src/dispatch.rs` - new request + notification arms.
- `crates/service/src/startup_invariants.rs` - whole-file orphan sweep + stale-row reconciliation against the flat cache.
- `crates/service/src/boot.rs` - install marker-helper drain phase, account-deletion marker drain.
- `crates/service-state/src/lib.rs` - `WriteDbState::from_arc`, `from_db_state`, `to_read_state` flip to `pub(crate)`.
- `crates/service-state/src/body_store_write.rs` - constructors flip to `pub(crate)`.
- `crates/service-state/src/inline_image_store_write.rs` - same.
- `crates/service-state/src/search_write.rs` - same.
- `crates/service-state/tests/lockdown.rs` - new constructor-visibility integration test.
- `crates/service/src/sync_markers.rs` (etc.) - migrate existing marker callers to the new shared helper in task 1.
- `crates/app/src/service_client.rs` - new IPC wrappers.
- `crates/app/src/handlers/provider.rs` - `kick_attachment_eviction`.
- `crates/app/src/update.rs` - `Message::SyncTick` fan-out gains the eviction kick.
- `crates/app/src/ui/add_account/state.rs` and `crates/app/src/ui/add_account/oauth.rs` - swap UI-side OAuth token-endpoint round-trip + persist for `oauth.exchange_code` IPC. Removes the last `with_write_conn` callers in `add_account/`.
- `crates/stores/src/attachment_cache.rs` - cache-miss path now calls the IPC; helper functions (read_cached / write_cached / remove_cached_relative) stay where they are because both UI (read) and Service (write + remove) call them.
- `crates/core/src/...` - dead UI-side OAuth call sites and imports pruned in task 2.
- `docs/architecture.md` - Phase 6b delta + new Settled Pattern for marker helper + forward-reference paragraph for pack-aware future work.
- `docs/service/implementation-roadmap.md` - mark Phase 6b "LANDED" (with the transitive `cargo metadata` lockdown explicitly deferred to 6c).

## Code-comment requirements

1. **`crates/service/src/handlers/oauth.rs::handle_exchange_code`** must contain:
   - "OAuth code is a one-shot bearer credential. The wire-type wrapper redacts both `Debug` and `Display`; logging frameworks reach for both. After Phase 6b the auth code never reaches the UI beyond the redirect handler that captures it; the IPC ships the code straight to Service. The handler runs token-endpoint exchange + userinfo round-trip, then either updates the existing row's tokens (re-auth, when `reauth_account_id` is set) or delegates account creation to `service::accounts::create_account_inner` - the shared helper that 6a's `account.create` also calls. One create-path, two entry points; one re-auth path, same handler."

2. **`crates/service/src/handlers/oauth.rs`** module-level doc-comment:
   - "Phase 6b moves the OAuth token-endpoint round-trip Service-side via `oauth.exchange_code`. Service-side OAuth refresh predates 6b - it has been Service-side since Phase 4 close-out, when the planned `oauth.refresh_request` IPC was removed in favor of per-provider `ensure_valid_token` helpers (jmap/graph/gmail/imap) that read the row + call the token endpoint + write back. There is no Phase-4 IPC for 6b to delete; refresh is already where it should be. `oauth.exchange_code` joins `RequestParams::bypasses_admission()` - the same admission-bypass list as `health.ping` and `boot.ready` - so the OAuth round-trip is not queued behind heavy traffic."

3. **`crates/service/src/handlers/attachment.rs::handle_fetch`** must contain:
   - "Wire ack carries `{ content_hash, size_bytes, relative_path }`. Bytes never cross the IPC (phase-1.5-plan.md backpressure policy). On the flat cache, the open fd is the pin against concurrent eviction - Linux `unlink` does not invalidate already-open fds, so a UI process holding the cache file open survives a concurrent sweep. When pack-aware reads land (Phase 1a), this handler grows lease semantics + a `PackStore::get_with_lease` API; that revision pass swaps the wire shape and adds a `lease_id` field. Until then, no leases."

4. **`crates/service/src/handlers/attachment.rs` module-level doc-comment** must contain:
   - "Attachment eviction is global, not per-account, because cache storage is content-addressed (`attachments.content_hash` is the dedup key, no account scope). The original Phase 6b draft framed this as a `PackRuntime` mirroring `CalendarRuntime`'s per-account shape; the arch-review revision dropped that framing. On the flat cache (pre-Phase-1a), eviction and GC collapse into a single sweep: each blob is one file, so unrefefenced files are dropped first and remaining files are dropped in `last_accessed_at` order until under cap or 200 MB reclaimed (whichever first). Single-flight is enforced by the module-level `ATTACHMENT_SWEEP_LOCK` Mutex - same pattern the GAL handler uses for the same reason (`NOTIFY_CAP=4` would otherwise duplicate reclaim work)."

5. **`crates/service/src/markers/` module-level doc-comment** must contain:
   - "Shared marker-file helper: sync, push, and account-delete recovery markers all live here as `MarkerFile<T>`. Each marker carries a step-completed list serialised as JSON. Recovery on boot: read marker -> identify next un-completed step -> run forward. Each step must be idempotent. Account-delete steps are ordered: body -> inline -> attachment-cache-clear -> search -> accounts row CASCADE; CASCADE is always last because external stores cannot be reverse-mapped by `account_id` once the row is gone."

6. **`docs/architecture.md` § "Action service as mutation gate"** new sentence:
   - "Phase 6b advanced the global write-half lockdown: 6a-part-2's positive grep allow-list at `crates/app/src/` plus the constructor-visibility integration test in `crates/service-state/tests/lockdown.rs`. The transitive `cargo metadata` check (`app` does not depend on `service-state`, directly or transitively) lands in Phase 6c when `cal::actions::*` relocates Service-side and `cal` drops out of `app/Cargo.toml`. After 6c the lockdown is three-layered."

7. **`docs/architecture.md` § "Settled Patterns"** new entry:
   - "Service marker files (`crates/service/src/markers/`). Multi-step recovery for crash-safe operations (sync, push, account-delete). Each marker is a versioned `MarkerFile<T>` carrying the step-completed list. Boot drain reads each marker, runs forward from the next un-completed step. All steps must be idempotent. New marker types extend the helper rather than introducing parallel patterns."

## Test plan

### Unit tests

- Wire-type round-trips for `oauth`, `attachment` modules. `OauthExchangeCodeParams` round-trips both with and without `reauth_account_id` set.
- Eviction LRU test: seed the cache with 100 MB of blobs (varying `last_accessed_at`), set a 50 MB cap, fire eviction; assert the oldest blobs are dropped first AND no more than 200 MB is reclaimed per kick.
- Eviction prefers orphans over LRU: seed the cache with 50 MB of files referenced by `attachments` rows + 50 MB of files NOT referenced; set a 75 MB cap; fire eviction; assert the unreferenced files are dropped before any age-based eviction touches the referenced set.
- Whole-file orphan boot pass: drop a file in `attachment_cache/` with no matching `attachments.content_hash`; run boot pass; assert the file is deleted. Drop a file whose hash matches a row; run boot pass; assert the file is NOT deleted.
- Stale-row reconciliation: insert an `attachments` row whose `local_path` points at a non-existent cache file; run boot pass; assert the cache columns clear via `clear_attachment_cache_fields_batch`.
- Cross-store invariant pass: simulate a half-finished account deletion (marker file present, some external-store cleanup steps un-completed); boot with the pass enabled; assert the cleanup runs forward from the next un-completed step.
- `bypasses_admission` includes `OauthExchangeCode`: regression test that locks the list.
- OAuth refresh path verification: every Service-side caller of an OAuth-bearing provider endpoint reaches an `ensure_valid_token` (or equivalent) call before the network round-trip. Static check; could be a `compile_fail` doctest or a per-provider unit test.

### Integration tests (in-process)

- `oauth_exchange_round_trips_initial_create`: stub provider token endpoint + userinfo endpoint; UI-side IPC call returns ack with the derived email; row in `accounts` has the expected encrypted token bytes; the create-path went through `service::accounts::create_account_inner` (assert via test hook or by observing the same default-folder set the `account.create` path produces).
- `oauth_exchange_round_trips_reauth`: as above, but with `reauth_account_id` set and a pre-existing account row; assert the row's tokens update and no new row is created.
- `service_refreshes_own_token_on_sync`: stub provider token endpoint; trigger sync against an account whose token is about to expire; assert refresh ran Service-side via `ensure_valid_token` and no IPC was used.
- `attachment_fetch_cache_hit`: file present in `attachment_cache/`; trigger the IPC; assert ack returns immediately with the existing relative path, no provider round-trip fires.
- `attachment_fetch_cache_miss`: clear the cache; trigger the IPC; assert ack carries the relative path AND the file is locally present after the ack returns; provider round-trip ran.
- `attachment_fetch_open_fd_survives_eviction`: open the cache file via the IPC ack; trigger eviction; assert the open fd still reads the bytes correctly even though the file is unlinked from the directory.
- `lockdown_constructor_visibility`: integration test in `crates/service-state/tests/lockdown.rs` that asserts the enumerated public constructors (`WriteDbState::from_arc`, `from_db_state`, `to_read_state`, plus the body / inline / search write-handle constructors) are inaccessible from outside the crate.
- `oauth_redacted_string_redacts_debug_and_display`: round-trip an `OauthExchangeCodeParams`; assert the auth code does not appear in either `format!("{:?}")` or `format!("{}")`.
- `marker_helper_round_trips_step_list`: write marker -> simulate crash -> read marker -> assert next-unfinished-step matches.
- `account_delete_marker_resumes_after_crash_at_each_step`: parameterised over each step (1-5); kill mid-step; restart; assert cleanup completes idempotently.

### Real-subprocess smoke tests

- `service_subprocess_oauth_full_flow`: spawn Service with an OAuth-enabled stub account; UI ships an auth code via IPC; Service exchanges + persists; UI re-reads the row over the existing read path. Assert that `oauth.exchange_code` does not queue behind a concurrent slow handler (validates the `bypasses_admission` wiring).
- `service_subprocess_attachment_eviction_bounded`: seed a 50 GB cache; fire eviction kick; assert reclaim reduces the cache by no more than 200 MB per kick (not the entire excess in one sweep).
- `service_subprocess_attachment_fetch_open_fd`: real subprocess, real cache; verify open-fd survives an eviction kick.

### Manual matrix updates

- OAuth flow end-to-end (Google, Microsoft) via real provider endpoints. Verify no auth code appears in any log output. Verify re-auth flow (existing account, expired refresh token) works through the same IPC.
- Cache-miss attachment open in the reading pane (assert the user sees no UI freeze during the IPC round-trip).
- Account deletion mid-flight crash recovery (kill -9 the Service mid-cleanup; restart; assert the orphaned external-store data is reclaimed on the next boot).

## Open questions

- **Eviction size cap default.** 5 GB chosen as a starting point; the cache today is uncapped. Plan picks 5 GB; revisit after the first weeks of dogfooding.
- **Marker helper format.** JSON for readability vs binary cookie for write-once-ness. Plan picks JSON - the markers are small and the boot drain is the only consumer; readability wins.
- **Pack-aware revision pass timing.** When Phase 1a + 1b land, 6b's flat-cache design needs a revision pass to add lease semantics, frame-orphan GC, and repack. Plan files that revision under "phase-6b-flat-to-pack.md" (or a 6b plan-doc revision) at that point. Until then, the flat-cache shape is what ships.

## Verification (end-to-end)

- The two 6b lockdown checks pass: 6a-part-2's positive grep allow-list (still clean) plus the constructor-visibility test in `crates/service-state/tests/lockdown.rs`. The transitive `cargo metadata` check is documented as a 6c deliverable; not gated here.
- An OAuth login (initial create AND re-auth) completes end-to-end via `oauth.exchange_code`. The auth code does not appear in any log output (Debug or Display).
- A token expires mid-sync and Service refreshes it via `ensure_valid_token` without any IPC.
- An attachment cache-miss completes via `attachment.fetch`; concurrent eviction does not break an open fd against the cache file.
- A Service crash mid-account-delete is recoverable on next boot - the marker drives cleanup forward from the next un-completed step; the `accounts` row CASCADE only fires after external stores are clean.
- The whole-file orphan boot pass deletes cache files with no `attachments.content_hash` match; live cache files are untouched.

## Promotion criteria

- All items in `In scope` landed.
- Calendar event mutations are the only remaining UI write surface, tracked in `docs/service/phase-6c-plan.md`.
- The pack-aware revision pass against the flat-cache design is filed (either as a "phase-6b-flat-to-pack.md" plan-doc stub or as a known-deferred section in the 6b plan retirement note) so the revision is not lost when Phase 1a lands.
- `docs/architecture.md` reflects the post-Phase-6b state and includes the new Settled Pattern entry for the marker helper plus the forward-reference paragraph for pack-aware future work.
- `phase-6b-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry; every code-comment requirement is present in the relevant file.
