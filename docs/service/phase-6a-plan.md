# The Service - Phase 6a Plan: small UI write-surface relocations + encryption-key handle

Companion to `phase-1-plan.md`, `phase-1.5-plan.md`, `phase-2-plan.md`, `phase-3-plan.md`, `phase-4-plan.md`, `phase-5-plan.md`. Implements the first half of Phase 6 of `implementation-roadmap.md`.

## Revision history

**2026-05-06 - initial draft.** Phase 5 closed the calendar/GAL relocation and IMAP cancellation depth. Phase 6 was originally scoped as a single milestone covering every remaining UI write surface plus the global lockdown. Splitting into 6a/6b/6c (calendar event mutations) keeps each plan small enough to review against the actual scope.

**2026-05-06 - post-Task-6-review revision (small).** Task 6 (`calendar.set_visibility`) shipped as the first end-to-end surface, then went through `review arch,bugs --oneshot` to validate the pattern before the remaining 11 surfaces inherited it. Reviewers caught four P1 issues that would compound across replication: (1) the ack was funnelled through an existing `CalendarMessage::EventSaved` variant whose handler had unrelated workflow side effects (closing the event editor mid-flight); (2) the eager UI flip had no rollback path on IPC failure or on `service_client = None`; (3) no wire-envelope round-trip test (`*_round_trips_from_method_params`) covering the dispatch-table; (4) per-handler `WriteDbState::from_arc(conn)` boilerplate would copy 12 times. Plan now codifies a "Per-surface checklist" + "UI-side ack message and rollback policy" section so the remaining 10 surfaces inherit the corrected shape rather than the original Task 6 anti-pattern. Three deferred items (per-entity ordering, AckUnknown reconciliation, typed `NotReady` error variant) are explicitly tracked rather than left as silent gaps. The Task 6 fixup commit lands these patterns directly.

**2026-05-06 - post-arch-review revision.** Two reviewers (claude + codex) independently flagged that the original inventory was too narrow: `Db::write_db_state()` (`crates/app/src/db/connection.rs:32`) returns the writable connection wrapped as `ReadDbState`, and 14 UI call sites use this method directly without going through `Db::with_write_conn`. The original `with_write_conn` grep missed all of them. The 6b lockdown's planned "app drops `service-state`" check is also already true today and would not prove the mutation gate. Plan revised: inventory enumerates *both* `Db::with_write_conn*` and `Db::write_db_state()` callers; verification flips from a negative grep to a positive allow-list (the two methods are deleted from `Db` once their callers go away in 6a, except for the cal::actions construction at `app.rs:336` which 6c removes). Other revisions: `prefs.set` recast around the actual `settings` (global key/value) and `thread_ui_state` (per-thread) tables - the original draft cited tables that do not exist; prefs switches to a typed enum matching the project's `MailOperation` exhaustive-match house style; the encryption-key handle survey now includes the bootstrap-snapshot decrypts at `app.rs:368-369`, which would otherwise become N IPC round-trips on every cold boot - the design moves the snapshot reads Service-side via a dedicated `internal.read_bootstrap_snapshots` IPC, scoping the residual encrypt/decrypt surface to genuinely one-shot uses; draft auto-save pivots from "synchronous IPC during shutdown" to a UI-side WAL drained by the Service on next boot, preserving the "guaranteed" semantics today's local SQLite write provides; `account.create` lands with a `Plaintext | Encrypted` credentials envelope from day one so the 6b OAuth two-step adds a variant rather than redefining the wire contract; `account.delete` moves the `cancel_and_await` step Service-side so a future caller cannot delete while runners hold references; save-as-smart-folder added to the pinned-search relocation.

## Context

After Phase 5 the UI write surface that bypasses the Service has two access patterns. The original draft inventoried only the first; the post-review revision adds the second:

**Pattern A: `Db::with_write_conn` (12 call sites).**

- **Account lifecycle** (5): `handle_submit_identity` (account creation), `persist_oauth_client_credentials` in `ui/add_account/state.rs` and the password persist in `ui/add_account/oauth.rs`, `handle_save_account_changes` (account update), the orchestrated-delete write block in `handle_delete_account_confirmed`.
- **Contacts/groups** (3): `Db::create_group`, `Db::update_group`, `Db::delete_group`.
- **Calendar** (4): `Db::create_calendar_event`, `Db::update_calendar_event`, `Db::delete_calendar_event` (Phase 6c) plus `Db::set_calendar_visibility` (Phase 6a).
- **Attachment collapse preference** (1): `persist_attachments_collapsed`.

**Pattern B: `Db::write_db_state()` returning a writable connection wrapped as `ReadDbState` (14 call sites).**

- **Preferences commit** (1): `handle_settings_event::PreferencesCommitted` writes a transaction of `set_setting` calls against the global `settings` table.
- **Account reorder** (1): `handle_save_account_order` (`handlers/core.rs`).
- **Signature CRUD** (3): `handle_save_signature`, `handle_delete_signature`, `handle_reorder_signatures`.
- **Pinned searches + smart folders** (6): all writes on `Db` in `crates/app/src/db/pinned_searches.rs` (`create_or_update_pinned_search`, `update_pinned_search`, `delete_pinned_search`, `delete_all_pinned_searches`, `create_smart_folder`, `expire_stale_pinned_searches`).
- **Compose draft auto-save** (2): the in-typing async path and the close-time sync path in `crates/app/src/handlers/pop_out/compose_draft.rs`.
- **Action context construction** (1): `app.rs:336` builds the `ActionContext` for `cal::actions`. Removed in Phase 6c when the calendar action pipeline relocates.

