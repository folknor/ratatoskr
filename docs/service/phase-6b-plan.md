# The Service - Phase 6b Plan: OAuth two-step + `attachment.fetch` + global write-half lockdown

Companion to `phase-6a-plan.md`. Implements the second half of Phase 6 of `implementation-roadmap.md`.

## Revision history

**2026-05-06 - initial draft.** Authored at the same time as `phase-6a-plan.md` so the 6a / 6b boundary is explicit on day one. Phase 6c (calendar event mutations) is carved out into its own future plan; this document does not address it.

## Context

Phase 6a closes the small mechanical UI write surfaces and the encryption-key handle. The genuinely tricky surfaces remain:

- **OAuth coordination.** UI captures the redirect (it is the visible app); the Service handles the code-for-token exchange + persists the token. Today's flow runs entirely UI-side: `core::oauth::exchange_code` calls the provider's token endpoint, and the result is persisted via the same UI-side `with_write_conn` paths Phase 6a relocated. The temporary `oauth.refresh_request` shim from Phase 4 still lives - Service requests a fresh token from the UI when sync needs one. Phase 6b reverses that direction: Service refreshes its own tokens and the UI ships the redirect-captured auth code over IPC.
- **`attachment.fetch` IPC.** Cache-miss reads currently happen UI-side: open a thread, the reading pane reads the pack file directly. The pack-store reader has been unified for some time; what is still UI-side is the *cache-miss orchestration* - if the file is not local, the UI fetches from the provider. After Phase 6b that fetch flows through a Service IPC.
- **Eviction policy + GC.** The blob-store writer was relocated as a Phase 3 sync dependency. Eviction (LRU + size cap) and garbage collection (drop blobs whose `messages` rows have been deleted) need to run Service-side; the policy + cadence design is open.
- **Cross-store invariant pass extension.** Phase 3 introduced a boot-time invariant pass for SQLite + Tantivy + body store + inline image store. Phase 6b extends it to the attachment cache (pack files): an account-deletion that crashed mid-cleanup leaves orphaned packs the boot pass should reconcile.
- **Global write-half lockdown.** With OAuth and `attachment.fetch` migrated, no UI-side write call site remains except the calendar event mutations deferred to Phase 6c. Phase 6b makes the constructors of `WriteDbState`, `BodyStoreWriteState`, `InlineImageStoreWriteState`, and `SearchWriteHandle` unreachable from the `app` crate at compile time. The `cal::actions` write-surface escape stays as an explicit `Current Exception` until 6c.

## Scope

### In scope

