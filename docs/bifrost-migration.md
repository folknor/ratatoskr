# Bifrost migration: governing plan

The strategic map for replacing ratatoskr's hand-rolled provider stack with
dependencies on the bifrost workspace. This document is the source-of-record
the spec-loop consumes: every work item below becomes one
technical-implementation-spec, run through the orchestrate.md seven steps. The
loop is running: Track B has begun (B1 has landed; see § 7).

`../bifrost` and `./research/bifrost` are at the same git commit. We keep both
because they serve two distinct purposes: `../bifrost` is the Cargo dependency
path the build resolves, and `./research/bifrost` is a reading-reference that
exists so agents can read bifrost source freely without tripping up the harness.
See § 11 for the full distinction.

## 1. Goal

Rip out ratatoskr's hand-rolled provider stack - the four provider crates
(`gmail`, `jmap`, `graph`, `imap`), the `provider-sync` crate, and the
`ProviderOps` / `ProviderSyncOps` / `ProviderError` / `ProviderState` /
`create_provider` surface in `common` - and replace it with dependencies on the
bifrost workspace, so ratatoskr speaks bifrost's unified `Account` /
`AccountError` / sync language natively.

This is a feature-preserving plumbing replacement. Every capability ratatoskr
ships today survives the migration. The goal is to delete ~33k LOC of
duplicated provider logic and inherit bifrost's unified, tested surface, not to
change what the client does.

**Maximal integration.** This is a total integration, not a provider-only swap.
The four provider crates and `jmap-client` are the largest target, but the rule
is general: wherever bifrost offers an equivalent for something ratatoskr
currently hand-rolls, or for an external dependency ratatoskr currently pulls
separately, that thing is replaced by bifrost. The end state is ratatoskr
depending on bifrost for everything bifrost covers, with no parallel
hand-rolled or duplicated dependency surviving alongside it.

## 2. First principle (this project's governing rule)

Bifrost exists to serve ratatoskr. The plan is written against an IDEAL
bifrost. Wherever bifrost's current shape is sub-optimal for ratatoskr, bifrost
is fixed FIRST, in the bifrost repo, before the corresponding ratatoskr work.

- ratatoskr is never contorted around a bifrost wart.
- Provider-reality differences (Gmail has no separate attachment-upload
  endpoint; IMAP has no native send; etc.) are absorbed by bifrost behind a
  uniform `Account` surface, or expressed as clean `AccountCapabilities` flags
  that ratatoskr reads declaratively. They never leak into ratatoskr as
  per-provider special-cases.
- A genuinely immutable provider limit (e.g. Gmail cannot update filters via
  API) becomes a capability flag the ratatoskr UI consults - not a code branch.

Track A below is the concrete definition of that ideal bifrost. It is a
specification of the target, not a list of bugs.

Bifrost's origin makes this concrete. Bifrost was started by ripping
ratatoskr's existing provider code - what now lives in the provider crates -
out of ratatoskr and unifying it. So if a capability looks mysteriously absent
from bifrost, that is not a design gap to engineer around; it means that part
was simply not carried over yet. The original code still in ratatoskr (the
current provider crates plus git history) is the reference for what bifrost
should already do.

**The side-quest protocol.** When any brick along the way surfaces that
ratatoskr would benefit from a rewrite or refactor of something in bifrost - a
missing capability, an awkward surface, a wart that would otherwise force a
ratatoskr workaround - that becomes a side-quest, never a ratatoskr
contortion. The orchestrator brings the tree to a clean boundary (landing what
is landable, reverting the blocked in-flight work so nothing parks dirty),
pauses the orchestration loop, and surfaces the brick to the user. The user
implements it in the bifrost repo and re-syncs the two folders (`../bifrost`
and `./research/bifrost` advanced together to a single new shared commit). Only
then does the loop resume, against the updated surface, with the new commit as
the frozen reference for the item.

## 3. Target architecture (the seam, post-migration)

Bifrost is a service-side dependency. The app stays bifrost-free: it depends on
`rtsk` (core) plus `service-api` wire types only. The service-to-app IPC wire
contract (`ActionWirePlan`, `OperationResult`, `SyncStatusEvent`) is the
firewall and stays stable; `AccountError` never crosses it.

Bifrost owns:

- protocol I/O and the unified `Account` surface (all four providers plus
  CalDAV/CardDAV, behind one trait);
- `AccountError` plus `RecoveryClass` (the structured recovery taxonomy);
- the sync engine: multiplexer, cursor state machine, adaptive polling, push,
  mutation pipeline, recovery dispatch;
- cursor state, as opaque versioned envelopes.

ratatoskr keeps:

- the main DB, body store, inline-image store, attachment file cache, and
  tantivy local search (storage and local search are app-level);
- the application sync layer: JWZ threading, AI bundling, filters, smart
  labels, notifications (`crates/sync`);
- account discovery (the five-stage pipeline) and OAuth authorization (browser
  redirect plus code exchange);
- the `MailActionIntent` action pipeline and the `service`-to-`app` wire
  contract;
- the entire app and UI.

Two seams connect them:

1. Sync seam. The bifrost `SyncEngine` emits a change stream; a new ratatoskr
   consumer persists items plus checkpoints and feeds the unchanged application
   sync layer at the existing `ProviderParsedMessage` / `SyncResult` boundary.
2. Action seam. The action pipeline drives bifrost `Account` mutations;
   `AccountError` maps down to the existing `OperationResult` taxonomy.

## 4. The four structural shifts

These define the character of the migration. They are why it is a rewrite, not
a dependency bump.

1. Persistence inversion. ratatoskr's protocol-sync layer writes the DB itself
   today. Bifrost-sync persists nothing: it emits a change stream and the
   consumer owns the DB write plus a `CheckpointStore` impl. Per-provider cursor
   tables (`jmap_sync_state`, `folder_sync_state`, `graph_*_delta_tokens`)
   retire in favor of opaque checkpoint envelopes.
2. Error-model upgrade. `ProviderError` (7 variants) plus a Transient/Permanent
   binary becomes `AccountError` to `RecoveryClass` (12 variants: Retry,
   Reconcile, Engine directives, AuthLost, NeedsAdminConsent, and more)
   internally, mapped down to `OperationResult` at the wire.
