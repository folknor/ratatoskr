# The Service - Phase 6d Plan: contacts pipeline + strict transitive lockdown

Companion to `phase-6a-plan.md`, `phase-6b-plan.md`, and `phase-6c-plan.md`. Implements the fourth (and final) sub-phase of Phase 6 of `implementation-roadmap.md`.

> **Best-effort first draft.** Authored after 6a/6b/6c have landed. The contacts work (6d-A) is a clean mechanical relocation against the IPC patterns 6a/6b already established. The structural lockdown work (6d-B) is larger than the implementation-roadmap entry hints at - the `service-state` Cargo dep lives not just on `common` but on `sync` and each of the four provider crates, so the "strict transitive blackout" requires four independent edge breaks, not one. The plan picks a recommended factoring (`provider-sync` orphan-impl crate, `sync` split into pure-logic + persistence halves) and calls out the alternatives.

## Revision history

**2026-05-06 - initial draft.** Covers contacts pipeline relocation (closing the residual UI write surface) plus the strict transitive `service-state` lockdown deferred from Phase 6c. Drafted against the post-6c repo state: `cal::actions` is Service-side, `WriteDbState` is unreachable from `app` directly and via `cal`, and the only remaining UI-reachable writer-half escape is the contacts `ActionContext`.

**2026-05-06 - mid-implementation scope reduction (6d-B).** First-pass drafting assumed the four `app -> ... -> {gmail,jmap,graph,imap} -> service-state` edges were closeable via "orphan-impls in `provider-sync` + drop the dep from each provider's Cargo.toml." Implementation discovered that each provider's `sync_*` function bodies use `service_state::WriteDbState` / `BodyStoreWriteState` / `InlineImageStoreWriteState` / `SearchWriteHandle` directly throughout the persistence layer (not just at the trait-method boundary). Dropping the Cargo dep would require relocating the entire provider sync subtree out of the provider crate - a much larger refactor than the trait-surface move. Sync crate carve-out (sync-logic) faces the same shape: `sync::persistence` is the entire raison d'etre of the crate's service-state dep. Closing those edges is structurally possible but is its own multi-phase project.

  Phase 6d-B's actual deliverable: extract `SyncProviderCtx` and the `ProviderSyncOps` trait into `provider-sync`, closing the `common -> service-state` edge only. The four `app -> ... -> {provider} -> service-state` edges and the `app -> ... -> sync -> service-state` edge remain open and are deferred to Phase 8 alongside the rest of the structural lockdown work. The strict-transitive lockdown test (6d-C) is dropped from 6d for the same reason - blessing the four provider crates + `sync` would weaken the test below the value of writing it.

  The Phase 6b direct-dep test (`app/Cargo.toml` does not list `service-state`) still rules out `use service_state::*` in app source. The Phase 6c-11 transitive test (`app -> cal`) still catches the cal-action regression class. 6d-B layers one more structural cut (`common` is no longer a service-state transit) without claiming the strict end-state.

## Context

Phase 6c retired the calendar event-mutation surface and the `app -> cal -> service-state` Cargo edge, but two things remain on the post-6c TODO surface that `architecture.md` § "Current Exceptions" and `implementation-roadmap.md` § "Phase 6c deferrals" both call out explicitly:

1. **Contacts pipeline.** `service::actions::contacts::{save_contact, delete_contact}` still run UI-side. Settings-panel save / delete in `crates/app/src/handlers/contacts.rs` builds an `ActionContext` (via `ReadyApp::action_ctx()`) and calls the action functions directly. The ActionContext is constructed at `app.rs::from_boot_ready` from `Db::phase_6c_pending_write_state()` (the single allow-listed Pattern B accessor that survived 6a/6b/6c), `body_store`, `inline_images`, `search`, plus an `encryption_key` loaded UI-side via `rtsk::load_encryption_key`. It is the *last* `WriteDbState`-shaped surface reachable from `app/src/`, and the only remaining UI-side `load_encryption_key` call.

