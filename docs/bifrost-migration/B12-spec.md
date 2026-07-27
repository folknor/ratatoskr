# B12: shared mailboxes plus public folders onto bifrost scopes

Technical implementation specification for item B12 of
`docs/bifrost-migration.md` section 7:

> B12. Shared mailboxes plus public folders. Rewire
> `ViewScope::SharedMailbox` / `PublicFolder` onto bifrost scopes. Needs A5.

## 0. Contract and required reading

This spec is written against `reference/technical-implementation-spec.md`
(the contract: every brick, obstacles resolved inline, no deferral, no
shoehorning, a named gate per brick, a keep/revert path, concrete
artifacts, a ground survey, a stopping rule). Reviewers judge it against
that document.

Implementers and reviewers MUST READ, not merely note:

- `reference/technical-implementation-spec.md` - the contract above.
- `reference/architecture.md` - the cross-cutting architecture contract.
  Binding here: the crate boundaries (bifrost stays out of `core` and
  `app`), the `MailActionIntent -> resolve_intent -> build_execution_plan
  -> batch_execute` pipeline this spec extends with namespace-aware
  destination resolution, the `OperationResult` taxonomy, and scope
  wiring (`ViewScope` is the sidebar's single source of truth).
- `docs/bifrost-migration.md` - the TODO source. Sections 2 (the
  side-quest protocol and the first principle: bifrost is fixed first,
  ratatoskr is never contorted), 3 (the seam), 4 (the four structural
  shifts), 7 (the B2 cursor-table disposition, the B3a-cut-graph note
  that hands `crates/provider-sync/src/graph/sync/` to B12, and the B6
  note that hands shared-mailbox and public-folder containers to B12),
  10 (behavioral gates are mandatory), 11 (the frozen bifrost commit).
- `reference/glossary/folders-labels.md` - BINDING. This spec adds two
  folder-id namespaces (`shared:` and `public:`) and makes
  `folders.namespace_type` authoritative. The glossary Identity table is
  the contract those additions must land in.
- `reference/glossary/harness.md` - the harness contract: sync-harness
  scripts, the provider mock-endpoint env vars, the sync-bench gate
  baselines, and the ratatoskr / brokkr / saehrimnir boundary this spec
  deliberately does not cross.
- `UI.md` - the app-side read paths and the sidebar surfaces change
  (public folders stop being pseudo-threads), so the UI conventions bind.
- `docs/roadmap/shared-mailboxes.md`, `docs/roadmap/public-folders.md`,
  `docs/roadmap/jmap-sharing.md` - the pre-bifrost design and status of
  the feature this spec rewires. They are the record of what the legacy
  stack intended; they are reconciled by the final brick.
- `./research/bifrost` - the bifrost surface this spec is written
  against, in particular `reference/graph.md` ("Foreign (shared/delegate)
  mailboxes", "Public-folder discovery", "Public-folder cursor"),
  `reference/imap.md` ("Shared / other-user folders (A5c)"),
  `reference/jmap.md` ("Foreign (shared/delegate) accounts"), and
  `reference/sync.md`.
- `./research/saehrimnir` - the mock surface the gates run against
  (`notes/ratatoskr-ews-surface.md`, `notes/ratatoskr-imap-surface.md`,
  `notes/fixture-format.md`).

Frozen dependency commit: `../bifrost` and `./research/bifrost` are at
`2622d9e` ("Bump deps.", whose parents carry `b96d446` graph delegate
Autodiscover and `c90077a` graph public folders - the A5 work this item
consumes). The freeze advances ONCE in this item, at brick B12a, to the
B12-SQ commit; it then holds for the item's full duration per section 11
of the governing plan. `crates/service/Cargo.toml` carries the bifrost
path deps; no new dependency is added.

## 1. What B12 is

Today shared mailboxes and public folders are a HALF-BUILT feature whose
provider legs are dead code and whose read paths are wired to tables
nothing writes. B12 makes both real by routing them through the machinery
B3 to B6 already built: bifrost discovers them as ordinary cursor scopes,
the resident engine syncs them, the existing `ChangeStreamConsumer`
persists them into the SAME `messages` / `threads` / `folders` tables as
personal mail, and the app reads them through one namespace-scoped query
family. Public-folder items stop being a bespoke pseudo-thread table and
become real threads, which is what gives them a reading pane, search,
and (where the provider allows it) actions.

This is a full rewrite of the shared-mailbox and public-folder data path,
not a local change. It deletes roughly 3.5k LOC of dead provider code and
five tables, and it is the landing that finally deletes the
`crates/provider-sync/src/graph/sync/` tree that B3a-cut-graph explicitly
retained for this item.

## 2. Ground survey

### 2.1 ratatoskr: the provider legs are dead code

Verified by call-graph grep across `crates/`, and cross-checked against
git history:

- `crates/provider-sync/src/graph/shared_mailbox_sync.rs`
  (`sync_shared_mailbox`, `sync_all_shared_mailboxes`) has NO caller. Not
  "no caller since B3a-cut-graph": `git grep` at `3ca0e228~1` (the commit
  before that cut) shows no caller either. Per-shared-mailbox mail sync
  has been unreachable since the Tauri removal restructure
  (`bb980259`), which dropped the command layer that called it.
- `crates/graph/src/public_folder_sync.rs` (`sync_all_pinned_folders`,
  `sync_pinned_public_folder`, `browse_public_folders`,
  `pin_public_folder`, `unpin_public_folder`) has NO caller.
- `crates/graph/src/autodiscover.rs` (`discover_shared_mailboxes`,
  `discover_public_folder_routing`, `discover_content_mailbox`) has NO
  caller outside `public_folder_sync.rs`.
- `crates/imap/src/public_folders.rs` (`discover_imap_public_folders`,
  `check_folder_rights`, `sync_imap_public_folder`) has NO caller.
- `crates/graph/src/ews/` is reachable ONLY from `autodiscover.rs` and
  `public_folder_sync.rs`. The four modules form a closed dead cluster.
- `crates/graph/src/client.rs::for_shared_mailbox` is called only from
  the dead `shared_mailbox_sync.rs`.
  `crates/graph/src/ops/mod.rs::{send_as_shared_mailbox,
  send_on_behalf_of}` have no caller (B5 moved send onto the engine).
- `crates/provider-sync/src/graph/sync/` (the retained legacy per-folder
  Graph sync tree, ~2k LOC) is reachable ONLY from
  `shared_mailbox_sync.rs`, so it is dead too. Its module doc says so and
  names B12 as its executioner.

Consequence, and the load-bearing fact for the ordering in section 6:
`threads.shared_mailbox_id` has exactly ONE writer today
(`provider-sync/src/graph/sync/persistence.rs::upsert_thread_record` via
the dead path) and `public_folder_items` / `public_folders` /
`public_folder_pins` / `public_folder_sync_state` /
`public_folder_content_routing` / `graph_shared_mailbox_delta_tokens`
have NO writer at all. Every read path over them therefore returns empty
in production. A read-layer rewrite is behavior-neutral by construction.

### 2.2 ratatoskr: what IS live

- JMAP shared-account DISCOVERY.
  `crates/provider-sync/src/jmap/aux_sync.rs::discover_shared_accounts`
  runs on the resident aux cadence, walks the JMAP session for
  `isPersonal == false` accounts, and writes / revokes
  `shared_mailbox_sync_state` rows. This is the only live producer of
  shared-mailbox rows, and it produces registry rows only - no mail.
- The app read + UI path is COMPLETE and unchanged since before the
  migration:
  - `ViewScope` (`crates/core/src/scope.rs`): `SharedMailbox {
    account_id, mailbox_id }` and `PublicFolder { account_id, folder_id }`.
  - Sidebar: `crates/app/src/ui/sidebar/scope.rs` (scope dropdown lists
    shared mailboxes), `ui/sidebar/public_folders.rs`
    (`pinned_public_folders_section`), `ui/sidebar/mod.rs` (the
    `SharedMailboxSelected` / `PublicFolderSelected` events plus the
    `Sidebar::shared_mailboxes` / `pinned_public_folders` state),
    `handlers/core.rs:65,69` (selection routing), `handlers/core.rs:1190`
    (scope name for the thread-list context), `handlers/core.rs:795`
    (scope reset on account delete).
  - Thread lists: `crates/app/src/helpers.rs` routes
    `ViewScope::SharedMailbox` to `get_threads_for_shared_mailbox` /
    `_starred` / `_snoozed` / `_label_group`
    (`crates/db-read/src/db/queries_extra/scoped_queries.rs:837-1013`,
    plus a STALE DUPLICATE of the same file in `crates/db`), and
    `ViewScope::PublicFolder` to `get_public_folder_items` rendered
    through `Thread::from_public_folder_item`
    (`crates/app/src/db/types.rs:142`).
  - Navigation: `crates/core/src/db/queries_extra/navigation.rs`
    `get_shared_mailbox_navigation`, `get_shared_mailboxes_sync`,
    `get_shared_mailbox_email_sync`, `get_pinned_public_folders_sync`,
    `rights_from_folder` (which reads the `folders.right_*` columns B6
    restored for JMAP).
  - Reading pane and actions SHORT-CIRCUIT on `ViewScope::PublicFolder`
    (`handlers/core.rs:1022`) because public-folder items are not real
    threads.
- Roughly thirty personal-scope query predicates spell
  `t.shared_mailbox_id IS NULL AND t.is_chat_thread = 0` across
  `crates/db-read/.../scoped_queries.rs`, `crates/db/.../scoped_queries.rs`,
  `crates/core/.../navigation.rs`, and `crates/dev-seed/src/pinned_searches.rs`.
  That predicate is the mechanism keeping namespaced threads out of
  personal views.
- `crates/db/src/db/queries_extra/thread_persistence.rs::upsert_thread_aggregate`
  already takes `shared_mailbox_id: Option<&str>` and COALESCEs it (set
  once, never cleared). The bifrost consumer passes `None`
  (`crates/service/src/bifrost/consumer/write.rs:284`).

### 2.3 ratatoskr: the consumer and attach path

- `crates/service/src/bifrost/resident.rs::attach_account` attaches the
  engine, then `prepare_folder_map` -> `containers::sync_containers`,
  then `subscribe_push`, then `register_routing_keys`, then spawns the
  consumer / control / aux tasks. The `HashMap<String, FolderKind>`
  folder map (keyed by bifrost `native_id`) is the ONLY container-derived
  state the consumer, the push subscription, and the action dispatch see.
- `push_subscribe_scopes` (`resident.rs:595`) builds the subscribe set
  from the folder map: every folder for Graph, Inbox plus user folders
  for IMAP, `CursorScope::Account` for Gmail and JMAP.
- `register_routing_keys` (`resident.rs:543`) registers Graph webhook
  ingress keys as `me/mailFolders/{folder_id}/messages`.
- `crates/service/src/bifrost/containers.rs` plus its
  `containers/tests.rs` submodule (`sync_containers`,
  `build_container_rows`, `classify`, `folder_kind_for`) is the single
  container persistence seam. It writes `namespace_type: None`
  unconditionally today (`containers.rs:243`), and the test module
  constructs `Container` values directly.
- `crates/service/src/bifrost/consumer/mod.rs`: `drive_resident` /
  `drive_receiver` route each `MultiplexerEvent` by
  `is_email_scope(&event.scope)` (accepts `Account`, `Type(Email)`,
  `Folder(_)` unconditionally, `FolderType { ty: Email }`), then
  `HydrateBatch::from_changes` -> `write::persist` -> post-persist ->
  search flush -> ack-last. `hydrate.rs` holds the per-provider arms;
  `write.rs` holds the per-provider membership strategies and the thread
  aggregate; `imap_threading.rs` accumulates the drive-end JWZ pass.
- `crates/service/src/actions/dispatch_target.rs` resolves a thread to
  bifrost `ObjectId`s from the `messages` table and resolves
  archive / trash / spam / move destinations by `FolderRole` over the
  folder map (`membership_scope_for` per provider).
- `crates/service/src/bifrost/factory.rs::factory_from_decrypted` builds
  the four provider factories. The Graph arm calls
  `GraphAccountFactory::new(client)` and optionally
  `with_push_endpoint`; it never calls `with_shared_mailbox`,
  `with_delegate_discovery`, or `with_public_folders`.

### 2.4 bifrost at the freeze: A5 is delivered, with named edges

Read from `./research/bifrost` at `2622d9e`.

- Graph shared / delegate mailboxes (`crates/graph/src/account/`,
  `reference/graph.md` "Foreign (shared/delegate) mailboxes"): a
  configured or Autodiscover-enumerated foreign mailbox surfaces as
  ordinary `CursorScope::FolderType { folder, ty: Email }` scopes whose
  `FolderId` carries the owning mailbox via the crate-private
  `account/foreign.rs` codec (`\u{1f}` separator). `client_for_scope`
  routes reads, `owner_of_scope` yields the `MailboxId`, per-message ids
  are foreign-encoded at every projection site so hydration, blobs, and
  MUTATIONS all route to `/users/{owner}`, and `bulk_move` rejects a
  cross-mailbox move as `Request(Malformed)`. Revocation quarantines the
  single foreign scope (`Authorization(PermissionDenied)` plus
  `owner.is_some()` -> `ScopeRevoked` -> `Engine(DisableScope(scope))`)
  instead of failing the account. Enumeration is
  `with_shared_mailbox(id)` (config) plus opt-in
  `with_delegate_discovery()` (POX Autodiscover `alternativeMailboxes`,
  best-effort, non-fatal), both default off.
- Graph public folders (`crates/graph/src/account/public_folder.rs`,
  `reference/graph.md` "Public-folder cursor" and "Public-folder
  discovery"): opt-in `with_public_folders()`. Discovery resolves
  hierarchy routing via Autodiscover `GetUserSettings`, browses from EWS
  `publicfoldersroot`, read-gates on `effective_rights.read`, resolves
  each folder's content mailbox, seeds a `routing_map`, and emits
  `CursorScope::Folder(folder)`. Sync is a watermark poll whose ENTIRE
  state (routing pair, `DateTimeReceived` watermark, full-scan clock,
  `live_ids` deletion baseline capped at 10000) rides inside the opaque
  cursor. `Message`, `CalendarItem`, and `Contact` items sync at identity
  level; `Task` / `DistributionList` / `PostItem` / `Meeting*` drop with a
  scoped warning. `push_subscribe` returns
  `Unsupported(PushSubscribe)` for ANY `Folder` scope in both push modes.
  Lost rights arrive as EWS `ErrorAccessDenied` and quarantine the scope.
- IMAP shared folders (`crates/imap/src/account/factory.rs`,
  `reference/imap.md` "Shared / other-user folders (A5c)"): `open`
  issues NAMESPACE after the personal LIST, enumerates each non-personal
  prefix, reads the principal per folder for an other-user root, gates
  candidates on MYRIGHTS `can_read()`, and tags each `FolderEntry` with
  `shared_owner`. Shared folders are ordinary `CursorScope::Folder`
  scopes; discovery and inventory additionally emit
  `MembershipScope::Mailbox(owner)`. A shared-folder SELECT denial
  routes to `SyncState(ScopeRevoked)` scoped to that cursor; the same
  denial on a personal folder stays account-terminal.
- JMAP foreign accounts (`crates/jmap/src/sync/`, `reference/jmap.md`
  "Foreign (shared/delegate) accounts"): auto-discovered at `open` from
  session accounts with `isPersonal: false`, one
  `CursorScope::Folder(encode_foreign(accountId, mailboxId))` per foreign
  mailbox, `MembershipScope::Mailbox(accountId)` owner tags, per-scope
  routing through `foreign_mail`. Named bifrost limitations at the
  freeze: foreign bulk MUTATIONS are not wired, `get_stream` hydration
  routes through the primary account, and foreign mailbox LIFECYCLE is
  not polled (a new foreign mailbox appears at the next reopen). Foreign
  submission (`send_as`) IS supported.
- Gmail has no delegation surface (Google API limitation). Its
  capabilities advertise nothing shared, so every namespace path must
  no-op cleanly on Gmail rather than branch on provider kind.
- Engine-side: `SyncEngine` has NO public per-scope enable / disable
  API (`crates/sync/src/engine.rs` public surface confirmed). Everything
  discovered is synced. `Engine(DisableScope)` is engine-internal,
  reachable only as a recovery directive.

### 2.5 The two attribution facts that constrain the whole design

1. `Change::ObjectChange` carries NO memberships (`crates/types/src/events.rs:177`),
   and the engine's inventory fan-out constructs
   `Change::ObjectChange { Created }` from an `InventoryEntry` WITHOUT
   reading `.memberships` - neither file mentions the field at all
   (`crates/sync/src/backfill/runner.rs:201`,
   `crates/sync/src/multiplexer/fusion.rs:141`). The
   `MembershipScope::Mailbox(owner)` tag that Graph, IMAP, and JMAP all
   carefully stamp on inventory entries therefore NEVER reaches
   ratatoskr's consumer.
2. On the changes path the owner is not a separate tag either: Graph's
   `ScopeChange.membership` for a foreign scope is
   `MembershipScope::Folder(encode_foreign(mailbox, folder))`
   (`crates/graph/src/account/inventory.rs::membership_from_value`), i.e.
   the owner is INSIDE an opaque id string whose codec is crate-private.

So the only attribution channel available to ratatoskr that does not
require parsing a bifrost-private separator is the SCOPE of the event
(`MultiplexerEvent.scope`) joined against a container index built at
attach time. That join is only possible if `containers_list` enumerates
namespaced containers under the SAME id strings the cursor scopes carry,
which today it does not (see obstacle A).

### 2.6 Harness and mock ground

- saehrimnir ALREADY mocks the IMAP shared-folder surface: `CAPABILITY`
  advertises `NAMESPACE ACL`, a top-level `[[acl]]` grant shares an owned
  mailbox with another declared account, `LIST "" "#user/*"` enumerates
  `#user/<owner>/<path>`, MYRIGHTS / GETACL report rights, a shared
  SELECT reads the owner's messages through `effective_account()`, and
  shared folders are read-only (`NO [NOPERM]` on STORE / COPY / MOVE /
  EXPUNGE).
- saehrimnir ALREADY mocks the EWS public-folder surface on its own
  listener: SOAP Autodiscover `GetUserSettings` (returning
  `ExternalEwsUrl` bound back to itself), `FindFolder` shallow / deep,
  `FindItem` default and IdOnly, `GetItem`, backed by org-wide
  `[[public_folder]]` / `[[public_item]]` fixture tables, plus the
  streaming Subscribe / GetStreamingEvents lifecycle.
- saehrimnir ALREADY serves `/v1.0/users/{userId}/...` for every Graph
  resource family, so Graph shared-mailbox routing works against the mock
  once a second account is declared.
- Three mock gaps block the gates outright (obstacles E, F, G), and two
  more make the strengthened gates unassertable: `[[public_item]]`
  carries no body (so the obstacle-N reading-pane assertion has nothing
  to fetch) and per-folder rights are not staged for the obstacle-W
  projection. All five are covered by B12-SQ-MOCK (section 5.2). The
  three blocking ones: no POX
  `autodiscover.xml` `alternativeMailboxes` response (delegate
  discovery), the EWS surface is on a listener ratatoskr has no endpoint
  override for, and the JMAP fixture loader HARD-REJECTS
  `is_personal = false` ("Stage 1 of the multi-account refactor still
  requires every declared account to be personal",
  `src/fixture.rs:3467`), so a JMAP foreign account cannot be staged.
- `brokkr.toml` declares nine `test_endpoint_env_*` names; there is no
  EWS entry, and `reference/glossary/harness.md` forbids reaching across
  the brokkr contract for anything beyond argv, env, stdout, artefacts,
  and exit status.

## 3. Obstacles, resolved inline

### A. Namespaced containers are invisible at the container seam

`Graph::containers_list` enumerates the PRIMARY client only
(`crates/graph/src/account/pim.rs:631` calls
`account.client.list_mail_folders_recursive()`), so neither foreign
mailbox folders nor public folders appear. `Imap::containers_list` DOES
return shared folders (it snapshots the whole `FolderRegistry`) but
projects no owner, so a shared folder is indistinguishable from a
personal one. JMAP lists primary mailboxes only.

Resolution: bifrost side-quest B12-SQ (section 5.1). `Container` grows an
explicit namespace dimension and `containers_list` enumerates namespaced
containers under the same id strings their cursor scopes carry. Per the
governing plan's first principle this is fixed in bifrost, not worked
around in ratatoskr; the alternative (ratatoskr parsing the `\u{1f}`
foreign separator) hard-codes a crate-private codec into the consumer and
is rejected.

### B. Owner attribution cannot ride the change stream

Section 2.5. Resolution: attribution is SCOPE-derived, not
membership-derived. `sync_containers` returns a `ContainerIndex` that
maps every bifrost `native_id` to `(FolderKind, NamespaceAttribution)`;
the consumer attributes each `MultiplexerEvent` by looking up its scope's
`FolderId`. `CursorScope::Account` and `CursorScope::Type(_)` are
Personal by construction (bifrost cannot express a foreign account-level
scope: `Type(Email)` would collide in the engine index, which is exactly
why JMAP uses `Folder` for foreign scopes - `reference/jmap.md`). No
bifrost change is needed for the join once obstacle A is fixed, and no
membership plumbing is added to the engine.

### C. A DB folder id must not contain a control character

Graph and JMAP foreign `native_id`s embed `\u{1f}`. Those strings are
fine as opaque scope keys but must never become `folders.id` values that
flow into the sidebar, search, and IPC.

Resolution: B12-SQ also exposes `Container::owner_local_id` - the native
id WITHIN the owner's namespace (Graph: the bare folder id; JMAP: the
bare mailbox id; IMAP: the full mailbox path). ratatoskr mints storage
ids from `owner` plus `owner_local_id` and never touches the encoded
form; the `ContainerIndex` holds the `native_id -> storage_id` mapping
so the encoded string stays confined to scope lookups.

### D. Per-mailbox opt-in has no engine-side expression

The legacy Graph path had a per-mailbox `is_sync_enabled` gate; the
engine has no scope-disable API (section 2.4), so anything bifrost
discovers is synced.

Resolution, named honestly rather than faked: opt-in moves to the
DISCOVERY boundary, which is the only place it can be enforced.
- Graph: the enabled `shared_mailboxes` rows feed
  `with_shared_mailbox(...)` and the account flag
  `delegate_discovery_enabled` feeds `with_delegate_discovery()`. A Graph
  user therefore keeps a real per-mailbox opt-in.
- IMAP and JMAP: discovery is protocol-automatic (NAMESPACE, session), so
  every readable shared container syncs. The registry row's flag is a
  VISIBILITY toggle there, and is renamed to say so (`is_visible`).
- Public folders: NOT a visibility-only toggle. See obstacle O - the
  public-folder hierarchy is unbounded, so `with_public_folders()` alone
  is not a safe gate and B12-SQ must carry a pin allowlist into
  discovery.
The ideal-bifrost fix for the SHARED case (an engine `disable_scope` /
`set_scope_enabled` API so a local opt-out suppresses sync of an
already-discovered shared mailbox) is filed as a follow-up in section 9,
NOT smuggled into this item as a ratatoskr-side filter that would
silently drop already-fetched data after paying for it. The PUBLIC case
cannot be deferred that way (obstacle O).

### E. Delegate discovery has no mock

bifrost's `discover_shared_mailboxes` speaks POX Autodiscover
(`/autodiscover/autodiscover.xml`, `alternativeMailboxes`); saehrimnir
implements only SOAP `autodiscover.svc` `GetUserSettings`.

Resolution: saehrimnir side-quest B12-SQ-MOCK (section 5.2) adds the POX
response projected from the declared accounts.

### F. The EWS mock is unreachable from a harness run

The EWS listener has no ratatoskr endpoint override and no brokkr
plumbing, and bifrost's Autodiscover base is the hardcoded production
host.

Resolution, chosen to keep the change inside the two repos the
side-quest protocol covers: saehrimnir MOUNTS the Autodiscover and EWS
routes on the GRAPH listener in addition to the dedicated one, and
bifrost derives its Autodiscover / EWS base from the Graph api-base
override when set (mirroring the existing Gmail, GCAL, and People
harness-redirect seams). `RATATOSKR_TEST_GRAPH_ENDPOINT` then reaches
public-folder discovery with no new env var.
REJECTED alternative: adding `test_endpoint_env_ews` plus an
`--ews-port` spawn to brokkr. It is a third-repo change that
`reference/glossary/harness.md` requires be surfaced as an explicit
design change with a lockstep doc edit in the brokkr repo, and it buys
nothing the co-mounted route does not.

### G. A JMAP foreign account cannot be staged

saehrimnir's fixture normalizer rejects `is_personal = false`.

Resolution: B12-SQ-MOCK relaxes that validation and advertises the
non-personal account in the session (`isPersonal: false`), which is the
exact signal bifrost's `foreign_mail_account_ids` selects on.

### H. A public-folder scope in the push subscribe set kills push

`GraphAccount::push_subscribe(scopes)` REJECTS THE WHOLE REQUEST if any
scope is not subscribable, and every `Folder` scope (i.e. every public
folder) is `Unsupported(PushSubscribe)`. Feeding the widened folder map
into `push_subscribe_scopes` unchanged would therefore turn push OFF for
the entire account the moment one public folder exists.

Resolution: `push_subscribe_scopes` filters on the namespace
attribution - public-folder containers are excluded (poll-only by
bifrost's design), Graph shared-mailbox containers are ALSO excluded
(obstacle R: delegated `.Shared` scopes cannot carry a Graph change
subscription), and IMAP shared folders are included (IDLE is per-folder
and the mock proves it end to end). The filter is therefore
"personal-only for Graph, personal-plus-shared for IMAP, never public".
Gated by a unit test on the pure function plus the existing push scripts
held green with a shared mailbox and a public folder present.

### I. Graph webhook ingress routing keys are personal-shaped

`register_routing_keys` builds `me/mailFolders/{folder_id}/messages`. A
foreign subscription's notification resource is
`users/{owner}/mailFolders/{native}/messages`, and the folder id in the
key must be the NATIVE id, never the `\u{1f}`-bearing encoded string.

Resolution: the routing-key builder consumes the `ContainerIndex` and
emits the existing `me/...` form for personal containers and NOTHING for
namespaced ones - neither public folders (bifrost refuses the
subscription) nor Graph shared mailboxes (obstacle R: the delegated
token cannot hold such a subscription, so a key for it would be dead
weight that only invites a mis-route). The `users/{owner}/...` shape is
specified and unit-tested as the builder's shared arm but is left
UNREACHABLE behind the obstacle-R gate, so the day an application-auth
mode lands the key form is already correct and proven. Gated by a unit
test over the index plus `graph-push-webhook.lua` held green.

### J. Namespace bleed across a thread

A shared-folder copy of a message could merge into a personal thread:
IMAP threading is JWZ over `Message-ID` / `References`, so the same
message present in `INBOX` and in `#user/alice/INBOX` would otherwise
land in one thread, and that thread would then be simultaneously personal
and shared.

Resolution, three invariants:
1. The IMAP drive-end threading accumulator PARTITIONS by
   `(account_id, namespace_key)`; `build_threads` runs once per partition,
   so a shared folder's messages can never merge with personal ones.
2. Partitioning the ALGORITHM is not enough, because the storage KEY
   still collides - see obstacle P. Storage ids are namespace-qualified
   at mint time, which is what actually makes the partitions disjoint in
   the database.
3. `threads.namespace_kind` / `namespace_id` are IMMUTABLE once set. The
   aggregate writer keeps the existing COALESCE-style set-once behavior,
   and a row arriving with a namespace that CONFLICTS with the persisted
   one is skipped with a warning rather than flipped. Per obstacle P this
   check is a REJECT-BEFORE-INSERT preflight, not an aggregate-time
   check, because `write::persist` inserts the thread placeholder and the
   messages before the aggregate runs
   (`crates/service/src/bifrost/consumer/write.rs:72-80`).
All three are unit-gated (section 6, bricks B12b and B12c).

### K. Public folders are read-only at this freeze

bifrost's public-folder support is poll-only and read-only (no
CreateItem / UpdateItem / DeleteItem in the EWS leg; the `Folder` scope
carries no mutation surface). Making public-folder items real threads
exposes them to the action pipeline, which must NOT degrade an
unsupported mutation into a silent `LocalOnly` that pretends to have
worked.

Resolution: the action pipeline resolves the target's namespace and, for
a `Public` namespace, fails the action with the existing `Failed`
outcome carrying an unsupported classification BEFORE any local write,
exactly as B7b's capability gate does for a calendar backend that cannot
write. Per obstacle S the gate CANNOT live in `dispatch_target.rs`: the
batch pipeline calls `op_local` before target resolution for every op
that is not a container move, so a rejection raised there arrives after
the durable local mutation and degrades to `LocalOnly`. The preflight
lands in `crates/service/src/actions/batch.rs` ahead of every `op_local`
call site. Gated by `graph-public-folder-read-only.lua` plus a unit test.

### L. Cross-namespace move must not be attempted

Graph rejects a cross-mailbox `bulk_move` as `Request(Malformed)`, and a
move from a shared mailbox to a personal folder is not expressible.

Resolution: destination resolution in
`crates/service/src/actions/dispatch_target.rs` resolves a `FolderRole`
destination WITHIN the source message's namespace (the archive of the
shared mailbox, not the personal archive), and a plan whose resolved
destination namespace differs from the source's is rejected as `Failed`
with an unsupported classification. The rejection is raised by the
`batch.rs` preflight of obstacle S, not from inside dispatch, so it lands
before `op_local`. Unit-gated.

### M. `crates/db` carries a stale duplicate of the read layer

`crates/db/src/db/queries_extra/scoped_queries.rs` duplicates
`crates/db-read/.../scoped_queries.rs`. B7c hit the same trap and
resolved it by editing both in lockstep.

Resolution: same rule here - every predicate and query-family change
lands in `db-read` (the live re-exported read side) AND in the `db`
duplicate in the same brick. The `-p db-read` test is the authoritative
gate because that is the copy production reads.

### N. Public-folder items have NO hydration route at the freeze

This is the largest hole review found, and it sits under the item's
headline claim ("public-folder items become real threads, which is what
gives them a reading pane").

bifrost's public-folder poll emits `InventoryEntry` / `ObjectChange`
carrying a raw EWS `ItemId` and NO content: `size: None`,
`blob_id: None` (`research/bifrost/crates/graph/src/account/public_folder.rs:440-448`).
Graph hydration is `get_stream` -> `fetch_batch` -> Graph REST `/$batch`
against `/me/messages/{id}` or `/users/{owner}/messages/{id}`
(`account/get.rs:22,95,315`); the strings `ews` / `Ews` / `EWS` do not
appear anywhere in `get.rs` or `blob.rs`, and `reference/graph.md`'s
"Per-scope inventory, changes, hydration" section lists no public-folder
hydration path. An EWS item id is not a Graph message id, so every
public-folder hydration would return `ItemOutcome::Failed` and
`graph-public-folder-sync.lua` as specified (a thread detail resolves for
one item) cannot pass.

Resolution: B12-SQ item 4 (section 5.1) adds a scope-aware EWS
`GetItem`-backed hydration route for `Public` scopes, projecting body and
attachment metadata into the same `HydratedObject` shape the Graph REST
arm produces, plus an EWS `GetAttachment` blob route. saehrimnir already
mocks `GetItem`, which is the signal that this was the original intent.
REJECTED alternative: scoping public folders to headers-only. It keeps
the spec honest but delivers a folder whose messages cannot be read,
which is not the feature B12 claims to land; if the SQ proves too large
the fallback is to split public folders out of B12 entirely rather than
ship an unreadable surface.

### O. Enabling public folders would sync the ENTIRE readable hierarchy

`docs/roadmap/public-folders.md` section 10 is binding here and states
the principle outright: "Public folder trees can contain thousands of
folders with millions of items across an organization. Only sync what the
user explicitly pins." The obstacle-D resolution as first written
contradicts that - `with_public_folders()` turns on discovery of every
readable folder, the engine syncs everything it discovers, and pins were
demoted to visibility.

Resolution: B12-SQ item 5 carries the allowlist INTO discovery.
`GraphAccountFactory::with_public_folders(allow: PublicFolderScope)`
takes either `PublicFolderScope::HierarchyOnly` (browse the tree, emit
NO cursor scopes) or `PublicFolderScope::Pinned(Vec<FolderId>)` (browse
the tree, emit cursor scopes only for the listed folders). ratatoskr
passes the enabled `public_folder_pins` rows. `public_folder_pins`
therefore returns to being a SYNC gate as the roadmap specifies, and
`is_visible` on it is a second, independent display toggle.
Hierarchy-only discovery still populates the `folders` rows so a future
browse affordance has a tree to show, which is why `HierarchyOnly` is a
real mode and not just an empty pin list.
Gated by a zero-fetch assertion: with `public_folders_enabled = 1` and no
pins, `meta.provider_requests` covers the hierarchy walk and NOTHING
else, and zero `messages` rows land.

### P. Namespace columns do not prevent storage-identity collisions

`threads` and `messages` are both keyed `PRIMARY KEY (account_id, id)`
(`crates/db/src/db/schema/02_mail.sql:113,51`). The namespace columns are
ATTRIBUTES of a row, not part of its key, so they cannot separate two
rows that arrive with the same id. Three concrete collisions:

- IMAP: `generate_thread_id` hashes only the root `Message-ID`
  (`crates/sync/src/threading.rs:200`). The same mail present in `INBOX`
  and in `#user/alice/INBOX` yields the SAME `imap-thread-<hash>` key in
  both partitions. Partitioning `build_threads` (obstacle J.1) stops the
  JWZ merge but not the primary-key clash.
- JMAP: foreign inventory emits bare account-scoped `Email` and `Thread`
  ids (`research/bifrost/crates/jmap/src/sync/inventory.rs:569`). JMAP
  guarantees id uniqueness per ACCOUNT, not per server, so a foreign
  account's id may equal a primary-account id.
- Graph: foreign message ids are foreign-encoded at projection, so Graph
  is the one provider that does not collide - which is exactly why the
  fix must be ratatoskr-side and uniform rather than per-provider.

Resolution: storage ids are NAMESPACE-QUALIFIED at mint time. A row whose
attribution is not `Personal` gets its `threads.id` and `messages.id`
prefixed with the namespace key (`shared:{owner}:` / `public:{folder}:`),
exactly the shape section 4.2 already mints for folder ids. The REMOTE
object id keeps living where the action path already reads it
(`messages.provider_message_id`), so `dispatch_target` is unaffected and
no encoded form leaks into a provider request. `generate_thread_id` gains
a namespace parameter rather than a wrapper, so no call site can forget
it. The namespace-conflict check of obstacle J.3 becomes a preflight in
`write::persist` BEFORE the placeholder insert.
Gated by a test that deliberately reuses identical message ids, thread
ids, and root `Message-ID`s across personal and TWO distinct shared
namespaces in one account and asserts three separate threads with three
separate message sets.

### Q. Unknown folder scopes must fail closed, not default to Personal

`attribution_for_scope` as first specified defaulted an unknown
`Folder(f)` to `Personal`. That is fail-OPEN: a namespaced scope missing
from a stale index would persist foreign mail into the personal views.

The index is genuinely a snapshot. `ResidentSlot`'s cached folder map is
built only on (re)attach; `refresh_folder_map`
(`crates/service/src/bifrost/resident.rs:388`) exists for a
dispatch-time destination miss and does not update the slot's cache. An
account reopen inside the engine can discover new foreign or public
scopes without ratatoskr rebuilding the index.

Resolution: `attribution_for_scope` returns
`Option<NamespaceAttribution>`. On a miss the consumer triggers exactly
ONE `refresh_containers` and retries the lookup; still missing, the batch
is SKIPPED with a scoped warning and NOT acked, so it is redelivered
rather than lost or mis-attributed. `Account` and `Type(_)` remain
`Personal` by construction. A new harness case
(`imap-shared-folder-added-after-attach.lua`) grants an ACL mid-run and
asserts the new shared folder's mail lands namespaced, never personal.

Note also the ORDERING dependency this leans on, which B12-SQ must not
break: `SyncEngine::attach` drives `discover_scopes` synchronously before
returning (`research/bifrost/crates/sync/src/engine.rs:244`), so Graph's
`routing_map` and `cursor_index` ARE seeded by the time ratatoskr calls
`sync_containers` (resident.rs:232-245, the "Obstacle A'" comment). The
`containers_list` projection of public folders in B12-SQ item 2 depends
on that; if the SQ ever moves public-folder discovery later than
`discover_cursor_scopes`, the projection silently empties.

### R. Graph shared-mailbox change subscriptions are not available to us

ratatoskr authenticates with DELEGATED scopes, including
`Mail.Read.Shared` / `Mail.ReadWrite.Shared`
(`crates/core/src/oauth.rs:42-52`). Microsoft's change-notification
documentation states that notifications on a shared or delegated mailbox
require APPLICATION permissions (`Mail.Read` as an app permission);
delegated sharing permissions do not support them. saehrimnir will
happily accept the subscription, so a green harness gate here would be
false confidence about production.

Resolution: Graph shared-mailbox scopes stay POLL-ONLY (obstacles H and
I). The gate is inverted accordingly - `graph-push-webhook.lua` must
prove that PERSONAL push remains active and healthy while a shared
mailbox and a public folder exist on the account, and a unit test pins
that `push_subscribe_scopes` emits no shared or public scope for Graph.
The application-auth mode that would enable shared push is named in
section 9, not attempted here.

### S. The namespace preflight must run in `batch.rs`, before `op_local`

`crates/service/src/actions/batch.rs` pre-resolves targets only for
container moves (`is_container_move`, batch.rs:198 and 561). For every
other op `op_local` runs FIRST (batch.rs:212, 418, 548, 574) and a
subsequent target-resolution failure is reported as `LocalOnly`
(batch.rs:228-240, 587-599). Label and label-group actions resolve
targets from their own modules (`actions/label.rs`,
`actions/label_group.rs`) and so bypass `dispatch_target`'s entry point
entirely.

Resolution: a `namespace_preflight(ctx, account_id, thread_id, &op)`
runs ahead of EVERY `op_local` call site in `batch.rs`, reading the
thread's `(namespace_kind, namespace_id)` and the `ContainerIndex`, and
returning `Failed { unsupported }` for a `Public` target or a
cross-namespace destination. It is one function with one caller shape, so
a new action cannot be added past it silently. The label and label-group
paths route through the same preflight. Gated by a test asserting no
local row changed on rejection, for a label op specifically (not only for
a move).

### T. Non-mail public folders need an explicit consumer drop

Section 9 claims `Container.content_class` keeps IPF.Appointment and
IPF.Contact folders out of the mail path, but nothing implements that.
`is_email_scope` accepts `CursorScope::Folder(_)` UNCONDITIONALLY
(`crates/service/src/bifrost/consumer/mod.rs:899-918`), and its doc
comment even asserts that "no non-IMAP provider produces a
`Folder`-scoped batch to mis-route here" - which B12 falsifies for Graph
public folders AND for JMAP foreign mailboxes.

Resolution: `is_email_scope` becomes a method on the consumer that
consults the `ContainerIndex`: a `Public` container whose `content_class`
is not `Mail` is NOT an email scope and its batches are dropped (acked,
not skipped - they are correctly-delivered non-mail data, not a
mis-attribution). The doc comment is rewritten to state the new truth
about `Folder` scopes. With obstacle O in force these folders are only
reachable if a user pins a non-mail folder, which is exactly the case the
gate stages. Gated by a NEGATIVE assertion in
`graph-public-folder-sync.lua`: the IPF.Appointment folder exists as a
`folders` row with its `content_class` and produces ZERO `messages` and
ZERO `threads` rows.

### U. Registry population must be provider-agnostic

The sidebar scope dropdown reads the shared-mailbox registry
(`crates/core/src/db/queries_extra/navigation.rs:678,710`). Today the
only writer is the JMAP aux pass, and B12c as first written rewired only
that pass. A Graph delegate mailbox or an IMAP `#user/alice/*` folder
would therefore sync mail into `folders` / `threads` and never become
selectable - `ViewScope::SharedMailbox` unreachable, the feature
invisible to a real user while the harness queries hard-coded scopes.

Resolution: the reconcile is driven off the `ContainerIndex` for ALL
providers, in `containers.rs` at the end of `sync_containers` (where the
attribution is already computed) rather than in a JMAP-specific aux
module: every `Shared` container upserts its `shared_mailboxes` row
(`owner` as `mailbox_id`, display name from the container, `is_visible`
defaulting to 1), and a row whose owner no longer appears gets
`revoked_at` stamped. The JMAP aux pass is deleted rather than rewired.
Every `Public` container upserts its `public_folder_pins` row with
`is_visible = 1` and `is_sync_enabled = 0`, which is what makes obstacle
O's pin allowlist bootstrappable: hierarchy-only discovery populates the
candidate rows, and enabling one is a single-column update.

Bootstrap order for Graph, stated because it is circular otherwise:
`with_shared_mailbox(...)` reads `shared_mailboxes` at factory build
time, and for a fresh Graph account that table is empty. The first
population therefore comes from `with_delegate_discovery()` (Autodiscover
`alternativeMailboxes`), whose containers the reconcile writes; a
subsequent attach can then serve them from the table even with delegate
discovery off. An explicitly configured mailbox arrives by row insert
from a future Settings surface. Gated by asserting a registry row exists
after each of the three provider sync scripts.

### V. The consumer golden snapshots select the dropped column

`crates/service/src/bifrost/consumer/golden_test.rs:93` selects
`shared_mailbox_id` from `threads` in a byte-identical frozen snapshot
across five fixtures (`{jmap,graph,gmail,imap}_consumer_membership_golden.json`
and `imap_drive_threading_golden.json`). B12b drops that column, so the
brick the spec calls behavior-neutral breaks the query and all five
goldens.

Resolution, stated explicitly because silently re-recording a
legacy-parity golden is precisely what this contract exists to prevent:
the SELECT is edited to `namespace_kind, namespace_id`, the goldens are
regenerated with `UPDATE_GOLDEN=1` in brick B12b, and the diff is
REVIEWED to contain nothing but the column rename (every value stays
`null`, since B12b has no namespace writer). Any other delta in that diff
is a B12b bug, not a rebaseline. The regenerated goldens land in the
B12b commit with that statement in the commit message.

### W. Graph and IMAP never populate `Container::rights`

The read layer's rights rendering (`folders.right_*`,
`rights_from_folder`) and obstacle K's read-only story both want rights,
but only JMAP populates `Container::rights`
(`research/bifrost/crates/types/src/container.rs:205-212` - "Only JMAP
populates this"; IMAP, Graph, and Gmail leave it `None`). saehrimnir's
IMAP shared folders are read-only via MYRIGHTS and bifrost already parses
both MYRIGHTS and EWS `effective_rights`, so this is a small projection
gap, not a missing capability.

Resolution: B12-SQ item 6 projects `ContainerRights` from IMAP MYRIGHTS
and from EWS `effective_rights` in `containers_list`. ratatoskr then
persists them into the existing `folders.right_*` columns for namespaced
containers, so obstacle K's read-only gate is RIGHTS-shaped (this folder
grants no write) rather than merely namespace-shaped, and an IMAP shared
folder does not render as writable.

### X. `FolderKind` needs real variants, not two constructors

Section 4.2 called these "two constructors". The type is a 5-variant enum
at `crates/types/src/folder_label.rs:94-101` (re-exported as
`common::types::FolderKind`) where every variant wraps a
provider-validated newtype, `parse(raw, provider)` is provider-dispatched
and tries `SystemFolderId::parse` first, and `storage_id()` is an
exhaustive match. Shared and public need VARIANTS, which fans out to
every exhaustive match over the enum.

Resolution:
- Two new variants, keeping the inner id PROVIDER-TYPED so the mutation
  path does not lose the validation it relies on:
  `Shared { owner: String, inner: Box<FolderKind> }` and
  `Public { native: String }`. A flat string is rejected: it would let an
  unvalidated id reach a provider request.
- `parse` gains a prefix pre-dispatch: a `shared:` or `public:` prefix is
  peeled first, then the remainder is parsed with the SAME provider
  dispatch, so a shared Graph folder still validates as a `GraphGuid`.
- Separator ambiguity is resolved explicitly. `validate_component`
  (folder_label.rs:504) rejects only empty strings and control
  characters, so `:` is legal inside an IMAP mailbox path and
  `shared:{owner}:{owner_local_id}` is ambiguous under naive splitting.
  Parsing is `splitn(3, ':')` FROM THE LEFT - owner ids never contain `:`
  (they are mailbox ids / email addresses / JMAP account ids), and the
  remainder is taken verbatim, colons included. Unit-tested with an IMAP
  path containing `:`.
- The fan-out sites the implementer must expect: `crates/types`, the
  three `folder_mapper.rs` files (graph, imap, jmap), the consumer's
  `hydrate.rs` and `message_membership.rs`, four sidebar modules,
  `core/navigation.rs`, `containers.rs`, `resident.rs`.
- The gate is `brokkr test -p types folder_kind_shared_and_public_round_trip`,
  not `-p common` - the code is in `crates/types`.

Beware the NAME COLLISION: `crates/core/src/db/queries_extra/navigation.rs:18`
declares a DIFFERENT `FolderKind` (`Universal` / `SmartFolder` /
`AccountFolder`, a sidebar display classification). Section 4.6 edits
that one; section 4.2 edits the identity enum. Adding variants to the
wrong one compiles and silently does nothing.

### Y. The new thread index does not serve the personal predicate

The spec keeps `idx_threads_shared_mailbox`'s shape as
`(account_id, namespace_kind, namespace_id, last_message_at DESC)` while
rewriting personal predicates to `t.namespace_id IS NULL`. Today's
personal thread-list queries are an exact prefix match on
`(account_id, shared_mailbox_id, last_message_at DESC)`
(`02_mail.sql:119`; predicates at `db-read/.../scoped_queries.rs:105,132,145,226`).
With `namespace_kind` UNCONSTRAINED between `account_id` and the ordering
column, SQLite can no longer use the index to satisfy
`ORDER BY last_message_at DESC` - a plausible regression on the hottest
query in the app, at the brick that claims no baseline moves.

Resolution: the personal predicate is `t.namespace_kind IS NULL` (not
`namespace_id`), which SQLite treats as an equality constraint on the
second index column and which restores the exact prefix match. The two
columns are always set or cleared together, so the predicate is
equivalent. Gated by an `EXPLAIN QUERY PLAN` assertion in `db-read` that
the personal inbox query uses `idx_threads_namespace` and reports no
`USE TEMP B-TREE FOR ORDER BY`.

### Z. The registry helpers move at B12b, not B12e

`crates/sync/src/state.rs:347-527` holds the helpers whose SQL names
`shared_mailbox_sync_state` and whose columns include the
`last_synced_at` / `sync_error` pair B12b drops. B12e was scheduled to
rename them, which leaves the tree red between B12b and B12e.

Resolution: the registry helpers' SQL, their signatures, and their own
tests move onto `shared_mailboxes` in B12b, in the same brick as the
schema edit. B12e deletes only the helpers whose COLUMNS are gone (the
delta-token and per-mailbox sync-status families), which is a pure
deletion by then.

### AA. JMAP foreign accounts are half-wired, and the gate hid it

Section 2.4 names bifrost's three JMAP foreign limitations at the freeze
(`research/bifrost/reference/jmap.md`) and then the gate list asks only
that "the non-personal session account's mailboxes sync as `Shared`
containers". A container-only assertion passes while JMAP shared mail is
UNREADABLE, which is the failure mode the contract's behavioral-gate rule
exists to catch. Taking the three in turn:

1. HYDRATION routes through the primary account. A foreign `Email` id
   fetched against the primary account is either a 404 or, worse, a
   different message. This one is fatal to the feature and is FIXED:
   B12-SQ item 7 routes `get_stream` and `blob_open` for a foreign scope
   through the `foreign_mail` client that `client_for_scope` already
   selects for the changes path. It is the same one-line selection the
   crate makes elsewhere; leaving it is not defensible.
2. Foreign MUTATIONS are not wired. Accepted as a limitation, not faked:
   the `batch.rs` preflight (obstacle S) rejects a mutation on a JMAP
   `Shared` namespace as `Failed { unsupported }` exactly as it does for
   `Public`, so a user sees a refusal rather than a silent local-only
   divergence. Named in section 9 as the follow-up that lifts it.
3. Foreign mailbox LIFECYCLE is not polled - a new foreign mailbox
   appears at the next reopen, and a REVOKED one likewise lingers. So the
   obstacle-U reconcile cannot detect revocation promptly for JMAP. Also
   accepted, with the honest consequence stated: for JMAP,
   `revoked_at` is stamped at the next reopen-driven
   `sync_containers`, not at revocation time.

The JMAP gate is strengthened accordingly: `jmap-shared-account-sync.lua`
must prove a hydrated MESSAGE with a body from the foreign account (not
just a container), that the personal thread list excludes it, and that a
mutation attempt on it returns `Failed` with no local row change.

## 4. Target architecture

### 4.1 Schema (v100 edit, per the pre-release no-v101 policy)

`crates/db/src/db/schema/02_mail.sql`:

```sql
-- folders: namespace attribution becomes authoritative.
--   namespace_type: NULL personal | 'shared' | 'public'
--   owner_id: the owning mailbox / principal id for 'shared', NULL else
ALTER-equivalent (edit the CREATE TABLE in place):
    namespace_type TEXT,          -- already present, now meaningful
    owner_id TEXT,                -- NEW
    content_class TEXT,           -- NEW: bifrost content class, public folders

-- threads: one attribution pair replaces shared_mailbox_id.
    namespace_kind TEXT,          -- NEW: NULL | 'shared' | 'public'
    namespace_id TEXT,            -- NEW: owner mailbox id, or public folder storage id
    -- shared_mailbox_id: DROPPED
CREATE INDEX idx_threads_namespace
    ON threads(account_id, namespace_kind, namespace_id, last_message_at DESC);
-- idx_threads_shared_mailbox: DROPPED
```

`crates/db/src/db/schema/10_sync.sql`:

- `shared_mailbox_sync_state` -> replaced by `shared_mailboxes`
  (`account_id`, `mailbox_id`, `display_name`, `email_address`,
  `is_sync_enabled`, `is_visible`, `discovered_at`, `revoked_at`). The
  per-mailbox `last_synced_at` / `sync_error` columns are DROPPED: the
  engine owns per-scope cursors and `ScopeRevoked` isolation now, so a
  ratatoskr-side per-mailbox sync-status column would be a second,
  drifting source of truth.
- `graph_shared_mailbox_delta_tokens`: DROPPED (the engine owns each
  foreign `FolderType` cursor inside the opaque `sync_cursors`
  envelope). This closes the B2 disposition entry for this table and the
  B2 open question about the JMAP `shared_account_id` dimension: bifrost
  puts the foreign account id INSIDE the `Folder` scope's `FolderId`, so
  `scope_key = folder:<encoded>` already discriminates it and no
  `CursorScope` variant is needed. `jmap_sync_state.shared_account_id`
  goes with the table at B15.

`crates/db/src/db/schema/11_collaboration.sql`:

- `public_folders`: DROPPED. The hierarchy is `folders` rows with
  `namespace_type = 'public'` (name, parent, rights, `content_class`).
- `public_folder_items`: DROPPED. Items are `messages` / `threads` rows.
- `public_folder_sync_state`: DROPPED (cursor state is bifrost's, inside
  the opaque envelope).
- `public_folder_content_routing`: DROPPED (bifrost's public-folder
  cursor carries the `X-AnchorMailbox` / `X-PublicFolderMailbox` routing
  pair; a ratatoskr copy would be a stale duplicate).
- `public_folder_pins` KEPT as the SYNC gate the roadmap requires
  (obstacle O) plus visibility and ordering: `account_id`, `folder_id`,
  `is_sync_enabled`, `is_visible`, `sort_order`. `sync_depth_days` and
  `last_sync_at` are DROPPED: sync depth is bifrost's watermark policy
  and last-sync is engine state. Rows are created by hierarchy-only
  discovery with `is_sync_enabled = 0` (obstacle U), so enabling a folder
  is a single-column update and nothing syncs until a user asks for it.

`crates/db/src/db/schema/01_core.sql`, `accounts`:

```sql
    delegate_discovery_enabled INTEGER NOT NULL DEFAULT 0,
    public_folders_enabled INTEGER NOT NULL DEFAULT 0,
```

### 4.2 Folder identity (glossary addition)

`FolderKind` (declared in `crates/types/src/folder_label.rs`, re-exported
as `common::types::FolderKind`) gains two VARIANTS - see obstacle X for
why constructors alone are not enough, for the exhaustive-match fan-out,
for the `splitn`-from-the-left parse rule, and for the `crates/core`
name collision.

```rust
pub enum FolderKind {
    // ... five existing variants ...
    Shared { owner: String, inner: Box<FolderKind> },
    Public { native: String },
}
```

Storage-id forms, added to the glossary Identity table in
`reference/glossary/folders-labels.md`:

- `Shared { owner, inner }` -> `shared:{owner}:{inner.storage_id()}`
- `Public { native }` -> `public:{native}`

`FolderKind::parse` round-trips both, peeling the prefix and then running
the existing provider dispatch on the remainder. Storage MESSAGE and
THREAD ids carry the same namespace prefix (obstacle P). No other prefix
semantics change.

### 4.3 The container index (new type)

`NamespaceKind` and `NamespaceAttribution` live in `crates/types`, NOT in
`service`. The read layer takes `NamespaceKind`
(`db-read/.../scoped_queries.rs`, `core/navigation.rs`) and `service`
depends on `db-read` transitively through `rtsk`, never the reverse;
`db-read` already depends on `types`, and `types` is the workspace's
minimal-dep shared-type crate, so it is the only home both sides can
reach. `service` imports them.

`crates/types/src/folder_label.rs` (alongside `FolderKind`):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum NamespaceKind { Personal, Shared, Public }

// `Hash` is required: this is a `HashMap` key below.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum NamespaceAttribution {
    Personal,
    Shared { owner: String },        // MailboxId string
    Public { folder_storage_id: String },
}
```

`crates/service/src/bifrost/containers.rs` (the file plus its
`containers/tests.rs` submodule):

```rust
#[derive(Debug, Clone, Default)]
pub struct ContainerIndex {
    /// bifrost native_id (the cursor-scope string) -> resolved kind.
    folder_map: HashMap<String, FolderKind>,
    /// bifrost native_id -> namespace attribution.
    namespaces: HashMap<String, NamespaceAttribution>,
    /// (namespace, FolderRole) -> storage id, for action destination
    /// resolution inside a namespace.
    roles: HashMap<(NamespaceAttribution, FolderRole), String>,
}