- **OAuth code exchange (`oauth.exchange_code`).** UI captures the redirect, ships the auth code + state + provider-specific context over IPC. Service exchanges the code for tokens against the provider's token endpoint and persists the result. The auth code is a one-shot bearer credential; wire types use the existing redacting wrapper to keep it out of the log.
- **Service-side OAuth refresh.** Service refreshes its own tokens during sync. The temporary `oauth.refresh_request` IPC from Phase 4 deletes itself (Service no longer needs the UI as a token broker). The UI-side `core::oauth::exchange_code` and `core::oauth::refresh_token` call sites get pruned where the Service-side path replaces them.
- **`attachment.fetch` IPC for cache-miss reads.** Wire shape returns `{ content_hash, size }` (per `phase-1.5-plan.md`'s backpressure policy - the IPC does not move bytes; UI reads the pack file positionally after the fetch settles).
- **Eviction policy.** LRU + total-size cap (default: 5 GB). Tracked per-blob in the existing pack-index; eviction runs on a Service-side cadence (`pack.kick`) and on cache-miss when allocation pressure is high.
- **Garbage collection.** Periodic sweep that drops blobs whose `messages` rows are gone. Same kick cadence; runs on a longer interval (default: 24 h).
- **Cross-store invariant pass extension.** `service::startup_invariants` adds a pack-file pass: scan the pack directory, drop files with no corresponding `attachments` row. Account-deletion cleanup gets a "marker file written, cleanup not finished" recovery path so a crash mid-deletion does not leak.
- **Global write-half lockdown.** `service-state` crate's public surface stops exporting `WriteDbState::from_arc` (move it to a `pub(crate)` constructor). Same for the body / inline / search write-handle constructors. The `app` crate stops depending on `service-state`. A regression test (CI script or explicit Cargo dependency check) prevents reintroduction.
- **`cal::actions` exception update.** `docs/architecture.md` Current Exceptions explicitly carries the calendar event mutation escape until Phase 6c lands.

### Out of scope

- **Calendar event mutations.** Phase 6c (`docs/service/phase-6c-plan.md`). Phase 6b leaves `db/calendar.rs:47,58,107` (`create_calendar_event`, `update_calendar_event`, `delete_calendar_event`) UI-side. The `cal::actions` write-surface escape stays in `docs/architecture.md` § Current Exceptions until 6c lands.
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
   service::oauth::exchange_code -> provider token endpoint ->
   db::queries_extra::accounts_crud::persist_oauth_tokens_sync

Service-side sync (any provider) -> service::oauth::refresh_for_account ->
   provider token endpoint -> persist
```

The IPC types live in `service-api/src/oauth.rs`:

- `OauthExchangeCodeParams { provider: ProviderKind, account_id: Option<String>, code: RedactedString, redirect_uri: String, code_verifier: Option<String>, ... }` - fields driven by the OAuth 2.0 PKCE flow that Ratatoskr already uses.
- `OauthExchangeAck { account_id: String, expires_in_secs: u32 }` - token bytes never cross the IPC; Service writes them directly to the DB.

Account creation (Phase 6a's `account.create`) accepts already-encrypted credential bytes. The post-6b flow becomes: UI captures redirect -> `oauth.exchange_code` IPC creates the account and persists tokens in one Service-side transaction. The Phase 6a `account.create` IPC is retained for non-OAuth providers (IMAP password auth, app passwords).

**Latency budget.** OAuth servers expect the redirect-to-token-exchange round-trip in seconds, not minutes. The `oauth.exchange_code` IPC must not queue behind heavy traffic. Two design choices:

1. **Higher-priority dispatch lane** for OAuth requests. Implementation cost: rework `dispatch.rs` to honor a priority field. Risk: priority inversion bugs that are hard to test.
2. **Bounded request-queue depth.** If the dispatch queue is over a threshold, the Service rejects new non-priority requests with `ServiceError::Busy`; OAuth always proceeds. Implementation cost: depth check + back-pressure surface in `service-client`.

Plan picks (2). It is the simpler shape and matches the existing actor-model dispatch. (1) becomes a follow-up if measurement shows OAuth contention.

### `attachment.fetch` IPC

Wire shape:

```
AttachmentFetchParams { account_id, message_id, attachment_id }
AttachmentFetchAck { content_hash: String, size_bytes: u64, pack_path: PathBuf, offset: u64, length: u64 }
```

The Service ensures the bytes are in the pack file before returning the ack. UI reads positionally from `pack_path` at `offset..offset+length`. The IPC is request-response; cancel-on-account-delete piggybacks the existing `cancel_and_await` flow (Phase 5 task 9 made the cancel IPC carry per-account run ids).

**Why not stream bytes over the IPC?** `phase-1.5-plan.md`'s backpressure policy: stdin/stdout JSON-RPC framing is for control plane, not data plane. Streaming a 50 MB attachment over the JSON pipe blocks every other request behind it.

### Eviction + GC

`PackRuntime` (new): same shape as `CalendarRuntime` (per-account map, panic supervisor, kick handler) but per-blob state lives in the existing `pack_index` table.

Two kicks:

- `pack.eviction_kick` (5-min cadence). Service-side staleness gate: only run if size cap is exceeded or last run > 1 h ago.
- `pack.gc_kick` (5-min cadence with 24 h staleness gate). Sweeps blobs whose `messages` rows are gone.

Both inherit from `Drop` notification class - missed kicks self-heal next tick.

**Account-deletion crash recovery.** Today's account-deletion writes a "marker file" before starting external-store cleanup. Phase 6b reads the marker on boot and resumes any half-finished cleanup. The marker carries the account_id + the cleanup steps already completed; resuming idempotently re-runs the unfinished steps. This pattern matches Phase 4's marker-file lifecycle for sync; Phase 6b extends it to attachment cache cleanup.

### Cross-store invariant pass extension

`service::startup_invariants` already covers SQLite (Phase 3 task 11), body store, inline image store, and Tantivy (Phase 4). Phase 6b adds:

- **Pack file orphan sweep.** Scan the pack directory; drop files with no corresponding `attachments` row.
- **Pack-index reconciliation.** `pack_index` rows whose underlying file is gone get deleted.

The pass runs once at boot inside `BootPhase::ConsistencyPass`, after the body / inline / search passes. Cost is bounded by directory size; expected runtime well under a second on typical caches.

### Global write-half lockdown

The `service-state` crate today exports `WriteDbState::from_arc` as a public constructor. The `app` crate currently uses it to mint a write handle from the `Arc<Mutex<Connection>>` it shares with the Service via the boot context (Phase 1.5).

After Phase 6b:

- `WriteDbState::from_arc` becomes `pub(crate)` to `service-state`.
- The same applies to write-handle constructors for `BodyStoreWriteState`, `InlineImageStoreWriteState`, `SearchWriteHandle`.
- `crates/app/Cargo.toml` drops the `service-state = ...` dependency. Any UI code that needs a read handle keeps using `db::ReadDbState` and friends from the read crate.

The compile-time enforcement is what matters: a regression that re-introduces `service-state` as an `app` dependency requires editing `Cargo.toml`, which is a focal-point review surface. Adding a CI check that fails on `app -> service-state` dependency closes the loop.

### Architecture-doc update

Phase 6a's architecture-doc rewrite captured the post-6a state. Phase 6b's update is smaller:

- **Action service as mutation gate** § "Enforcement": replace the "global lockdown lands at Phase 6" text with "global lockdown landed at Phase 6b. The `app` crate no longer depends on `service-state`."
- **Current Exceptions** § `cal::actions`: keep until Phase 6c lands.
- **Settled Patterns** § "Service kick handlers": add `pack.eviction_kick` and `pack.gc_kick`.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

**0. Inventory + open questions resolution.** Survey today's UI-side OAuth call sites (`core::oauth::exchange_code`, `core::oauth::refresh_token`, persistence call paths). Confirm the auth-code redacting type's existing wrapper (or add it). Document decisions inline.

**1. `oauth.exchange_code` IPC.** Wire types in `service-api/src/oauth.rs`. Service handler in `service/src/handlers/oauth.rs`. UI-side `service_client.rs` async wrapper. UI redirect handler routes through the new IPC.

**2. Service-side OAuth refresh.** `service/src/oauth/refresh.rs` per-provider refresh helper. Sync paths consume it instead of the `oauth.refresh_request` IPC. Delete the temporary IPC and the UI handler.

**3. `attachment.fetch` IPC.** Wire types. Service handler that ensures the file is local before returning the ack. UI-side cache-miss path replaces `core::attachment_cache::fetch_*` direct calls with the IPC.

**4. `PackRuntime` skeleton.** Per-account map, panic supervisor, kick handler. No eviction policy yet - the runtime exists, but kicks are no-ops. Lands first to keep the lifecycle pieces small.

**5. Eviction policy.** LRU + size cap implementation inside `PackRuntime`. `pack.eviction_kick` notification. UI-side `kick_pack_eviction` on `Message::SyncTick`.

**6. Garbage collection.** Sweep for orphan blobs. `pack.gc_kick` notification.

**7. Cross-store invariant pass extension.** Pack orphan sweep + index reconciliation in `service::startup_invariants`. Account-deletion crash recovery via marker file.

**8. Global lockdown.** `service-state` constructors become `pub(crate)`. `crates/app/Cargo.toml` drops the `service-state` dependency. CI script (or Cargo metadata test) enforces the absence of the dependency. Any compile errors get fixed by routing through existing IPC methods.

**9. `core::oauth` cleanup.** Remove the now-dead UI-side OAuth call sites. Imports from `crates/app/`.

**10. `docs/architecture.md` Phase 6b delta.** Per § "Architecture-doc update" above. Updates `implementation-roadmap.md` Phase 6 entry to reflect 6b "LANDED" status.

## File-by-file changes

**New files:**
- `crates/service-api/src/oauth.rs` - OAuth wire types.
- `crates/service-api/src/attachment.rs` - attachment-fetch wire types.
- `crates/service-api/src/pack.rs` - pack eviction/gc notifications.
- `crates/service/src/handlers/oauth.rs` - OAuth handlers.
- `crates/service/src/handlers/attachment.rs` - attachment-fetch handler.
- `crates/service/src/handlers/pack.rs` - pack kick handlers.
- `crates/service/src/oauth/refresh.rs` - per-provider refresh helpers.
- `crates/service/src/pack.rs` - `PackRuntime`.

**Modified files:**
- `crates/service-api/src/lib.rs` - module declarations.
- `crates/service-api/src/request.rs` - new `RequestParams` variants + 5 s timeouts (OAuth gets a longer timeout - 30 s - to accommodate provider token-endpoint latency).
- `crates/service-api/src/notification.rs` - new `ClientNotification::PackEvictionKick` + `PackGcKick`.
- `crates/service/src/dispatch.rs` - new request + notification arms.
- `crates/service/src/startup_invariants.rs` - pack-orphan sweep + index reconciliation.
- `crates/service/src/boot.rs` - install `PackRuntime` slot.
- `crates/service/src/boot_state.rs` - `pack_runtime` accessor.
- `crates/service-state/src/lib.rs` - constructor visibility flips.
- `crates/app/Cargo.toml` - drop `service-state` dependency.
- `crates/app/src/service_client.rs` - new IPC wrappers.
- `crates/app/src/handlers/provider.rs` - `kick_pack_eviction` + `kick_pack_gc`.
- `crates/app/src/update.rs` - `Message::SyncTick` fan-out.
- `crates/core/src/oauth/` - dead UI-side call site removal.
- `crates/core/src/attachment_cache.rs` - cache-miss path now calls the IPC.
- `docs/architecture.md` - Phase 6b delta.
- `docs/service/implementation-roadmap.md` - mark Phase 6b "LANDED".

## Code-comment requirements

1. **`crates/service/src/handlers/oauth.rs::handle_exchange_code`** must contain:
   - "OAuth code is a one-shot bearer credential. The wire-type wrapper redacts it from logs. After Phase 6b the auth code never reaches the UI process beyond the redirect handler that captures it; the IPC ships the code straight to Service, which exchanges + persists in one transaction."

2. **`crates/service/src/handlers/oauth.rs`** module-level doc-comment:
   - "Phase 6b reverses the Phase 4 `oauth.refresh_request` direction. Phase 4 added a temporary IPC where Service-side sync asked the UI for a fresh token; Phase 6b makes Service refresh its own tokens via the per-provider helpers in `service::oauth::refresh`. The temporary IPC is deleted in the same commit that lands the refresh helpers."

3. **`crates/service/src/handlers/attachment.rs::handle_fetch`** must contain:
   - "Wire ack carries `{ content_hash, size, pack_path, offset, length }`. Bytes never cross the IPC - phase-1.5-plan.md backpressure policy. UI reads positionally from `pack_path` at the returned offset+length window."

4. **`crates/service/src/pack.rs` module-level doc-comment** must contain:
   - "PackRuntime structurally mirrors CalendarRuntime (per-account map, panic supervisor, kick handler). Diverges on: per-blob state lives in `pack_index` rather than in-memory; no per-account semaphore (eviction is global). Eviction policy is LRU + size cap (5 GB default). GC drops blobs whose `messages` rows are gone."

5. **`docs/architecture.md` § "Action service as mutation gate"** new sentence:
   - "Phase 6b closed the global write-half lockdown: the `app` crate no longer depends on `service-state`, and the constructors of `WriteDbState`, `BodyStoreWriteState`, `InlineImageStoreWriteState`, and `SearchWriteHandle` are not reachable from any UI source file."

## Test plan

### Unit tests

- Wire-type round-trips for `oauth`, `attachment`, `pack` modules.
- `oauth.exchange_code` redacting test: serialize a `OauthExchangeCodeParams` with a known auth code; assert the code does not appear in the `Debug` output.
- Pack-eviction LRU test: seed a pack with 100 MB of blobs, set a 50 MB cap, fire eviction; assert the oldest blobs are dropped first.
- Pack-GC orphan-sweep test: insert a pack file with no corresponding `attachments` row; fire GC; assert the file is deleted.
- Cross-store invariant pass: simulate a half-finished account deletion (marker file present, some packs not yet cleaned); boot with the pass enabled; assert the cleanup completes.

### Integration tests (in-process)

- `oauth_exchange_round_trips`: stub provider token endpoint; UI-side IPC call returns ack; row in `accounts` has the expected encrypted token bytes.
- `service_refreshes_own_token_on_sync`: stub provider token endpoint; trigger sync against an account whose token is about to expire; assert refresh ran Service-side and the IPC `oauth.refresh_request` was not invoked.
- `attachment_fetch_cache_miss`: clear the pack cache; trigger the IPC; assert the file lands at the expected pack offset.
- `lockdown_app_does_not_depend_on_service_state`: parse `crates/app/Cargo.toml`; assert the `service-state` dependency is absent.

### Real-subprocess smoke tests

- `service_subprocess_oauth_full_flow`: spawn Service with an OAuth-enabled stub account; UI ships an auth code via IPC; Service exchanges + persists; UI re-reads the row over the existing read path. Asserts no `oauth.refresh_request` IPC fires during the test.
- `service_subprocess_pack_eviction`: seed a pack at 110% of the size cap; fire eviction kick; assert the pack drops to the cap.

### Manual matrix updates

- OAuth flow end-to-end (Google, Microsoft) via real provider endpoints.
- Cache-miss attachment open in the reading pane (assert the user sees no UI freeze during the IPC round-trip).
- Account deletion mid-flight crash recovery (kill -9 the Service mid-cleanup; restart; assert the orphaned packs disappear on the next boot).

## Open questions

- **Eviction size cap default.** 5 GB chosen as a starting point; the cache today is uncapped. Plan picks 5 GB; revisit after the first weeks of dogfooding.
- **OAuth IPC priority lane.** Plan defers to "bounded queue depth" rather than priority dispatch. If measurement shows OAuth contention with heavy concurrent IPC load, the priority-lane refactor lands as a follow-up commit (not a separate plan).
- **Marker-file format for account-deletion crash recovery.** JSON for readability or a binary cookie for write-once-ness? Plan picks JSON - the marker is small (account id + step list); readability wins.

## Verification (end-to-end)

- `git grep with_write_conn crates/app/src/` returns only the calendar event mutation sites (Phase 6c).
- `crates/app/Cargo.toml` does not list `service-state` as a dependency.
- The `WriteDbState::from_arc` constructor is unreachable from the `app` crate (compile error if a UI source file tries to import it).
- An OAuth login completes end-to-end without the UI invoking `core::oauth::exchange_code` directly.
- A token expires mid-sync and Service refreshes it without invoking `oauth.refresh_request`.
- An attachment cache-miss completes via `attachment.fetch`; the UI never reads the pack file before the Service has confirmed the bytes are present.
- A Service crash mid-account-delete is recoverable on next boot - the orphaned packs disappear.

## Promotion criteria

- All items in `In scope` landed.
- Calendar event mutations are the only remaining UI write surface, and they are explicitly tracked in the Phase 6c plan (or roadmap entry, if the 6c plan has not yet been drafted).
- `docs/architecture.md` reflects the post-Phase-6b state.
- `phase-6b-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry; every code-comment requirement is present in the relevant file.