2. **Strict transitive lockdown.** The Phase 6c-11 lockdown test asserts `app` cannot reach `cal` via path-deps. The roadmap pins `service-state` as the more useful target - `cal` was only a regression-class proxy for the writer-half surface - but the structural path `app -> rtsk -> common -> service-state` (via `common::types::SyncProviderCtx`) is open today, plus three more parallel paths the implementation-roadmap entry does not enumerate:
   - `app -> rtsk -> common -> service-state` (`SyncProviderCtx` lives in `common::types`).
   - `app -> rtsk -> sync -> service-state` (`crates/sync/Cargo.toml` carries the dep for the persistence layer).
   - `app -> rtsk -> {gmail,jmap,graph,imap} -> service-state` (each provider's Cargo.toml lists the dep because each provider impls `ProviderOps::sync_initial`/`sync_delta`, both of which take `&SyncProviderCtx`).

   Closing the structural transit therefore requires four independent edge breaks, not one. The `cal -> service-state` edge that 6c-11 closed via the `app/Cargo.toml` cal-drop is a special case: `cal` was the only crate UI-reachable through both `app` directly *and* the dep cone of `rtsk`'s sync infrastructure. The four 6d edges all live deeper in the cone and require structural moves rather than a single Cargo line drop.

The two items are unrelated in detail but share a single goal: "every writer-half handle in the workspace is unreachable from `app/src/` at the *Cargo dep-graph level*, not just at the Rust visibility level." Once 6d closes both, the `service-state` lockdown is genuinely transitive and the contacts allow-list comment in `lockdown.rs` retires.

## Scope

### Entry criteria

- **Phase 6c landed.** `cal::actions` is Service-side, `cal_action.execute_plan` IPC is wired, the `app -> cal -> service-state` Cargo edge is closed, and the 6c-11 lockdown test asserts that closure.
- **`brokkr check` is clean** at the start of 6d. No outstanding test ignores related to the contacts surface.
- **Phase 6a's contacts wire types are in place.** `ContactSaveParams` exists at `crates/service-api/src/contacts.rs` and is used by the bulk-import path; the local-only handler (`contacts.contact_save`) is wired but does not yet do provider write-back.

### In scope

#### 6d-A: Contacts pipeline relocation

- **Service-side `contacts.contact_save` grows provider write-back.** Today's handler (`crates/service/src/handlers/contacts.rs::handle_contact_save`) does the local DB upsert and stops. Post-6d-A it dispatches to the same `dispatch_write_back` body that lives in `service::actions::contacts` today (Google / JMAP / Graph; CardDAV is still a stub). The handler consumes the encryption key from the Service's already-validated handle - no `load_encryption_key` round-trip, no IPC for the key. Failure modes mirror the action function:
  - Local save failure -> `ServiceError` with the original `ActionError` mapped to a wire-friendly string. UI surfaces "Contact save failed."
  - Local save OK + provider write-back failure -> ack returns `ContactSaveAck { provider_writeback: WritebackOutcome::LocalOnly { reason } }` (new field). UI shows the contact in the list and logs the degraded state.
  - Local + provider both OK -> `ContactSaveAck { provider_writeback: WritebackOutcome::Success }`.
  - Synced contact missing `account_id` / `server_id` -> same `LocalOnly` shape with the existing message text.
- **New `contacts.contact_delete` IPC.** Mirrors the wire shape of `contact_save`: `ContactDeleteParams { id }` + `ContactDeleteAck { provider_outcome: DeleteOutcome }`. Provider-first for synced JMAP/Google/Graph contacts (matches today's behavior in `service::actions::contacts::delete_contact`); failure short-circuits before the local DB delete, surfacing as `ServiceError`. CardDAV stub returns `LocalOnly`. Local-only contacts return `Success` immediately. The wire struct re-uses `WritebackOutcome` from `contact_save` so the UI side has one match arm.
- **`WritebackOutcome` enum lands in `service-api/src/contacts.rs`.**
  ```rust
  #[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
  pub enum WritebackOutcome {
      Success,
      LocalOnly { reason: String },
  }
  ```
  Avoids leaking `ActionError` / `ActionOutcome` (in `action-types`, not serde) over the wire. The wire stays serde-only; the Service-side handler converts from `ActionOutcome` to `WritebackOutcome` at the IPC boundary, same shape mail and calendar use.
- **UI handlers strip `service::actions::contacts::*` direct calls.** `crates/app/src/handlers/contacts.rs::handle_save_contact` and `handle_delete_contact` route through `client.save_contact_with_writeback(...)` / `client.delete_contact(...)` (new `ServiceClient` methods). Existing `client.save_contact(...)` (Phase 6a, local-only) renames to `save_contact_local_only(...)` to make the distinction visible at call sites; the bulk-import path migrates to the renamed method.
  - Settings panel save / delete: full pipeline (with provider write-back).
  - Bulk import: local-only path - existing behavior preserved. Import is high-volume and provider write-back per row would be O(N) HTTPS round-trips; the Settings UI will gain an explicit "sync uploaded contacts to provider" affordance later if needed.
- **Delete `app.action_ctx` field, `ReadyApp::action_ctx()` accessor, and the action_ctx construction in `app.rs::from_boot_ready`.** With contacts on IPC there are no remaining consumers. The `subscription.rs:102` gating check goes too. `Option<service::actions::ActionContext>` no longer appears in the app crate.
- **Delete `Db::phase_6c_pending_write_state` accessor.** `crates/app/src/db/connection.rs` loses the method entirely. The doc-comment was already explicit that this was a 6c-pending escape hatch, scheduled for removal once contacts relocate.
- **Delete the UI-side `rtsk::load_encryption_key` call** at `app.rs:331`. The contacts pipeline was the last consumer; the comment block at lines 320-330 already flagged that this load disappears once the action_ctx does. The Service holds the validated key handle; the UI never holds plaintext key bytes after 6d-A.
- **Update `crates/service-state/tests/lockdown.rs`** to drop the contacts-pipeline allow-list comment in the module-level doc. `phase_6c_pending_write_state` no longer exists; the test's first assertion (`app_crate_must_not_directly_depend_on_service_state`) is unchanged but now load-bearing without the contacts caveat.
- **Update `docs/architecture.md`.** Strike the `action_ctx` for-contacts entry from § "Current Exceptions". The "Service-side write surfaces" paragraph loses the trailing "the action_ctx still carries an encryption_key, removed alongside the ActionContext in Phase 6c" hedge - that promise resolves here.

#### 6d-B: Strict transitive `service-state` lockdown

- **Extract `SyncProviderCtx` and the sync trait surface into a new `provider-sync` crate.** New crate at `crates/provider-sync/` (workspace member). Hosts:
  - `pub struct SyncProviderCtx<'a>` (moved verbatim from `common::types`).
  - `#[async_trait] pub trait ProviderSyncOps: Send + Sync` with `sync_initial` and `sync_delta` (the two methods today on `ProviderOps` that take `&SyncProviderCtx`).
  - Orphan-impls `impl ProviderSyncOps for {GmailClient, JmapClient, GraphClient, ImapClient}`. The orphan rule is satisfied because `provider-sync` defines the trait. Each impl moves the body out of the corresponding provider crate's `ops.rs` (today's `impl ProviderOps for GmailClient { async fn sync_initial(...) ... }` block splits: the action / folder / profile methods stay in `gmail::ops`, the two sync methods move to `provider-sync::gmail`).
  - `provider-sync` deps: `service-state`, `common`, the four provider client crates, plus the persistence helpers each sync impl already calls into.
- **`common::ProviderOps` loses `sync_initial` and `sync_delta`.** Trait shrinks to action / folder / send / draft / attachment / profile / connection methods - none of which take a writer-half handle. `common::types::SyncProviderCtx` deletes; the `use service_state::{...}` import in `crates/common/src/types.rs` deletes. `crates/common/Cargo.toml` drops `service-state = { path = "../service-state" }`. The `crates/common/src/ops.rs` import line drops `SyncProviderCtx, SyncResult` - those move to `provider-sync` along with the trait surface.
- **`crates/sync/` splits into `sync-logic` (UI-safe) + `sync` (Service-side).**
  - `sync-logic` (new crate at `crates/sync-logic/`) holds the four pure-logic modules `rtsk` re-exports: `bundling`, `filters`, `smart_labels`, `threading`, plus `config`. No `service-state` dep, no `WriteDbState` consumers.
  - `sync` (existing crate, narrowed) holds `pipeline`, `persistence`, and any other module that touches writer halves. Keeps the `service-state` dep.
  - `core (rtsk)` updates its re-exports: `pub use sync_logic::{bundling, filters, smart_labels, threading}; pub use sync_logic::config;`. The `crate::sync::pipeline::*` callers in `core/src/provider/account_resync.rs` (today: `clear_account_history_id`, `clear_all_folder_sync_states`) are the only `rtsk -> sync` references; that file moves to the Service side OR the two `pipeline::*` helpers move to `sync-logic` (they take a `&Connection`, not a `&WriteDbState`, so they are mechanically UI-safe and the move is a one-file shuffle).
  - Resolution: prefer the helpers-move-to-sync-logic path. `account_resync.rs` already runs in the action / orchestrator layer that crosses the IPC boundary; the two helpers it calls are pure SQL against a borrowed `&Connection`. Moving them to `sync-logic` keeps the call site's import surface narrow without dragging the file Service-side.
  - `core (rtsk)` drops the `sync = { path = "../sync" }` dep entirely; the only `sync::*` references in `rtsk` were the four re-exports + `sync::config` + the two `pipeline::*` calls, all of which migrate to `sync-logic`.
- **Each provider crate drops `service-state` from its Cargo.toml.** The four provider crates (`gmail`, `jmap`, `graph`, `imap`) lose the `service-state = { path = "../service-state" }` line. Their `ops.rs` files lose the two sync trait method bodies (which moved to `provider-sync`). The `SyncProviderCtx` import + any `service_state::*` import disappears.
- **Sync dispatch updates.** `crates/service/src/sync_dispatch.rs` (the only consumer of the trait via `ProviderSyncOps` post-6d-B) imports `provider_sync::ProviderSyncOps` instead of reaching it via `common::ProviderOps`. The dispatch site grabs `&dyn ProviderSyncOps` from the existing per-account provider registry; the registry signature grows a parallel getter, or the registry returns a struct holding both `Arc<dyn ProviderOps>` and `Arc<dyn ProviderSyncOps>`. Decision: the registry returns a single struct - cleaner than two parallel maps and avoids a dispatch-site case split.
- **Consumer audit:** `grep -rn "SyncProviderCtx" crates/` post-6d-B should return only `crates/provider-sync/src/`, `crates/service/src/sync_dispatch.rs`, and any `*_sync.rs` files within `provider-sync`'s sub-modules. Anything else is a regression.

#### 6d-C: Strict transitive lockdown test

- **Extend `crates/service-state/tests/lockdown.rs` with a fourth assertion.** Mirrors `app_crate_must_not_transitively_depend_on_cal` but targets `service-state`:
  ```rust
  #[test]
  fn app_crate_must_not_transitively_depend_on_service_state() {
      // BFS from `app`, target = `service-state`, blessed = ["service"].
      // Failure prints the dep chain that re-introduces the regression.
  }
  ```
  Blessed list is `["service"]` only. The four provider crates and `sync` are unblessed - they no longer carry `service-state` as a direct dep post-6d-B, so the BFS cannot pass through them anyway.
- **Strike the contacts caveat from the lockdown test's module-level doc** (matched task with 6d-A; mentioned here for completeness). Post-6d, the doc reads as "every writer-half handle is unreachable from `app/src/` at the Cargo dep-graph level, not just at the Rust visibility level."
- **Retire the 6c-11 cal-target test or keep it as a regression guard?** Decision: keep. The cal-target lockdown is narrower (UI-side reachability of the `cal` crate, not service-state) and remains a useful regression gate against a different shape of mistake (e.g., re-introducing `cal::actions` inside `app/src/` via a new dep). Two tests, two distinct invariants.

### Out of scope

- **Service-side bulk-import IPC.** `execute_contact_import` in `crates/app/src/handlers/contacts.rs` issues N `client.save_contact(...)` calls in a loop. A batch IPC would reduce wire overhead but the per-row UPSERT semantics are simple and the import path is human-paced (user clicks "Import"). Out of scope until a real perf complaint surfaces. Phase 6a explicitly deferred this.
- **Provider write-back for bulk-imported contacts.** Today's import is local-only (Phase 6a's design call - imports run as `contacts.contact_save` (renamed `save_contact_local_only` post-6d-A), not `save_contact_with_writeback`). 6d-A preserves this asymmetry: `import` calls the local-only path, `Settings -> Save` calls the full-pipeline path. A future "sync imported contacts to provider" Settings affordance is its own work.
- **Fully migrating CardDAV contact write-back.** CardDAV is a stub (`Err(ActionError::not_implemented(...))`) for both save and delete in today's `service::actions::contacts::dispatch_*`. Relocating the stub is one-for-one; implementing it (vCard generation + PUT) is separate work and not blocking 6d.
- **Non-contacts UI write surfaces that may have crept in since 6c.** A Phase 6d entry-criterion-step audit of `crates/app/src/` for `with_write_conn` / `phase_6c_pending_write_state` / `service_state::*` references should find nothing post-6c, but if it surfaces a new escape, that escape is its own line item, not a 6d carry-on.
- **Sync-logic crate ergonomics polish.** The mechanical move from `sync` to `sync-logic` will leave some module re-exports in awkward places. A pass to flatten / rename can land later; 6d-B holds the namespace stable to keep the diff reviewable.
- **`provider-sync` rename.** Other plausible names: `provider-sync-ops`, `service-sync-ops`, `sync-trait`. 6d-B picks `provider-sync` and stays with it; bikeshed is a follow-up if a real ambiguity surfaces in code review.