impl ContainerIndex {
    pub fn folder_map(&self) -> &HashMap<String, FolderKind>;
    /// `None` on an unknown `Folder` id - the caller must refresh once
    /// and then fail closed (obstacle Q). Never defaults to `Personal`.
    pub fn attribution_for_scope(&self, scope: &CursorScope) -> Option<NamespaceAttribution>;
    pub fn storage_id_for_native(&self, native: &str) -> Option<&str>;
    pub fn role_target(&self, ns: &NamespaceAttribution, role: FolderRole) -> Option<&str>;
    pub fn is_push_subscribable(&self, native: &str) -> bool;
    pub fn graph_routing_resource(&self, native: &str) -> Option<String>;
    /// `false` for a `Public` container whose `content_class` is not
    /// `Mail` (obstacle T).
    pub fn is_mail_container(&self, native: &str) -> bool;
    pub fn content_class(&self, native: &str) -> Option<ContainerContentClass>;
}

pub(crate) async fn sync_containers(
    engine: &SyncEngine,
    account_id: &str,
    write_db: &WriteDbState,
) -> Result<ContainerIndex, String>;
```

`attribution_for_scope`: `Account` and `Type(_)` map to
`Some(Personal)` by construction (bifrost cannot express a foreign
account-level scope). `Folder(f)` and `FolderType { folder: f, .. }` look
`f.0` up in `namespaces` and return `None` on a miss. Per obstacle Q the
consumer answers `None` with one `refresh_containers` and a retry, then
skips the batch WITHOUT acking - it never guesses `Personal`.

`ResidentSlot::folder_map: Arc<HashMap<..>>` becomes
`ResidentSlot::containers: Arc<ContainerIndex>`; `refresh_folder_map`
becomes `refresh_containers` and keeps its callers' shape.

### 4.4 Consumer attribution

`ChangeStreamConsumer` gains `with_containers(Arc<ContainerIndex>)`
(replacing `with_folder_map`) and threads the per-event attribution
through:

```rust
// consumer/mod.rs, per MultiplexerEvent
let ns = match self.resolve_attribution(&event.scope).await {
    Some(ns) => ns,
    // Obstacle Q: refreshed once inside `resolve_attribution` and still
    // unknown. Warn, skip, do NOT ack - the batch is redelivered.
    None => { self.warn_unattributed(&event.scope); continue; }
};
// Obstacle T: a non-mail public container's batch is dropped and ACKED.
if !self.is_mail_scope(&event.scope) { self.ack(&event); continue; }
let batch = HydrateBatch::from_changes(&engine, &account, provider, &self.containers, &ns, changes).await?;
write::persist(&stores, &account_id, provider, &ns, &batch.rows, &batch.deleted_ids).await?;
```

The free function `is_email_scope` (`consumer/mod.rs:899`) becomes the
method `is_mail_scope` so it can consult the index, and its doc comment -
which today asserts that no non-IMAP provider produces a `Folder`-scoped
batch - is rewritten: B12 makes Graph public folders and JMAP foreign
mailboxes `Folder`-scoped. Any downstream logic in `hydrate.rs` that
inferred "IMAP" from a `Folder` scope is audited in the same brick.

- `hydrate.rs`: `ConsumerMessageRow` gains `namespace: NamespaceAttribution`.
  For a `Shared` or `Public` attribution the row's folder membership is
  resolved through `ContainerIndex::storage_id_for_native` so a shared
  INBOX lands on `shared:alice:INBOX`, never on the personal `INBOX`, and
  the row's STORAGE message id and thread id are namespace-prefixed
  (obstacle P) while `provider_message_id` keeps the remote id verbatim.
  Labels are not synthesized for namespaced rows (a foreign mailbox's
  keywords stay per-message; no shared-namespace label rows are minted).
  Public-folder rows hydrate through the EWS `GetItem` route B12-SQ adds
  (obstacle N).
- `write.rs`: the namespace preflight runs BEFORE the thread-placeholder
  insert (obstacle J.3, obstacle P) - a row whose namespace conflicts
  with the persisted thread's is dropped with a warning and never
  inserted. `upsert_thread_aggregate` then receives
  `(namespace_kind, namespace_id)` instead of `None`, keeping its
  set-once COALESCE behavior.
- `imap_threading.rs`: the accumulator keys by
  `(account_id, namespace_key)` and runs one `build_threads` per
  partition at drive end; `generate_thread_id` takes the namespace so
  the two partitions cannot mint the same key (obstacle P). The
  deferred-ack contract is unchanged.
- `post_persist.rs`: `seen_ingest` is namespace-agnostic and unchanged.

### 4.5 Factory wiring

`crates/service/src/bifrost/factory.rs`:

```rust
// Graph arm, after push endpoint wiring:
for mailbox in decrypted.row.enabled_shared_mailboxes.iter() {
    factory = factory.with_shared_mailbox(mailbox.clone());
}
if decrypted.row.delegate_discovery_enabled {
    factory = factory.with_delegate_discovery();
}
if decrypted.row.public_folders_enabled {
    // Obstacle O: pins are a SYNC allowlist, not a display toggle.
    factory = factory.with_public_folders(if pinned.is_empty() {
        PublicFolderScope::HierarchyOnly
    } else {
        PublicFolderScope::Pinned(pinned)
    });
}
```

`read_bifrost_account_credentials` grows the two flags, the enabled
shared-mailbox routing keys, and the sync-enabled public-folder pins (two
extra queries against `shared_mailboxes` and `public_folder_pins`, on the
attach path only). IMAP, JMAP, and Gmail arms are unchanged: IMAP and
JMAP discover automatically, Gmail has no surface. Obstacle U states the
bootstrap order for the Graph `shared_mailboxes` read on a fresh
account.

### 4.6 Read layer

`crates/db-read/src/db/queries_extra/scoped_queries.rs` (and the `db`
duplicate, in lockstep):

- `get_threads_for_shared_mailbox{,_starred,_snoozed,_label_group}` become
  `get_threads_for_namespace{,_starred,_snoozed,_label_group}(conn,
  account_id, kind: NamespaceKind, namespace_id, folder_or_label, limit)`.
  The CTE that pre-filters thread ids by `shared_mailbox_id` filters by
  `(namespace_kind, namespace_id)` instead.
- Every personal-scope predicate changes from
  `t.shared_mailbox_id IS NULL` to `t.namespace_kind IS NULL` (roughly
  thirty sites across `db-read`, `db`, `core/navigation.rs`,
  `dev-seed/pinned_searches.rs`). `namespace_kind`, not `namespace_id`:
  obstacle Y explains why the index depends on it.
- `core/navigation.rs`: `get_shared_mailbox_navigation` becomes
  `get_namespace_navigation(conn, account_id, kind, namespace_id)`;
  for `Public` it returns the single public folder plus its unread count
  rather than the universal folder set (a public folder has no Sent /
  Drafts / Trash). `get_shared_mailboxes_sync` reads the renamed
  `shared_mailboxes` table; `get_pinned_public_folders_sync` reads
  `public_folder_pins` joined to the `namespace_type = 'public'` folder
  rows for display name and unread count. The private helper
  `load_label_group_unread_counts_for_shared_mailbox`
  (`navigation.rs:502`, plus its `t.shared_mailbox_id = ?2` predicate and
  its caller at `navigation.rs:577`) is renamed and rewired onto the
  namespace pair in the same pass.
- `get_public_folder_items` and `Thread::from_public_folder_item` are
  DELETED. `crates/app/src/helpers.rs` routes
  `ViewScope::PublicFolder` to `get_threads_for_namespace(Public, ..)`,
  so the thread list, reading pane, search, and thread detail all work
  with no public-folder special case. The
  `matches!(scope, ViewScope::PublicFolder { .. })` short-circuit at
  `handlers/core.rs:1022` is removed.

`crates/core/src/send_identity.rs:19,40` carries a `shared_mailbox_id`
field on `SendIdentityContext`. It is renamed to `namespace_id` with a
`namespace_kind` companion in this brick so the field name does not
outlive the column, even though the compose wiring that would populate it
stays out of scope (section 9). `select_from_address`'s behavior is
unchanged and its existing tests are updated for the rename only.

`ViewScope` itself is UNCHANGED. Its two namespaced variants already
carry exactly `(account_id, mailbox_id | folder_id)`, which is the
`(kind, namespace_id)` pair the new query family takes. Nothing in the
app learns a new scope concept.

### 4.7 Action layer

`crates/service/src/actions/batch.rs` (the preflight, obstacle S):

- A new `namespace_preflight(ctx, account_id, thread_id, &op)` runs ahead
  of EVERY `op_local` call site (batch.rs:212, 272, 418, 548, 574). It
  reads the thread's `(namespace_kind, namespace_id)` and the
  `ContainerIndex`, and returns `Failed { unsupported }` for a `Public`
  target, for a rights-denied `Shared` target (obstacle W), or for a
  destination whose namespace differs from the source's (obstacle L).
- The label and label-group paths (`actions/label.rs`,
  `actions/label_group.rs`), which resolve targets from their own
  modules, route through the same preflight.

`crates/service/src/actions/dispatch_target.rs`:

- `resolve_thread_messages` additionally reads the thread's
  `(namespace_kind, namespace_id)`, and resolves remote object ids from
  `provider_message_id` (the storage id is namespace-prefixed now,
  obstacle P).
- Role destinations resolve via `ContainerIndex::role_target(ns, role)`
  so archive / trash / spam stay inside the namespace.
- The namespace checks are repeated here as a defence-in-depth assertion
  (an unreachable `Failed`), because dispatch is also entered from the
  retry worker.
- Everything else - intent resolution, planning, the `action_jobs`
  journal, optimistic UI, undo - is untouched.

## 5. Prerequisite side-quests

Both land BEFORE any ratatoskr brick, through the section 2 side-quest
protocol (one Opus agent confined to `./research/<repo>`, orchestrator
reviews / validates in place / commits / promotes via
`bash scripts/bifrost.sh` or `bash scripts/saehrimnir.sh`).

### 5.1 B12-SQ (bifrost): namespaced containers

1. `bifrost_types::Container` gains:
   - `namespace: ContainerNamespace` (new `#[non_exhaustive]` enum:
     `Personal`, `Shared`, `Public`), defaulting to `Personal` for every
     existing construction site.
   - `owner: Option<MailboxId>` - the owning mailbox / principal for a
     `Shared` container.
   - `owner_local_id: Option<String>` - the native id within the owner's
     namespace (Graph: bare folder id; JMAP: bare mailbox id; IMAP: full
     mailbox path). Never the foreign-encoded form.
   - `content_class: Option<ContainerContentClass>` (new enum: `Mail`,
     `Calendar`, `Contacts`, `Other`) - populated for Graph public
     folders from the EWS `FolderClass` bifrost already parses, `None`
     elsewhere.
