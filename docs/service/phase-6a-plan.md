# The Service - Phase 6a Plan: small UI write-surface relocations + encryption-key handle

Companion to `phase-1-plan.md`, `phase-1.5-plan.md`, `phase-2-plan.md`, `phase-3-plan.md`, `phase-4-plan.md`, `phase-5-plan.md`. Implements the first half of Phase 6 of `implementation-roadmap.md`.

## Revision history

**2026-05-06 - initial draft.** Phase 5 closed the calendar/GAL relocation and IMAP cancellation depth. Phase 6 was originally scoped as a single milestone covering every remaining UI write surface plus the global lockdown. Splitting into 6a/6b/6c (calendar event mutations) keeps each plan small enough to review against the actual scope.

## Context

After Phase 5 the UI write surface that bypasses the Service is much narrower than the roadmap's original "long tail." Concretely, `git grep with_write_conn crates/app/src/` returns 13 sites (excluding the helper definitions in `db/connection.rs`):

- **Account lifecycle** (5 sites): account creation in `ui/add_account/identity.rs:34`; OAuth client-credential persistence in `ui/add_account/state.rs:493` and `ui/add_account/oauth.rs:149`; account update in `handlers/core.rs:801`; account orchestrated-delete in `handlers/core.rs:708`.
- **Contacts/groups** (3 sites): `db/contacts.rs:189,217,235` - group create/update/delete.
- **Calendar** (4 sites): `db/calendar.rs:47,58,107` (event create/update/delete) + `:176` (calendar visibility toggle). The first three are Phase 6c (calendar event mutations); the visibility toggle is Phase 6a because it is a flat boolean preference flag, not a series-vs-occurrence mutation.
- **Attachment collapse preference** (1 site): `db/threads.rs:184`.

Phase 6a closes everything in this list except OAuth two-step (Phase 6b) and event mutations (Phase 6c). It also lands the encryption-key handle (Phase 2 carry-forward 19d): the UI today re-reads `ratatoskr.key` from disk in `from_boot_ready` even though the Service has already loaded and validated the same file at boot, leaving a TOCTOU window.

The phase ships as one milestone with a clean commit-level split. A regression should bisect to the right commit.

## Scope

### In scope