## Architecture

### What the Cargo dep cone looks like before / after

**Before 6d (post-6c):**

```text
app
├── service                    [blessed: legitimately Service-side]
│   └── service-state          [legitimate]
└── rtsk
    ├── common
    │   └── service-state      ←── 6d-B closes (move SyncProviderCtx out)
    ├── sync
    │   └── service-state      ←── 6d-B closes (split sync-logic out)
    ├── gmail
    │   └── service-state      ←── 6d-B closes (move sync impl to provider-sync)
    ├── jmap
    │   └── service-state      ←── 6d-B closes (same)
    ├── graph
    │   └── service-state      ←── 6d-B closes (same)
    └── imap
        └── service-state      ←── 6d-B closes (same)
```

**After 6d (clean):**

```text
app
├── service                    [blessed]
│   ├── service-state          [legitimate]
│   ├── provider-sync          [Service-only consumer of the sync trait]
│   │   └── service-state      [legitimate]
│   └── sync                   [narrowed: Service-side persistence]
│       └── service-state      [legitimate]
└── rtsk
    ├── common                 [no service-state; sync types moved out]
    ├── sync-logic             [pure: bundling, filters, threading, smart_labels, config]
    ├── gmail                  [no service-state; sync impl moved to provider-sync]
    ├── jmap                   [same]
    ├── graph                  [same]
    └── imap                   [same]
```