2. `containers_list` enumerates namespaced containers under the SAME id
   strings their cursor scopes carry (`native_id` is the scope string):
   - Graph: each `shared_clients` entry's folders (foreign-encoded
     `native_id`, `owner`, `owner_local_id`, `namespace = Shared`), and
     each `routing_map` public folder (`namespace = Public`,
     `content_class`).
   - JMAP: each `foreign_mail` account's mailboxes
     (`native_id = encode_foreign(accountId, mailboxId)`,
     `owner = MailboxId(accountId)`, `owner_local_id = mailboxId`).
   - IMAP: project the existing `FolderEntry.shared_owner` onto
     `owner` / `namespace = Shared` / `owner_local_id = path`.
   A per-mailbox enumeration failure degrades to a `Warning` and the
   remaining containers, matching the discovery path's existing shape.
   The additive fields land as new `with_*` builder setters and
   `Container::new`'s positional arity is UNCHANGED. This is load-bearing
   for B12a's "purely additive" claim: `Container` is deliberately not
   `#[non_exhaustive]` (`research/bifrost/crates/types/src/container.rs:169`)
   and ratatoskr constructs it directly in
   `crates/service/src/bifrost/containers/tests.rs:41,61,314` via
   `Container::new(..).with_*()`, so the claim holds only under the
   builder discipline the type's own doc comment prescribes.