Phase 6a closes every Pattern A and Pattern B site except OAuth client-credential persistence (Phase 6b's `oauth.exchange_code` flow), event mutations (Phase 6c), and the action-context construction (Phase 6c). It also lands the encryption-key handle (Phase 2 carry-forward 19d): the UI today re-reads `ratatoskr.key` from disk in `from_boot_ready` (`app.rs:327`) even though the Service has already loaded and validated the same file at boot, and the bootstrap-snapshot reads at `app.rs:368-369` decrypt many secure settings under that key on every cold boot.

The phase ships as one milestone with a clean commit-level split. A regression should bisect to the right commit.

## Scope

### In scope

- **Settings (`settings.set`).** The actual table is the global `settings` key/value store (`crates/db/src/db/schema/01_core.sql:47`), not the `app_preferences` table the original draft cited. Wire shape uses a **typed `SettingValue` enum** with one variant per persisted setting (`ShowSyncStatus(bool)`, `BlockRemoteImages(bool)`, `Theme(ThemeKind)`, secure settings, etc.). Matches the project's `MailOperation` / `WireMailOperation` exhaustive-match house style: every dispatch site is a compile-error check. The original draft proposed handler-side schema validation - it was a parallel style to the rest of the codebase, dropped in revision.
- **Per-thread UI state.** `thread_ui_state.set { account_id, thread_id, attachments_collapsed: Option<bool> }` (the actual table is `thread_ui_state`, not `thread_attachment_state`). Separate IPC because the key tuple is `(account_id, thread_id)` and the row carries thread-scoped UI flags that are not "settings" semantically.
- **Account create / update / delete / reorder.** Four methods on `service-api`:
  - `account.create { provider, email, credentials: AccountCredentials, ... }` where `AccountCredentials` is a sum type (`Plaintext { password: RedactedString }` for IMAP, `Encrypted { ciphertext: Vec<u8> }` for callers that hand-encrypt, `Oauth { auth_code: RedactedString, redirect_uri: String, code_verifier: String }` added in 6b). Landing the envelope in 6a means the 6b OAuth two-step adds a variant rather than redefining the contract.
  - `account.update { account_id, params }` mirrors `update_account_sync`.
  - `account.delete { account_id }`. **Cancel-and-await runs Service-side inside the handler**: handler first cancels per-account runners (sync, push, calendar) and awaits their terminal completions, then runs `delete_account_orchestrate`, then drives the four external-store cleanups (body, inline, attachment cache, search index), then returns an `AccountDeletionAck` carrying the cleanup report. This closes the runner-quiescence invariant Service-side instead of trusting the UI to call `cancel_and_await` first. Timeout is overridden to 60 s on this request because external-store cleanup is the bulk of the work and a 5 s ceiling would routinely time out.
  - `account.reorder { ordering: Vec<(String, i32)> }`.
- **Signature CRUD + reorder.** `signature.create | update | delete | reorder`. Mirrors today's `handle_save_signature` / `handle_delete_signature` / `handle_reorder_signatures`. Signatures are flagged in `architecture.md` § Current Exceptions as "not yet a settled architecture surface" - Phase 6a closes the write-surface migration without changing the product/spec shape.
- **Local draft auto-save (`draft.save` + UI-side WAL).** Today's `save_compose_draft_sync` (`compose_draft.rs:112`) is a sub-millisecond synchronous local SQLite write that runs from the window-close path. Replacing it with an async IPC + 500 ms-per-draft ceiling would convert "guaranteed before exit" into "best-effort with 5 s shutdown stall for ten editors" - a real semantic regression.
  Resolution: drafts go through a **UI-side WAL** stored in the user data dir (`<data_dir>/drafts.wal`). The auto-save tick (debounced text-change) and the window-close path both append to the WAL synchronously. The Service drains the WAL on next boot via a new `BootPhase::DrainingDraftWAL` step, replaying entries into the `local_drafts` table. The user-facing surface ("draft saved") is driven by the WAL append, not the Service write. This preserves the "data is durable before window close" semantics today's path provides; it tolerates a Service crash mid-shutdown; and shutdown latency stays sub-millisecond.
  Read paths stay UI-side (the UI reads `local_drafts` rows for the editor restore on boot, after the Service has finished the WAL drain).
- **Pinned searches + smart folders.** `pinned_search.create_or_update | update | delete | delete_all | expire_stale` plus `smart_folder.create`. Today's UI-side `create_smart_folder` lives in the same `db/pinned_searches.rs` module; relocating one without the other would split the file's ownership model. Read paths stay UI-side. The expire-stale entry is cadence-driven; Phase 6a moves the cadence to a Service-side `pinned_search.kick` notification (5-min `SyncTick` with 24 h staleness gate), mirroring `gal.kick`.
- **Contacts/groups CRUD.** `contacts.group_create | group_update | group_delete`. Mirrors today's `db/contacts.rs` API. The Phase 6a IPC keeps the UI-facing semantics unchanged - the editor still works on a shadow copy, commit fires the IPC, ack returns the canonical row for re-render.
- **Calendar visibility toggle.** `calendar.set_visibility { calendar_id, visible }`. Added to the existing `calendar.*` surface (Phase 5 introduced `calendar.start_account_sync` / `cancel_account_sync` / `kick`). This is the flat-boolean half of `db/calendar.rs`; event mutations stay UI-side until Phase 6c.
- **Encryption-key handle (Phase 2 carry-forward 19d).** UI stops calling `rtsk::load_encryption_key` (`app.rs:327`). The decrypt inventory is wider than the original draft acknowledged: `app.rs:368-369` runs `get_ui_bootstrap_snapshot(conn, &encryption_key)` and `get_settings_bootstrap_snapshot(conn, &encryption_key)` on every cold boot, each decrypting many secure settings (`claude_api_key`, `openai_api_key`, `gemini_api_key`, `copilot_api_key`, plus everything in `SECURE_SETTING_KEYS`). Under a naive per-decrypt IPC, every cold boot becomes N round-trips before the UI can render. Resolution:
  - **`internal.read_bootstrap_snapshots { } -> { ui: UiBootstrapSnapshot, settings: SettingsBootstrapSnapshot }`.** Service runs both snapshot reads with the in-memory key, returns the already-decrypted structs. One round-trip per cold boot, hides the encryption boundary entirely (addresses the "general decryption oracle" critique).
  - **`internal.encrypt_for_storage` and `internal.decrypt_for_storage`.** Residual one-shot encrypt/decrypt for credential persistence (account-add password persist, the rare re-auth path that decrypts a stored password to re-display). Domain-specific variants (e.g., `account.persist_password { account_id, plaintext }`) are a follow-up tightening if a hot path emerges; the generic IPC stays narrow because the call sites are bounded.
  - **`encrypt + decrypt + bootstrap-read land together.** Splitting them across phases lets a half-migrated UI sit in a state where it can write a blob it can no longer read, or boot without decrypting its own settings. All three lap into the same commit.
- **Service-side helpers reuse.** Each new IPC method is a thin wrapper around an existing `db::queries_extra::*_sync` function (or `rtsk::*` helper). No business-logic relocation in 6a - the UI shape stays identical, just on the other side of the boundary.
- **`Db` write-surface lockdown.** Once the Pattern A and Pattern B callers above are gone, `Db::with_write_conn`, `Db::with_write_conn_sync`, and `Db::write_db_state` are deleted from `crates/app/src/db/connection.rs`. The single Pattern B remainder - the `ActionContext` construction at `app.rs:336` for `cal::actions` - is documented as a temporary holdout and removed in Phase 6c. After 6c, `Db` exposes only read methods. (Phase 6b's separate work is to delete `app -> service-state` so the body / inline / search writer halves go too.)
- **`docs/architecture.md` update.** The doc has not been touched since before Phase 4. By the end of 6a it must reflect: Phase 5's `CalendarRuntime` + dual-notification routing + GAL kick + IMAP cancellation depth; the post-Phase-6a state of the UI write surface (`Db::write_db_state` and `Db::with_write_conn` deleted; only the `app.rs:336` cal::actions construction remains, gated for 6c); the encryption-key handle's mediation through the Service. The current "**Current Exceptions**" entry for body / inline / search write halves staying UI-side stays accurate (Phase 6b lands the global lockdown for those), but its phrasing needs the Phase 5 + Phase 6a deltas.

### Out of scope

- **OAuth two-step.** Phase 6b. The `oauth.exchange_code` IPC and the elimination of the temporary `oauth.refresh_request` from Phase 4 land there. Account creation in 6a takes already-acquired credentials; the redirect-capture round-trip stays UI-side this phase.
- **`attachment.fetch` IPC for cache-miss reads.** Phase 6b. The pack-store reader path stays UI-side this phase.
- **Eviction / GC for the attachment cache.** Phase 6b.
- **Cross-store invariant pass extension** (blob-store reconciliation). Phase 6b.
- **Calendar event mutations (`cal::actions::*`).** Phase 6c (`docs/service/phase-6c-plan.md`). The existing UI-side mutation handlers in `handlers/calendar.rs` continue to call `cal::actions` directly with a UI-side `ActionContext`. Phase 5 documented this as a known exception; 6a does not change it.
- **Global write-half lockdown.** Phase 6b. Removing the public constructor of `WriteDbState` and the body / inline / search write halves from the `app` crate is meaningful only after OAuth and `attachment.fetch` are gone. 6a leaves `app` depending on `service-state` for the surfaces 6b will close out; the type-level lockdown is the Phase 6b promotion gate.
- **Signature spec rework.** The architecture doc explicitly flags signatures as "not yet a settled architecture surface." 6a relocates the write surface without changing the product shape; whatever spec work happens later can edit the IPC types in place.

## Architecture

### Per-surface checklist (codified after Task 6 review)

Each new IPC surface follows the same six-step shape, validated against the post-Task-6 arch+bugs review:

1. **Wire types** in `crates/service-api/src/<surface>.rs`: a typed `Params` struct + `Ack` (or named result) struct. Both serde-derived. Even unit-struct acks use a named type (not `()`) so adding a field later is a single-site edit, and the handler always routes through `serde_json::to_value(ack)` so the construction path is identical.
2. **`RequestParams` variant** with appropriate timeout (most 5 s; `account.delete` 60 s; `internal.read_bootstrap_snapshots` 10 s). Plus the `method_name`, `timeout`, `params_value`, `from_method_params` arms.
3. **Service-side handler** in `service/src/handlers/<surface>.rs`. **Uses `BootSharedState::write_db_state()`** (added in Task 6 fixup) rather than re-implementing the `db_conn()? -> WriteDbState::from_arc` boilerplate per handler. Pure write-through; no runtime; six to twelve lines.
4. **Dispatch arm** in `service/src/handlers/mod.rs`.
5. **Service-client async helper** in `crates/app/src/service_client.rs`.
6. **UI-side IPC swap + dedicated ack message variant** (see § "UI-side ack message and rollback policy" below). Old `Db::*` helpers are deleted in the same commit.

**Required tests per surface**:

- Inner-struct serde round-trip in the wire-types module.
- `*_method_name_is_dotted`, `*_timeout_is_<n>_seconds`, and **`*_round_trips_from_method_params`** in `service-api/src/request.rs`. The wire-envelope round-trip catches dispatch-table typos that the inner-struct test cannot. The post-Task-6 review pointed out this gap; closing it on every surface makes future copy-paste typos a compile/test failure.

### Service-side handler template

Handlers follow this template after the Task 6 fixup:

```rust
pub(crate) async fn handle_signature_create(
    boot_state: &Arc<BootSharedState>,
    params: SignatureCreateParams,
) -> Result<Value, ServiceError> {
    let write_db = boot_state.write_db_state()?;
    let signature = write_db
        .with_conn(move |conn| db::queries_extra::signatures::create_signature_sync(conn, &params))
        .await
        .map_err(ServiceError::Internal)?;
    // Always go through `to_value` (even for unit-struct acks).
    serde_json::to_value(signature).map_err(|e| ServiceError::Internal(e.to_string()))
}
```

Pure write-through. No runtime, no kick handler, no notification dispatch beyond the request-response cycle. The handler exists to (a) cross the boundary, (b) hold the `WriteDbState` borrow on the Service side. The body of each handler is six to twelve lines.

### UI-side ack message and rollback policy

The Task 6 review caught two bugs that compound across replicated surfaces:

1. **Reusing existing message variants for IPC acks is forbidden.** The original Task 6 implementation funnelled `calendar.set_visibility`'s ack through `CalendarMessage::EventSaved`, whose handler arm clobbered `workflow = Idle`, dismissed the active modal, and set a "Save failed:" status string. Pre-IPC the local DB write closed in <1 ms so the race was unreachable; post-IPC a user can toggle a checkbox, open an event editor while the IPC is in flight, and have the late ack close the editor. **Every IPC gets its own ack `Message` variant**, with a handler arm that touches only state relevant to the surface.

2. **Eager-update surfaces need an explicit failure policy.** When the UI flips local state before the IPC settles (the common case for fast-feeling toggles, list reorders, etc.), the ack handler captures the requested value and on `Err` rolls back **only if the local state still matches the failed request** - if the user clicked again mid-flight, the newer intent is preserved. The corresponding ack message variant carries the requested value:

   ```rust
   VisibilityToggled {
       calendar_id: String,
       requested_value: bool,
       result: Result<(), String>,
   }
   ```

   The Err arm rolls back conditionally; the Ok arm reloads canonical state.

**`service_client = None` policy**: surfaces that cannot tolerate a silent drop (the toggle would persist only in memory) must surface a status-bar message and skip the eager flip. Visibility-style "if the toggle is lost, next reload catches up" surfaces can `log::warn` and proceed. Each surface's policy is decided at implementation time and documented in the dispatch arm's comment.

### Deferred for later sub-phase

The Task 6 review surfaced three architectural questions that do not block the Phase 6a per-surface work but need explicit follow-up:

- **Per-entity ordering / generation tokens.** Rapid-click sequences on a "set" surface can land out of order if the Service dispatches handlers concurrently and the blocking pool is not order-preserving. Visibility tolerates the staleness (idempotent, recovery on next reload); signatures-reorder, account-update, and other surfaces with stronger ordering needs may want a per-entity coalescing or a UI-side generation token that the Service rejects on stale. **Plan**: monitor as surfaces land; add a generation-token wrapper if a real ordering bug appears. Document the per-surface tolerance in each dispatch arm's comment.
- **AckUnknown reconciliation on timeout.** A 5 s timeout on the UI side does not stop the Service handler - the write may still commit while the UI surfaces failure. For idempotent surfaces (visibility, settings, attachment-collapse) re-trying is safe. For surfaces with side effects (account create, draft save), an idempotency key in the wire shape lets retries dedupe Service-side. **Plan**: account create + draft save handle this explicitly (Task 11 + Task 10b); the small surfaces tolerate the gap.
- **Typed `ServiceError::NotReady` variant.** Today's "boot not ready" branch returns `ServiceError::Internal("...")`. Across Service respawn the branch becomes reachable (post-respawn pre-`boot.ready` window) and the UI cannot differentiate it from genuine internal errors. **Plan**: add `ServiceError::NotReady` in a follow-up commit (separate from per-surface work) and have UI surface a "service starting..." retry. Not blocking Phase 6a.

### `settings.set` shape (typed enum, not handler-side validation)

The actual table is the global `settings` key/value store (`crates/db/src/db/schema/01_core.sql:47`), keyed on `key` only - there is no `app_preferences` table and prefs are not account-scoped today. The IPC method is `settings.set { value: SettingValue }` where `SettingValue` is a typed enum:

```rust
pub enum SettingValue {
    ShowSyncStatus(bool),
    BlockRemoteImages(bool),
    Theme(ThemeKind),
    SyncStatusBar(bool),
    // ... one variant per persisted setting
    SecureClaudeApiKey(RedactedString),
    SecureOpenaiApiKey(RedactedString),
    // ... etc
}

pub struct SettingsSetParams {
    pub value: SettingValue,
}
```

Why typed enum, not handler-side validation: the project's `MailOperation` / `WireMailOperation` are exhaustively matched at seven dispatch sites because that compile-time discipline is what made the email pipeline safe to extend. Punting `settings.set` to handler-side runtime validation would introduce a parallel style ("the action pipeline is exhaustive, but settings are stringly-typed") that future contributors would copy. The cost (~30 lines per sweep of new variants when settings get added) is small; the architectural cost of two styles is forever. The original plan draft proposed the runtime-validated form; this revision corrects that based on the arch review.

Secure (encrypted) settings are still values on the same enum. Service-side handler decides whether the variant payload needs to flow through `crypto::encrypt_value` before the DB write; that classification lives in one place (a `is_secure(self) -> bool` method on `SettingValue`), not scattered across the UI call sites that today individually decide whether to pre-encrypt.

### `thread_ui_state.set` shape

The thread-scoped UI state lives in `thread_ui_state` (`crates/db/src/db/schema/02_mail.sql:183`), keyed on `(account_id, thread_id)`. Today the only field UI writes is `attachments_collapsed`, but the table holds other thread-scoped UI flags. IPC takes the full row:

```rust
pub struct ThreadUiStateSetParams {
    pub account_id: String,
    pub thread_id: String,
    pub attachments_collapsed: Option<bool>,
    // ... other thread-scoped flags as they relocate
}
```

Separate from `settings.set` because the key shape and storage row are different. Same exhaustive-match discipline applies: changes to the schema are compile-time-visible at every call site.

### Encryption-key handle: bootstrap snapshot relocation + narrow encrypt/decrypt

Two designs from `phase-2-plan.md` § 19d (handle-based vs trusted-bytes-once). Picking handle-based again, but with the inventory widened to capture the bootstrap-snapshot decrypts the original draft missed.

**Decrypt-call inventory** (verified against `app.rs` HEAD):

- `app.rs:368` - `get_ui_bootstrap_snapshot(conn, &encryption_key)`. Cold-boot read; decrypts UI-side secure settings.
- `app.rs:369` - `get_settings_bootstrap_snapshot(conn, &encryption_key)`. Cold-boot read; decrypts settings-side secure settings (the API keys, etc.).
- Re-auth wizard - reads back a stored account password to pre-populate the password field.
- (Phase 6c) `cal::actions` ActionContext at `app.rs:336` carries `encryption_key` for calendar provider construction. Removed in 6c.

A naive design that replaced `&encryption_key` arguments with a per-decrypt `internal.decrypt_for_storage` IPC would issue N round-trips during cold boot - one per secure setting read by the snapshot helpers. That is a real hot path (every cold boot, blocks UI render).

**Resolution: three IPC methods.**

- **`internal.read_bootstrap_snapshots { } -> { ui: UiBootstrapSnapshot, settings: SettingsBootstrapSnapshot }`.** Service runs the snapshot reads with the in-memory key and returns the already-decrypted structs. One round-trip on cold boot. Hides the encryption boundary entirely - the UI never sees an encrypted byte for the bootstrap path. Addresses the codex-review concern that a generic `decrypt_for_storage` becomes a "general decryption oracle for UI callers."
- **`internal.encrypt_for_storage { plaintext: Vec<u8> } -> { ciphertext: String }`.** Used at credential persist (account-add password). Wire format is the existing `iv:ciphertext_with_tag` `String` shape that `crypto::encrypt_value` already produces - not a fresh `Vec<u8>` shape. Bounded number of call sites (account-add, re-auth confirm, the rare hand-built persistence in tests).
- **`internal.decrypt_for_storage { ciphertext: String } -> { plaintext: Vec<u8> }`.** Used at re-auth wizard prefill. Bounded.

A future tightening pass can replace the generic encrypt/decrypt IPCs with domain-specific ones (`account.persist_password { account_id, plaintext }`, `account.reveal_password { account_id }`). The plan does not reach for that now because the call sites are few and naming each domain inflates the wire surface for a small architectural win.

**`internal.read_bootstrap_snapshots` ordering.** This IPC must complete before the UI runs `from_boot_ready`. Sequence: UI receives `BootReady` -> issues `internal.read_bootstrap_snapshots` -> uses the result to build the rest of `ReadyApp` state. The bootstrap-read step is part of the cold-boot critical path; the IPC gets a 10 s timeout (the snapshot reads include some IO and key-stretch under contention).

**Land-together rule.** `read_bootstrap_snapshots`, `encrypt_for_storage`, `decrypt_for_storage`, and the deletion of `rtsk::load_encryption_key` from `app.rs` all land in the same commit. Splitting lets a half-migrated UI either (a) write a blob it cannot read, or (b) try to decrypt settings without a key.

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

`account.delete { account_id }` runs the full sequence Service-side: cancel runners (sync + push + calendar) -> await their terminal completions -> `delete_account_orchestrate` -> external-store cleanup (body, inline, attachment cache, search index) -> return `AccountDeletionAck` carrying the cleanup report.

The original draft kept the UI's existing `cancel_and_await` call before the deletion request, but the arch review flagged the runner-quiescence invariant: a future caller could call `account.delete` without first cancelling, leaving sync runners writing into a CASCADEd-out account. Keeping cancel + delete as separate IPCs leaves the invariant in the UI's hands. Folding both into the Service handler closes the loop - any caller of `account.delete` gets safe ordering for free.

Implementation reuses the Phase 5 cancel infrastructure verbatim: the handler calls `SyncRuntime::cancel_account`, `PushRuntime::cancel_account`, `CalendarRuntime::cancel_account` and awaits their terminal completions before running the delete. The UI-side `cancel_and_await` helper in `service_client.rs` is no longer needed by the deletion path; it stays for any caller that needs to explicitly cancel without deleting (today there is none, but the surface is small enough to keep).

**Timeout override.** `account.delete` does not get the default 5 s request timeout. External-store cleanup is the bulk of the work and routinely runs longer than 5 s on a heavily-cached account. The request gets a 60 s timeout; Service-side, the cleanup uses `tokio::time::timeout` per external store with smaller budgets so a wedged store does not stall the outer 60 s ceiling. The UI surfaces a "deletion in progress" affordance for the duration; today's `Message::AccountDeleted` arm consumes the ack the same way it consumes the post-cancel delete completion today.

The UI surface stays unchanged from the user's perspective - the existing handler still receives `AccountDeletionAck` (renamed from the local `AccountDeletionCleanupReport` struct).

### Draft auto-save: UI-side WAL

Today's `save_compose_draft_sync` (`crates/app/src/handlers/pop_out/compose_draft.rs:112`) is a sub-millisecond synchronous local SQLite write that runs from the window-close path. The function comment explicitly notes that "an async Task would race against `iced::exit()`" - the current shape is correctness-load-bearing.

The original draft replaced this with an async `draft.save` IPC, with a 500 ms per-draft ack ceiling at shutdown. The arch review flagged the trade:

- **Bounded shutdown latency goes from sub-ms to 500 ms × N.** Ten dirty editors stalls exit by up to 5 s.
- **Guaranteed write becomes best-effort.** Today's path returns `bool` and the caller acts on failure. The IPC version times out, logs, and proceeds - data loss bounded by "since last keystroke."
- **Existing race in the dirty-flag clear.** `compose_draft.rs:88` clears `draft_dirty` before the async DB write completes; if an autosave is in flight when shutdown fires, the close barrier may skip a final save because the dirty flag is already false.

Resolution: drafts go through a **UI-side WAL** stored in the user data dir as `<data_dir>/drafts.wal`. Both the auto-save tick (debounced text-change) and the window-close path append to the WAL synchronously. The Service drains the WAL on next boot via a new `BootPhase::DrainingDraftWAL` step, replaying each entry into `local_drafts` via the existing helper before calling the boot sequence done.

WAL format: append-only, one entry per line, `{ epoch_ms, draft_id, fields }` JSON. The WAL is never truncated by the UI - the Service rotates it on successful drain by writing a `<data_dir>/drafts.wal.replayed` marker and renaming the active WAL out of the way; a second drain pass on next boot ignores any `*.replayed` files.

Properties this preserves vs the original draft:

- **Sub-ms shutdown.** WAL append is a local file write, not an IPC.
- **Crash-safe.** A Service crash mid-shutdown leaves the WAL on disk; next boot drains it.
- **Guaranteed durability.** The WAL append is the durability point. The user-facing "draft saved" surface is driven by the WAL append, not by Service ack.

Read paths stay UI-side: the editor restore on cold boot reads `local_drafts` via the existing read API, after the Service has finished `DrainingDraftWAL`.

`draft.save` IPC still exists for the steady-state autosave, but its ack does not gate any UI behavior - it is a correctness fast-path that lets the Service trim the WAL early when the network is healthy. If the Service is slow or absent, the UI keeps appending to the WAL and the boot drain catches up.

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

**0. Re-verify inventory.** Re-grep both Pattern A (`Db::with_write_conn`) and Pattern B (`Db::write_db_state`) call sites against HEAD before starting. Document any new sites that landed since the plan was written. Resolve open questions inline.

**1. `service-api` wire types.** New modules: `settings`, `thread_ui_state`, `account`, `signature`, `draft`, `pinned_search`, `contacts`, `internal`. Extend the existing `calendar` module with `CalendarSetVisibilityParams` + ack. Add `RequestParams` variants with the appropriate timeouts (most 5 s; `account.delete` 60 s; `internal.read_bootstrap_snapshots` 10 s). Serde round-trip tests per type.

**2. `service::handlers::*` skeleton.** New handler files for each surface. Each handler is a thin wrapper around the existing `db::queries_extra::*_sync` (or `rtsk::*::*_sync`) function. Service-side dispatch arms in `dispatch.rs`.

**3. Encryption-key handle (read_bootstrap_snapshots + encrypt + decrypt land together).** `internal.read_bootstrap_snapshots`, `internal.encrypt_for_storage`, `internal.decrypt_for_storage`. Wire types, handlers, dispatch arms. UI side: `service_client.rs` adds the three async methods. UI `from_boot_ready` calls `read_bootstrap_snapshots` instead of running the snapshot helpers locally; the `rtsk::load_encryption_key` call at `app.rs:327` is deleted. Credential-persist and re-auth-decrypt call sites route through `encrypt_for_storage` / `decrypt_for_storage`.

**4. Settings (`settings.set` typed enum).** UI replaces the `handle_settings_event::PreferencesCommitted` block in `handlers/core.rs:514` with the new IPC. Each variant of `SettingValue` corresponds to one persisted setting; the existing `set_setting` calls become arms in the Service-side handler's exhaustive match.

**5. Per-thread UI state (`thread_ui_state.set`).** UI replaces the `persist_attachments_collapsed` call in `db/threads.rs`. Today's only field is `attachments_collapsed`; the IPC carries the full row so future thread-scoped flags get a wire-shape they can extend.

**6. Calendar visibility (`calendar.set_visibility`).** UI replaces `Db::set_calendar_visibility`. Smallest commit; lands first in the calendar surface so the Phase 6c calendar-event work has a precedent.

**7. Pinned searches + smart folders.** UI replaces all six `Db::write_db_state()` callers in `db/pinned_searches.rs`. The expire-stale cadence moves to `pinned_search.kick`; UI's `Message::SyncTick` gains `kick_pinned_search_expire`. Delete the UI-side `expire_stale_pinned_searches` call site. `create_smart_folder` relocates in the same commit because it lives in the same module and routes the same way.

**8. Contacts/groups.** UI replaces `Db::create_group`, `Db::update_group`, `Db::delete_group`.

**9. Signatures.** UI replaces `handle_save_signature`, `handle_delete_signature`, `handle_reorder_signatures`. Three handler functions, three IPC methods.

**10. Local drafts: UI-side WAL + drain.**
   - **10a.** UI-side WAL writer. Both autosave-tick and window-close paths append to `<data_dir>/drafts.wal`. The existing `save_compose_draft_sync` becomes a WAL append; the dirty-flag clear race is irrelevant because the close path always appends regardless of dirty state for any open editor.
   - **10b.** Service-side `BootPhase::DrainingDraftWAL`. Reads the WAL, replays into `local_drafts` via `local_draft_save_sync`, marks the WAL replayed.
   - **10c.** Optional `draft.save` IPC for steady-state autosave (lets the Service trim the WAL early). Best-effort - failure does not affect the user-facing surface.

**11. Account create (`account.create` with `Plaintext | Encrypted` envelope).** UI replaces `handle_submit_identity`, `persist_oauth_client_credentials`, and the password-persist site. Wire shape uses the credentials envelope so 6b's OAuth two-step adds a `Oauth { auth_code, redirect_uri, code_verifier }` variant rather than redefining the contract.

**12. Account update / reorder.** UI replaces `handle_save_account_changes` (Pattern A) and `handle_save_account_order` (Pattern B). Reorder is a separate IPC because it operates on multiple rows in a single transaction; folding it into update would conflate single-row and multi-row semantics.

**13. Account delete (Service-side cancel-and-await + cleanup).** UI replaces `handle_delete_account_confirmed`'s entire body with one `account.delete` IPC. The Service handler runs cancel-and-await for sync/push/calendar runners, then `delete_account_orchestrate`, then external-store cleanup. 60 s request timeout. Deletes the UI-side cleanup orchestration.

**14. `Db` write-surface lockdown.** Once tasks 4-13 land, the only Pattern A/B caller remaining in `crates/app/src/` is `app.rs:336` (cal::actions construction, removed in 6c). Delete `Db::with_write_conn`, `Db::with_write_conn_sync`, and `Db::write_db_state` from `crates/app/src/db/connection.rs`. Replace the `app.rs:336` site with a temporary helper that documents the cal::actions exception and is the single allow-listed write-conn access in the app crate.

**15. CI script for write-surface invariant.** Add a script that fails if `crates/app/src/` references any of: `Db::with_write_conn`, `Db::with_write_conn_sync`, `Db::write_db_state`, or `service_state::WriteDbState`. Allow-list the single cal::actions construction site by symbol pattern (not line number) until 6c removes it.

**16. `docs/architecture.md` rewrite.** Final commit of 6a. Per § "docs/architecture.md update plan" above. Updates `implementation-roadmap.md` Phase 6a entry to "LANDED" status as well.

## File-by-file changes

**New files:**
- `crates/service-api/src/settings.rs` - `SettingValue` typed enum + `SettingsSetParams`.
- `crates/service-api/src/thread_ui_state.rs` - per-thread UI state wire types.
- `crates/service-api/src/account.rs` - account wire types including `AccountCredentials` envelope.
- `crates/service-api/src/signature.rs` - signature wire types.
- `crates/service-api/src/draft.rs` - draft wire types (steady-state autosave only; WAL is UI-local).
- `crates/service-api/src/pinned_search.rs` - pinned-search + smart-folder wire types + `ClientNotification::PinnedSearchKick`.
- `crates/service-api/src/contacts.rs` - contacts/groups wire types.
- `crates/service-api/src/internal.rs` - bootstrap-snapshot + encrypt + decrypt wire types.
- `crates/service/src/handlers/settings.rs` - settings handler.
- `crates/service/src/handlers/thread_ui_state.rs` - per-thread UI state handler.
- `crates/service/src/handlers/account.rs` - account create/update/delete/reorder handlers; delete handler runs cancel-and-await + cleanup.
- `crates/service/src/handlers/signature.rs` - signature handlers.
- `crates/service/src/handlers/draft.rs` - steady-state draft.save handler.
- `crates/service/src/handlers/pinned_search.rs` - pinned-search handlers + kick handler.
- `crates/service/src/handlers/contacts.rs` - contacts handlers.
- `crates/service/src/handlers/internal.rs` - read_bootstrap_snapshots / encrypt_for_storage / decrypt_for_storage handlers.
- `crates/service/src/draft_wal.rs` - WAL drain step for `BootPhase::DrainingDraftWAL`.
- `crates/app/src/draft_wal.rs` - UI-side WAL writer (autosave + close paths append here).
- `scripts/check_app_write_surface.sh` (or equivalent) - CI script enforcing the lockdown invariant.

**Modified files:**
- `crates/service-api/src/lib.rs` - module declarations.
- `crates/service-api/src/calendar.rs` - add `CalendarSetVisibilityParams` + ack.
- `crates/service-api/src/request.rs` - new `RequestParams` variants with appropriate timeouts.
- `crates/service-api/src/notification.rs` - new `ClientNotification::PinnedSearchKick`.
- `crates/service-api/src/boot.rs` (or equivalent) - new `BootPhase::DrainingDraftWAL`.
- `crates/service/src/dispatch.rs` - new request arms + pinned-search kick arm.
- `crates/service/src/boot.rs` - new boot-phase invocation for the WAL drain.
- `crates/app/src/db/connection.rs` - `Db::with_write_conn`, `Db::with_write_conn_sync`, `Db::write_db_state` deleted.
- `crates/app/src/service_client.rs` - new async wrappers per IPC method.
- `crates/app/src/app.rs::from_boot_ready` - call `internal.read_bootstrap_snapshots` instead of UI-side `get_*_bootstrap_snapshot`; drop `rtsk::load_encryption_key`.
- `crates/app/src/handlers/pop_out/compose_draft.rs` - autosave + close paths write to the WAL instead of `local_drafts` directly.
- `crates/app/src/handlers/core.rs` - replace `handle_settings_event::PreferencesCommitted`, `handle_save_account_changes`, `handle_save_account_order`, and `handle_delete_account_confirmed` write blocks with IPC calls.
- `crates/app/src/handlers/signatures.rs` - replace `write_db_state` callers with IPC calls.
- `crates/app/src/db/threads.rs` - replace `persist_attachments_collapsed` with `thread_ui_state.set` IPC call.
- `crates/app/src/db/calendar.rs` - replace `set_calendar_visibility` with IPC call (event mutations untouched - 6c).
- `crates/app/src/db/contacts.rs` - replace group CRUD with IPC calls.
- `crates/app/src/db/pinned_searches.rs` - replace write paths with IPC calls; remove `expire_stale_pinned_searches` (moves to kick handler); relocate `create_smart_folder`.
- `crates/app/src/ui/add_account/{state,oauth,identity}.rs` - replace `with_write_conn` blocks with `account.create` IPC calls.
- `crates/app/src/handlers/provider.rs` - new `kick_pinned_search_expire` task helper.
- `crates/app/src/update.rs` - extend `Message::SyncTick` fan-out with pinned-search kick.
- `docs/architecture.md` - per § "docs/architecture.md update plan" above.
- `docs/service/implementation-roadmap.md` - mark Phase 6a entry as "LANDED" and adjust the Phase 6 split text.

## Code-comment requirements

1. **`crates/service/src/handlers/internal.rs` module-level doc-comment** must contain:
   - "The encryption-key handle is the boundary that closes Phase 2 carry-forward 19d. After Phase 6a the UI no longer reads `ratatoskr.key` from disk and no longer calls the snapshot-decrypt helpers locally. Cold-boot bootstrap data flows through `read_bootstrap_snapshots` (one IPC, both `UiBootstrapSnapshot` and `SettingsBootstrapSnapshot` returned already-decrypted). Credential persistence flows through `encrypt_for_storage`. Re-auth pre-fill flows through `decrypt_for_storage`. All three IPCs land in the same commit - splitting them lets a half-migrated UI either write a blob it cannot read or boot without decrypting its settings."

2. **`crates/app/src/app.rs::from_boot_ready`** at the deletion point of the `load_encryption_key` call:
   - "Phase 6a: the encryption key is no longer loaded UI-side. Cold-boot reads call `internal.read_bootstrap_snapshots` to fetch the already-decrypted UI and settings bootstraps in one round-trip. The N-decrypt-per-boot anti-pattern (one IPC per secure setting under a generic `decrypt_for_storage`) was rejected in plan revision; see `docs/service/phase-6a-plan.md` § "Encryption-key handle: bootstrap snapshot relocation + narrow encrypt/decrypt"."

3. **`crates/app/src/draft_wal.rs` module-level doc-comment** must contain:
   - "Drafts use a UI-side WAL because the window-close path needs sub-millisecond durability and an async IPC cannot meet that bound. The auto-save tick and the close path both append to `<data_dir>/drafts.wal` synchronously. The Service drains the WAL on next boot via `BootPhase::DrainingDraftWAL` before the UI re-reads `local_drafts`. The optional steady-state `draft.save` IPC lets the Service trim the WAL early when the network is healthy; failure is logged, not surfaced. This is the only UI write path that survives Phase 6a's lockdown - the WAL is local, not a SQLite write."

4. **`crates/service/src/handlers/account.rs::handle_delete`** must contain:
   - "`account.delete` runs cancel-and-await for sync/push/calendar runners before issuing the orchestrated DB delete and the four external-store cleanups. Cancel + delete + cleanup land in one IPC so a future caller cannot delete while runners hold references - that runner-quiescence invariant used to live in the UI's `cancel_and_await` step before the delete request, which made it caller-trusted; this handler enforces it. The 60 s request timeout overrides the default 5 s because external-store cleanup is the bulk of the work."

5. **`crates/service/src/handlers/pinned_search.rs::handle_kick`** must contain:
   - "The expire-stale cadence moved Service-side at Phase 6a. Notification class is `Drop` - missed kicks self-heal on the next `SyncTick`. Same shape as `gal.kick` and `calendar.kick`."

6. **`crates/app/src/db/connection.rs`** at the deletion point of `Db::with_write_conn` / `Db::with_write_conn_sync` / `Db::write_db_state`:
   - "Phase 6a deleted these methods. `Db` exposes only read APIs after this commit. The Phase 6c `cal::actions` ActionContext construction is the single allow-listed write-conn access in the app crate (gated by symbol pattern in the CI lockdown script); 6c removes that final remaining caller and `Db` becomes purely read-only."

7. **`docs/architecture.md` § "Action service as mutation gate"** new sentence:
   - "Phase 6a relocated the small UI write surfaces (settings, per-thread UI state, account CRUD, signatures, drafts, pinned searches, smart folders, contacts, calendar visibility) to Service-side handlers and deleted `Db::with_write_conn` + `Db::write_db_state` from the app crate. Phase 6b will close out OAuth two-step coordination, `attachment.fetch`, and the body / inline / search write halves; the global lockdown (write halves of all four state types unreachable from `app`) lands at the end of 6b. Calendar event mutations (Phase 6c) and the `cal::actions` write-surface escape are tracked as Current Exceptions until 6c lands."

## Test plan

### Unit tests

- Wire-type round-trips for every new `service-api` module (one test per `Params` and `Ack` type, matching the Phase 5 review-pass discipline). `SettingValue` exhaustive-match shape regression test (mirroring `mail_side_mirror_is_exhaustive`).
- `internal.encrypt_for_storage` round-trip: encrypt arbitrary plaintext, decrypt via `internal.decrypt_for_storage`, assert byte-equality. Use a constructed `BootSharedState` with a synthetic `SecretKey`.
- `internal.read_bootstrap_snapshots` returns decrypted snapshots: seed a Service with a key + secure settings; call the IPC; assert the response carries the plaintext values without round-tripping each setting.
- `internal.read_bootstrap_snapshots` partial-failure tolerance: corrupt one secure setting; call the IPC; assert the response carries the rest plus a per-field error for the corrupt one.
- `pinned_search.kick` handler test: seed a stale pinned search, fire kick, assert the row is deleted.
- WAL writer / drainer round-trip: append entries; run the drain; assert `local_drafts` rows are correct and the WAL is rotated to `*.replayed`.

### Integration tests (in-process)

- `account_create_round_trips_through_ipc`: Service handler creates an account via the `Plaintext` envelope; UI-side reader observes the same row via the existing read path.
- `account_create_with_encrypted_envelope`: same shape but using the `Encrypted` envelope (the path that 6b's `Oauth` variant will replace; verifying the envelope shape lands ready in 6a).
- `account_delete_orchestrates_external_stores`: seed an account with body store + inline image entries + an in-flight sync runner; fire `account.delete`; assert (a) the runner observes cancellation, (b) the cleanup report's per-store counts match the seed, (c) no external-store data persists for the deleted account.
- `draft_wal_survives_service_crash`: open the WAL, append entries, kill the Service mid-shutdown without draining; restart; assert `BootPhase::DrainingDraftWAL` recovers the entries.
- `draft_wal_handles_partial_drain`: simulate a crash mid-drain (some rows persisted to `local_drafts`, some not); restart; assert the remainder is replayed without duplicating the persisted rows. Idempotency property: replay is safe to re-run.

### Real-subprocess smoke tests

- `service_subprocess_account_lifecycle`: account create -> update -> delete via real IPC. Asserts the cleanup report arrives back over the wire.
- `service_subprocess_signature_crud`: same pattern for signatures.
- `service_subprocess_draft_wal_drain`: spawn Service with a pre-seeded `drafts.wal`; assert the boot phase drains it and the `local_drafts` rows are present after `boot.ready`.
- `service_subprocess_account_delete_cancels_runners`: spawn Service with sync running; issue `account.delete`; assert sync runner observes cancellation and external-store cleanup completes before the IPC ack.

### Manual matrix updates

Add the new IPC methods to `docs/service/manual-test-matrix.md`. Particularly: account create flow with OAuth credentials (verifies the encryption-key handle closes the loop end-to-end), and a window-close-during-typing test that verifies the draft WAL captures the in-flight character-stroke.

## Open questions

- **`account.delete` cleanup-report shape**: today's `AccountDeletionCleanupReport` is a flat struct of counts. Should it become a typed result (`{ bodies_deleted: u64, errors: Vec<String> }`) or stay flat? Probably flat - the UI does not act on the errors today, only logs them.
- **Pinned-search kick cadence**: 5-min `SyncTick` is the natural home, but pinned-search expire-stale runs against a 90-day window. A 1-hour cadence would be over-frequent; a 24-hour cadence would be under-frequent during long sessions. Plan keeps the 5-min cadence with the same self-gating logic the GAL handler uses (24 h expire-stale staleness check inside the handler).
- **Draft WAL rotation cadence**: the WAL grows monotonically until next boot drains it. Should the Service trim it on the steady-state `draft.save` ack path too, or only at boot? Plan picks "trim on ack only when the network is healthy" - Service writes a sentinel offset into the WAL header indicating "everything before offset N is durably in `local_drafts`," and the UI's WAL writer skips entries before that offset on next read. Decide on shape during task 10b.
- **`internal.read_bootstrap_snapshots` failure shape**: if one of the snapshot reads fails (e.g., a decrypt error on a corrupt secure setting), does the IPC return partial data with the error noted, or fail outright? Plan picks "partial data + per-field error list" so the UI can render the parts that worked - matches the existing tolerance model where an unparseable secure setting falls back to its default.

## Verification (end-to-end)

- The CI lockdown script returns clean: `crates/app/src/` references no `Db::with_write_conn`, `Db::with_write_conn_sync`, `Db::write_db_state`, or `service_state::WriteDbState`, except the single allow-listed `cal::actions` ActionContext construction at `app.rs:336`.
- `Db::with_write_conn`, `Db::with_write_conn_sync`, and `Db::write_db_state` are deleted from `crates/app/src/db/connection.rs`.
- A new account can be created end-to-end; OAuth tokens persist via the `Encrypted` envelope variant and the `internal.encrypt_for_storage` IPC.
- Cold boot runs `internal.read_bootstrap_snapshots` exactly once; the UI surfaces secure settings without ever holding the encryption key.
- Window-close on a dirty composer leaves an entry in the WAL; the next boot's `DrainingDraftWAL` phase persists it before `boot.ready`.
- Account deletion cancels per-account runners and completes external-store cleanup inside the single `account.delete` IPC; no UI-side `cancel_and_await` orchestration is required.
- Pinned-search expire-stale runs Service-side on the 5-min `SyncTick`; UI no longer dispatches the call directly.
- Calendar visibility toggle works through the IPC and the UI re-renders on the ack.
- `docs/architecture.md` reflects the post-Phase-6a state.

## Promotion criteria

- All items in `In scope` landed.
- All items in `Out of scope` are explicitly tracked in their target phase plan (6b for OAuth/attachment.fetch/eviction/cross-store invariant; 6c for calendar event mutations) - none are silently deferred.
- `crates/app/` no longer calls `rtsk::load_encryption_key`. The TOCTOU window flagged in `phase-2-plan.md` § 19d is closed.
- The CI lockdown script is wired into the brokkr check pipeline.
- `docs/architecture.md` post-Phase-6a state is reviewer-confirmed.
- `phase-6a-plan.md` is then retirement-ready: every deferral has an explicit roadmap entry; every code-comment requirement is present in the relevant file.