The lockdown test's BFS from `app` blocks descent through `service` and finds no path to `service-state`. The four UI-reachable provider crates remain (clients, action ops, folder ops, profile, attachments) but their dep cones no longer include `service-state`.

### Why orphan-impl in `provider-sync` instead of per-provider sync sub-crates

Two factorings considered:

- **Factoring A: One `provider-sync` crate that orphan-impls `ProviderSyncOps` for each provider's client.** One new crate. Each provider's `ops.rs` shrinks (sync methods extracted); each provider's Cargo.toml loses `service-state`. The orphan rule is satisfied because `provider-sync` defines the trait.

- **Factoring B: Eight crates - `gmail-actions` + `gmail-sync`, `jmap-actions` + `jmap-sync`, etc.** Symmetric, but doubles the per-provider boundary count and forces every internal reference to pick which half to import. The split is an aesthetic concern more than a structural one - both factorings achieve the same Cargo edge break.

Default is Factoring A. It costs one crate vs. four and the orphan-impl pattern is mechanical. Reviewers worried about "trait impls living far from the type they impl" can revisit during review.

### Provider registry shape post-6d-B

Today's per-account provider registry returns `Arc<dyn ProviderOps>`. Post-6d-B it returns a small struct:

```rust
pub struct ProviderHandle {
    pub ops: Arc<dyn common::ops::ProviderOps>,
    pub sync_ops: Arc<dyn provider_sync::ProviderSyncOps>,
}
```