3. Graph Autodiscover and EWS bases honor the harness api-base override
   (obstacle F), mirroring `from_token_source_with_api_base`.
4. EWS `GetItem` hydration for `Public` scopes (obstacle N). `get_stream`
   dispatches on the scope: a `Folder` scope present in `routing_map`
   routes to an EWS `GetItem` request (body, headers, recipients,
   attachment metadata) with the cursor's `X-AnchorMailbox` /
   `X-PublicFolderMailbox` routing pair, projected into the same
   `HydratedObject` the Graph REST arm returns. Attachment bytes route to
   EWS `GetAttachment` from `blob_open`. Without this the item's headline
   deliverable does not work at all.
5. `with_public_folders(scope: PublicFolderScope)` where
   `PublicFolderScope` is `HierarchyOnly` or `Pinned(Vec<FolderId>)`
   (obstacle O). `discover_public_folder_scopes` keeps browsing and
   seeding `routing_map` for the full readable hierarchy (so
   `containers_list` can project it) but emits `CursorScope::Folder` ONLY
   for allowlisted folders. `HierarchyOnly` emits none. The existing
   no-argument form is replaced, not overloaded - a silently-permissive
   default is what created the obstacle.
6. `ContainerRights` projection for IMAP (from MYRIGHTS, already parsed)
   and Graph (from EWS `effective_rights`, already parsed) in
   `containers_list` (obstacle W).