3. Object-level mutations. Thread-level named ops (`archive`, `star`,
   `add_label`) become object-level bulk mutations plus capability-dispatched
   conveniences (`set_starred`, `apply_label`, `move_thread`). Thread-to-message
   expansion happens consumer-side.
4. Capability dispatch. Per-provider special-casing (star as Gmail-label vs
   IMAP-keyword vs Graph-category) moves OUT of ratatoskr and INTO bifrost,
   behind `AccountCapabilities`. ratatoskr calls the convenience; bifrost picks
   the primitive.

## 5. Inventory

Deleted (~33k LOC): the four provider crates `gmail` / `jmap` / `graph` /
`imap` (~24.7k LOC, 51 files); the `provider-sync` crate (~8.5k LOC of
per-provider sync impls); `common`'s `ProviderOps` / `ProviderSyncOps` /
`ProviderError` / `ProviderState` / `create_provider`; the external
`jmap-client` dependency.

Survives untouched: the application sync layer (threading, bundling, filters,
notifications, smart labels in `crates/sync`); discovery and OAuth
authorization in `core`; the DB, stores, and tantivy search; the app; the
service-to-app wire contract.

Rewired: provider construction in `core` / `service`; the action pipeline
bottom; the `calendar` crate; contacts sync; attachments; server-side search;
server-side filters; identities/settings; shared-mailbox and public-folder
scoping.

## 6. Track A: making bifrost ideal (bifrost repo, lands first)

Each item is a bifrost-repo spec, filed as a bifrost TODO and run through
bifrost's own loop. Verify each against current bifrost before speccing; some
may be partially built already. A1 is the universal first domino.

- A1. Uniform token rotation. Every factory accepts an
  `Arc<dyn TokenSource>` (or every factory exposes `set_access_token`), so
  ratatoskr drives refresh plus DB write-back through one generic path. Today
  only JMAP and Graph expose rotation; Gmail, IMAP, CalDAV, CardDAV, SMTP do
  not. This blocks all of Track B.
- A2. IMAP send. `Account::send_message` / `draft_send` work for IMAP via an
  SMTP backend wired inside the IMAP account, so ratatoskr sees one uniform
  send surface with no IMAP-plus-SMTP composition leaking up.
- A3. Raw RFC822 hydration. A projection that yields real assembled MIME bytes
  uniformly across all providers (JMAP, Google, Graph included), for the body
  store, attachment dedup, and forwarding. Today raw-MIME assembly is deferred
  on the HTTP providers.
- A4. Scheduled send. A first-class `Account` capability across the providers
  that support it (Gmail, Graph, JMAP), uniform surface plus a capability flag
  where unsupported.
- A5. Shared mailboxes plus public folders. First-class bifrost scopes: Graph
  (EWS / Autodiscover for shared and public folders) and IMAP (NAMESPACE /
  ACL), surfaced through `discover_cursor_scopes` / `discover_memberships` and
  the `Account` surface, fully sync-integrated (not CRUD-only). This is the
  largest Track A unknown; size it after verifying bifrost's current story.
- A6. Cloud-storage attachments. A bifrost surface for large-attachment hosting
  plus share-link generation (Google Drive, Microsoft OneDrive), uniform across
  providers that support it, capability-flagged.
- A7. DAV as first-class synced accounts. CalDAV / CardDAV emit cursor scopes
  and sync-integrate (closing the composed-account dead-end), and compose into
  any account, not just IMAP.
- A8. Provider-wart absorption sweep. Anywhere a wart would otherwise force a
  ratatoskr special-case, bifrost absorbs it behind the uniform surface or
  expresses it as a clean capability flag: Graph `remove_from_container`,
  blob-range uniformity, Gmail attachment inlining behind `attachment_upload`,
  and the rest. Immutable provider limits become capability flags, never
  consumer branches.

## 7. Track B: the ratatoskr rip (this repo, against the ideal surface)

Written purely against the ideal bifrost from Track A. No adapters around
warts, no per-provider branches.

B1 (dependency wiring plus construction plumbing) is done and its TODO entry is
removed per repo convention; the items below that name it as a prerequisite
("Needs B1") have that dependency satisfied. For what B1 delivered - the bifrost
path deps wired into `service`, the `service`-side `build_account_factory` and
generic `DbWriteBackTokenSource` (the construction module at
`crates/service/src/bifrost/`), and the `AccountError`-to-`OperationResult`
mapping - read the B1 landing commit. It is additive: nothing live routes through
it yet (that is B3/B4) and no legacy provider surface was removed.

One load-bearing rule B1 fixed binds every later item: bifrost must not become a
dependency of `core` (`rtsk`). The app depends on `rtsk` (`crates/app/Cargo.toml`),
so any bifrost type that lands in `core` is pulled into the UI build - directly
contradicting § 3's "the app stays bifrost-free; it depends on `rtsk` plus
`service-api` wire types only." Bifrost is confined to `service` and other
writer-side crates; only ratatoskr-owned DTOs and the `service`-to-`app` wire
types cross the core/UI boundary.

B2 (CheckpointStore plus cursor schema) is done and its TODO entry is removed per
repo convention; the items below that name it as a prerequisite ("Needs B1-B2",
"B1 to B2 to B3") have that dependency satisfied. It landed additively: a new
opaque `sync_cursors` table in `crates/db/src/db/schema/10_sync.sql`, the
service-side `SqliteCheckpointStore` over bifrost's own `encode_envelope` /
`decode_envelope` codec (`crates/service/src/bifrost/checkpoint_store.rs`), and a
table-by-table disposition decision for every protocol/sync-state table. The
store and table sit dormant - written by no one, read by no one - until B3 wires
the engine to them; no legacy cursor table or writer was removed. The disposition
enumeration is the reconciliation source for the later deletion cuts: it pins,
for each of `folder_sync_state`, `jmap_sync_state`, `graph_folder_delta_tokens`,
`graph_contact_delta_tokens`, `graph_shared_mailbox_delta_tokens`,
`public_folder_sync_state`, `jmap_push_state`, `graph_subscriptions`,
`shared_mailbox_sync_state`, `public_folder_content_routing`, `pending_operations`,
and `clean_shutdown_cursors`, whether it retires into the opaque envelope (B3/B8/
B12), moves to bifrost ownership (B3b), or is retained app-side - so none is left
orphaned when its writer is dropped. Two pinned open questions ride into the
cutover: the JMAP `shared_account_id` dimension does not fold cleanly into any
`CursorScope` variant (a B3/B12 concern gated on bifrost growing a shared-mailbox
scope), and `jmap_push_state` / `graph_subscriptions` are subscription state, not
checkpoint cursors, so they move to bifrost's `SubscriptionRegistry` at B3b. For
the full disposition table, the brick-by-brick gates, and the design rationale,
read the B2 landing commit.