Action-side dispatch reaches `.ops`; sync dispatch reaches `.sync_ops`. The two `Arc`s can wrap the same underlying provider client, since the orphan-impl makes `GmailClient` (etc.) implement both traits. No double-allocation.

### Encryption-key flow post-6d-A

Pre-6d-A: Service loads key at boot, validates, holds in memory. UI re-loads from disk during `from_boot_ready` purely to populate the contacts ActionContext. Two reads of the same file - the TOCTOU window has been on the deferral list since Phase 2 (carry-forward 19d) and was knocked down to "the contacts ActionContext is the last caller" in 6a/6b.

Post-6d-A: UI never opens `ratatoskr.key`. Service holds the key handle and uses it directly inside `handle_contact_save` / `handle_contact_delete` to construct provider clients (`JmapClient::from_account` etc. take `&[u8; 32]`). Phase 6b's `internal.encrypt_for_storage` / `decrypt_for_storage` IPC is unchanged - those serve the bootstrap-snapshot path, not the action path.

## Touchpoints

### 6d-A files

- `crates/service-api/src/contacts.rs` - add `WritebackOutcome` enum, extend `ContactSaveAck` with `provider_writeback`, add `ContactDeleteParams` + `ContactDeleteAck`.
- `crates/service-api/src/request.rs` - new `RequestParams` variants `ContactsContactSaveWithWriteback` (or extend the existing `ContactsContactSave` to carry a `with_writeback: bool` flag - decision: separate variant, the wire envelope is otherwise identical and the dispatch arms stay clean), `ContactsContactDelete`. Method names: `contacts.contact_save_with_writeback`, `contacts.contact_delete`. Timeouts: 30 s (provider HTTPS round-trip).
- `crates/service-api/src/lib.rs` - re-exports.
- `crates/service/src/handlers/contacts.rs` - `handle_contact_save_with_writeback` (new), `handle_contact_delete` (new). Both consume the Service-validated encryption key handle. Move the body of `service::actions::contacts::dispatch_write_back` and `dispatch_delete` into the handler module (or keep them in `service::actions::contacts` as helpers and call them from the handler - decision: keep in `service::actions::contacts` so the existing tests / call sites continue to work; the handler is a thin wrapper).
- `crates/service/src/dispatch.rs` - dispatch arms for the two new methods.
- `crates/app/src/service_client.rs` - rename `save_contact` to `save_contact_local_only`; add `save_contact_with_writeback(params) -> Result<WritebackOutcome, ClientError>` and `delete_contact(id) -> Result<WritebackOutcome, ClientError>`.
- `crates/app/src/handlers/contacts.rs::handle_save_contact` / `handle_delete_contact` - replace `service::actions::contacts::*` direct calls with `client.save_contact_with_writeback(...)` / `client.delete_contact(...)`. Bulk import path (`execute_contact_import`) updates the renamed `save_contact_local_only` call - mechanical rename only.
- `crates/app/src/app.rs` - delete `pub(crate) action_ctx: Option<service::actions::ActionContext>` field, the construction block at lines 338-354, the `encryption_key` load at line 331, and the `action_ctx` field assignment at line 429.
- `crates/app/src/handlers/commands.rs` - delete `pub(crate) fn action_ctx(&self) -> Option<...>` accessor.
- `crates/app/src/subscription.rs:102` - delete the `if self.action_ctx.is_some()` gate (the surrounding subscription decision-tree branches without it).
- `crates/app/src/db/connection.rs` - delete `Db::phase_6c_pending_write_state` accessor.
- `crates/service-state/tests/lockdown.rs` - update the module-level doc-comment to drop the contacts caveat.
- `docs/architecture.md` - strike the `action_ctx` for-contacts entry from § "Current Exceptions"; update the "Service-side write surfaces" paragraph; update the "Action service as mutation gate" enforcement note (the `Db::phase_6c_pending_write_state` accessor reference disappears).

### 6d-B files