7. JMAP foreign hydration routes through the foreign account (obstacle
   AA.1): `get_stream` and `blob_open` select the `foreign_mail` client
   for a foreign scope, the same selection `client_for_scope` already
   makes on the changes path.

bifrost-side gates (its own repo rules: small deterministic unit tests
only): container projection tests per provider, a round-trip test that
`native_id` equals the string `discover_cursor_scopes` emits for the same
container, a test that `HierarchyOnly` and a one-element `Pinned` list
emit zero and one cursor scope respectively while both populate
`routing_map`, an EWS `GetItem` projection test, a rights-projection test
per provider, a test that a foreign JMAP scope's hydration request carries
the FOREIGN account id, and the api-base override test. Validated in
place with
`brokkr check` inside `./research/bifrost` before the commit, then
ratatoskr's own `brokkr check` after `bifrost.sh` (the authoritative gate
for a path dep compiled from source).

### 5.2 B12-SQ-MOCK (saehrimnir)

1. POX Autodiscover `/autodiscover/autodiscover.xml` returning
   `alternativeMailboxes` projected from the declared accounts
   (obstacle E).
2. Mount the Autodiscover and EWS routes on the GRAPH listener in
   addition to the dedicated EWS listener (obstacle F).
3. Allow `is_personal = false` in the fixture normalizer and advertise
   the account as non-personal in the JMAP session (obstacle G).