B3a-infra (the engine harness plus the provider-agnostic durability framework)
is done and its TODO entry is removed per repo convention; the B3a-cut-* / B3b /
B3c sub-items below that name it as a prerequisite ("Needs B3a-infra", "Needs
B3a") have that dependency satisfied. For what B3a-infra delivered - the
`service`-owned `BifrostSyncEngine` (the first live wiring of B2's
`SqliteCheckpointStore` into `SyncEngineBuilder::checkpoints`), the
change-stream-to-DB consumer module (`crates/service/src/bifrost/consumer/`) with
REAL writes to all four stores (main DB, body, inline-image, search), the
`RecvError::Lagged` detach/re-attach recovery, search `flush_now`-before-ack,
ack-last ordering, the single-txn replay-safety marker keyed `(account_id, scope,
checkpoint)`, the per-item Succeeded / Failed / Uncertain hydration taxonomy, the
baseline provider-agnostic membership write, the per-provider post-persist HOOK
SEAM with only the shared `seen_ingest` arm filled, the one-shot
attach/drive/detach driver with completion synthesis from
`backfill_registry().snapshot()` plus a fixed-2s idle cadence, the
`provider_sync::consumer_support` facade that makes the existing `provider-sync`
helpers reachable without relocating them, the test-only attach path
(`TestBifrost*` requests driven by synthetic injected batches), and the new
`bifrost-consumer-*` durability + hot-path harness instruments - read the
B3a-infra landing commit. It landed ADDITIVELY: it cuts no provider over, rewires
no production sync, deletes nothing - legacy `provider-sync` stays live and
authoritative for all four providers, and the consumer was reached only through
the test-only attach path until B3a-cut-jmap (below) routed JMAP onto it.
The HARD ordering constraint the remaining cut specs inherit: no B3a-cut-* may
splice the consumer's lag-recovery driver into production `run_sync` before B3c
lands its backoff / pause-and-surface recovery (the recovery is
correct-but-unbounded and can livelock a structurally-slow consumer), unless that
cut carries its own gated bounded-retry stopgap (B3a-cut-jmap carried one - the
minimal bounded lag-backoff in `engine_sync.rs` - so the remaining cuts can reuse
the same stopgap shape).

B3a-cut-jmap (the first per-provider cutover) is done and its TODO entry is
removed per repo convention; the per-provider cutovers that name it as
establishing the per-provider mechanics (B3a-cut-graph and B3a-cut-gmail, now
also done below, and the remaining B3a-cut-imap sub-item) have those mechanics
available to reuse. It landed as ONE coexistence cutover: JMAP accounts now sync
through `BifrostSyncEngine` + `ChangeStreamConsumer`, while Gmail, Graph, and IMAP
stay on legacy `provider-sync`. For what it delivered - the coexistence dispatch
in `run_sync` (`crates/service/src/sync.rs`) that routes JMAP accounts to the new
engine path and every other provider to the unchanged legacy
`sync_dispatch::sync_for_account`; the service-owned one-shot JMAP runner
(`crates/service/src/bifrost/engine_sync.rs`, with the minimal bounded lag-backoff
the B3a-infra HARD constraint requires); the filled JMAP arms of the consumer's
hydrate / write / post-persist hooks (real engine-driven JMAP hydration over the
`dc670ef` `SyncEngine` passthrough, the folders-only-recompute membership strategy
with keyword labels, the JMAP deletion path, thread participants + chat-state, and
the `is_important` aggregate); the relocation of the four entangled JMAP auxiliary
passes (shared-account discovery, identity resolution, contacts sync,
ShareNotification polling) into `crates/provider-sync/src/jmap/aux_sync.rs` and the
runner branch, not dropped; and the DELETION of the legacy JMAP `provider-sync`
sync impl (`crates/provider-sync/src/jmap/sync/`, `shared_mailbox_sync.rs`, and
`jmap_impl.rs`) - read the B3a-cut-jmap landing commit. The `jmap_sync_state`
`"Email"`/`"Mailbox"` change-cursor writer retires (the engine owns that cursor
via the opaque `sync_cursors` envelope); the `"ShareNotification"` writer survives
because its call site was re-homed; the table schema stays (additive-green) until
B15. The JMAP `ProviderSyncOps::{sync_initial, sync_delta}` arm is gone, but the
JMAP `ProviderOps` action methods survive (B4/B15). The coexistence dispatch is
in-tree provider-kind routing, removed by the final per-provider cut
(B3a-cut-imap). Gated against the bifrost freeze `ae73e92` (§ 11): `brokkr check`
green, `brokkr service-suite` 63/63, and every spec § 6 gate passes -
`jmap-initial`, `jmap-bulk-initial` (10001 msgs), `jmap-steady-state-delta`,
`jmap-incremental-steps`, `jmap-email-set-delta`, `jmap-contacts-initial`,
`jmap-production-lag-backoff`, `jmap-multi-account-{primary,secondary}-isolation`,
the `golden` membership-equality + `hydrate` service tests,
`bifrost-consumer-lag-recovery`, and `parent_sigkill`. The two
multi-account-isolation gates initially read as a redundant wire `Email/get`
regression, but the root cause was the harness matcher: it counted bifrost's empty
open-time `Email/get(ids=[])` `Account::open` probe (which hydrates nothing) as a
fetch. saehrimnir's per-`accountId` state was correct; the fix was a
`count_email_hydrations` helper that counts only `Email/get` calls carrying a
non-empty `ids[]`, preserving the strict "delta fetches zero emails after a
foreign-account mutation" intent while ignoring the no-op probe.

B3a-cut-graph (the second per-provider cutover) is done and its TODO entry is
removed per repo convention; the remaining B3a-cut-imap sub-item reuses
the mechanics it shares with B3a-cut-jmap and B3a-cut-gmail. It landed as ONE coexistence cutover:
Graph (Microsoft) accounts now sync through `BifrostSyncEngine` +
`ChangeStreamConsumer`, while Gmail and IMAP stay on legacy `provider-sync`. For
what it delivered - the `"graph"` arm of the `run_sync` coexistence dispatch
(`crates/service/src/sync.rs`); the service-owned one-shot Graph runner
`sync_graph_account` (`crates/service/src/bifrost/engine_sync.rs`), a near-clone
of the JMAP runner with the same bounded lag-backoff and a single connected
`GraphClient` (`aux_client`) built per kick and shared by the folder-map prepare
and the post-drive aux passes; the filled Graph arms of the consumer's
hydrate / write / post-persist hooks. The consumer machinery was generalized,
not forked: the `jmap_folder_map` field/setter became the provider-agnostic
`folder_map` / `with_folder_map`, and `is_email_scope` now also matches Graph's
per-folder `CursorScope::FolderType { ty: Email, .. }` (JMAP emits
account/type-level email scopes; Graph emits one cursor per folder). The Graph
hydration arm resolves opaque `parentFolderId` containers through the folder map,
translates bifrost's `category:<name>` flags into `cat:` `graph_category` labels
(kept OUT of the keyword set), normalizes bifrost's backslash `\seen` / `\flagged`
canonical flags into the `$`-forms the consumer reads, hydrates with
`HydrationProjection::FullWithBlobs` (Graph omits blob handles under plain `Full`,
which would drop every attachment), and surfaces importance as both the
`importance:*` undeletable labels and the `threads.is_important` aggregate - the
last enabled by a bifrost side-quest (`importance` added to Graph's typed
`hydrate_select`) that advanced the freeze to `7c576bdd` (§ 11). Graph hard-deletes
arrive as a per-folder `ScopeChange{Removed}` (Graph delta has no
created/updated/destroyed split), so a drive-level reconcile accumulates
`removed - live` ids across every per-folder scope batch and deletes only the
remainder at drive end - a move (source-folder `Removed` + destination-folder
`Updated`) survives because the destination's full-replace membership already
corrects it. The five entangled auxiliary passes were relocated into
`crates/provider-sync/src/graph/aux_sync.rs` (folder-map prepare + `importance:*`
seed, master-category labels, contacts, Exchange groups, and a reaction
seeder+refresh poll - the legacy refresher only re-polled messages that already
had reaction rows, so the poll's selection was broadened to also seed
recently-received messages), driven on the existing per-account `graph_sync_cycle`
counter (`increment_graph_sync_cycle`, repurposed from the personal path's
priority-tier scheduling to the aux cadence: master categories + Exchange groups
every 20th cycle, reactions + contacts-delta every 5th - contacts dropped from the
legacy 20th to the 5th so the `graph-contacts-incremental` gate reaches a delta
within its 120s ceiling under the one-shot per-kick connect cost, to be restored
once B3b amortizes it), not collapsed to every-kick (which would inflate the
steady-state Graph request count). The legacy
Graph `provider-sync` sync impl `graph_impl.rs` (the `ProviderSyncOps` orphan) is
DELETED and Graph removed from the `sync_dispatch` / `create_provider` provider-kind
dispatch; the personal Graph path stops writing `graph_folder_delta_tokens` (the
engine owns each `FolderType` cursor via the opaque `sync_cursors` envelope) while
the retained shared-mailbox leg still writes it until B12, and `graph_sync_cycle`
is repurposed rather than dropped. The retired-table schema stays additive-green
until B15. The `graph/sync/` tree is
RETAINED (not deleted) as a deliberate documented deviation: its only remaining
consumer is `graph/shared_mailbox_sync.rs` (B12), which calls the FULL
`graph_{initial,delta}_sync`, so re-homing a "minimal shim" would relocate ~the
whole tree for ~0 net LOC and high risk - B12 deletes/rewires it. The Graph
`ProviderOps` action methods survive (B4/B15). Gated by `brokkr check` green,
`brokkr service-suite`, the `graph-initial` / `graph-steady-state-delta` sync-bench
(the new `graph_steady_state_delta` gate, `meta.provider_requests` pinned
`max_delta = 0`), the `graph_consumer_membership_equals_legacy` membership golden,
the `hydrate_change_graph_category_and_importance_mapping` unit test, the
`graph_drive_reconciles_move/purge` in-process move-vs-purge tests, and the
existing `graph-attachment-*` / `graph-master-category-label-sync` /
`graph-contacts-*` scripts held green across the cut - read the B3a-cut-graph
landing commit.

B3a-cut-gmail (the third per-provider cutover) is done and its TODO entry is
removed per repo convention; the remaining B3a-cut-imap sub-item reuses the
mechanics it shares with B3a-cut-jmap and B3a-cut-graph. It landed as ONE
coexistence cutover: Gmail (Google) accounts now sync through `BifrostSyncEngine`
+ `ChangeStreamConsumer`, while only IMAP stays on legacy `provider-sync`. For
what it delivered - the `run_sync` coexistence-dispatch arm
(`crates/service/src/sync.rs`) that routes Gmail to the engine path, matching the
canonical provider string `gmail_api` (a deliberate sound deviation from the
literal `gmail`: `gmail_api` is the string the account row actually carries); the
service-owned one-shot Gmail runner `sync_gmail_account`
(`crates/service/src/bifrost/engine_sync.rs`), a near-clone of the JMAP/Graph
runners with the same bounded lag-backoff and a single connected legacy
`GmailClient` (`aux_client`) built per kick and shared by the label-folder-map
prepare and the post-drive aux passes; the bifrost factory Gmail arm
(`crates/service/src/bifrost/factory.rs`) honoring `RATATOSKR_TEST_GMAIL_ENDPOINT`
via `GoogleAccountFactory::from_token_source_with_api_base` (the harness redirect
added by the frozen-commit side-quest, mirroring the Graph arm); and the filled
Gmail arms of the consumer's hydrate / write hooks. The Gmail write arm
(`crates/service/src/bifrost/consumer/write.rs`) computes full-thread membership
by the per-message-rows-plus-recompute resolution - per-message `message_folders`
/ `message_labels` rows written through `replace_message_membership_and_recompute`
-> `recompute_thread_folders_from_messages`, which reads ALL of a thread's
persisted message rows on every recompute, so a partial delta batch is correct by
construction (Gmail joins Graph on this helper rather than its legacy
`replace_thread_membership_from_full_coverage`, the lowest-risk of the three
coverage options weighed at spec time). The hydrate arm
(`crates/service/src/bifrost/consumer/hydrate.rs`) maps Gmail labels onto folders
+ labels through the prepared folder map; routes label-only `ScopeChange` rows
(Gmail `labelsAdded`/`labelsRemoved` on an existing message arrive as
`ScopeChange`-only with no `ObjectChange`) through a Gmail `ScopeChange`
re-hydration that updates membership rather than acking the change into oblivion;
synthesizes an `archive` folder membership when `INBOX` is removed and no other
system container (`SENT`/`DRAFT`/`TRASH`/`SPAM`/`archive`) remains (a deliberate
sound deviation, harness-gated by `gmail-incremental-steps`); reproduces reaction
insertion (`insert_gmail_reaction`, resolving the target via `In-Reply-To` ->
`message_id_header`); and treats `STARRED`/`UNREAD` as message STATE
(`threads.is_starred` / `messages.is_read`), NOT `message_labels` rows, while
`IMPORTANT` rides the label/flag surface into the `threads.is_important`
aggregate. The three entangled auxiliary passes were relocated into
`crates/provider-sync/src/gmail/aux_sync.rs` (the label folder-map prepare
`sync_gmail_label_folder_map`; the BIDIRECTIONAL `sendAs` signature sync
`sync_gmail_signatures`, run every kick and made non-fatal-on-error -
log-and-continue rather than legacy-fatal, a deliberate sound deviation
consistent with the aux-pass non-fatal framing; and Google contacts +
other-contacts at the legacy once-on-initial / every-20th-delta cadence driven by
`increment_gmail_sync_cycle`), not dropped. The legacy Gmail `provider-sync` sync
impl is DELETED: `gmail_impl.rs` (the `ProviderSyncOps` orphan) and the entire
`gmail/sync/` subtree (`mod.rs`, `delta.rs`, `storage.rs`, `labels.rs`), with
Gmail removed from the `sync_dispatch` / `create_sync_provider` provider-kind
dispatch; the Gmail `ProviderOps` action methods (`GmailOps`) survive (B4/B15).
The `accounts.history_id` change-cursor WRITER retires (the engine owns the
history-id cursor inside the opaque `sync_cursors` envelope); the column stays
additive-green until B15. Accepted coverage gap, recorded not deferred-as-a-hole:
the multi-message-thread partial-delta sibling scenario (a label change on ONE
message of a multi-message thread, asserting the OTHER messages' membership
survives end-to-end) is NOT integration-gated. The per-message-recompute
resolution makes the partial-batch case correct by construction (the recompute
reads every persisted thread message row), and the
`gmail_consumer_membership_equals_legacy` golden covers the multi-message union
invariant; exercising the partial delta end-to-end would need external
`saehrimnir` multi-message-thread grouping plus a single-message history-delta
emitter plus a new Gmail fixture, deferred as a separate follow-up. Gated against
the bifrost freeze `002e7b9` (full `002e7b9f1b7cfe218b491520f4e1ea7efc7f7997`,
§ 11), advanced from `7c576bdd` for the Gmail mock-redirect side-quest
(`GoogleAccountFactory::from_token_source_with_api_base` /
`from_access_token_with_api_base`): `brokkr check` green, and every Gmail gate
green - `gmail-initial`, `gmail-incremental-steps`, `gmail-production-lag-backoff`,
`gmail-attachment-initial`, `gmail-attachment-prefetch`, `gmail-oauth-multi-account`,
`gmail-send-as-multi-account-import`, the `gmail_consumer_membership_equals_legacy`
membership golden, the Gmail hydrate unit test, and `parent_sigkill`;
`gmail-steady-state-delta` is behaviorally correct with its host-sensitive
`provider_requests` baseline pinned at land. This cut also required `saehrimnir`
Gmail mock extensions (a message-list + per-message get including `format=raw`, an
incremental `history.list` projection, and an oauth refresh grant that resolves
the account from the presented refresh token) - an installed external binary, not
commit-pinned here. Read the B3a-cut-gmail landing commit.

B3a-cut-imap (the fourth and FINAL per-provider cutover) is done and its TODO entry
is removed per repo convention. With it the per-provider cutover series
(B3a-cut-jmap, B3a-cut-graph, B3a-cut-gmail, B3a-cut-imap) is COMPLETE: every
personal-account provider's mail sync now runs through `BifrostSyncEngine` +
`ChangeStreamConsumer`, and only B3b (push) and B3c (control / pause / recovery)
remain open in B3. It landed as the final, fully intrusive cutover and removed the
coexistence scaffolding WHOLESALE. IMAP personal-account mail sync now routes
through the engine + consumer; the legacy IMAP `provider-sync` sync impl is DELETED
- `imap_impl.rs` (the `ProviderSyncOps` orphan) and the entire `imap/` sync subtree
(`imap_initial.rs`, `imap_delta.rs`, `imap_delta_janitor.rs`, `sync_pipeline.rs`,
`thread_store.rs`) - AND so are `sync_dispatch.rs`, the `ProviderSyncOps` trait +
`SyncProviderCtx`, and `create_sync_provider` (the sync-only factory). Because IMAP
was the last legacy provider the `run_sync` coexistence dispatch is removed ENTIRELY
- no provider routes to a legacy path now, so the former `Ok(_)` legacy
fall-through becomes an explicit unsupported-provider error. `create_provider` (the
SEPARATE action-ops factory, `actions/provider.rs`) SURVIVES: it backs ~20 action /
prefetch / attachment call sites, and the IMAP `ProviderOps` action methods
(`imap::ops::ImapOps`) live in the `imap` crate, untouched until B4/B15. The
IMAP-specific engineering this cut required, none of it shared with the HTTP cuts:
drive-end JWZ threading (`threading::build_threads` run ONCE per drive over a
post-adoption-id `ThreadableMessage` accumulator, at the legacy per-cycle boundary,
so cross-batch subject-merge stays byte-identical to legacy output) with the IMAP
change-cursor ack DEFERRED to drive end for crash-safety - a crash after per-batch
persist but before drive-end threading re-drives the un-threaded messages on the
next drive rather than stranding them on provisional thread ids past an advanced
cursor, gated by a new `crash_before_drive_end_threading` consumer hook + the
`bifrost-consumer-imap-crash-before-drive-end-threading.lua` script; identity
adoption by `(account_id, imap_folder, imap_uid)` before insert so existing
legacy-synced rows are reused, not duplicated; the bifrost IMAP CONDSTORE/QRESYNC
delta surfaced as `ScopeChange{Added|Removed}` + `ObjectChange{Updated}` with
`CursorScope::Folder` routing (`is_email_scope` extended to accept it);
`HydrationProjection::Full` for IMAP (`FullWithBlobs` is redundant - bifrost IMAP
returns empty blob handles, so the re-parsed RFC822 MIME tree is the SOLE source of
attachment + inline-image rows); and the relocated IMAP aux passes in
`crates/provider-sync/src/imap/aux_sync.rs` (folder-map prepare + a per-mailbox
`PERMANENTFLAGS` keyword-capability probe, run every kick because `PERMANENTFLAGS`
is per-mailbox and a new mailbox can appear at any time). The cross-provider
`is_important` follow-up that B3a-cut-gmail filed under this item is RESOLVED here,
not carried forward: `is_important` is recomputed as a sticky-OR of the batch
against the persisted `threads.is_important`, so a delta carrying a non-important
sibling can no longer clear a previously-important thread (applied to the JMAP /
Graph / Gmail write arms; IMAP has no importance). A deliberate, validated
DEVIATION rode along: `SyncEvent::Done(checkpoint)` persistence was GENERALIZED to
all providers (the consumer previously ignored `Done`), closing the empty-delta
cursor-durability gap for every provider - the JMAP, Gmail, and Graph steady-state
sync-bench gates were re-run and all held at delta = 0 (no request-count
regression). This cut needed NO bifrost change: the frozen `../bifrost` stays at
`002e7b9` (§ 11; bifrost's IMAP CONDSTORE/QRESYNC behavior was already correct). The
mock work was entirely in `saehrimnir`, whose IMAP mock gained real CONDSTORE/QRESYNC
support (parse `SELECT (CONDSTORE)`, return `HIGHESTMODSEQ`, honor
`FETCH ... CHANGEDSINCE`, emit QRESYNC `VANISHED`) - an installed external binary,
not commit-pinned here, recorded like the sibling cuts' mock extensions. Gated by
`brokkr check` green; the `imap_consumer_membership_equals_legacy` and
`imap_drive_threading_equals_legacy` goldens; the hydrate, identity-adoption, and
threading-reassign unit tests; all ten `imap-*` sync-harness scripts; the three
`bifrost-consumer-*` durability scripts including the new IMAP deferred-ack crash
gate; `parent_sigkill`; `brokkr service-suite` 63/63; and the `imap_steady_state_delta`
sync-bench (baseline pinned on the clean tree at land). Read the B3a-cut-imap landing
commit.

- B3. The bifrost-sync consumer (center of gravity), carved into per-provider
  cutovers so no single landing carries the whole rip. The `SyncEngine` and the
  change-stream-to-DB consumer that persists each batch (messages, body store,
  inline images, search), flushes search, then acks the bifrost checkpoint LAST
  are stood up by B3a-infra (done; see the done-note above), and JMAP, Graph,
  Gmail, and IMAP are ALL routed onto it by B3a-cut-jmap, B3a-cut-graph,
  B3a-cut-gmail, and B3a-cut-imap (all done; see the done-notes above). The
  per-provider cutover series is therefore COMPLETE: every personal-account
  provider syncs through the consumer, and each provider's ACTUAL post-persist
  processing - asymmetric, not a uniform pipeline - is now replicated in it: IMAP
  runs JWZ `threading::build_threads` at drive end;
  JMAP / Graph / Gmail write the thread aggregate inline (`upsert_thread_aggregate`)
  with per-provider membership strategies;
  `seen_ingest` is the one shared pass;
  bundling, filters, smart labels, and `evaluate_notifications` have NO sync-time
  callers today and stay unwired (whether they should auto-fire on new mail is a
  separate product item, explicitly not B3's scope - feature-preserving means the
  consumer reproduces today's behavior, not that it inherits an unwired gap as a
  feature). What remains in B3 is B3b and B3c. The sub-items:
  - B3b. Push plus invalidation. Wire the out-of-process push ingress (Gmail
    Pub/Sub, Graph webhooks) and the invalidation sink; move `jmap_push_state` /
    `graph_subscriptions` to bifrost's `SubscriptionRegistry`. Needs B3a.
    Follow-up surfaced by B3a-cut-jmap: the one-shot JMAP runner opens TWO JMAP
    connections per kick (the shared legacy `aux_client` for the auxiliary passes
    plus the bifrost engine attach), and the consumer's `COMPLETION_IDLE_INTERVAL`
    idle cadence costs ~2s/kick - both are artefacts of the attach/drive/detach
    one-shot shape. B3b's keep-attached lifecycle amortizes them back toward the
    legacy steady-state (~4 provider requests, ~22ms) by holding the engine open
    across kicks instead of re-attaching per sync.
    Follow-up surfaced by B3a-cut-gmail: the service account factory
    (`crates/service/src/bifrost/factory.rs`) wraps each account's
    `DbWriteBackTokenSource` in bifrost's `OAuthRefresher`, which starts `Empty`
    and so forces a token-endpoint refresh on the FIRST token read of every sync
    kick, discarding a still-valid cached token - a needless token round-trip per
    one-shot kick for every provider (surfaced by this cut's
    `gmail-oauth-multi-account` analysis). The fix is either a bifrost
    `OAuthRefresher` change to accept an initial token, or a ratatoskr factory
    change to hand the engine the `DbWriteBackTokenSource` directly; B3b's
    keep-attached lifecycle also cuts its frequency by holding the engine open
    across kicks.
  - B3c. Control, pause, recovery. The keep-attached lifecycle for steady-state /
    push, pause/resume, and `RecoveryClass` dispatch. Needs B3a, B3b.
  The worked-out design - the per-provider seam survey, durability ordering, and
  the lag / hydration / completion policies each sub-spec is carved from - was
  produced during B3 spec review and is consumed by each sub-spec's author.
- B4. Action pipeline rewire. Dispatch onto `Account` conveniences plus bulk
  mutations over `MutationTarget`; thread-to-message expansion; map
  `AccountError` to `OperationResult`; rebuild the pending-ops / retry journal
  on `RecoveryClass`. Likely splits into B4a (mutation dispatch) and B4b (error
  plus retry). Needs B1, A1-A2.
- B5. Send plus drafts. Rewire onto `send_message` / `draft_*` plus scheduled
  send. Needs A2, A4.
- B6. Folders, labels, containers. Rewire onto `container_*` / `apply_label`
  plus folder/label sync. Needs B3.
- B7. Calendar. Replace the `calendar` crate's per-provider sync (the largest
  single auxiliary; Graph alone is ~41k LOC today) with the bifrost calendar
  surface. Needs B1; A7 for DAV.
- B8. Contacts. Replace Google People, Graph contacts, JMAP contacts, and
  Google other-contacts sync with the bifrost contact surface. Needs B1; A7 for
  DAV.
- B9. Attachments plus cloud attachments. Rewire `fetch_attachment` onto blobs
  and the cloud-storage surface. Needs B1, A6.
- B10. Search. Drive bifrost server-side search/filters where used; the local
  tantivy search stays app-level. Needs B1.
- B11. Server-side filters / Sieve. Rewire onto `filter_*`. Needs B1.
- B12. Shared mailboxes plus public folders. Rewire
  `ViewScope::SharedMailbox` / `PublicFolder` onto bifrost scopes. Needs A5.
- B13. Identities, signatures, vacation, quota. Rewire onto the bifrost
  settings surface. Needs B1.
- B14. Account construction, discovery, verify. Use `AccountFactory::open` as
  the connection test; keep the five-stage discovery and OAuth authorization.
  Needs B1.
- B15. Deletion and collapse. Remove the four provider crates, `provider-sync`,
  `common`'s provider surface, the external `jmap-client` dep, and the
  workspace members; remove any transitional scaffolding. The final green cut.
  Needs all above.

  This explicit list is a floor, not the full scope. The § 1 maximal-integration
  rule (no parallel hand-rolled or duplicated dependency surviving alongside a
  bifrost equivalent) is stronger than this enumeration, so B15 must run a
  mechanical dependency-and-module audit of the whole workspace - every crate's
  `Cargo.toml` plus its module tree - and delete every bifrost-covered equivalent
  it finds, not only the named targets. Known instance to confirm in that audit:
  `crates/service/Cargo.toml` still carries
  `bifrost-jmap = { path = "/home/folk/Programs/jmap-client/crates/jmap" }` (the
  out-of-tree `jmap-client` checkout, used by the JMAP-specific contact action
  handlers). That dependency, and any others the audit surfaces, retire here -
  subject to the § 9 caveat that retiring the external `jmap-client` must not
  strand bifrost-jmap (confirm bifrost-jmap's own internal JMAP dependency first).
- B16. Reference-doc reconciliation. Update `reference/architecture.md`,
  `AGENTS.md`, and the crate map. Bundled with B15 per repo convention (never a
  standalone markdown commit).

Estimated scope: ~8 bifrost specs plus ~16-20 ratatoskr specs.

## 8. Sequencing and green-tree strategy

- Track A lands first. Each A-item lands as it unblocks its B dependents; A1
  is the literal first task.
- Within Track B, two trunk lines: B1 to B2 to B3 (sync), and B1 to B4
  (actions). B5 through B14 are branches off them. B15 closes.
- Each spec is one coherent, fully intrusive landing, kept or reverted on its
  gate results. Where a subsystem cannot cut over atomically across all four
  providers in one commit, the spec author may stage via a short-lived
  ratatoskr-internal adapter deleted within that same spec's final landing -
  never a runtime or env switch, never a probe left in the tree. Per the spec
  methodology, complete-but-unorderable is a failed spec; the exact green-tree
  ordering is pinned per spec, not here.

## 9. Risks

- The sync inversion (B3) is the highest-risk landing: persistence ownership
  moves, the cursor schema changes, and the application sync layer's input
  contract must be preserved exactly. Mitigate by freezing the
  `ProviderParsedMessage` / `SyncResult` contract as the seam and validating
  that threading and bundling outputs are unchanged across the cut. "Validate"
  here means named behavioral gates, not a compile check: B3 (and every other
  sync-touching spec) must pin explicit `brokkr service-test` / sync-harness runs
  plus the relevant `brokkr sync-bench` so a compile-only replacement cannot pass
  the gate. See § 10 for the workspace-wide gate requirement this is an instance
  of.
- Calendar (B7) is the largest single rewire and depends on bifrost calendar
  maturity (high on native providers; A7 for DAV).
- Shared mailboxes and public folders (A5 / B12) is the largest bifrost-side
  unknown; verify bifrost's current support before sizing.
- Token rotation (A1) gates everything; it is the first task in the whole
  project.
- Retiring the external `jmap-client` dependency must not strand bifrost-jmap;
  confirm bifrost-jmap's own internal JMAP dependency first.

## 10. Methodology

This document is the source TODO the spec-loop consumes. Each B-item, and each
A-item in the bifrost repo, becomes one technical-implementation-spec, run
through the orchestrate.md seven steps. Items are processed serially, the tree
green at every boundary, nothing deferred.

Behavioral gates are mandatory, not optional. Because this migration swaps the
entire provider stack underneath unchanged application behavior, a green
`brokkr check` is necessary but not sufficient - it proves the new code
compiles and passes unit tests, not that real provider sync still behaves. Every
spec that touches sync, actions, calendar, or contacts must pin, in its gate
section, the explicit behavioral gates it has to pass: the relevant
`brokkr service-test` scripts, the sync-harness runs (real provider sync against
the `saehrimnir` mock servers - see the harness doc), and `brokkr sync-bench`
where performance is in scope. A spec the loop can satisfy with a compile-only
replacement is under-gated and must be rejected at review.

## 11. Bifrost source and dependency paths

Bifrost lives in two places relative to this repo's top-level folder, and they
serve two distinct purposes - do not conflate them:

- Reading-reference: `./research/bifrost`. This is where agents inspect bifrost
  source - to verify a Track A item against bifrost's current shape, to read the
  `Account` / `AccountError` / `SyncEngine` surface a Track B spec is written
  against, or to confirm a type signature before speccing. Spec authors and
  reviewers read here; it is the ground a bifrost-facing spec is judged against.
  `./research/bifrost/reference/` holds per-crate and per-protocol
  quick-reference sheets (`net.md`, `sync.md`, `error-model.md`, `jmap.md`,
  `imap.md`, `graph.md`, `google.md`, `smtp.md`, `caldav.md`, `carddav.md`,
  `sasl.md`) - start there for a crate's surface, then drop into the source.
- Dependency path: `../bifrost/`. This is what Cargo `path = "..."` deps resolve
  to. The path deps on the bifrost crates (added by B1, extended by later items)
  point at `../bifrost/`, not at the reading-reference copy.

A spec that touches bifrost cites `./research/bifrost` as required reading for
its implementers and reviewers, and any spec that adds or changes a bifrost
dependency pins the `../bifrost/` path explicitly.

Track A is complete at commit `ff56478` (the A8-closing commit). The current
frozen reference is `002e7b9`: six bifrost side-quests have landed since the
A8-closing commit (see § 2's side-quest protocol), and both `./research/bifrost`
and `../bifrost` were re-synced together to each in turn. First `aa9172d` ("sync:
make backfill checkpoints consumer-ack-deferred"), surfaced during B3's spec
review, made backfill checkpoints consumer-ack-deferred for at-least-once
cold-start hydration. Then `dc670ef` ("sync: expose read-only hydration
passthrough on SyncEngine"), surfaced during B3a-cut-jmap's spec review, added a
read-only hydration passthrough cluster (`get_stream` / `message_hydrate` /
`thread_hydrate` / `open_raw_rfc822` / `open_blob` / `open_blob_range`) so the
change-stream consumer can reach the engine-private `Arc<dyn Account>` to hydrate
a broadcast `Change` into real rows - the blocker B3a-cut-jmap § 4.2 named.
Then two further side-quests surfaced during B3a-cut-jmap's step-7 gate
validation, both in `run_backfill_orchestrator` (`crates/sync/src/engine.rs`):
the OpenPages loop treated a short inventory page (`seen < chunk`, the common case
when a JMAP server caps `Email/query` below the requested chunk) as
end-of-inventory and dropped the rest of a bulk backfill - fixed to terminate on
an empty page; and the orchestrator never read `get_backfill` from the
`CheckpointStore`, so a one-shot re-attach re-walked backfill from page 0 every
kick (inflating delta `Email/query` and failing the steady-state gates) - fixed to
resume/skip from the persisted backfill checkpoint. Both landed and advanced the
freeze to `ae73e92`. Two further per-cut side-quests advanced the freeze again:
B3a-cut-graph added Graph `importance` to the typed `hydrate_select` (advancing
to `7c576bdd`), and B3a-cut-gmail added the Gmail mock-redirect seam to
`GoogleAccountFactory` (`from_token_source_with_api_base` /
`from_access_token_with_api_base`, advancing to `002e7b9`). (Note: separate from
these bifrost commits, B3a-cut-jmap also
required several `saehrimnir` mock extensions - JMAP `Thread/get`, `ContactCard/get`,
and per-`accountId` `Email`/`Mailbox` state - which are an installed external
binary, not commit-pinned here.) B3a-cut-imap required NO bifrost change - the
freeze stays at `002e7b9` because bifrost's IMAP CONDSTORE/QRESYNC behavior was
already correct - so the side-quest count holds at six; its mock work was entirely
in `saehrimnir`, whose IMAP mock gained real CONDSTORE/QRESYNC support (parse
`SELECT (CONDSTORE)`, return `HIGHESTMODSEQ`, honor `FETCH ... (CHANGEDSINCE ...)`,
emit QRESYNC `VANISHED`), again an installed external binary, not commit-pinned
here. Each Track B spec records, in its ground
survey, the exact `../bifrost` commit it was authored and gated against, and
`../bifrost` stays frozen at that commit for the full duration of the item -
including the hours a step-4 implement run can take. This is load-bearing: the
dependency compiles from source, so a `../bifrost` that is red OR merely
mutating underneath an in-flight ratatoskr step makes every ratatoskr gate
meaningless, and a later bifrost change would silently shift the surface the
spec was built against.

## 12. Review reconciliation

This plan was reviewed twice before the loop launched (R1 / opus, R2 / codex);
both reports live under `docs/bifrost-migration/`. Every valid finding from both
is now folded into the sections above: the stale commit pin (advanced from
`416cbd4` to `ff56478` in § 11), the core-boundary leak (the core-boundary rule
in § 7), the maximal-integration deletion audit and the out-of-tree
`bifrost-jmap` dep (§ 7 B15), the incomplete B2 cursor-table set (which expanded
B2's scope to a full table-by-table disposition and has since been satisfied by
the B2 landing - see the B2-done note in § 7), and the compile-only-replacement
gate gap (§ 9 B3 and § 10).

Findings considered but not folded, with reasons:

- R1's pre-loop status observations about the tree (and its note on the then-open
  B1 spec) were point-in-time facts, not changes to this governing plan, and have
  since been overtaken by the B1 landing (which added the bifrost deps and
  `crates/service/src/bifrost/`). No edit to the plan was needed for them.
- R1's "could not verify the `../bifrost` sibling from the sandbox" and the
  accompanying open question, plus R2's open question on whether `core` may take a
  bifrost dependency, are both resolved rather than folded as caveats: the
  user confirmed both `../bifrost` and `./research/bifrost` are at `ff56478`
  (§ 11), and the core-boundary rule (§ 7) settles that `core` stays
  bifrost-free. No residual uncertainty to record.