- **New crate `crates/provider-sync/`.** `Cargo.toml` (deps: `common`, `service-state`, `db`, all four provider client crates, the persistence helpers each sync impl needs). `src/lib.rs` exposes `ProviderSyncOps` trait + `SyncProviderCtx` struct. Sub-modules `gmail.rs`, `jmap.rs`, `graph.rs`, `imap.rs` each carry the orphan-impl for their respective client.
- `Cargo.toml` (workspace) - register `crates/provider-sync` as a workspace member.
- `crates/common/src/types.rs` - delete the `use service_state::{...}` import; delete `SyncProviderCtx` struct definition.
- `crates/common/src/ops.rs` - remove `sync_initial` / `sync_delta` from `ProviderOps`. Update import line.
- `crates/common/Cargo.toml` - drop `service-state = { path = "../service-state" }`.
- `crates/{gmail,jmap,graph,imap}/src/ops.rs` - remove the two sync method impl bodies (move to `provider-sync/src/<provider>.rs`).
- `crates/{gmail,jmap,graph,imap}/Cargo.toml` - drop `service-state = { path = "../service-state" }`.
- **New crate `crates/sync-logic/`.** Holds `bundling`, `filters`, `smart_labels`, `threading`, `config` modules moved verbatim from `crates/sync/src/`. `Cargo.toml` deps: subset of today's `sync` crate's deps minus `service-state` and minus the persistence-side imports. Plus the two `pipeline::clear_*` helpers if they migrate (default decision per § Architecture).
- `crates/sync/src/` - delete the moved modules; what remains is `pipeline`, `persistence`, plus any non-pure helpers. `Cargo.toml` keeps `service-state`.
- `crates/core/src/lib.rs` (rtsk) - update re-exports: `pub use sync_logic::{bundling, filters, smart_labels, threading};` (and `config` if it stays a top-level re-export).
- `crates/core/src/provider/account_resync.rs` - update `crate::sync::pipeline::*` references to wherever the helpers landed (`sync_logic::pipeline::*` if migrated, or via a service IPC if the file moves).
- `crates/core/Cargo.toml` - drop `sync = { path = "../sync" }`; add `sync-logic = { path = "../sync-logic" }`.
- `crates/service/Cargo.toml` - keeps `sync`; add `provider-sync = { path = "../provider-sync" }`.
- `crates/service/src/sync_dispatch.rs` - update import to `provider_sync::ProviderSyncOps` and the registry shape per § Architecture. Method calls (`provider.sync_initial(...)`, `provider.sync_delta(...)`) are unchanged at call site - the trait method signatures are identical.
- Provider registry construction site (`crates/service/src/...` - audit during 6d-B) - returns `ProviderHandle { ops, sync_ops }` instead of `Arc<dyn ProviderOps>`.

### 6d-C files

- `crates/service-state/tests/lockdown.rs` - new `app_crate_must_not_transitively_depend_on_service_state` test mirroring the 6c-11 cal target.

## Tasks (commit-by-commit)

### 6d-A: Contacts pipeline relocation

1. **6d-A-1** - `service-api`: add `WritebackOutcome`, extend `ContactSaveAck`, add `ContactDeleteParams` + `ContactDeleteAck`, register `ContactsContactSaveWithWriteback` + `ContactsContactDelete` request variants. Round-trip tests for both new wire types.
2. **6d-A-2** - Service: implement `handle_contact_save_with_writeback` + `handle_contact_delete` in `crates/service/src/handlers/contacts.rs`, dispatch arms in `dispatch.rs`. Handlers consume the encryption key from the Service handle and call into the existing `service::actions::contacts::dispatch_write_back` / `dispatch_delete` helpers. Unit tests for the LocalOnly / Success outcome paths against stub providers.
3. **6d-A-3** - `ServiceClient`: add `save_contact_with_writeback` + `delete_contact` methods, rename existing `save_contact` to `save_contact_local_only`. Update the bulk-import call site at `handlers/contacts.rs::execute_contact_import`.
4. **6d-A-4** - UI: rewire `handle_save_contact` / `handle_delete_contact` to the new IPC methods; map `WritebackOutcome` to the existing `Settings(SettingsMessage::ContactsLoaded(...))` / `ContactSaved` / `ContactDeleted` arms. Behavior parity test (manual): save a synced JMAP contact, observe provider PATCH; delete a Google contact, observe provider DELETE.
5. **6d-A-5** - Delete `app.action_ctx` field, accessor, construction, `encryption_key` load, `subscription.rs:102` gate, and `Db::phase_6c_pending_write_state` accessor. Update lockdown test's module-level doc. `cargo check` confirms no dangling references; `brokkr check` clean.
6. **6d-A-6** - Architecture doc updates: strike the `action_ctx` exception, update the "Service-side write surfaces" paragraph, update the mutation-gate enforcement bullet.

### 6d-B: Strict transitive lockdown - structural moves