4. `[[public_folder]]` gains a `folder_class` field so an IPF.Note vs
   IPF.Appointment folder can be staged.
5. `[[public_item]]` gains body and attachment fields, and EWS `GetItem`
   returns them, so the obstacle-N hydration route has something to
   hydrate and the reading-pane assertion can be real.
6. `GETACL` / `MYRIGHTS` responses and the EWS `effective_rights`
   projection are staged per folder so the obstacle-W rights projection
   is observable end to end (a read-only shared folder and a writable one
   in the same fixture).

Gated in place by saehrimnir's own integration tests, then behaviorally
by the ratatoskr sync-harness scripts after `saehrimnir.sh` reinstalls
the binary.

## 6. Bricks, in landing order

Each brick is one coherent, fully intrusive landing, kept or reverted on
its gates. `brokkr check` is green at every boundary.

### B12a. Promote the prerequisites

Land and promote B12-SQ and B12-SQ-MOCK; record the new `../bifrost`
HEAD as the item's freeze in section 11 of the governing plan (bundled
with the B12b code commit, never a standalone markdown commit).

Gates:

```
brokkr check
brokkr service-suite
```

`service-suite` proves the widened `Container` shape did not disturb the
existing container persistence (the additive fields default to
`Personal` / `None`).

### B12b. Schema and read layer (behavior-neutral)

The whole storage and read rewrite, landing while nothing yet WRITES a
namespace. Behavior-neutral by the section 2.1 finding: the tables being
dropped have no writer and the queries being rewritten return empty in
production today.

- Schema edits of section 4.1 (all five files).
- `FolderKind::shared` / `::public` plus `parse` round-trip (4.2), and
  the `reference/glossary/folders-labels.md` Identity-table addition.
- The namespace query family and the roughly thirty predicate rewrites in
  `db-read` AND the `db` duplicate, in lockstep (obstacle M).
- `core/navigation.rs`: `get_namespace_navigation`, the renamed
  registry readers, `rights_from_folder` unchanged.
- App: `helpers.rs` routes both namespaced scopes to the namespace
  family, the public-folder pseudo-thread path
  (`get_public_folder_items`, `Thread::from_public_folder_item`, the
  `handlers/core.rs:1022` short-circuit) is deleted, sidebar state reads
  the renamed registry.
- `upsert_thread_aggregate` takes `(namespace_kind, namespace_id)` with
  the set-once rule (obstacle J.3). All current callers pass
  `(None, None)`.
- The registry helpers in `crates/sync/src/state.rs:347-527` move onto
  `shared_mailboxes` here, not at B12e (obstacle Z).
- The consumer golden SELECT is edited and the five goldens are
  regenerated with `UPDATE_GOLDEN=1`, with a reviewed all-null diff
  (obstacle V).
- `send_identity.rs`'s `shared_mailbox_id` field is renamed (4.6).
- `dev-seed`: `pinned_searches.rs` predicates, and a seeded shared
  mailbox plus public folder so the dev app renders the surfaces.

Gates:

```
brokkr check
brokkr test -p db-read namespace_threads_are_excluded_from_personal_scope
brokkr test -p db-read get_threads_for_namespace_partitions_shared_and_public
brokkr test -p db-read personal_inbox_query_uses_namespace_index_without_temp_btree
brokkr test -p db namespace_is_set_once_and_conflict_is_skipped
brokkr test -p types folder_kind_shared_and_public_round_trip
brokkr test -p types folder_kind_shared_parses_imap_path_containing_colon
brokkr test -p rtsk namespace_navigation_public_folder_has_no_universal_folders
brokkr test -p service consumer_golden
brokkr service-suite
```

The four `*_containers_attach` sync-bench gates and the four
`*_steady_state_delta` gates are re-run to prove the schema edit moved
nothing; no baseline should shift at this brick.

### B12c. The live cut: container index, attribution, factory, push

The intrusive landing that makes namespaced mail arrive.

- `ContainerIndex` (4.3) plus the `build_container_rows` extension that
  writes `namespace_type` / `owner_id` / `content_class` and mints
  namespaced storage ids.
- `resident.rs`: `ResidentSlot::containers`, `refresh_containers`,
  namespace-aware `push_subscribe_scopes` (obstacle H) and
  `register_routing_keys` (obstacle I).
- Consumer attribution (4.4) across `mod.rs`, `hydrate.rs`, `write.rs`,
  `imap_threading.rs`.
- Factory wiring (4.5) plus the two `accounts` flags and the
  `shared_mailboxes` read.