- **Preferences (`prefs.set` IPC).** Bulk + targeted setters for `app_preferences` rows. Today the UI writes preferences directly via the action service for ones routed through `MailOperation`-shaped paths, and inline via `db.with_write_conn` for surfaces that do not fit the action shape (theme, layout flags, attachment collapse, calendar visibility). Phase 6a unifies these behind `prefs.set`. Inventory the call sites during task 0 - the design here turns on whether the surface is "set one key" or "set a coherent group" (e.g., calendar visibility per account, attachment collapse per thread).
- **Account create / update / delete / reorder.** Three new methods on `service-api`: `account.create { provider, email, ... }`, `account.update { account_id, params }` (mirror of `update_account_sync`), `account.delete { account_id }` (today's `cancel_and_await` + orchestrated delete already lives Service-side; the IPC-side wrapper hides the multi-store cleanup orchestration). Reorder is its own typed request (`account.reorder { ordering }`). Account creation deliberately stays separate from OAuth two-step (Phase 6b) - the UI hands the Service a "create this account with these credentials" request; OAuth coordination is the Phase 6b problem.
- **Signature CRUD + reorder.** `signature.create | update | delete | reorder`. Mirrors the existing `db::queries_extra::signatures` API; the wire types live in `service-api::signature`. Signatures are flagged in `architecture.md` § Current Exceptions as "not yet a settled architecture surface" - Phase 6a closes the write-surface migration without changing the product/spec shape, so the exception entry stays.
- **Local draft auto-save (`draft.save`).** Today's UI-side write hits `local_drafts` directly. The IPC version hands the Service a `LocalDraftRow`-equivalent payload; Service writes through the same DB helper. **Window-close ordering** is the load-bearing concern: `iced::exit()` fires before `draft.save` round-trips, so the user can lose the latest character-stroke. Resolution shape (decided in task 0): UI emits one synchronous `draft.save` per editor before issuing `service.shutdown`, awaiting the ack with a 500ms ceiling per draft. If the ack times out, the UI logs and proceeds - data loss is bounded by the in-memory editor state since the last keystroke, same as today's "what's typed since last auto-save" window.
- **Pinned searches.** `pinned_search.create_or_update | delete | delete_all | expire_stale`. Read paths stay UI-side (they hit `&ReadDbState` via `db::pinned_searches`). The expire-stale entry is naturally cadence-driven; Phase 6a moves the cadence to a Service-side `pinned_search.kick` notification (5-min `SyncTick`), mirroring `gal.kick`'s shape.
- **Contacts/groups CRUD.** `contacts.group_create | group_update | group_delete`. Mirrors today's `db/contacts.rs` API. The Phase 6a IPC keeps the UI-facing semantics unchanged - the editor still works on a shadow copy, commit fires the IPC, ack returns the canonical row for re-render.
- **Attachment collapse preference.** `prefs.set_attachments_collapsed { account_id, thread_id, collapsed }`. Folded into the prefs surface above; called out separately because it is the only thread-scoped preference and the IPC needs to carry both ids.
- **Calendar visibility toggle.** `calendar.set_visibility { calendar_id, visible }`. Added to the existing `calendar.*` surface (Phase 5 introduced `calendar.start_account_sync` / `cancel_account_sync` / `kick`). This is the flat-boolean half of `db/calendar.rs`; event mutations stay UI-side until Phase 6c.
- **Encryption-key handle (Phase 2 carry-forward 19d).** UI stops calling `rtsk::load_encryption_key` (`crates/app/src/app.rs:331` in `from_boot_ready`). Service exports an encrypt-for-storage handle: `internal.encrypt_for_storage { plaintext } -> { ciphertext }`. UI-side credential persistence (OAuth tokens, account passwords) routes through this. Default design choice: handle-based (option a from `phase-2-plan.md` § 19d). Survey credential-persist call sites during task 0 to confirm the per-encrypt round-trip cost is tolerable; fall back to option b (one-shot bytes export) if measurement reveals a hot path.
- **Service-side helpers reuse.** Each new IPC method is a thin wrapper around an existing `db::queries_extra::*_sync` function (or `rtsk::*` helper). No business-logic relocation in 6a - the UI shape stays identical, just on the other side of the boundary.
- **`docs/architecture.md` update.** The doc has not been touched since before Phase 4. By the end of 6a it must reflect: Phase 5's `CalendarRuntime` + dual-notification routing + GAL kick + IMAP cancellation depth; the post-Phase-6a state of the UI write surface (which call sites went through IPC vs which still write directly); the encryption-key handle's mediation through the Service. The current "**Current Exceptions**" entry for body / inline / search write halves staying UI-side stays accurate (Phase 6b lands the global lockdown), but its phrasing needs the Phase 5 + Phase 6a deltas.

### Out of scope

- **OAuth two-step.** Phase 6b. The `oauth.exchange_code` IPC and the elimination of the temporary `oauth.refresh_request` from Phase 4 land there. Account creation in 6a takes already-acquired credentials; the redirect-capture round-trip stays UI-side this phase.
- **`attachment.fetch` IPC for cache-miss reads.** Phase 6b. The pack-store reader path stays UI-side this phase.
- **Eviction / GC for the attachment cache.** Phase 6b.
- **Cross-store invariant pass extension** (blob-store reconciliation). Phase 6b.
- **Calendar event mutations (`cal::actions::*`).** Phase 6c (`docs/service/phase-6c-plan.md`). The existing UI-side mutation handlers in `handlers/calendar.rs` continue to call `cal::actions` directly with a UI-side `ActionContext`. Phase 5 documented this as a known exception; 6a does not change it.
- **Global write-half lockdown.** Phase 6b. Removing the public constructor of `WriteDbState` and the body / inline / search write halves from the `app` crate is meaningful only after OAuth and `attachment.fetch` are gone. 6a leaves `app` depending on `service-state` for the surfaces 6b will close out; the type-level lockdown is the Phase 6b promotion gate.
- **Signature spec rework.** The architecture doc explicitly flags signatures as "not yet a settled architecture surface." 6a relocates the write surface without changing the product shape; whatever spec work happens later can edit the IPC types in place.

## Architecture

### Wire-type pattern

Each new IPC method gets:

1. A typed `Params` struct in `crates/service-api/src/{prefs,account,signature,draft,pinned_search,contacts,calendar,internal}.rs`.
2. An `Ack` (or named result) struct mirroring the existing local return type.
3. A variant in `RequestParams` with a 5 s timeout (consistent with Phase 5's calendar-request budget).
4. A serde round-trip test colocated with the type (matching the Phase 5 review-pass discipline).

The IPC side does not need to mirror every internal helper. Where today's UI calls a single `db::queries_extra::*_sync` function, the IPC method wraps that single function. Where today's UI orchestrates two writes in sequence (e.g., the account-creation path that writes `accounts` and then `account_provider_credentials`), the IPC method wraps both - the orchestration moves with the writes.

### Service-side handler shape

Handlers follow the `service::handlers::*` pattern Phase 5 established:

```rust
pub(crate) async fn handle_signature_create(
    boot_state: &Arc<BootSharedState>,
    params: SignatureCreateParams,
) -> Result<Value, ServiceError> {
    let Some(conn) = boot_state.db_conn() else {
        return Err(ServiceError::Internal(
            "signature.create received before db_conn available; UI must wait for boot.ready".into()
        ));
    };
    let write_db = WriteDbState::from_arc(conn);
    let signature = write_db
        .with_conn(move |conn| db::queries_extra::signatures::create_signature_sync(conn, &params))
        .await
        .map_err(ServiceError::Internal)?;
    serde_json::to_value(signature).map_err(|e| ServiceError::Internal(e.to_string()))
}
```

Pure write-through. No runtime, no kick handler, no notification dispatch beyond the request-response cycle. The handler exists to (a) cross the boundary, (b) hold the `WriteDbState` borrow on the Service side. The body of each handler is six to twelve lines.

### `prefs.set` shape

Two design questions on the prefs IPC, settled here:

1. **Single key or batch?** Single key. The `app_preferences` table is keyed on `(account_id, key)`; the UI's existing call sites set one key at a time. Adding a batch variant later is additive (`prefs.set_many { entries: Vec<PrefEntry> }`); leading with single-key keeps the wire type small.

2. **Type-erased value or per-key dispatch?** Type-erased. `PrefValue { kind: PrefValueKind, value: serde_json::Value }`. The Service-side handler decodes against the key's expected schema and rejects mismatches with `ServiceError::InvalidParams`. Per-key dispatch (one IPC method per pref) explodes the wire surface for no real benefit - prefs are intrinsically a key-value store. The validation responsibility lives in `db::queries_extra::preferences::set_pref_sync`, which already has the shape.

The `set_attachments_collapsed` call site is the one departure from this model - the table is `thread_attachment_state`, not `app_preferences`, and the key tuple is `(account_id, thread_id)`. It gets its own `prefs.set_attachments_collapsed { account_id, thread_id, collapsed }` IPC for the same reason `gal.kick` is its own IPC: shoehorning a 3-tuple key into a generic `PrefValue` payload is more code than just naming the surface explicitly.

### Encryption-key handle: handle-based design

Two designs from `phase-2-plan.md` § 19d:

- **(a) Handle-based** - Service holds the raw 32 bytes; UI calls `internal.encrypt_for_storage { plaintext } -> ciphertext` per credential persist. Adds one IPC round-trip per encrypt; the Service is the sole holder of the bytes after boot.
- **(b) Trusted-bytes-once** - Service exports the bytes once via a one-shot IPC; UI keeps them in memory. No per-encrypt round-trip; weakens the "Service is sole holder" property.

**Decision: option (a) by default.** Survey the credential-persist call sites in task 0 (account create, OAuth token persist, password persist):

- Account creation runs once per account-add - human-paced, IPC overhead is invisible.
- OAuth token persist runs at every refresh. Phase 6b moves OAuth refresh entirely Service-side, so by the end of 6b this is a no-op for the UI.
- Password persist runs at account-add and at re-auth. Same human-paced cadence as account creation.

No hot path. Option (a) is correct without measurement. The `internal.encrypt_for_storage` IPC method gets a 5 s timeout and lives in `service-api/src/internal.rs` (new module).

The IPC accepts `Vec<u8>` plaintext and returns `Vec<u8>` ciphertext (the `iv:ciphertext_with_tag` shape that `crypto::encrypt_value` already produces). The UI never sees the key; it consumes the wire format directly.

The corresponding decrypt path stays UI-side as a `internal.decrypt_for_storage { ciphertext } -> plaintext` IPC because today's UI also decrypts (e.g., when reading account passwords back to populate the re-auth wizard). Phase 6b's OAuth two-step relocation deletes the OAuth refresh decrypt, but the password decrypt remains. **Both encrypt and decrypt land in 6a together** - splitting them lets a half-migrated UI sit in a state where it can write an encrypted blob it can no longer read.

### Pinned-search expire-stale cadence

Today the UI dispatches `expire_stale_pinned_searches` on `Message::SyncTick` (5-min cadence). Phase 5 collapsed `SyncTick` to "three notifications + one request fan-out"; adding a fourth notification (`pinned_search.kick`) keeps the pattern.

```rust
Message::SyncTick => {
    let sync_task = self.sync_all_accounts();
    let pending_task = self.process_pending_ops();
    let gal_task = self.kick_gal_refresh();
    let cal_task = self.kick_calendar_sync();
    let pinned_task = self.kick_pinned_search_expire(); // NEW
    Task::batch([sync_task, pending_task, gal_task, cal_task, pinned_task])
}
```

`pinned_search.kick` is a `Drop`-class notification. Service handler computes the staleness threshold (today's 90-day default) and calls `expire_stale_pinned_searches_sync`. No per-account targeting; the table is global.

### Account-deletion handler

Phase 5 already moved `cancel_and_await` to a piggyback model that tears down sync + push + calendar runners before returning. The remaining UI-side work in `handlers/core.rs:708` (the post-cancel orchestrated delete) is two pieces: the `delete_account_orchestrate` DB call and the four external-store cleanups (body, inline, attachment cache, search index).

`account.delete { account_id }` fans these out Service-side. The IPC ack returns an `AccountDeletionReport` carrying the per-store cleanup counts (already a struct: `rtsk::account::types::AccountDeletionCleanupReport`). The UI surface unchanged - the existing `Message::AccountDeleted` arm consumes the report.

The UI's existing `cancel_and_await` call before the deletion request stays in place - the request-side ordering is "cancel runners, await terminal completions, then issue `account.delete`." Bundling cancel + delete into one IPC would conflate two concerns; keeping them separate keeps each handler narrow.

### docs/architecture.md update plan

Out-of-date sections, with the per-section delta:

- **Crate Boundaries** (line 11). Mention `service`, `service-api`, `service-state`, `service-state-types`, `crypto-key`. Strike the implicit assumption that `app -> rusqlite` is a normal call path.
- **Action service as mutation gate** (line 42). The "body / inline / search write halves stay UI-side until Phase 3 (sync) and Phase 6 (rest); the global lockdown lands at Phase 6" text needs a Phase 5 + Phase 6a + Phase 6b breakdown.
- **Calendar workflow state owns meaning** (line 75). Add a paragraph on the Phase 5 `CalendarRuntime`: the four-layer state (view/workflow/editor/surface) lives UI-side, but the periodic sync + cache-refresh work runs Service-side via `CalendarRuntime`. Event mutations (`cal::actions`) still bypass; that exception moves to 6c.
- **Settled Patterns** (line 119). Update the "State types are `Clone`" entry: read halves stay UI-side, write halves are progressively Service-only. Add a "Service kick handlers" pattern entry covering `gal.kick`, `calendar.kick`, `pending_ops.kick`, `pinned_search.kick` (new in 6a).
- **Current Exceptions** (line 139). Strike the obsolete ones; add `cal::actions` (deferred to 6c) explicitly. The signatures-as-unsettled exception stays.

The doc rewrite lands as the final commit of Phase 6a so the diff captures the fully-realized state, not the half-migrated state.

## Detailed task list

In recommended commit order. Each item is one focused commit unless noted.

**0. Inventory + open questions resolution.** Survey `with_write_conn` call sites in `crates/app/src/` (already done at plan-draft time, see § Context). Confirm credential-persist cadence for the encryption-key handle decision (a vs b). Confirm `prefs.set` single-vs-batch shape against the actual UI call patterns. Document the decisions inline; no code changes.

**1. `service-api` wire types.** New modules: `prefs`, `account`, `signature`, `draft`, `pinned_search`, `contacts`, `internal`. Extend the existing `calendar` module with `CalendarSetVisibilityParams` + ack. Add `RequestParams` variants with 5 s timeouts. Serde round-trip tests per type.

**2. `service::handlers::*` skeleton.** New handler files for each surface. Each handler is a thin wrapper around the existing `db::queries_extra::*_sync` (or `rtsk::*::*_sync`) function. Service-side dispatch arms in `dispatch.rs`.

**3. Encryption-key handle.** `internal.encrypt_for_storage` + `internal.decrypt_for_storage`. Wire types, handler, dispatch arm. UI side: `service_client.rs` adds `encrypt_for_storage` / `decrypt_for_storage` async methods. UI `from_boot_ready` (`app.rs:331`) drops the `rtsk::load_encryption_key` call site; credential-persist call sites route through the new helpers.

**4. Preferences (`prefs.set` + `prefs.set_attachments_collapsed`).** UI replaces direct `with_write_conn` calls in `db/threads.rs:184` with the new IPC. Survey other UI-side preference writes (theme, layout flags, calendar visibility) and route them all through `prefs.set` or the dedicated `prefs.set_attachments_collapsed`.

**5. Calendar visibility (`calendar.set_visibility`).** UI replaces `db/calendar.rs:176`. Smallest commit; lands first in the calendar surface so the Phase 6c calendar-event work has a precedent.

**6. Pinned searches.** UI replaces direct calls in `db/pinned_searches.rs`. The expire-stale cadence moves to `pinned_search.kick`; UI's `Message::SyncTick` gains `kick_pinned_search_expire`. Delete the UI-side `expire_stale_pinned_searches` call site.

**7. Contacts/groups.** UI replaces `db/contacts.rs:189,217,235`.

**8. Signatures.** UI replaces all `db::queries_extra::signatures::*_sync` write calls (find them; the inventory above did not enumerate them by file because the helpers are in `core/src/db/queries_extra/signatures.rs` and the UI call sites are in `handlers/`).

**9. Local drafts (`draft.save`).** Window-close ordering: UI emits one synchronous `draft.save` per dirty editor before `service.shutdown`. Per-draft 500 ms ack ceiling. Add a unit test that asserts the shutdown sequence drains pending draft saves.

**10. Account create.** UI replaces `ui/add_account/identity.rs:34` and the credential-persist call sites in `state.rs:493` + `oauth.rs:149` (the latter two now route through `internal.encrypt_for_storage` from task 3, then ship the ciphertext blobs over the new `account.create` IPC). The OAuth code-exchange IPC stays UI-side this phase; `account.create` accepts already-encrypted credential bytes.

**11. Account update / reorder.** UI replaces `handlers/core.rs:801`. Reorder is a separate IPC because it operates on multiple rows in a single transaction; folding it into update would conflate single-row and multi-row semantics.

**12. Account delete.** UI replaces `handlers/core.rs:708`. The four external-store cleanups move into the Service handler. UI keeps the `cancel_and_await` step before the delete request - cancel and delete remain separate IPC concerns.

**13. `app` crate read/write surface check.** `git grep with_write_conn crates/app/src/` should now return only the calendar event mutation sites (`db/calendar.rs:47,58,107`) and the OAuth flow sites that 6b will close. Add a regression assertion in `crates/app/tests/` (or a CI script) that fails if any other write call site appears.

**14. `docs/architecture.md` rewrite.** Final commit of 6a. Per § "docs/architecture.md update plan" above. Updates `implementation-roadmap.md` Phase 6a entry to "LANDED" status as well.

## File-by-file changes

**New files:**
- `crates/service-api/src/prefs.rs` - prefs wire types.
- `crates/service-api/src/account.rs` - account wire types.
- `crates/service-api/src/signature.rs` - signature wire types.
- `crates/service-api/src/draft.rs` - draft wire types.
- `crates/service-api/src/pinned_search.rs` - pinned-search wire types + `ClientNotification::PinnedSearchKick`.
- `crates/service-api/src/contacts.rs` - contacts/groups wire types.
- `crates/service-api/src/internal.rs` - encryption-key handle wire types.
- `crates/service/src/handlers/prefs.rs` - prefs handlers.
- `crates/service/src/handlers/account.rs` - account create/update/delete/reorder handlers.
- `crates/service/src/handlers/signature.rs` - signature handlers.
- `crates/service/src/handlers/draft.rs` - draft handler.
- `crates/service/src/handlers/pinned_search.rs` - pinned-search handlers + kick handler.
- `crates/service/src/handlers/contacts.rs` - contacts handlers (today's `actions/contacts.rs` keeps the action handlers; the new file owns the non-action CRUD).
- `crates/service/src/handlers/internal.rs` - encrypt_for_storage / decrypt_for_storage handlers.

**Modified files:**
- `crates/service-api/src/lib.rs` - module declarations.
- `crates/service-api/src/calendar.rs` - add `CalendarSetVisibilityParams` + ack.
- `crates/service-api/src/request.rs` - new `RequestParams` variants + 5 s timeouts.
- `crates/service-api/src/notification.rs` - new `ClientNotification::PinnedSearchKick`.
- `crates/service/src/dispatch.rs` - new request arms + the pinned-search kick arm.
- `crates/app/src/service_client.rs` - new async wrappers per IPC method.
- `crates/app/src/app.rs::from_boot_ready` - drop `rtsk::load_encryption_key` call.
- `crates/app/src/db/threads.rs` - replace `set_attachments_collapsed` with IPC call.
- `crates/app/src/db/calendar.rs` - replace `set_calendar_visibility` with IPC call (event mutations untouched - 6c).
- `crates/app/src/db/contacts.rs` - replace group CRUD with IPC calls.
- `crates/app/src/db/pinned_searches.rs` - replace write paths with IPC calls; remove `expire_stale_pinned_searches` (moves to kick handler).
- `crates/app/src/handlers/core.rs` - replace `handle_save_account_changes` write + the orchestrated-delete write block with IPC calls.
- `crates/app/src/ui/add_account/{state,oauth,identity}.rs` - replace `with_write_conn` blocks with `account.create` + `internal.encrypt_for_storage` IPC calls.
- `crates/app/src/handlers/provider.rs` - new `kick_pinned_search_expire` task helper.
- `crates/app/src/update.rs` - extend `Message::SyncTick` fan-out with pinned-search kick.
- `docs/architecture.md` - per § "docs/architecture.md update plan" above.
- `docs/service/implementation-roadmap.md` - mark Phase 6a entry as "LANDED" and adjust the Phase 6 split text to reflect 6a/6b/6c.

## Code-comment requirements

1. **`crates/service/src/handlers/internal.rs` module-level doc-comment** must contain:
   - "The encryption-key handle is the boundary that closes Phase 2 carry-forward 19d. After Phase 6a the UI no longer reads `ratatoskr.key` from disk. Credential persistence flows through `encrypt_for_storage` (UI ships plaintext, Service returns wire-format ciphertext). The corresponding decrypt path stays in `decrypt_for_storage`. Splitting encrypt and decrypt across phases is forbidden - a half-migrated UI could write a blob it can no longer read."

2. **`crates/app/src/app.rs::from_boot_ready`** at the deletion point of the `load_encryption_key` call:
   - "Phase 6a: encryption key is no longer loaded UI-side. Credential persistence routes through `internal.encrypt_for_storage` / `decrypt_for_storage`. See `docs/service/phase-6a-plan.md` § "Encryption-key handle: handle-based design" for the round-trip-cost rationale (no hot path; per-encrypt IPC is invisible at human-paced cadences)."

3. **`crates/service/src/handlers/draft.rs::handle_save`** must contain:
   - "Window-close ordering. UI emits one synchronous `draft.save` per dirty editor before issuing `service.shutdown`, awaiting per-draft acks with a 500 ms ceiling. If an ack times out, UI logs and proceeds - data loss is bounded by the in-memory editor state since the last keystroke (same as today's pre-IPC auto-save window). The 500 ms ceiling is calibrated against the 5 s default request timeout: short enough that ten dirty editors do not stall shutdown by 50 s, long enough that a non-stalled Service answers comfortably."

4. **`crates/service/src/handlers/pinned_search.rs::handle_kick`** must contain:
   - "The expire-stale cadence moved Service-side at Phase 6a. Notification class is `Drop` - missed kicks self-heal on the next `SyncTick`. Same shape as `gal.kick` and `calendar.kick`."

5. **`docs/architecture.md` § "Action service as mutation gate"** new sentence:
   - "Phase 6a relocated the small UI write surfaces (preferences, account CRUD, signatures, drafts, pinned searches, contacts, calendar visibility) to Service-side handlers. Phase 6b will close out OAuth two-step coordination and the `attachment.fetch` cache-miss path; the global lockdown (write halves of all four state types unreachable from `app`) lands at the end of 6b. Calendar event mutations (Phase 6c) and the `cal::actions` write-surface escape are tracked as Current Exceptions until 6c lands."

## Test plan

### Unit tests

- Wire-type round-trips for every new `service-api` module (one test per `Params` and `Ack` type, matching the Phase 5 review-pass discipline).
- `internal.encrypt_for_storage` round-trip: encrypt arbitrary plaintext, decrypt via `internal.decrypt_for_storage`, assert byte-equality. Use a constructed `BootSharedState` with a synthetic `SecretKey`.
- `prefs.set` invalid-value rejection: send a `PrefValue` whose `kind` does not match the key's expected schema; assert `ServiceError::InvalidParams`.
- `pinned_search.kick` handler test: seed a stale pinned search, fire kick, assert the row is deleted.

### Integration tests (in-process)

- `account_create_round_trips_through_ipc`: Service handler creates an account; UI-side reader observes the same row via the existing read path.
- `account_delete_orchestrates_external_stores`: seed an account with body store + inline image entries; fire `account.delete`; assert the report's per-store counts match the seed.
- `draft_save_drains_at_window_close`: simulate the shutdown sequence with two dirty editors; assert both `draft.save` requests complete before `service.shutdown` is issued.

### Real-subprocess smoke tests

- `service_subprocess_account_lifecycle`: account create -> update -> delete via real IPC. Asserts the cleanup report arrives back over the wire.
- `service_subprocess_signature_crud`: same pattern for signatures.

### Manual matrix updates

Add the new IPC methods to `docs/service/manual-test-matrix.md`. Particularly: account create flow with OAuth credentials (verifies the encryption-key handle closes the loop end-to-end).

## Open questions

- **`prefs.set` validation responsibility**: Service-side handler decodes against the key's expected schema, or does the wire type carry a typed enum? Plan picks the former (handler validates), but the latter lets `cargo` catch a missing pref at compile time. Decide during task 1 implementation; the call is on whether the per-pref typed enum is small enough to be readable.
- **`account.delete` cleanup-report shape**: today's `AccountDeletionCleanupReport` is a flat struct of counts. Should it become a typed result (`{ bodies_deleted: u64, errors: Vec<String> }`) or stay flat? Probably flat - the UI does not act on the errors today, only logs them.
- **Pinned-search kick cadence**: 5-min `SyncTick` is the natural home, but pinned-search expire-stale runs against a 90-day window. A 1-hour cadence would be over-frequent; a 24-hour cadence would be under-frequent during long sessions. Plan keeps the 5-min cadence with the same self-gating logic the GAL handler uses (24 h expire-stale staleness check inside the handler).

## Verification (end-to-end)

- `git grep with_write_conn crates/app/src/` returns only `db/calendar.rs:47,58,107` (Phase 6c) and the OAuth flow sites (Phase 6b).
- A new account can be created end-to-end; the OAuth tokens it persists arrive at disk via `internal.encrypt_for_storage` rather than UI-side encryption.
- Window-close on a dirty composer drains the draft save before the Service exits.
- Pinned-search expire-stale runs Service-side on the 5-min `SyncTick`; UI no longer dispatches the call directly.
- Calendar visibility toggle works through the IPC and the UI re-renders on the ack.
- `docs/architecture.md` reflects the post-Phase-6a state without a "Phase 5" prefix on every subsection.

## Promotion criteria

- All items in `In scope` landed.
- All items in `Out of scope` are explicitly tracked in their target phase plan (6b for OAuth/attachment.fetch/eviction/cross-store invariant; 6c for calendar event mutations) - none are silently deferred.
- `crates/app/` no longer calls `rtsk::load_encryption_key`. The TOCTOU window flagged in `phase-2-plan.md` § 19d is closed.
- `docs/architecture.md` post-Phase-6a state is reviewer-confirmed.
- `phase-6a-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry; every code-comment requirement is present in the relevant file.