7. **6d-B-1** - Create `crates/provider-sync/` workspace member with `ProviderSyncOps` trait + `SyncProviderCtx` struct (moved from `common::types`). No impls yet. Workspace `Cargo.toml` registers the new crate.
8. **6d-B-2** - Move sync impl bodies from each provider's `ops.rs` to `crates/provider-sync/src/<provider>.rs`. Each provider's `ops.rs` shrinks; each provider crate keeps `service-state` *temporarily* (still referenced by other modules until 6d-B-3).
9. **6d-B-3** - Audit each provider crate for remaining `service_state::*` references. The expected post-step state: zero. Drop `service-state = { path = "../service-state" }` from each of `crates/{gmail,jmap,graph,imap}/Cargo.toml`. `cargo check` confirms.
10. **6d-B-4** - Update `crates/service/src/sync_dispatch.rs` and the per-account provider registry to return `ProviderHandle { ops, sync_ops }`. Each call site grabs the right Arc; the existing per-method dispatch logic is unchanged.
11. **6d-B-5** - `common::ProviderOps` loses `sync_initial` + `sync_delta`. `common::types::SyncProviderCtx` deletes. `common::types` `use service_state::{...}` import deletes. `crates/common/Cargo.toml` drops `service-state`. `cargo check` clean.
12. **6d-B-6** - Carve out `crates/sync-logic/`: move `bundling`, `filters`, `smart_labels`, `threading`, `config` modules verbatim from `crates/sync/src/`. Update internal references inside the moved modules to use the new crate paths.
13. **6d-B-7** - Migrate the two `pipeline::clear_*` helpers used by `crates/core/src/provider/account_resync.rs` to `sync-logic::pipeline` (the helpers take `&Connection`, no `WriteDbState`, so the move is mechanical). Update the call site.
14. **6d-B-8** - `crates/core/Cargo.toml` drops `sync`; adds `sync-logic`. Update `crates/core/src/lib.rs` re-exports. `cargo check` clean.
15. **6d-B-9** - Verify `crates/sync/Cargo.toml` no longer needs to expose the moved modules; `service` is the only consumer; deps unchanged.

### 6d-C: Lockdown test

16. **6d-C-1** - Add `app_crate_must_not_transitively_depend_on_service_state` test to `crates/service-state/tests/lockdown.rs`. Blessed: `["service"]`. The test should pass on the post-6d-B tree; running it against a synthetic regression (e.g. re-add `service-state = { path = "..." }` to `crates/common/Cargo.toml` in a local branch) should fail with the expected chain.

### Final

17. **6d-final** - `docs/service/implementation-roadmap.md` § Phase 6 updates: mark Phase 6d as LANDED with a status block mirroring the 6a/6b/6c entries. Strike the carry-forward entries. The "DEFERRED to Phase 6d/8" notes in 6c get retired.

## Exit criteria

- `crates/app/src/` contains no references to `service::actions::contacts`, `action_ctx`, `phase_6c_pending_write_state`, or `service_state::*`. `git grep` for each returns zero hits in `crates/app/src/`.
- `crates/app/src/` contains no `rtsk::load_encryption_key` call. The Service is the only key reader.
- `crates/{common,sync,gmail,jmap,graph,imap}/Cargo.toml` no longer list `service-state`. Only `crates/{service,provider-sync,sync,service-state}/Cargo.toml` (and `service-state` itself) carry the dep.
- `crates/service-state/tests/lockdown.rs` includes `app_crate_must_not_transitively_depend_on_service_state` and the test passes.
- `brokkr check` is clean against the post-6d tree, including the `clippy::unwrap_used` and `cognitive_complexity` lints for the moved code.
- The settings-panel save path for a synced JMAP / Google / Graph contact triggers a provider PATCH (verified manually); the delete path triggers a provider DELETE for the same providers.
- The contacts allow-list comment in `lockdown.rs` is gone; the doc reads as "every writer-half handle in the workspace is unreachable from `app/src/` at the Cargo dep-graph level."
- `docs/architecture.md` § "Current Exceptions" no longer lists the `action_ctx` for contacts entry.

## Risks / open questions

- **6d-B is the largest structural move in Phase 6.** Four Cargo edges + one new crate + one crate split. The diff will be large and review-heavy. Mitigation: each task lands as a separate commit (one provider per commit if reviewer pressure mounts), and the `cargo check` pass between commits gives a bisect bisection point. The work is mechanical (no logic changes, just relocations), so the risk is in catching every `use` site that needs updating.
- **`provider-sync` orphan-impl risk.** Two of the four providers (`gmail`, `jmap`) construct their client via `Client::from_account(&db, account_id, &encryption_key)` - the impl body needs read access to the same DB the surrounding sync code does. Today the sync method takes `&SyncProviderCtx` which carries the writer-half handles; the orphan-impl in `provider-sync` does the same. No new pattern, just a relocation. Verify during 6d-B-2 that no provider sync impl reaches into private state of the provider crate that orphan-impl can't see (`pub` audit).
- **`sync-logic` carve-out: are the four pure modules really pure?** Quick spot-check of `crates/sync/src/{bundling,filters,smart_labels,threading,config}.rs` for `service_state::*` imports needed before 6d-B-6 commits. If any module reaches into a writer half, it stays in `sync` and the re-export path adjusts. The decision tree:
  - All four pure → migrate cleanly to `sync-logic`. (Expected outcome.)
  - One module mixed → split that module: pure half to `sync-logic`, writer half stays in `sync`. Doable but adds review surface.
  - Multiple modules mixed → the carve-out gets larger and may warrant deferring `sync-logic` to its own sub-step.