- The provider-agnostic registry reconcile at the end of
  `sync_containers` (obstacle U): `Shared` containers upsert
  `shared_mailboxes` rows and absent owners get `revoked_at`; `Public`
  containers upsert `public_folder_pins` candidate rows with
  `is_sync_enabled = 0`. The JMAP-only aux pass
  (`provider-sync/src/jmap/aux_sync.rs::discover_shared_accounts`) is
  DELETED, not rewired - it was the last live consumer of the legacy JMAP
  session surface in that file, and a JMAP-shaped reconcile is exactly the
  gap obstacle U closes.
- Fail-closed attribution with one refresh-and-retry (obstacle Q), and
  the `is_mail_scope` content-class drop (obstacle T).
- Namespace-qualified storage ids for messages and threads, the
  `generate_thread_id` namespace parameter, and the pre-insert conflict
  preflight in `write::persist` (obstacle P).

Gates (each script is new unless noted):

```
brokkr check
brokkr test -p service container_index_attributes_scope_to_namespace
brokkr test -p service unknown_folder_scope_refreshes_once_then_fails_closed
brokkr test -p service push_subscribe_scopes_excludes_public_and_graph_shared
brokkr test -p service graph_routing_resource_uses_owner_path_and_native_id
brokkr test -p service containers_persist_namespace_owner_rights_and_content_class
brokkr test -p service registry_reconcile_upserts_and_revokes_for_all_providers
brokkr test -p service imap_threading_partitions_by_namespace
brokkr test -p service identical_ids_across_namespaces_do_not_collide
brokkr test -p service non_mail_public_container_batches_are_dropped
brokkr service-test crates/app/tests/sync-harness/imap-shared-folder-sync.lua
brokkr service-test crates/app/tests/sync-harness/imap-shared-folder-revoked.lua
brokkr service-test crates/app/tests/sync-harness/imap-shared-folder-added-after-attach.lua
brokkr service-test crates/app/tests/sync-harness/graph-shared-mailbox-sync.lua
brokkr service-test crates/app/tests/sync-harness/graph-delegate-discovery.lua
brokkr service-test crates/app/tests/sync-harness/graph-public-folder-sync.lua
brokkr service-test crates/app/tests/sync-harness/graph-public-folder-unpinned-zero-fetch.lua
brokkr service-test crates/app/tests/sync-harness/jmap-shared-account-sync.lua
brokkr service-test crates/app/tests/sync-harness/graph-push-webhook.lua
brokkr service-test crates/app/tests/sync-harness/imap-push-idle.lua
brokkr service-suite
```

New fixtures under `crates/app/tests/sync-fixtures/`:
`shared-imap-small.toml` (two accounts plus an `[[acl]]` grant, a
deliberately shared `Message-ID` across the personal and shared folder,
and one read-only plus one writable shared folder),
`shared-graph-small.toml` (two accounts; the delegate variant drives POX
Autodiscover), `shared-jmap-small.toml` (a second account with
`is_personal = false`, carrying a message with a body),
`public-folder-small.toml` (`[[public_folder]]` / `[[public_item]]` with
bodies, one IPF.Note and one IPF.Appointment folder).

What each script must prove, so a compile-only replacement cannot pass:

- `imap-shared-folder-sync`: after a sync, the shared folder exists as a
  `folders` row with `namespace_type = 'shared'`, `owner_id = alice`, and
  the read-only `right_*` columns MYRIGHTS reported (obstacle W); the
  owner's messages are `messages` rows; their threads carry
  `namespace_kind = 'shared'` / `namespace_id = alice`; the PERSONAL
  thread list does not contain them; the namespace thread list does; a
  `shared_mailboxes` registry row exists so the scope dropdown can offer
  it (obstacle U). The fixture stages the SAME `Message-ID` in the
  personal INBOX and the shared folder, and the script asserts two
  distinct threads with distinct ids (obstacle P).
- `imap-shared-folder-revoked`: revoking the ACL grant mid-run
  quarantines that scope only - the shared threads survive locally, a
  scoped warning is observed, and the personal folders keep syncing
  (assert a subsequent personal delta still completes).
- `imap-shared-folder-added-after-attach`: an ACL grant appears mid-run;
  the new shared folder's mail is attributed `shared`, never personal
  (obstacle Q). A variant hook that suppresses the index refresh asserts
  the batch is SKIPPED and redelivered rather than mis-attributed.
- `graph-shared-mailbox-sync`: same planes as the IMAP script,
  driven through `/users/{owner}/...`, plus the request log showing the
  foreign folder delta was fetched against the owner path and NOT
  against `/me`, plus the `shared_mailboxes` registry row.
- `graph-delegate-discovery`: with `delegate_discovery_enabled = 1` and
  no configured mailbox, the POX Autodiscover response alone produces the
  shared containers AND the registry rows (this is the bootstrap path of
  obstacle U); with the flag off, it produces none.
- `graph-public-folder-sync`: with the IPF.Note folder PINNED
  (`is_sync_enabled = 1`), its items land as real threads under
  `namespace_kind = 'public'`, the folder row carries `content_class`, and
  the reading pane path resolves a thread detail WITH A BODY for one of
  them (`TestQueryDbState` plus a thread-detail request) - which is the
  assertion that proves the obstacle-N EWS hydration route works. The
  co-staged IPF.Appointment folder, also pinned, produces ZERO `messages`
  and ZERO `threads` rows (obstacle T).
- `graph-public-folder-unpinned-zero-fetch`: with
  `public_folders_enabled = 1` and NO pins, the hierarchy `folders` rows
  and the `public_folder_pins` candidate rows appear, `meta.provider_requests`
  covers the hierarchy walk and nothing more, and zero `messages` rows
  land (obstacle O).
- `jmap-shared-account-sync`: the non-personal session account's
  mailboxes sync as `Shared` containers keyed by JMAP account id, a
  message from that account is hydrated WITH A BODY through the foreign
  client (obstacle AA.1), the personal thread list excludes it, and a
  mutation attempt on it returns `Failed` with no local row change
  (obstacle AA.2).
- `graph-push-webhook` / `imap-push-idle` (existing, extended): PERSONAL
  push remains subscribed and healthy with a shared mailbox and a public
  folder present, no subscription is attempted for either (obstacle R),
  and for IMAP a shared-folder IDLE notification routes to the right
  namespace.

Sync-bench, re-recorded at this brick because attach now enumerates more
containers:

```
brokkr sync-bench crates/app/tests/sync-harness/graph-initial.lua --gate graph_containers_attach
brokkr sync-bench crates/app/tests/sync-harness/imap-initial.lua --gate imap_containers_attach
brokkr sync-bench crates/app/tests/sync-harness/jmap-initial.lua --gate jmap_containers_attach
brokkr sync-bench crates/app/tests/sync-harness/gmail-initial.lua --gate gmail_containers_attach
brokkr sync-bench crates/app/tests/sync-harness/graph-steady-state-delta.lua --gate graph_steady_state_delta
brokkr sync-bench crates/app/tests/sync-harness/imap-steady-state-delta.lua --gate imap_steady_state_delta
brokkr sync-bench crates/app/tests/sync-harness/jmap-steady-state-delta.lua --gate jmap_steady_state_delta
brokkr sync-bench crates/app/tests/sync-harness/gmail-steady-state-delta.lua --gate gmail_steady_state_delta
```

Budget contract, stated so a regression is a gate failure and not a
shrug: on an account with NO shared mailbox and NO public folder (every
existing fixture), `meta.provider_requests` must not move at all - the
namespace legs are gated off at the factory, so there is no new round
trip. Only the four `*_containers_attach` baselines may move, and only if
the extra `shared_mailboxes` DB read shows up in elapsed; a
`provider_requests` change on a personal-only fixture is a BUG, not a
rebaseline.

A new gate pins the namespaced steady state:

```
brokkr sync-bench crates/app/tests/sync-harness/graph-shared-mailbox-steady-state.lua --gate graph_shared_mailbox_steady_state
brokkr sync-bench crates/app/tests/sync-harness/graph-public-folder-steady-state.lua --gate graph_public_folder_steady_state
```

with `meta.provider_requests` exact-matched (one delta per foreign
folder, one watermark poll per PINNED public folder and none for
unpinned ones, no personal-path duplication) and `meta.correct = 1`.
Recorded with `--as-baseline` on the landing host, `brokkr.toml` gate
block added in the same commit. The public-folder gate is what keeps
obstacle O honest over time: the request count is a function of the pin
count, not of the hierarchy size, and the fixture stages more folders
than it pins so a regression to hierarchy-wide sync shows up as a
multiplied request count rather than a shrug.

### B12d. Action layer

Namespace-aware destination resolution plus the read-only gate (4.7,
obstacles K and L).

Gates:

```
brokkr check
brokkr test -p service role_destination_resolves_within_source_namespace
brokkr test -p service cross_namespace_move_is_rejected_before_local_write
brokkr test -p service public_namespace_action_fails_without_local_mutation
brokkr test -p service public_namespace_label_action_fails_without_local_mutation
brokkr test -p service rights_denied_shared_folder_action_fails_before_local_write
brokkr test -p service jmap_shared_namespace_mutation_is_rejected
brokkr service-test crates/app/tests/sync-harness/graph-shared-mailbox-action-writeback.lua
brokkr service-test crates/app/tests/sync-harness/graph-public-folder-read-only.lua
brokkr service-test crates/app/tests/sync-harness/graph-action-writeback.lua
brokkr service-test crates/app/tests/sync-harness/imap-writeback-flags.lua
brokkr service-suite
```

`graph-shared-mailbox-action-writeback` must verify by SERVER ROUND-TRIP
in the B4a style: the `action.completed` summary shows
`remote_succeeded >= 1`, `remote_failed == 0`, `local_only == 0`,
`conflicts == 0` (so a silent local-only degrade fails the gate), and a
resync shows the message filed in the SHARED mailbox's destination
folder. `graph-public-folder-read-only` asserts the action surfaces as
`Failed` AND that no local row changed - for a LABEL action as well as a
move, since the label path has its own target resolution (obstacle S).
The Graph shared-mailbox writeback script uses a WRITABLE shared folder;
the read-only one from the IMAP fixture drives
`rights_denied_shared_folder_action_fails_before_local_write`.

### B12e. Residue sweep and doc reconciliation

Delete the dead cluster the survey pinned, in one commit:

- `crates/provider-sync/src/graph/shared_mailbox_sync.rs` and the entire
  retained `crates/provider-sync/src/graph/sync/` tree (`mod.rs`,
  `delta_tokens.rs`, `folders.rs`, `persistence.rs`, `stores.rs`), plus
  their `mod.rs` wiring.
- `crates/graph/src/public_folder_sync.rs`,
  `crates/graph/src/autodiscover.rs`, `crates/graph/src/ews/`.
- `crates/imap/src/public_folders.rs`.
- `crates/graph/src/client.rs::for_shared_mailbox`.
- `crates/sync/src/state.rs`: the shared-mailbox delta-token helpers
  (`save_/load_/delete_shared_mailbox_delta_token{,s}`) and the
  per-mailbox sync-status helper `update_shared_mailbox_sync_status`
  whose columns the schema dropped. The registry helpers (including
  `get_enabled_shared_mailboxes`, which the factory now reads) survived
  the rename back at B12b (obstacle Z), so this is a pure deletion.
- `crates/db/src/db/queries_extra/provider_sync_writes.rs`: the six
  public-folder helpers (`upsert_public_folders`,
  `update_public_folder_rights`, `upsert_public_folder_items`,
  `delete_stale_public_folder_items`, `delete_all_public_folder_items`,
  `get_public_folder_sync_depth`), and `pin_public_folder` /
  `delete_public_folder_pin` / `get_pinned_folder_ids` narrowed to the
  new pin shape.

Doc reconciliation bundled with this code commit:
`reference/glossary/folders-labels.md` (namespaces; already touched at
B12b), `reference/architecture.md` (scope wiring: namespaced threads and
the action-layer namespace rule), `reference/glossary/harness.md` (the
new scripts, fixtures, and the Graph-listener EWS co-mount),
`docs/roadmap/shared-mailboxes.md`, `docs/roadmap/public-folders.md`,
`docs/roadmap/jmap-sharing.md` (re-cut against the bifrost path),
`docs/bifrost-migration.md` (the B12 done-note plus the section 11
freeze advance), and `AGENTS.md` if the crate map changes.

Gates:

```
brokkr check --all
brokkr service-suite
```

`--all` here on purpose: a deletion sweep of this size must show every
diagnostic, not a changed-files subset.

## 7. Keep/revert and ordering

Five landings, each independently revertable:

- B12a is additive in bifrost and saehrimnir; reverting is a freeze
  rollback plus a mock reinstall.
- B12b is behavior-neutral (section 2.1) and self-contained in the
  storage and read layer; reverting restores the dead tables and the
  pseudo-thread read path. One artifact is not inert: the five consumer
  goldens are regenerated here (obstacle V). Reverting B12b reverts them
  with it, and the review rule is that the regeneration diff contains
  nothing but the column rename with every value still null.
- B12c is the real cut. If its gates fail it reverts alone: B12b's
  namespace columns simply stay NULL and the app behaves exactly as
  before (no namespaced mail arrives, the sidebar surfaces render empty -
  today's production state).
- B12d and B12e are strictly downstream of B12c and revert alone.

Why the tree stays green at each boundary: B12b writes no namespace, so
its query family is exercised only by its own tests; B12c is the first
writer; B12d only tightens an action path that B12c made reachable; B12e
deletes code that had no caller before B12a and none after B12c.
Complete-but-unorderable is a failed spec; this ordering is the pinned
one.

## 8. Stopping rule

The teardown stops at these edges:

- The four provider crates' REMAINING surfaces (`ProviderOps` action
  methods, `create_provider`, the send / draft / folder-CRUD /
  attachment / MDN / prefetch call sites) are B15's, not B12's. B12
  deletes only the shared-mailbox and public-folder cluster the survey
  proved dead.
- The retired tables' schema rows are DELETED here rather than left
  additive-green, following the B8 precedent (which dropped four
  contact-map tables in a v100 edit) rather than the B3 precedent (which
  left cursor tables for B15), because B12 rewrites every reader of these
  tables in the same item. `jmap_sync_state.shared_account_id` is the one
  exception left to B15, since the table itself is B15's.
- No new user-facing IPC or UI is added. The sidebar, scope dropdown,
  thread list, reading pane, and navigation surfaces that exist today
  keep their shape and simply receive real data.

## 9. Out of scope, named not deferred

- Compose send-as from a shared mailbox.
  `rtsk::send_identity::select_from_address` exists and is tested but has
  never been wired into `crates/app/src/pop_out/compose/`; bifrost's
  `SendRequest::send_as` is live for Graph and JMAP. Wiring them is a
  compose-UI feature item (a `UI.md` change), not a plumbing rewire, and
  B12 is feature-preserving. Filed as its own item.
- A user-facing browse-and-pin affordance for public folders and a
  settings surface for the two new account flags. No such affordance
  exists today (the legacy `browse_public_folders` had no caller); the
  flags and the pin rows are set by direct row edit until a Settings item
  lands. The consequence must be stated plainly rather than glossed,
  because obstacle O makes pins a SYNC gate: on shipping B12 alone, a
  real user with `public_folders_enabled = 1` gets the hierarchy in the
  sidebar and NO public mail, because nothing pins a folder for them.
  That is the correct failure direction (the alternative is syncing an
  organization's millions of items unbidden), but it means B12 delivers
  public folders as a complete and gated BACKEND whose last mile is the
  browse-and-pin item. Shared mailboxes have no such gap - obstacle U's
  reconcile makes them selectable the moment they sync.
- Per-scope sync opt-out for an already-discovered SHARED mailbox. Needs
  an engine `set_scope_enabled` API (obstacle D); filed as a bifrost
  follow-up. Public folders do not wait on it (obstacle O moves their
  gate into discovery).
- Graph change notifications for shared mailboxes. Blocked on an
  application-auth mode (obstacle R), which is an OAuth and consent
  change, not a plumbing one. Shared Graph mail is poll-only until then.
- JMAP foreign mutations and foreign mailbox lifecycle polling
  (obstacle AA.2 and AA.3), both bifrost follow-ups. Until they land,
  JMAP shared mail is readable but not writable, and revocation is
  observed at the next reopen.
- `B8-groups` (Exchange distribution lists) is a separate open item and
  is untouched.
- Non-mail public folders (IPF.Appointment, IPF.Contact). bifrost
  surfaces no per-ITEM class, so B12 gates on the FOLDER's
  `Container.content_class`: such a folder is discovered and visible, its
  batches are dropped at the consumer (obstacle T), and it produces no
  mail rows. Routing their items into the calendar and contact stores is
  a follow-up gated on bifrost surfacing item class.

## 10. Lateral findings surfaced by the survey

Recorded, not silently carried:

1. `crates/db/src/db/queries_extra/scoped_queries.rs` is a stale
   duplicate of the live `db-read` copy (obstacle M). It has now bitten
   two consecutive items (B7c, B12). Deleting the duplicate is worth its
   own cleanup item.
2. bifrost's engine drops `InventoryEntry.memberships` on the backfill
   fan-out (section 2.5). Every provider pays to compute membership tags
   that no consumer can ever see. Either the fan-out should carry them as
   `ScopeChange` companions or the protocol crates should stop stamping
   them; a bifrost follow-up either way.
3. bifrost's `LiveSupersedes` cold-start de-dup set is still a no-op
   (nothing calls `.add()`), recorded in section 11 of the governing plan
   during B6 and still unfixed at this freeze.
4. `crates/graph/src/ops/mod.rs::{send_as_shared_mailbox,
   send_on_behalf_of}` are dead (B5 moved send to the engine) and ride
   out with the B15 `ProviderOps` sweep.
5. The `shared_mailbox_sync_state.sync_error` column was the only
   surface for "access revoked" in the sidebar. The replacement is
   `shared_mailboxes.revoked_at` plus bifrost's per-scope `ScopeRevoked`
   warning; the sidebar's greyed-out rendering for a revoked mailbox
   (which `docs/roadmap/jmap-sharing.md` specified and no code ever
   implemented) stays unimplemented, and is named in the roadmap
   reconciliation rather than quietly dropped.
6. `crates/service/src/actions/batch.rs` pre-resolves targets only for
   container moves and otherwise runs `op_local` first, so ANY future
   pre-dispatch refusal (not just a namespace one) degrades into
   `LocalOnly` after a durable local write. Obstacle S fixes the
   namespace case by adding a preflight; the general shape - "the only
   place a refusal can be raised safely is before `op_local`, and nothing
   enforces that" - is a pipeline smell worth its own cleanup so the next
   capability gate does not have to rediscover it.
7. `crates/sync/src/threading.rs::generate_thread_id` derives a
   PRIMARY-KEY component from a djb2 hash of a single client-supplied
   header. Beyond the namespace collision obstacle P fixes, djb2 over a
   32-bit space in a 150 GB-class mailbox is a birthday-collision risk on
   its own (roughly even odds around 80k distinct roots). Widening it is a
   migration, hence not B12's, but it should be an item.
8. `is_email_scope`'s doc comment
   (`crates/service/src/bifrost/consumer/mod.rs:900-907`) documents an
   invariant B12 falsifies. It is fixed here (obstacle T); the lateral
   lesson is that the comment encoded a provider assumption in a
   deliberately provider-agnostic predicate, and a debug assertion would
   have caught the drift that a comment did not.
9. `ContainerRights` is populated by exactly one provider
   (`research/bifrost/crates/types/src/container.rs:205-212`) despite
   three of the four parsing rights on the wire. B12-SQ item 6 closes it
   for IMAP and Graph; the pattern (a unified field that only one
   protocol crate fills) is worth a sweep across `Container`'s optional
   fields.

## 11. Review consolidation

Two independent reviews (`B12-R1-opus.md`, `B12-R2-codex.md`) were
validated finding by finding against the tree and against
`research/bifrost` / `research/saehrimnir` at the freeze. Everything they
raised that held up is folded above, at the obstacle or gate it belongs
to, not appended as a list. The mapping, so a reader of either report can
find where it landed:

- Public-folder hydration has no route -> obstacle N, B12-SQ item 4
  (raised by both reports; the strongest finding in either).
- Public folders would sync the whole hierarchy -> obstacle O, B12-SQ
  item 5, the `graph_public_folder_steady_state` gate and the
  unpinned-zero-fetch script (R2).
- Storage-identity collisions across namespaces -> obstacle P (R2).
- The action refusal lands after `op_local` -> obstacle S, section 4.7
  (R2).
- Graph shared-mailbox push needs application permissions -> obstacle R,
  which inverted obstacles H and I (R2).
- Unknown scopes fail open as personal -> obstacle Q (both).
- JMAP foreign accounts half-wired, gate too weak -> obstacle AA,
  B12-SQ item 7 (R2).
- Nothing populates the shared-mailbox registry for Graph or IMAP ->
  obstacle U (both).
- Non-mail public folders are not actually excluded -> obstacle T (both).
- Consumer goldens select the dropped column -> obstacle V (R1).
- `FolderKind` needs variants, not constructors; wrong crate in the gate;
  separator ambiguity; the `crates/core` name collision -> obstacle X
  (R1, with the separator point also in R2).
- The new thread index does not serve the personal predicate ->
  obstacle Y (R1).
- Registry helpers scheduled after the schema that requires them ->
  obstacle Z (R1).
- `NamespaceKind` in the wrong crate; `NamespaceAttribution` missing
  `Hash` -> section 4.3 (both).
- Graph and IMAP never populate `Container::rights` -> obstacle W (R1).
- `Container` is not `#[non_exhaustive]`, so B12a's additive claim rests
  on builder setters -> B12-SQ item 2 (R1).
- `is_email_scope`'s doc comment is falsified -> section 4.4 and lateral
  finding 8 (R1).
- `containers.rs` has a `tests.rs` submodule; the saehrimnir citation is
  line 3467; the memberships fan-out claim is better phrased as
  "constructs without reading `.memberships`";
  `load_label_group_unread_counts_for_shared_mailbox` was missing from
  the rename list; `send_identity.rs`'s `shared_mailbox_id` field ->
  sections 2.3, 2.5, 2.6, 4.6 (R1).

### Findings rejected, and why

1. R1 finding 2's PREMISE - "`routing_map` is seeded during discovery,
   *after* attach, so on first attach the map is empty and the first
   public-folder batch is persisted as personal mail". Rejected as a
   factual matter: `SyncEngine::attach` drives `discover_scopes`
   SYNCHRONOUSLY inside `attach_inner`, before it returns
   (`research/bifrost/crates/sync/src/engine.rs:244`), and ratatoskr
   deliberately calls `sync_containers` after `attach` for exactly this
   reason (`crates/service/src/bifrost/resident.rs:232-245`, the
   "Obstacle A'" comment). `routing_map` and `cursor_index` are populated
   by then, on the same `Arc<dyn Account>` `containers_list` resolves
   through. The finding's REMEDY was still folded (obstacle Q: never
   default `Personal`), and its ordering dependency is now documented
   there as a constraint B12-SQ must not break - but there is no
   first-attach corruption to fix.
2. R1 finding 1's fallback option - "or B12 must scope public folders to
   headers-only and say so". Rejected as a resolution: a public folder
   whose messages cannot be opened is not the feature this item claims,
   and the contract forbids shipping a shoehorned partial. Obstacle N
   takes the other branch (fix bifrost) and names splitting public
   folders out of B12 entirely as the fallback if the SQ proves too large.
3. R2's framing of finding 3 as grounds to "narrow its claim from
   delivering the feature to backend preparation only". Rejected for the
   JMAP hydration half, which is a one-selection bifrost fix (obstacle
   AA.1) and so is fixed rather than narrowed. Accepted for foreign
   mutations and lifecycle, which are genuinely bifrost work and are now
   named limitations with a gate that proves the refusal is honest.
4. R1's suggestion that unknown-scope handling could "have
   `containers_list` consume/trigger the cached `cursor_index`".
   Rejected: `cursor_index` is a cache of what discovery emitted, so
   consulting it would make the container projection depend on discovery
   ORDER rather than on the account's own enumeration. B12-SQ item 2
   projects from `shared_clients` and `routing_map` directly, which is
   the same data without the ordering coupling.
5. R2's "smaller defect" that `shared:{owner}:{owner_local_id}` needs
   "escaping or length encoding". Rejected in that form: escaping changes
   the folder-id alphabet the whole glossary Identity table rests on, for
   an ambiguity that a left-anchored `splitn(3, ':')` removes outright
   (owner ids never contain `:`). The parse rule and a colon-bearing IMAP
   path test are specified instead (obstacle X).