- **The provider registry change in 6d-B-4 is workspace-wide.** Every registry construction site, every per-account provider-handle dispatch site updates. The registry is well-encapsulated today (one constructor, one getter), so the blast radius should be small, but a 6d-B-4 prep step grep should enumerate sites before the commit.
- **Bulk-import path semantics.** Today's import is local-only (Phase 6a's choice). After 6d-A, the rename from `save_contact` to `save_contact_local_only` makes that explicit. A future product decision to "import + write-back to provider" is straightforward (call the writeback method instead) but is not a 6d concern. The Settings UI gains an explicit "sync uploaded contacts" affordance later; the wire IPC for that affordance already exists post-6d-A.
- **Manual matrix item.** Real-provider verification of save / delete write-back belongs in `manual-test-matrix.md` since the integration tests today don't exercise live HTTPS round-trips. Add a matrix row each for JMAP, Google, Graph.
- **`subscription.rs:102` audit.** The `if self.action_ctx.is_some()` gate today suppresses some subscription work. Verify post-6d-A that the surrounding subscription decision tree behaves correctly without the gate (e.g., always-on path doesn't break when contacts haven't been touched). Could be a one-line behavior change disguised as a deletion.
- **CardDAV stub remains a stub.** The `not_implemented` error survives 6d. CardDAV save / delete returns `LocalOnly` post-6d the same way it did pre-6d. Document this in the Settings UI status surface when it lands; not a 6d task.

## Test plan

### Unit tests (added in 6d)

- **6d-A-1**: Round-trip tests for `ContactSaveParams` (with the new ack shape), `ContactDeleteParams`, `WritebackOutcome` enum (all variants).
- **6d-A-2**: Handler-level tests for the four outcome paths of `handle_contact_save_with_writeback`: local-fail, local-OK + provider-fail, local-OK + provider-OK, synced-without-server_id. Same shape for `handle_contact_delete`. Use a stub provider client (the existing test infra has these).
- **6d-B-2**: Orphan-impl smoke test per provider - construct the client, call `sync_initial` against a stub `SyncProviderCtx`, observe the first network request shape. Already covered by existing per-provider sync tests; the move should not regress them.
- **6d-C-1**: `app_crate_must_not_transitively_depend_on_service_state` - the test runs the BFS, asserts no path. Also includes a synthetic-regression verification (developer-side: temporarily re-add `service-state` to `common` and confirm the test fails with the expected chain).

### Integration tests (existing, regression-guarded)

- All existing `crates/service/tests/*` covering provider sync, dispatch, drain ordering: pass post-6d unchanged. The trait method signatures move crates but stay the same shape; call sites adjust mechanically.
- `crates/service-api/tests/*` notification catalog tests: no new variants (`WritebackOutcome` is a wire type carried inside an ack, not a notification).
- `crates/app/tests/*` UI-side IPC harness tests: the renamed `save_contact_local_only` and the new `save_contact_with_writeback` / `delete_contact` get smoke tests via the existing harness.

### Manual matrix (new rows)

- **Contact save with provider write-back: JMAP.** Add a contact in Settings, observe a `ContactCard/set update` request hitting the JMAP server.
- **Contact save with provider write-back: Google.** Same flow, observe a People API `updateContact` PATCH.
- **Contact save with provider write-back: Graph.** Same flow, observe `/me/contacts/<id>` PATCH.
- **Contact delete with provider write-back: each of JMAP / Google / Graph.** Observe the corresponding destroy / delete request.
- **CardDAV save/delete returns LocalOnly with the existing message.** No regression - just verify the stub still surfaces.
- **Bulk import behavior unchanged.** Import 100 contacts; observe local-only saves (no provider HTTPS), no UI-thread hitch.

### Cross-cutting

- Phase 6c manual matrix items remain in place; 6d does not touch the calendar pipeline.
- The "two `service_subprocess` flaky tests" carry-forward to Phase 8 is unchanged; 6d does not unblock them.

## Carry-forward to later phases

- **CardDAV write-back implementation.** vCard generation + PUT lives outside 6d. Tracked in the contacts product surface, not in the Service roadmap.
- **Settings "sync imported contacts" affordance.** Product decision for whether bulk-import should also write back to providers; orthogonal to 6d.
- **Phase 8 cross-store invariant pass extension.** No 6d-shaped invariants to add; the contacts pipeline is pure SQLite.
- **Phase 8 strict-mode lockdown polish.** The 6d-C test catches one regression class; if a future structural change introduces a *different* writer-half escape (e.g., a new crate that depends on `service-state` and gets added to `app`'s cone), the test catches it. No further phases of the lockdown shape are foreseen.
