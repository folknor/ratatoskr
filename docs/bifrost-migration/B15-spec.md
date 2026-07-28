# B15: deletion and collapse - the final green cut

Tracked for the item's implementation window; removed at landing (bundled with
the B15f code commit, never standalone).

Authored against: ratatoskr `eea1e7c3` (the B8-groups landing plus its
baseline re-record; the survey below reflects that tree, not the older
`829ad380` an earlier draft of this line cited), frozen bifrost
`../bifrost` = `./research/bifrost` at `59b9e2d`, saehrimnir installed at
`45514fa`.

## 0. Contract and required reading

This spec is written against `reference/technical-implementation-spec.md` - the
contract for what a spec must pin. Implementers and reviewers must READ, not
merely note:

- `reference/technical-implementation-spec.md` - the contract itself.
- `reference/architecture.md` - crate boundaries, the action pipeline, the
  `OperationResult` taxonomy, generation counters, scope wiring. B15 deletes
  crates and moves modules; every boundary decision below is judged against
  this document, and B16 (bundled here) rewrites its provider sections.
- `docs/bifrost-migration.md` - the TODO source. Specifically: § 1 (the
  maximal-integration first principle, which makes the named deletion list a
  FLOOR), § 7's B15/B16 items and every done-note that says "stays until B15"
  or "additive-green until B15", the B12 methodology finding (carve narrow,
  pure unit-pinnable wiring functions, a bifrost side-quest is done only when
  the consumer's harness gates pass against the promoted surface), § 11 (the
  freeze protocol and the two measurement findings carried into B15), and § 9's
  jmap-client caveat.
- `reference/glossary/folders-labels.md` - B15 moves the Graph master-category
  label sync and the IMAP keyword-capability flag, both of which write the
  `labels` / folder surfaces this glossary governs.
- `reference/glossary/harness.md` - B15 adds two sync-harness gates, re-records
  steady-state baselines, and drives saehrimnir affordances.
- `./research/bifrost/reference/` sheets for the touched crates: `sync.md`,
  `jmap.md`, `imap.md`, `graph.md`, `error-model.md` - the side-quests
  below are specified against these surfaces, and `error-model.md`'s batch
  contract is binding on § 5.2.

Bifrost dependency path: `../bifrost` (path deps). Reading/staging copy:
`./research/bifrost`. The freeze at `59b9e2d` holds until B15's side-quests
advance it; each brick below records the freeze it lands against.

Revision note: this document has been reconciled against two independent
reviews (`R1.md`, `R2.md`). Every accepted finding is folded into the body
below; § 12 records what was rejected and why. The largest structural change
is the removal of B15-SQ-imap: the flag it existed to feed has no reader.

## 1. What B15 is

The migration's closing cut. Every provider behavior - sync, push, actions,
send, drafts, folders, labels, calendar, contacts, groups, attachments,
identities, verify, shared mailboxes, public folders - already routes through
the resident bifrost `SyncEngine`. What remains of the legacy stack is:

1. Three auxiliary passes still riding legacy provider clients (JMAP
   shared-owner email + ShareNotification, Graph master categories + Exchange
   reactions, IMAP keyword-capability probe). ONE of them is ported (Graph),
   one is partly ported (JMAP: owner email ported, ShareNotification poll
   deleted), and one is deleted outright (IMAP: the probe's only output is a
   column with no readers - see Obstacle A).
2. A dead action-ops surface (`ProviderOps`, `create_provider`) with zero
   callers.
3. A handful of LIVE pure helpers stranded inside crates scheduled for
   deletion (`jmap::rfc822`, `imap::parse`, `imap::client::
   extract_attachment_from_rfc822`, and provider-sync's provider-agnostic
   consumer-support modules).
4. The four provider crates, `provider-sync`, the legacy `smtp` crate, the
   external `jmap-client` checkout (`bifrost-jmap` git dep), and the retired
   sync-state tables/columns kept additive-green by earlier items.

B15 rewires (1), deletes (2) and (4), re-homes (3), runs the § 1 mechanical
workspace audit, and reconciles the reference docs (B16, bundled). The tree is
green at every brick boundary; after the final brick no crate, module,
dependency, table, or column duplicates a bifrost-covered surface.

Per the B12 methodology finding, this is carved as SIX narrow bricks (B15a-f),
each a coherent keep-or-revert landing, with TWO narrow bifrost side-quests
(Graph and JMAP) rather than one broad one. The orchestrator may promote any
brick to its own loop item without re-derivation; the bricks are
self-contained.

## 2. Ground survey

Verified on `eea1e7c3`. Every claim below was established by direct inspection;
re-verify with the named greps before implementing, since the tree moves.

### 2.1 The dead action surface (zero callers)

- `crates/service/src/actions/provider.rs`: `create_provider` has NO callers -
  only its definition and three comments (`prefetch.rs:22`, `actions/
  folder.rs:8`, `actions/mod.rs:28`) mention it. Verify:
  `grep -rn "create_provider" crates/service/src`.
- `HarnessOfflineProvider` (same file) is therefore unreachable. Its
  `fetch_attachment` map (`harness_attachments()`) is written by
  `register_harness_attachment` - still called from
  `handlers/test_helpers.rs:1112` - but READ only by the unreachable
  `fetch_attachment`. The live half of that handler is the DB-row seeding
  (`insert_harness_remote_attachment`), which stays.
- `common/src/ops.rs` (`ProviderOps`, 158 lines): implementors are
  `gmail/src/ops.rs`, `graph/src/ops/`, `jmap/src/ops.rs`, `imap/src/ops.rs`,
  and `HarnessOfflineProvider`. No live dispatch anywhere. All remaining
  `ProviderOps` mentions outside these files are comments.
- EXCEPTION that blocks naive deletion: `imap::ops::ImapOps::new(..)
  .load_config(..)` is used by the resident IMAP aux pass
  (`bifrost/resident.rs:1152`) purely to load `ImapConfig` (which pulls
  `smtp::types::SmtpConfig` via `imap/src/account_config.rs`). This dies in
  B15b, BEFORE the ops deletion in B15e.

### 2.2 The three live auxiliary passes

`bifrost/resident.rs::run_aux_pass` (lines ~1083-1176) is the only production
entry into legacy provider clients.

CADENCE (corrected; an earlier draft of this spec said "every kick" and was
wrong): `run_aux_pass` is driven by `resident_aux_loop`
(`resident.rs:1041-1080`) on a WALL-CLOCK timer -
`RESIDENT_AUX_INITIAL_DELAY = 5s` after attach, then
`RESIDENT_AUX_CADENCE = 300s` (`resident.rs:43-44`). Sync kicks do NOT invoke
aux. The loop also latches `initial_sync_completed` from an attach-time
snapshot, so a freshly attached account's FIRST pass is the initial pass and
every later pass is a delta pass. Everything downstream in this spec - bench
predictions (Obstacle I), harness drivability (B15a) - follows from this
number, not from a per-kick model.

- JMAP: builds legacy `jmap::client::JmapClient::from_account` (the OLD
  external jmap-client under the hood) and calls
  `provider_sync::consumer_support::run_jmap_auxiliary_sync` ->
  `provider-sync/src/jmap/aux_sync.rs` (325 lines):
  - `resolve_shared_account_identities`: for each non-personal session
    account, resolves the owner EMAIL (principals capability ->
    `Principal/get`; fallback: session account name containing `@`) and
    caches it via `sync::state::set_shared_mailbox_email` into the LIVE
    `shared_mailboxes.email_address` column - read by pop-out compose
    (`core/src/db/queries_extra/navigation.rs::get_shared_mailbox_email_sync`,
    consumed at `app/src/handlers/pop_out/window_lifecycle.rs:125`). This is
    a real feature, not vestige.
  - `poll_share_notifications`: cursors on `jmap_sync_state` type
    `"ShareNotification"`, fetches/logs/destroys notifications, and on a
    Mailbox change does NOTHING but log - the comment at aux_sync.rs:233
    states the container snapshot owns discovery/reconciliation since B6/B12.
    The only durable write is its own cursor. `jmap_sync_state`'s SOLE
    remaining writer/reader is this poll (verify:
    `grep -rn "jmap_sync_state" crates --include="*.rs"`).
  - `fetch_all_mailboxes_for` in the same file has no callers outside the
    file's own tests (verify before deleting).
- Graph: builds legacy `graph::client::GraphClient::from_account` and calls
  `provider-sync/src/graph/aux_sync.rs` (224 lines):
  - initial pass ONLY (`!initial_sync_completed_before_run` early-RETURNS
    after it): `graph::label_sync::graph_label_sync` (master-category
    definitions -> `labels` rows via `upsert_labels`, colors via
    `label_colors::preset_colors`). Note the mapping is FALLIBLE
    (`LabelKind::graph_category(&display_name)?`) and additionally appends
    `importance_label_rows` - four `ImportanceLevel` rows with
    `is_undeletable: true` - to the same `upsert_labels` batch
    (`graph/src/label_sync.rs:48,75-84`). `OutlookCategory.id` is
    deserialized but unused downstream.
  - delta passes, cadenced by `sync::state::increment_graph_sync_cycle`
    (settings-table counter): reactions refresh every 5th cycle (a `$batch`
    of `singleValueExtendedProperties` GETs keyed on `REACTIONS_GUID`,
    writing `message_reactions` rows via `upsert_message_reaction_update_type`
    / `delete_message_reaction`), master categories every 20th. At 300s per
    pass that is a reaction refresh roughly every 25 minutes and a category
    refresh every ~100 minutes.
  - Reaction write semantics, exactly (`graph/aux_sync.rs:143-212`): batch
    items with `status != 200` are SKIPPED entirely (no row touched); a 200
    with a non-empty `OwnerReactionType` upserts the owner row keyed on
    `accounts.email`; a 200 WITHOUT it deletes the owner row; a 200 with
    `ReactionsCount` upserts a `__count__` row. There is NO delete branch for
    the `__count__` row - see § 2.8 for the two latent bugs this shape
    carries, which B15 ports faithfully rather than fixing.
- IMAP: builds `ImapOps::load_config` + `imap::connection::connect`, then
  `provider-sync/src/imap/aux_sync.rs` (52 lines): SELECTs every folder path
  (from the `folders` table, B6a) and ANDs
  `imap::client::mailbox_supports_custom_keywords` over the PERMANENTFLAGS
  responses into `set_account_supports_keywords`. N extra SELECTs per aux
  pass (i.e. per 300s), on a dedicated legacy connection. The column it
  writes has NO readers anywhere in the tree - see § 2.9 and Obstacle A.
- Gmail: NO legacy aux remains. `run_aux_pass`'s Gmail arm calls
  `super::gmail_signatures::sync_gmail_signatures` (bifrost identities, B13).
  `provider-sync/src/gmail/mod.rs` is a bare re-export
  (`pub use ::gmail::{client, parse, types}`) with no aux module; the gmail
  crate has no live consumer at all once `create_provider` goes.

### 2.3 Live pure helpers stranded in doomed crates

- `jmap::rfc822` (283 lines; imports only `mail_parser`,
  `common::email_parsing`, `common::text`): `parse_rfc822`, `Rfc822Parsed`,
  `Rfc822Attachment`, `format_addr_field`, `snippet_from_body`. Reached via
  `provider_sync::consumer_support` by `bifrost/consumer/hydrate.rs` (all
  providers' raw-RFC822 fidelity re-parse) - fully live.
- `imap::parse::parse_message`: used by `bifrost/consumer/hydrate.rs:708`
  (IMAP hydration re-parse - the SOLE source of IMAP attachment/inline rows).
- `imap::client::extract_attachment_from_rfc822`: used by
  `bifrost/attachment.rs:169` (IMAP attachment bytes from `open_raw_rfc822`).
- `provider-sync`'s provider-AGNOSTIC modules reached via `consumer_support`.
  An earlier draft called all of them live; they are NOT. Exact split,
  established from `service/src/bifrost/consumer/write.rs:9-14` and
  `hydrate.rs:15` (the only two importers):
  - LIVE, must be re-homed: `keyword_membership.rs` (`KeywordProvider`,
    `replace_message_keywords`, `recompute_thread_keyword_labels`),
    `persistence.rs` (`store_message_bodies`, `store_inline_images`,
    `index_search_documents`), and TWO of `thread_membership.rs`'s three
    strategies - `replace_message_membership_and_recompute` and
    `replace_message_folders_and_recompute` (the latter is live in `write.rs`
    despite the `consumer_support.rs:38-40` comment calling it "reserved" -
    the comment is stale, not the code).
  - DEAD, must be DELETED not re-homed: `seen_ingest.rs` /
    `ingest_from_messages` (the consumer re-implements seen-ingest inline in
    `post_persist.rs` so the marker insert shares the counter-increment txn;
    `consumer_support.rs:30-36` says so in as many words, and the only
    remaining reference to the module is the re-export itself), and
    `thread_membership.rs::replace_thread_membership_from_full_coverage`
    (marked reserved, no caller). Re-homing these would carry residue across
    the very cut B15 exists to make.
  - The `ensure_folder_rows` / `insert_folders_batch` / `FolderWriteRow`
    re-exports are just `db` pass-throughs: point the consumer at `db`
    directly, do not re-home.
- `provider-sync/src/graph/sync/` is an EMPTY directory (B12 residue).

### 2.4 Manifest and dependency ground

- Workspace members to remove (`Cargo.toml:24-27,38` plus smtp):
  `crates/gmail`, `crates/jmap`, `crates/graph`, `crates/imap`,
  `crates/provider-sync`, `crates/smtp`.
- `crates/service/Cargo.toml`: deps on all four provider crates,
  `provider-sync`, AND the old external jmap-client
  (`bifrost-jmap = { git = "file:///home/folk/Programs/jmap-client", rev =
  "b3d207c", .. }`, line 53) - which NO service source file uses (only
  `bifrost_jmap_new::` appears; verify
  `grep -rn "use bifrost_jmap::" crates/service/src` is empty). Drop the git
  dep line the moment provider-sync's dep is gone; it is pure weight today.
- `crates/core/Cargo.toml`: deps on `gmail`, `jmap` (both UNREFERENCED in
  core src), `graph` (referenced solely by `core/src/lib.rs:18`
  `pub(crate) use graph;` for `cloud_attachments.rs`), and the old
  `bifrost-jmap` git dep (UNREFERENCED). Plus `gmail/jmap/graph` legs in the
  `hotpath` / `hotpath-alloc` feature lists (lines 67-68).
- `crates/provider-sync/Cargo.toml`: four provider crates + old
  `bifrost-jmap`.
- `crates/smtp`: consumers are ONLY `imap/src/ops.rs` and
  `imap/src/account_config.rs`, plus `core/src/lib.rs:38` `pub use smtp;`
  (no `smtp::` / `rtsk::smtp` user anywhere else; verify
  `grep -rn "use smtp\|smtp::" crates --include="*.rs"`). bifrost-smtp is the
  covering equivalent - the crate dies with `imap`.
- `core/src/cloud_attachments.rs`: `enrich_onedrive_link(&GraphClient, ..)`
  and `enrich_gdrive_link` have NO callers (B9 kept the incoming carve-out
  "whole, unwired"). The pure `detect_cloud_links` /
  `extract_gdrive_file_id` / `CloudProvider` stay.
- `lettre` in service: live use is `lettre::message::Mailbox` parsing in
  `service/src/send.rs` only. Audit item (B15f), not a named deletion.
- `bifrost-jmap-new` is the workspace alias for bifrost's own
  `bifrost-jmap` package (`Cargo.toml:118`). The `-new` suffix exists only to
  dodge the old git dep's name. Once the git dep is gone the trap goes with
  it: rename the alias to `bifrost-jmap`.

### 2.5 Retired schema surface (additive-green debts falling due)

- `jmap_sync_state` (+ `shared_account_id`): sole writer is the
  ShareNotification poll (2.2). Goes with B15d. Companions that must go in
  the SAME edit, none of which an earlier draft named: the table body at
  `schema/10_sync.sql:15-23`, the unique index
  `idx_jmap_sync_state_shared` (`10_sync.sql:24-25`), the retired-table
  roll-call comment at `10_sync.sql:37`, and the migrations note at
  `migrations.rs:50-52`.
- `folder_sync_state`: writers/readers are only the orphaned helpers
  `db/src/db/queries_extra/ai_state.rs` (`db_get/upsert/delete/clear/
  get_all_folder_sync_state*`), `db-read/src/db/queries_extra/ai_state.rs`
  (two getters), `sync/src/pipeline.rs::clear_all_folder_sync_states`.
  Confirm no handler/test reaches them, then delete helpers + table.
- `graph_folder_delta_tokens`: helpers in `sync/src/state.rs` (lines
  ~128-180). B12 deleted the shared-mailbox leg that last wrote it; confirm
  the helpers are now caller-free, delete helpers + table.
- `jmap_push_state`, `graph_subscriptions`: no writers (B3b);
  `resident.rs:1341-1368` (`push_state_tables_have_no_writer`) carries the
  no-writer regression list naming both. NOTE what that test actually is: a
  SOURCE-TEXT scan over nine `include_str!`ed files for `"{VERB} {table}"`
  substrings. It holds no DB connection. "Strengthening it into a
  schema-absence assert" therefore means RELOCATING it, not editing it - see
  B15f step 1 for the exact destination.
- `accounts.history_id`: NOT a clean drop. An earlier draft disposed of this
  in one line and was wrong. The ground truth:
  - The WRITE path is genuinely dead: `set_account_history_id` /
    `save_account_history_id` / `load_account_history_id` /
    `get_account_history_id` have zero callers outside their own
    definitions.
  - But `sync::pipeline::clear_account_history_id` is LIVE, called on every
    dirty account by the boot invariant pass
    (`service/src/startup_invariants.rs:228`, counted into
    `history_ids_cleared` at :65/:231, logged at :288, and commented "Clear
    JMAP cursor (load-bearing)").
  - Its load-bearing half is not the column. The statement is
    `UPDATE accounts SET history_id = NULL, initial_sync_completed = 0, ...`
    (`account_sync_writes.rs:87-95`); the `initial_sync_completed = 0` reset
    is what forces the post-crash initial-style resync that the Tantivy
    orphan sweep and the `clean_shutdown_cursors` design both lean on
    (`startup_invariants.rs:239`, `boot.rs:1393`,
    `schema/10_sync.sql:132-134`).
  - `history_id: Option<String>` is a field on the `Account` row struct in
    BOTH `db/src/db/types.rs:28` and `db-read/src/db/types.rs:28`, selected
    by each crate's `from_row_impls.rs:21` and read in each crate's
    `queries_extra/accounts_messages.rs:14`.
  Disposition: the column still drops in B15f, but as a properly scoped
  four-part edit, NOT as an "orphaned helper" deletion - see B15f step 1a.
- LIVE, RETAINED (do not touch): `shared_mailboxes` (email cache, 2.2),
  `pending_operations` (action retry journal), `clean_shutdown_cursors`,
  `sync_cursors`, `seen_ingest_markers`, the `settings`-table cadence
  counters (`graph_sync_cycle` stays - the rewired Graph aux still uses it).
- `public_folder_sync_state` / `public_folder_content_routing` /
  `graph_shared_mailbox_delta_tokens`: no `.rs` references remain; B12
  disposed of them. B15f's audit confirms schema absence (or drops leftovers).

### 2.6 bifrost at `59b9e2d` - what exists, what is missing

- bifrost's jmap crate CARRIES `principal/` and `share_notification/` client
  modules (it is the evolved fork of jmap-client), but `sync::Account`
  exposes no client accessor, so ratatoskr cannot drive them without a
  surface addition.
- `Container` (types crate) already carries `namespace`, `owner:
  Option<MailboxId>`, `owner_local_id`, `rights`, `is_subscribed`, `style`,
  `system` - but NO owner email and NO keyword-capability field.
- bifrost-graph's `GraphClient` HTTP verbs are `pub(crate)`; there is no
  master-category surface and no reaction/extended-property READ surface
  (`singleValueExtendedProperties` appears only in write-side pim.rs).
- bifrost-imap parses PERMANENTFLAGS (codec + SELECT dispatch;
  `imap/src/types/mailbox.rs:438`) but hardcodes
  `mutation.set_keyword: true` (`imap/src/account/capabilities.rs:66`) and
  surfaces no per-mailbox keyword verdict. B15 does NOT add one - see
  Obstacle A; the follow-up lives in § 9.
- `AccountOperation` (`types/src/error/scope.rs:111-140`) has NO variant for
  category-definition or reaction reads. B15-SQ-graph must add two, since
  every `Unsupported` stub needs one.
- `BatchOutcome<T>` (`types/src/batch.rs`, `reference/error-model.md`
  § "Batch and stream outcomes") is bifrost's three-lane
  succeeded/failed/uncertain result, and `finalize(&expected)` ENFORCES that
  every submitted item lands in exactly one lane. Any chunked read surface
  B15 adds must return it, not a bare `Vec`.

Hence the two side-quests in § 5. Precedent: B8-groups (a Graph-only
surface with `Unsupported` stubs elsewhere) and B12-SQ (additive `Container`
fields with `with_*` setters).

### 2.7 Harness and gate ground

- Existing gates that must hold across B15: `graph-master-category-label-
  sync.lua`, `jmap-shared-account-sync.lua`, `imap-jmap-shared-state.lua`,
  the ten `imap-*` mail gates, the B12 shared/public suites, all
  `*-action-writeback` / `*-send-writeback` / MDN / container-crud /
  contacts / groups scripts, `brokkr service-suite`, and the steady-state
  sync `--bench` baselines (`jmap_steady_state_delta`,
  `graph_steady_state_delta`, `gmail_steady_state_delta`,
  `imap_steady_state_delta`, `containers-attach`, `contacts_cadence`,
  `graph_groups_pull`).
- Exact counts, since an earlier draft guessed: there are NINETEEN
  `imap-*.lua` sync-harness scripts, not ten (`ls
  crates/app/tests/sync-harness/ | grep '^imap'`). "containers-attach" is
  FOUR gates: `gmail_containers_attach`, `graph_containers_attach`,
  `imap_containers_attach`, `jmap_containers_attach`.
- Harness routing, since an earlier draft conflated them: the new B15a
  instruments are SYNC-harness scripts, so they run under `brokkr sync`, not
  `brokkr service-suite`. `service-suite` discovers
  `crates/app/tests/service-harness/`; the sync sweep is
  `brokkr sync --all` (`reference/glossary/harness.md` § "Brokkr CLI
  surface"). § 6 gives literal commands for both.
- NO gate pins the Graph reaction refresh. Per contract rule 5, that
  instrument is built FIRST (B15a). There is likewise no gate on IMAP
  `supports_keywords` - and B15 builds none, because the flag is deleted
  rather than ported (Obstacle A).
- The TODO's § 11 measurement rules (`docs/bifrost-migration.md`) bind every
  bench here: run the comparison at
  the parent commit before attributing a request-count delta, and treat
  `baseline_label` as the durable record when re-recording.
- `imap_steady_state_delta` DELIBERATELY EXCLUDES the aux pass from its
  measured window: the script sleeps 6000ms past the 5s aux start, spins
  until the mock request log goes quiet, then clears it
  (`imap-steady-state-delta.lua:72-88`), and `brokkr.toml:431`'s
  `baseline_label` records that exclusion in prose ("the aux pass is waited
  out and cleared before the measured window"). No aux-pass change can move
  this gate. Obstacle I depends on this fact.

### 2.8 Two latent bugs in the Graph reaction write (ported, not fixed)

Both are pre-existing and both survive B15c's faithful port. They are named
here so the B15a instrument is written against REAL legacy behavior rather
than intended behavior, and so the port cannot be mistaken for endorsement.

1. The `__count__` row is never deleted. The owner row has a
   delete-on-absent branch; the count row has an upsert branch only
   (`graph/aux_sync.rs:183-211`). A message whose reactions are cleared on
   the server keeps its `__count__` row forever. Consequence for B15a: the
   instrument may assert removal of the OWNER row and MUST NOT assert
   removal of the `__count__` row, or it cannot go green on legacy code.
2. `upsert_message_reaction_update_type` can never take its update branch.
   `message_reactions`' unique key is
   `(message_id, account_id, reactor_email, reaction_type)`
   (`schema/09_security.sql:29`), and the helper's
   `ON CONFLICT(...) DO UPDATE SET reaction_type = ?4`
   (`calendar_contacts_writes.rs:1060-1079`) sets the very column that is in
   the conflict target - so a changed emoji or a changed count does not
   conflict at all, it INSERTS a second row. Stale rows accumulate. The
   owner-side accumulation is masked because `delete_message_reaction`
   deletes by `(message_id, account_id, reactor_email, source)` and ignores
   `reaction_type`, sweeping every stale owner row on removal; the
   `__count__` side has no such sweep and accumulates without bound.

Fixing either is a behavior change, which § 8 forbids B15. Both are carried
to § 9 as a named follow-up with the shape the fix needs (a `source`-keyed
unique constraint or a delete-then-insert, plus value-change and removal
tests, plus a migration under the v100 pre-release policy).

### 2.9 `accounts.supports_keywords` has no readers

Established by whole-tree grep (`grep -rn "supports_keywords" crates`). The
column has exactly three references and none of them is a read:

- the writer, `provider-sync/src/imap/aux_sync.rs:42-46`;
- the setter, `db/.../account_sync_writes.rs:100-115`
  (`UPDATE accounts SET supports_keywords = ?1`);
- the column declaration, `schema/01_core.sql:50`.

No SELECT, no struct field on either `Account` row type, no handler, no app
read, no harness assertion. The behavior the column NARRATES - refusing a
keyword STORE against a mailbox whose PERMANENTFLAGS lack `\*` - is real but
lives somewhere else entirely and is per-MAILBOX, decided at SELECT time
inside the legacy IMAP client: `imap/src/client/commands.rs:107` and `:144`
(`set_keyword_if_supported` / `set_keyword_batch_if_supported`, both calling
`mailbox_supports_custom_keywords` on the just-SELECTed mailbox). That code
dies with `crates/imap` in B15e regardless of what happens to the column.
This is the single most consequential survey correction in the document; see
Obstacle A.

## 3. Obstacles, resolved inline

### A. The IMAP keyword probe writes a column nobody reads - delete it

An earlier draft of this spec justified a whole bifrost side-quest with "the
flag gates keyword write-back". It gates nothing (§ 2.9). The probe, the
setter, and the column form a closed write-only loop; the real gate is
per-mailbox and SELECT-local (`imap/src/client/commands.rs:107,144`), and it
dies with `crates/imap` in B15e whatever we do with the column.

Weigh the alternative honestly, because it was the plan of record: preserving
the flag costs a `bifrost-types` field, a bifrost-imap per-mailbox state
cache, a saehrimnir fixture knob, a new sync-harness gate, and a pure
`derive_supports_keywords` - to keep a value that no code path consults. § 1's
maximal-integration-is-a-FLOOR principle does not permit spending a bifrost
API on dead database state.

RESOLUTION: delete, do not port. B15b deletes the probe
(`provider-sync/src/imap/`), the setter
(`db/.../account_sync_writes.rs::set_account_supports_keywords`), the column
(`schema/01_core.sql:50`), and `read_imap_folder_paths` if it is left
caller-free. B15-SQ-imap is CANCELLED; there is no
`Container.keywords_supported`, no `derive_supports_keywords`, and no
`imap-keyword-capability.lua`.

Two consequences, both named rather than hidden:

- Per-mailbox keyword gating stops being enforced by ratatoskr at the moment
  `crates/imap` is deleted. It was already not enforced through this column;
  what B15e removes is the legacy client's own SELECT-time refusal. Under
  bifrost the refusal belongs INSIDE bifrost-imap, which already parses
  PERMANENTFLAGS (`imap/src/types/mailbox.rs:438`) and already knows the
  mailbox it is issuing the STORE against. That is a bifrost bugfix
  side-quest, not a ratatoskr projection: § 9 carries it as a named
  follow-up ("bifrost-imap should decline `set_keyword` on a mailbox whose
  PERMANENTFLAGS lack `\*`, or report it as a per-item `BatchFailure`,
  instead of hardcoding `mutation.set_keyword: true` at
  `capabilities.rs:66`"). Naming it here is what keeps the deletion honest:
  we are removing a narration, and filing the behavior where it belongs.
- If review overturns this and the column must live, the projection route is
  still the right shape BUT it is not enough on its own: the verdict would be
  recorded at SELECT time while persistence happens only during container
  sync, and container sync runs at attach and on `refresh_containers` cache
  misses (`resident.rs:292,440,577`) - never after a later SELECT changes a
  verdict. A reinstated side-quest must therefore name its post-SELECT
  persistence edge and should carry the verdict PER FOLDER rather than
  collapsing it to an account-wide AND. Rejected sub-alternative either way:
  deriving from `account_capabilities().mutation.set_keyword` - it is
  static-true and account-scoped; making it dynamic would overload a
  capability contract every other consumer reads as static.

### B. Graph categories and reactions have no bifrost surface

Master-category definitions and Exchange-native reactions are real Outlook
features with no bifrost equivalent, and bifrost-graph's client verbs are
crate-private. Keeping the legacy `GraphClient` (its own OAuth refresh, its
own batch plumbing) for two aux reads is the § 1 parallel-stack case.
Resolution: B15-SQ-graph (§ 5.2) adds two narrow, capability-dispatched
`Account` methods + `SyncEngine` forwarders (B8-groups shape). The B6-SQ
adjudication that Graph categories are message flags, NOT containers, is
respected: the new surface is a definitions LIST, not a container projection.
The ratatoskr-side label mapping (`LabelWriteRow` construction, preset-color
resolution) moves out of `graph/label_sync.rs` into a pure unit-pinnable
function in service; the reaction row-write logic likewise (pure
classification + the existing `upsert_message_reaction_update_type` /
`delete_message_reaction` writes).

Two contract points the earlier draft left loose, both load-bearing:

- The label mapping is FALLIBLE and not purely a category map. It threads
  `LabelKind::graph_category(&display_name)?` and appends four undeletable
  `ImportanceLevel` rows to the same batch (§ 2.2). The ported function's
  signature must reflect both, so it is
  `category_label_rows(defs, account_id) -> Result<Vec<LabelWriteRow>, String>`
  and the importance rows stay inside it.
- The reaction read is a partial-failure surface. Legacy SKIPS non-200 batch
  items and DELETES on a 200 that carries no owner property; collapsing
  those two into one "no state" answer would let a transient Graph error
  wipe cached reactions. § 5.2 pins the bifrost side to `BatchOutcome`
  accordingly, and the pure classifier only ever sees successfully-read
  items.

### C. The JMAP owner-email cache is a live feature on a dead client

`shared_mailboxes.email_address` feeds pop-out compose sender identity; its
only writer is the legacy-client principal resolution. Resolution:
B15-SQ-jmap (§ 5.3) - bifrost-jmap resolves the owner email during
`containers_list` for `Shared`-namespace containers and projects an additive
`Container.owner_email: Option<String>` (other protocols: Graph sets it when
the `/users/{id}` routing key is a UPN/email it already holds, IMAP leaves
`None`). ratatoskr's container persistence upserts the cache through the
existing `set_shared_mailbox_email` write.

Three corrections to the earlier draft's version of this, each of which
changes observable behavior if missed:

- The fallback is NOT "byte-identical to the legacy fallback" as previously
  claimed. Legacy gates the WHOLE routine on the SESSION advertising
  `urn:ietf:params:jmap:principals` and returns immediately otherwise
  (`jmap/aux_sync.rs:22-24`); the account-name-contains-`@` heuristic is
  reached only when the session HAS that capability and the individual
  account lacks `urn:ietf:params:jmap:principals:owner` (:59-77). Resolving
  from the account name on a session with no principals capability would
  populate owner emails where legacy left them NULL. § 5.3 pins the
  two-level gate explicitly.
- Legacy is WRITE-ONCE. `Ok(Some(_)) => continue` at `jmap/aux_sync.rs:42`
  means a resolved email is never overwritten. The ratatoskr-side upsert
  must preserve that: write only when `owner_email` is `Some` AND the cached
  value is absent. An unconditional write on every `containers_list` can
  churn or clear a live pop-out-compose sender identity (read path:
  `core/.../navigation.rs::get_shared_mailbox_email_sync` ->
  `app/src/handlers/pop_out/window_lifecycle.rs:125`).
- ORDERING is load-bearing and a pure row-mapping golden cannot catch it.
  `set_shared_mailbox_email` is a bare
  `UPDATE shared_mailboxes ... WHERE account_id = ?1 AND mailbox_id = ?2`
  (`sync/src/state.rs:233-250`) that silently no-ops when the row is absent.
  The `INSERT INTO shared_mailboxes` lives in
  `bifrost/containers.rs::reconcile_namespace_registry` (:457-467). The
  email write must run AFTER that insert, in the same pass; B15d pins this
  with an integration-level assertion, not only the golden.

There is also a failure-domain hazard: this moves a best-effort metadata
lookup onto the ATTACH path. `prepare_containers` failing makes the resident
detach the account and fail attach outright (`resident.rs:292-302`), whereas
legacy logged and continued on any `Principal/get` or DB error
(`jmap/aux_sync.rs:39-47`). § 5.3 therefore requires the bifrost side to be
FAIL-SOFT: any principal-resolution error yields `owner_email: None` for that
container and never propagates out of `containers_list`.

### D. The ShareNotification poll is vestigial - delete, do not port

Its durable effects are: its own cursor (`jmap_sync_state`), log lines, and
server-side destroy of processed notifications. Discovery/reconciliation of
shared mailboxes is container-owned since B6/B12 (stated in the code itself,
aux_sync.rs:233), and `jmap-shared-account-sync.lua` pins that behavior
without the poll. Resolution: DELETE the poll and the `jmap_sync_state`
table with it (B15d). The one lost behavior - destroying processed
ShareNotification objects server-side - is named as an accepted deviation:
no ratatoskr feature reads them, RFC 9670 servers expire them, and porting a
poll whose only purpose is server-side garbage collection would re-create a
JMAP client surface B15 exists to delete. If review overturns this, the
fallback is a bifrost-jmap side-quest folding notification destruction into
its own session/push machinery - NOT a ratatoskr-side client.

### E. Live helpers must move before their crates die

Re-homing targets, chosen so no dependency arrow reverses:

- `jmap/src/rfc822.rs` -> `common/src/rfc822.rs` (`common::rfc822`). Its only
  deps (`mail_parser`, `common::email_parsing`, `common::text`) already live
  in or below `common`; `common` already carries `mail-parser`-adjacent
  parsing (`parsed_message.rs`, `email_parsing.rs`). Public names unchanged.
- `imap/src/parse.rs::parse_message` + its private helpers, and
  `imap/src/client.rs::extract_attachment_from_rfc822` (+ whatever
  `utf7_imap` / `types` items the compiler proves they pull) ->
  `service/src/bifrost/consumer/imap_mime.rs`. DECIDED, not deferred to the
  implementer: service is the sole consumer (`consumer/hydrate.rs:708` and
  `bifrost/attachment.rs:169`, both service), and growing `common` with an
  IMAP-wire-shaped MIME module would put protocol specifics in the crate
  every other crate depends on. The `common` variant is rejected. If the
  compiler proves a pulled item is genuinely protocol-neutral (a text or
  charset helper), that ONE item goes to its existing `common` home rather
  than dragging the module.
- provider-sync's `keyword_membership.rs`, `persistence.rs`, and
  `thread_membership.rs` (minus its dead third strategy) ->
  `service/src/bifrost/consumer/support/` (new module), imports rewritten
  from `provider_sync::consumer_support::` to
  `crate::bifrost::consumer::support::`. Their deps (`db`, `service-state`,
  `store`, `search`) are all already service deps. `seen_ingest.rs` and
  `replace_thread_membership_from_full_coverage` are DELETED, not re-homed
  (§ 2.3); the `FolderWriteRow` / `ensure_folder_rows` /
  `insert_folders_batch` re-exports are replaced by direct
  `db::db::queries_extra` imports in `write.rs`. The `consumer_support`
  facade then has zero exports and dies with the crate.

### F. `MailProviderKind` parse of dead harness strings

`create_provider` was the only consumer of the `"harness-offline"` /
`"harness-slow-sync"` early-return. `bifrost/factory.rs` independently
rejects those strings (`UnknownProvider`, factory.rs:962) and its unit test
seeds them - that path is the surviving, correct behavior (harness-offline
accounts exist to never sync). Deleting `create_provider` needs no
`MailProviderKind` change.

### G. The service `[dependencies]` comment block lies

`service/Cargo.toml:18-22` still documents the action handler as
"constructs `ProviderOps` instances per account". B15e rewrites the comment
with the dep prune (comment-only edits ride the code commit, per repo
convention).

### H. Ordering constraint: ops before crates, aux before ops

`ImapOps::load_config` (2.1) is live until the IMAP aux rewire. Therefore:
aux rewires (B15b-d) strictly precede the ops/`create_provider` deletion and
crate removal (B15e). Within B15e everything lands as ONE cut - deleting
`ops.rs` files without their crates would leave half-dead crates across a
boundary for no gain.

### I. Bench baselines DO NOT move - the steady-state gates exclude aux

An earlier draft predicted that B15b would drop `imap_steady_state_delta` by
the per-kick SELECT count and instructed a re-record. That prediction was
built on the per-kick cadence error (§ 2.2) and is doubly impossible: the aux
pass is not per-kick, AND `imap-steady-state-delta.lua:72-88` deliberately
waits the aux pass out and CLEARS the request log before the measured window,
with `brokkr.toml:431`'s `baseline_label` recording that exclusion in prose
(§ 2.7). Following the old instruction would have manufactured a
stop-the-line false alarm under this same obstacle's rule.

Corrected rule for every brick in B15:

- The four steady-state gates (`imap`/`jmap`/`graph`/`gmail`
  `_steady_state_delta`) measure a delta KICK, not an aux pass. B15b, B15c,
  and B15d are all expected to leave them UNCHANGED. Run them as regression
  guards; do NOT pass `--as-baseline`; do not touch `baseline_label`.
- Any movement in those gates is by definition unpredicted, hence
  stop-the-line: compare at the parent commit (the TODO's § 11 finding) before
  attributing.
- The aux-pass request saving from B15b (N SELECTs plus a dedicated LOGIN,
  per 300s) is therefore UNMEASURED by the existing instruments. B15 does not
  invent a new benchmark for it: an attach/aux-window bench is real work with
  its own flakiness surface, and the saving is not a B15 acceptance
  criterion. § 9 carries "an attach-window / aux-pass request bench" as a
  named follow-up so the win is recorded rather than silently claimed.

### J. The Graph reaction gate is not drivable without a cadence affordance

The reaction refresh fires on `cycle.is_multiple_of(5)`
(`graph/aux_sync.rs:40`) of a 300s loop, and a fresh account's FIRST aux pass
early-returns after the category import (`graph/aux_sync.rs:15-28`), so the
first reaction refresh is roughly 25 minutes of wall clock after attach. An
earlier draft's "drive enough kicks/cycles to cross the every-5th cadence" is
not a runnable instruction - sync kicks do not invoke aux at all.

Resolution: the harness affordance is REQUIRED, not the parenthetical option
it was. B15a builds it first, on the shape B8-groups already established for
exactly this problem (`test.group_pull` plus `graph_groups_cadence.lua`
asserting the settings counter): a harness command that (a) writes the
`graph_sync_cycle` settings counter to a chosen value and (b) invokes one aux
pass synchronously and acks when it has completed, so the script asserts
against a finished pass rather than a sleep. Reuse the existing
`increment_graph_sync_cycle` write path for (a) so the harness and production
share one counter definition. Without (b) the script is a timing race and
will flake; with it the gate is deterministic and survives B15c's rewire
unchanged, which is the whole point of building it against legacy first.

## 4. Target architecture

After B15:

- Workspace members: no `gmail`, `jmap`, `graph`, `imap`, `provider-sync`,
  `smtp`. The only JMAP dependency anywhere is bifrost's own crate, wired as
  `bifrost-jmap = { path = "../bifrost/crates/jmap" }` (alias renamed, `-new`
  suffix gone; the `package = "bifrost-jmap"` indirection collapses).
- `service` reaches providers exclusively through `bifrost-*` crates. Its
  consumer-support helpers live at `service/src/bifrost/consumer/support/`;
  aux passes live at `service/src/bifrost/aux/graph.rs` (categories +
  reactions via engine passthroughs, cadenced by the retained
  `graph_sync_cycle` counter) - the JMAP and IMAP aux arms are GONE from
  `run_aux_pass` (JMAP: nothing left; IMAP: nothing left; Gmail unchanged on
  `gmail_signatures`). `run_aux_pass` keeps its 5s-then-300s
  `resident_aux_loop` driver unchanged.
- `common` gains `rfc822` and (if E's first target holds) `imap_mime`;
  loses `ops.rs` and the `ProviderOps`-only items of `types.rs`
  (`ActionProviderCtx`, `ProviderCtx`, `FetchedAttachment`, ... - exact set
  compiler-driven; `LabelKind`, `ImportanceLevel` and everything with live
  consumers stays). `common`'s crate description drops "for Ratatoskr email
  providers".
- `core` has no provider crate deps and no old jmap-client dep;
  `cloud_attachments` keeps only the pure detectors; `pub use smtp` gone;
  hotpath feature lists pruned.
- Schema: `jmap_sync_state` (+ its index and roll-call comment),
  `folder_sync_state`, `graph_folder_delta_tokens`, `jmap_push_state`,
  `graph_subscriptions`, `accounts.history_id`, and
  `accounts.supports_keywords` are gone from `crates/db/src/db/schema/`
  (v100 edit, pre-release no-v101 policy), with their orphaned helpers
  deleted and dev-seed reconciled.
- `accounts.supports_keywords` and every line that writes it are GONE. No
  `derive_supports_keywords`, no `Container.keywords_supported`. Per-mailbox
  keyword refusal is bifrost-imap's job and is filed as a follow-up
  (Obstacle A, § 9).
- `clear_account_history_id` is renamed to reflect what it actually does
  after the column drop - it resets `initial_sync_completed` - and the boot
  invariant pass keeps calling it. The `Account` row struct in `db` and
  `db-read` no longer carries `history_id`.
- Wire contracts, app crate, and the service-api surface are UNCHANGED - B15
  deletes below the seam the app never saw.

## 5. Prerequisite side-quests

BOTH follow the § 2 side-quest protocol: staged in `./research/bifrost`,
promoted via `bash scripts/bifrost.sh`, and NOT DONE until the consuming
brick's harness gates pass against the promoted surface (B12 rule). Each is
additive; bifrost stays green under `brokkr check` in its own repo. The mock
work rides saehrimnir side-quests where named (installed binary, not
commit-pinned).

### 5.1 B15-SQ-imap: CANCELLED

The keyword-verdict projection previously specified here is withdrawn. The
column it fed has no readers (§ 2.9) and the probe is deleted rather than
ported (Obstacle A). Nothing in bifrost-types or bifrost-imap changes for
B15. The section number is retained so cross-references in the loop log
resolve. The related bifrost bugfix (declining `set_keyword` on a mailbox
without PERMANENTFLAGS `\*`) is a follow-up in § 9, not a B15 prerequisite.

### 5.2 B15-SQ-graph: category definitions + reaction reads

- `bifrost-types`: `CategoryDefinition { name: String, color: Option<String> }`
  (the Graph preset token, e.g. `preset0`, passed through verbatim - ratatoskr
  owns preset->color resolution) and
  `MessageReactionState { id: ObjectId, owner_reaction: Option<String>,
  reactions_count: Option<i64> }`. Two `PimMethodSupport` flags:
  `category_definitions`, `message_reactions`.
- `bifrost-types` `AccountOperation` (`types/src/error/scope.rs`): two new
  variants, `CategoryDefinitionsList` and `MessageReactionsRead`. The enum is
  `#[non_exhaustive]`, so this is additive, but the `Unsupported` stubs below
  cannot be written without them (§ 2.6).
- `Account` trait, with CONCRETE signatures - an earlier draft gave neither
  return types nor failure behavior:
  - `async fn category_definitions_list(&self) -> AccountResult<Vec<CategoryDefinition>>`
    - whole-call success or a single `AccountError` scoped to
    `CategoryDefinitionsList`. It is one GET; there is no partial lane.
  - `async fn message_reactions(&self, ids: &[ObjectId]) -> AccountResult<BatchOutcome<MessageReactionState>>`
    - chunked internally to Graph's `$batch` limit of 20, with the chunks
    merged into ONE `BatchOutcome` whose `finalize(&expected)` accounts for
    every submitted `ObjectId` across the succeeded / failed / uncertain
    lanes (`reference/error-model.md` § "Batch and stream outcomes").
    Mapping from the legacy semantics, which is the whole point of using the
    three lanes: a batch item with `status == 200` goes to `succeeded`
    (carrying a `MessageReactionState` whose fields are `None` when the
    property is absent - that is a real "no reaction" answer); a non-200 goes
    to `failed`; a chunk that never returned goes to `uncertain`. A `failed`
    or `uncertain` item MUST NOT be reported as an empty state, because
    ratatoskr's classifier deletes on empty and would wipe cached reactions
    on a transient Graph error.
  - Both capability-dispatched, `Unsupported` stubs on the other five
    protocol crates. Two 1:1 `SyncEngine` forwarders in the contact/pim
    passthrough cluster.
- bifrost-graph impl: `/me/outlook/masterCategories` for definitions; the
  `singleValueExtendedProperties` `$filter` pair (OwnerReactionType,
  ReactionsCount - the same GUID-qualified ids the legacy code builds) inside
  `$batch` for reactions.
- saehrimnir: the Graph mock must serve `/me/outlook/masterCategories` and
  the extended-property `$batch` reads with stageable per-message reaction
  values (verify what already exists before writing - the legacy label-sync
  gate implies masterCategories is already mocked).

### 5.3 B15-SQ-jmap: owner email on shared containers

- `bifrost-types`: additive `Container.owner_email: Option<String>` +
  builder setter.
- `bifrost-jmap` `containers_list`: for each `Shared`-namespace container,
  resolve once per foreign account, reproducing the legacy TWO-LEVEL gate
  exactly (`jmap/aux_sync.rs:22-77`):
  1. If the SESSION does not advertise `urn:ietf:params:jmap:principals`,
     `owner_email` is `None` for every container. No fallback. This level is
     what the earlier draft omitted, and omitting it populates emails where
     legacy left NULL.
  2. Otherwise, if the foreign account advertises
     `urn:ietf:params:jmap:principals:owner` with a principal id, issue
     `Principal/get` for that principal and use its email.
  3. Otherwise fall back to the foreign account's session NAME if it
     contains `@`; else `None`.
  Cache the per-account resolution for the call; do not refetch per
  container.
- FAIL-SOFT, mandatory: any error in step 2 (transport, timeout,
  `Principal/get` method error, missing email on the returned principal)
  yields `owner_email: None` for that account and MUST NOT propagate out of
  `containers_list`. Rationale in Obstacle C - ratatoskr detaches and fails
  attach on a container-preparation error (`resident.rs:292-302`), so a
  best-effort metadata lookup that can fail the call turns a cosmetic
  degradation into an account outage. bifrost-side unit tests cover: no
  session capability -> `None`; principal error -> `None` and call still
  succeeds; principal with no email -> `None`; name fallback with and
  without `@`.
- Graph: populate from the `/users/{id}` owner key when it is addressable
  (contains `@`); IMAP: `None`.
- saehrimnir already stages JMAP foreign accounts (B12); verify a principals
  fixture exists or extend the mock so ALL THREE resolution outcomes are
  drivable (no session capability, principal hit, name fallback), plus a
  `Principal/get` error response for the fail-soft case.

## 6. Bricks, in landing order

Every brick: `brokkr check` green at its boundary, plus the named gates.
Contract rule 5 wants copy-pasteable commands, so the shorthands used below
are defined here literally and NOTHING below is left as a script family.

| shorthand | literal command |
| --- | --- |
| SERVICE-SUITE | `brokkr service-suite` |
| SYNC-ALL | `brokkr sync --all` |
| SYNC-IMAP | `brokkr sync --all --filter imap` |
| SYNC-GRAPH | `brokkr sync --all --filter graph` |
| SYNC-JMAP | `brokkr sync --all --filter jmap` |
| CHECK-ALL | `brokkr check --all` |

Two routing facts that an earlier draft got wrong (§ 2.7): the B15a
instruments are SYNC-harness scripts and run under `brokkr sync`, never
under `brokkr service-suite`; and the full sync sweep is `brokkr sync --all`,
which is a distinct harness from `service-suite`. `--filter` substring-matches
the discovered SCRIPT NAME, so SYNC-IMAP covers all nineteen `imap-*.lua`
including `imap-jmap-shared-state.lua` (which SYNC-JMAP also picks up -
harmless overlap, each gets its own run dir).

Single-script and benched forms, spelled out once:

- one sync script: `brokkr sync crates/app/tests/sync-harness/<NAME>.lua`
- one service script: `brokkr service-test crates/app/tests/service-harness/<NAME>.lua`
- one unit test: `brokkr test -p <crate> <NAME>`
- one benched gate: `brokkr sync <SCRIPT> --gate <GATE> --bench`

`--as-baseline` appears NOWHERE in B15 (Obstacle I).

### B15a. The instrument, and the affordance that makes it drivable

ONE gate, not two. `imap-keyword-capability.lua` is cancelled with
B15-SQ-imap (Obstacle A) - there is no point pinning a column B15b deletes.
What remains must be built against LEGACY behavior so B15c's rewire lands
against a pinned contract.

1. Harness affordance FIRST, per Obstacle J. Add a sync-harness command that
   sets the `graph_sync_cycle` settings counter to a caller-chosen value
   (through the same `sync::state` write path production uses) and then runs
   exactly one `run_aux_pass` for the account synchronously, acking on
   completion. Model: B8-groups' `test.group_pull` + `graph_groups_cadence.lua`.
   Without the synchronous ack the script is a sleep-race and will flake;
   with it the gate is deterministic. This affordance is test-only wiring in
   the harness handler surface, not a production behavior change.
2. `crates/app/tests/sync-harness/graph-reaction-refresh.lua`: seed a Graph
   account, let the initial aux pass import categories (recall the first pass
   early-returns after categories, § 2.2), stage a message with an owner
   reaction + count on the mock, set the cycle counter to 4 and drive one
   pass so the increment lands on 5, then assert both `message_reactions`
   rows (owner emoji row keyed on `accounts.email`, and the `__count__` row).
   Then clear the owner property on the mock, drive another cadence-crossing
   pass, and assert the OWNER row is gone.
   MUST NOT assert that the `__count__` row disappears: legacy has no delete
   branch for it (§ 2.8 bug 1) and such an assertion cannot go green on
   legacy code. Add a comment in the script naming § 2.8 so the omission
   reads as deliberate rather than forgotten.
   Optionally assert the § 2.8 bug 2 shape too (change the emoji, observe a
   SECOND owner row rather than an update) - pinning the bug makes the
   eventual fix visible as a gate change rather than a silent drift.

Gates:
- `brokkr sync crates/app/tests/sync-harness/graph-reaction-refresh.lua`
- SYNC-GRAPH (proves the new script does not disturb its neighbors)
- SERVICE-SUITE
- CHECK-ALL

### B15b. IMAP aux DELETION (no side-quest, no bifrost dependency)

Restructured from a port into a deletion by Obstacle A. This brick now
depends on nothing and may land first.

1. Delete the `run_aux_pass` IMAP arm's legacy session
   (`resident.rs:1149-1172` collapses to nothing - the arm keeps only the
   shared contact/group cadence tail), and `provider-sync/src/imap/`
   (aux_sync + mod).
2. Delete the write-only flag end to end:
   `db/.../account_sync_writes.rs::set_account_supports_keywords`, the
   `supports_keywords INTEGER` column at `schema/01_core.sql:50`, and
   `read_imap_folder_paths` if now caller-free. Reconcile dev-seed and any
   `TestQueryDbState` projection that touches the accounts row shape.
   Re-verify emptiness immediately before deleting:
   `grep -rn "supports_keywords" crates` must return only the lines this
   step removes.
3. Note in the commit message that per-mailbox keyword refusal
   (`imap/src/client/commands.rs:107,144`) is unaffected here and dies with
   `crates/imap` in B15e, with the bifrost follow-up filed per § 9. The
   deletion is only honest if the successor is named.

Gates:
- SYNC-IMAP (all nineteen `imap-*.lua`)
- SERVICE-SUITE
- CHECK-ALL
- `brokkr sync crates/app/tests/sync-harness/imap-steady-state-delta.lua --gate imap_steady_state_delta --bench`
  - expected UNCHANGED, not improved (Obstacle I). Movement here is
    stop-the-line. No `--as-baseline`, no `baseline_label` edit.

### B15c. Graph aux cut (needs B15-SQ-graph promoted)

1. Promote 5.2; record the freeze.
2. New `service/src/bifrost/aux/graph.rs`:
   - `category_label_rows(defs: &[CategoryDefinition], account_id: &str) ->
     Result<Vec<LabelWriteRow>, String>` - pure port of
     `graph::label_sync`'s mapping (`cat:` ids via
     `LabelKind::graph_category`, which is FALLIBLE, `preset_colors`
     resolution with the `"None"` preset mapping to `(None, None)`,
     `sort_order` = enumeration index) INCLUDING the four appended
     `ImportanceLevel` rows with `is_undeletable: true` (§ 2.2). Unit-pinned
     against the legacy output: `brokkr test -p service category_label_rows`.
   - `classify_reaction_updates(succeeded: &[MessageReactionState],
     account_id: &str, owner_email: &str) -> Vec<ReactionRowOp>` - pure port
     of the upsert/delete/count decision. Note the explicit `owner_email`
     input: legacy keys the owner row on `accounts.email`, read once per
     refresh (`graph/aux_sync.rs:110-120`), and the function is not pure
     without it. It takes ONLY the `succeeded` lane of the `BatchOutcome`;
     `failed` and `uncertain` items are logged and skipped, never turned
     into a delete (§ 5.2). Faithful to § 2.8: emit a delete op for an absent
     owner property, emit NO delete op for an absent count. Unit-pinned,
     with an explicit case asserting that a failed item produces zero ops.
   - `run_graph_auxiliary_sync(engine, ..)` driving both through
     `engine.category_definitions_list` / `engine.message_reactions`, keeping
     the initial-pass-only category import (including its early return) and
     the every-5th / every-20th cadence on `increment_graph_sync_cycle`,
     including the legacy candidate query (seeded-or-recent LIMIT 60)
     unchanged. Chunking to 20 moves INSIDE bifrost (§ 5.2), so the service
     side passes the whole id list.
3. Rewire the `run_aux_pass` Graph arm to it; delete
   `provider-sync/src/graph/` (aux_sync, mod, the empty `sync/` dir) and the
   resident GraphClient construction.

Gates:
- `brokkr sync crates/app/tests/sync-harness/graph-master-category-label-sync.lua`
- `brokkr sync crates/app/tests/sync-harness/graph-reaction-refresh.lua`
  (the B15a instrument, green ACROSS the cut - this is the whole contract)
- `brokkr test -p service category_label_rows`
- `brokkr test -p service classify_reaction_updates`
- SYNC-GRAPH
- SERVICE-SUITE
- CHECK-ALL
- `brokkr sync crates/app/tests/sync-harness/graph-steady-state-delta.lua --gate graph_steady_state_delta --bench`
  - expected UNCHANGED (Obstacle I); no re-record
- `brokkr sync crates/app/tests/sync-harness/graph-shared-mailbox-steady-state.lua --gate graph_shared_mailbox_steady_state --bench`
- `brokkr sync crates/app/tests/sync-harness/graph-public-folder-steady-state.lua --gate graph_public_folder_steady_state --bench`

### B15d. JMAP aux cut (needs B15-SQ-jmap promoted)

1. Promote 5.3; record the freeze.
2. Container persistence upserts `shared_mailboxes.email_address` from
   `Container.owner_email` for Shared-namespace rows via the existing
   `set_shared_mailbox_email` write. Three pins, per Obstacle C:
   - WRITE-ONCE. Write only when `owner_email` is `Some` AND
     `get_shared_mailbox_email` currently returns `None` for that
     `(account_id, mailbox_id)`. Never overwrite, never clear.
   - ORDERED. The write must execute AFTER
     `reconcile_namespace_registry`'s `INSERT INTO shared_mailboxes`
     (`bifrost/containers.rs:457-467`) within the same container-persist
     pass, because `set_shared_mailbox_email` is a bare UPDATE that silently
     no-ops on a missing row. Put the email write inside
     `reconcile_namespace_registry`'s owner loop, immediately after the
     upsert of that owner's row, so the ordering is structural rather than
     conventional and cannot drift.
   - PINNED AT TWO LEVELS. The pure row-mapping golden
     (`brokkr test -p service owner_email_populates_shared_mailbox_cache`)
     cannot catch a mis-ordering or a no-op UPDATE, so it is joined by an
     integration-level assertion that a Shared container carrying
     `owner_email` produces a NON-NULL `shared_mailboxes.email_address`
     after one container sync, plus a second pass asserting the value is not
     rewritten. `jmap-shared-account-sync.lua` is the natural host.
3. Delete the ShareNotification poll and shared-identity aux whole:
   `provider-sync/src/jmap/` (aux_sync + mod), the `run_aux_pass` JMAP arm's
   client construction (arm keeps only the cadence tail),
   `sync::state::{load,save}_jmap_sync_state{,_for}`.
4. Drop the `jmap_sync_state` surface from `schema/10_sync.sql` in the same
   landing (its last writer died here; splitting the drop to B15f would
   leave a zombie table across two boundaries for nothing): the table body
   (`10_sync.sql:15-23`), the unique index `idx_jmap_sync_state_shared`
   (`:24-25`), and the roll-call comment (`:37`). Reconcile dev-seed and the
   migrations note at `migrations.rs:50-52`.

Gates:
- `brokkr sync crates/app/tests/sync-harness/jmap-shared-account-sync.lua`
- `brokkr sync crates/app/tests/sync-harness/imap-jmap-shared-state.lua`
- `brokkr test -p service owner_email_populates_shared_mailbox_cache`
- SYNC-JMAP
- SERVICE-SUITE
- CHECK-ALL
- `brokkr sync crates/app/tests/sync-harness/jmap-steady-state-delta.lua --gate jmap_steady_state_delta --bench`
  - expected UNCHANGED (Obstacle I); no re-record
- `brokkr sync crates/app/tests/sync-harness/jmap-initial.lua --gate jmap_containers_attach --bench`
  - this brick adds work to the ATTACH path, which is exactly what this gate
    measures. A `Principal/get` per foreign account is a predicted, bounded
    increase; record the predicted count in the commit message BEFORE
    running, and treat any other number as stop-the-line. This is the one
    place in B15 where a `baseline_label` update may be warranted; make it a
    conscious, separately justified decision.
- No pop-out compose harness assert exists (survey found the read path only
  in `app/src/handlers/pop_out/window_lifecycle.rs:125`); step 2's
  integration assertion is the pinned instrument standing in for it.

### B15e. The deletion cut

One landing, ordered internally by the compiler:

1. Re-home live helpers per Obstacle E: `common::rfc822`,
   `service/src/bifrost/consumer/imap_mime.rs` (decided, not deferred), and
   the consumer `support/` modules MINUS the two dead members
   (`seen_ingest.rs` and
   `thread_membership::replace_thread_membership_from_full_coverage`, both
   deleted outright per § 2.3). Rewrite the `consumer_support` imports in
   `service/src/bifrost/consumer/write.rs:9-14` and
   `service/src/bifrost/consumer/hydrate.rs:15`, pointing the
   `FolderWriteRow` / `ensure_folder_rows` / `insert_folders_batch`
   pass-throughs straight at `db::db::queries_extra`. Fix the stale comment
   at `crates/service/src/eviction.rs:102` (NOTE the path: `eviction.rs`
   sits at the service crate root, NOT under `bifrost/`; § 2's `bifrost/`
   path convention does not apply to it).
2. Delete the dead action surface: `service/src/actions/provider.rs` whole
   (`create_provider`, `HarnessOfflineProvider`, the attachment map),
   `register_harness_attachment` call in `test_helpers.rs` (keep the DB-row
   seeding + ack), `common/src/ops.rs`, and the `ProviderOps`-only residue
   of `common/src/types.rs` (compiler-driven; keep `LabelKind`,
   `ImportanceLevel`, every type with a surviving consumer). Update the
   three referencing comments (2.1).
3. Delete `core`'s legs: `pub(crate) use graph;` (`core/src/lib.rs:18`), the
   two `enrich_*` GraphClient functions in `cloud_attachments.rs`,
   `pub use smtp;` (`core/src/lib.rs:38`), and in `core/Cargo.toml` the
   `smtp` (`:33`), `gmail` (`:36`), `jmap` (`:37`), `graph` (`:38`), and
   `bifrost-jmap` (`:51`) dep lines PLUS all four `smtp/hotpath`,
   `gmail/hotpath`, `jmap/hotpath`, `graph/hotpath` legs and their
   `-alloc` twins on lines 67-68. The `smtp` dep and its hotpath legs were
   missing from an earlier draft of this enumeration even though § 2.4
   correctly names smtp as dying; since this step enumerates exact lines it
   has to be complete.
4. Delete the crates: `crates/gmail`, `crates/jmap`, `crates/graph`,
   `crates/imap`, `crates/provider-sync`, `crates/smtp`; remove the six
   workspace members; prune `service/Cargo.toml` (four provider deps,
   `provider-sync`, the old `bifrost-jmap` git dep, the stale comment per
   Obstacle G).
5. Rename the alias: workspace `bifrost-jmap-new` -> `bifrost-jmap`
   (`Cargo.toml:118`, `service/Cargo.toml:57`, and every
   `bifrost_jmap_new::` use path). `Cargo.lock` drops the
   `file:///home/folk/Programs/jmap-client` source entirely - verify with
   `grep -n "jmap-client" Cargo.lock` (expect zero).
Gates:
- CHECK-ALL (full-workspace, no scope filter - a deletion cut's diagnostics
  land outside changed files)
- SERVICE-SUITE, 63/63
- SYNC-ALL (every provider family - this brick touches the consumer's shared
  re-parse path, so every hydration golden and `bifrost-consumer-*`
  durability script must hold)
- `brokkr sync crates/app/tests/sync-harness/bifrost-consumer-hot-path.lua --gate bifrost-consumer-hot-path --bench`
- The four steady-state benches, all UNCHANGED (this brick must be
  request-neutral; no re-record):
  - `brokkr sync crates/app/tests/sync-harness/imap-steady-state-delta.lua --gate imap_steady_state_delta --bench`
  - `brokkr sync crates/app/tests/sync-harness/jmap-steady-state-delta.lua --gate jmap_steady_state_delta --bench`
  - `brokkr sync crates/app/tests/sync-harness/graph-steady-state-delta.lua --gate graph_steady_state_delta --bench`
  - `brokkr sync crates/app/tests/sync-harness/gmail-steady-state-delta.lua --gate gmail_steady_state_delta --bench`
- `brokkr service-test crates/app/tests/service-harness/parent_sigkill.lua`

### B15f. Schema residue, the § 1 audit, and B16 docs

1. Drop the remaining retired schema: `folder_sync_state`,
   `graph_folder_delta_tokens`, `jmap_push_state`, `graph_subscriptions`
   tables, and their orphaned helpers (`ai_state.rs` in both `db` and
   `db-read`, `sync/src/pipeline.rs::clear_all_folder_sync_states`,
   `sync/src/state.rs` graph-token fns); dev-seed + migrations-comment
   reconciliation. Confirm § 2.5's "no `.rs` references" claims for the
   B12-era tables and drop any schema leftovers found.
1a. `accounts.history_id`, scoped properly per § 2.5. This is FOUR edits, not
   a helper deletion, and it touches the boot path, so it is called out
   separately and gated separately:
   - Delete the dead write/read helpers:
     `db/.../account_sync_writes.rs::{set_account_history_id,
     get_account_history_id}` and
     `sync/src/state.rs::{save_account_history_id, save_account_history_id_sync,
     load_account_history_id}`. All are caller-free today; re-verify with
     `grep -rn "account_history_id" crates` before cutting.
   - PRESERVE the live reset. `sync::pipeline::clear_account_history_id` is
     called by the boot invariant pass and its load-bearing effect is
     `initial_sync_completed = 0`, NOT the column. Rename it and its `db`
     helper to say so - `reset_initial_sync_state` /
     `clear_account_initial_sync_completed` - drop `history_id = NULL` from
     the UPDATE, and update the three call/log sites
     (`startup_invariants.rs:9,228,233`, the `history_ids_cleared` stat name
     at `:65,:231,:288`, and the prose comments at `:239`, `boot.rs:1393`,
     `service/src/calendar.rs:26`, `schema/10_sync.sql:132-134`).
   - Strip the struct field in BOTH crates: `db/src/db/types.rs:28`,
     `db-read/src/db/types.rs:28`, each crate's `from_row_impls.rs:21`
     column list, and each crate's `queries_extra/accounts_messages.rs:14`
     row read.
   - Drop the column at `schema/01_core.sql:11` and reconcile dev-seed.
   Gate this sub-step explicitly, since it is the only boot-path edit in
   B15: `brokkr service-test crates/app/tests/service-harness/parent_sigkill.lua`
   and `brokkr service-test crates/app/tests/service-harness/respawn_after_sigkill.lua`,
   both of which exercise a non-graceful exit and therefore the dirty-account
   invariant pass. If neither actually asserts the post-crash resync, add the
   assertion rather than assuming coverage.
1b. Relocate, do not edit, the no-writer regression test.
   `push_state_tables_have_no_writer` (`resident.rs:1341-1368`) is a
   SOURCE-TEXT scan over nine `include_str!`ed files, not a DB test (§ 2.5),
   so "strengthen into a schema-absence assert" means writing a NEW test in
   the `db` crate that opens a real migrated connection and asserts each
   retired table is absent from `sqlite_master`, then deleting the text scan.
   Destination: `crates/db/src/db/migrations.rs`'s test module, alongside the
   existing v100 schema tests. Cover all seven retirees:
   `jmap_push_state`, `graph_subscriptions`, `jmap_sync_state`,
   `folder_sync_state`, `graph_folder_delta_tokens`,
   `public_folder_sync_state`, `graph_shared_mailbox_delta_tokens` - plus the
   two dropped columns, asserted absent from `PRAGMA table_info(accounts)`.
2. The mechanical audit the TODO mandates, as a reviewable script committed
   at `scripts/b15-audit.sh` - DECIDED, not the implementer's call: the
   audit's value is that it can be re-run after the next deletion item, and
   a one-shot deleted after review cannot be. It walks every `crates/*/
   Cargo.toml` and module tree; flag (a) any dependency with a bifrost
   equivalent (`reqwest` outside service/common http plumbing, `lettre` -
   Obstacle 2.4's disposition decided here, `async-imap`-family remnants,
   any `jmap`/`imap`/`graph`/`gmail` string in a dep name), (b) any module
   whose name or doc claims provider transport duty, (c) any
   `RATATOSKR_TEST_*` env consumer that no longer exists. Every flag is
   either deleted in this brick or retained with a one-line rationale in the
   commit message. Known items to adjudicate: `lettre` (drop if the
   `to_bifrost_send_request` parse covers `send.rs`'s `Mailbox` use; else
   retain with rationale), `common::http` / `common::token` /
   `common::test_endpoint` (delete if their only consumers died with the
   legacy clients - survey suggests yes, compiler decides),
   `core/src/caldav/` parse module (touched by B8 notes as dormant - delete
   if caller-free), `core/src/discovery/jmap_wellknown.rs` (live discovery,
   stays - it serves account setup, not a provider transport).
3. B16, bundled: reconcile `reference/architecture.md` (provider sections,
   crate map, action-pipeline provider references), `AGENTS.md` (the "Four
   email providers ... `ProviderOps` trait (`common/src/ops.rs`)" gotcha and
   the jmap-client gotcha section are now false; the crate list drops the
   deleted crates), `reference/glossary/folders-labels.md` (keyword-flag
   derivation, master-category source), `reference/glossary/harness.md` (new
   gates), and `docs/bifrost-migration.md` itself (B15/B16 done-notes, its § 11
   freeze narrative for the two side-quests). `folders-labels.md` records
   that the IMAP keyword flag is GONE (not "derived") and points at the
   bifrost follow-up. Markdown rides the code commit.

Gates:
- CHECK-ALL
- SERVICE-SUITE
- SYNC-ALL
- `brokkr sync crates/app/tests/sync-harness/contacts_cadence.lua --gate contacts_cadence --bench`
- The four containers-attach gates (schema edits touch their tables'
  neighbors). NOTE their scripts are the `*-initial.lua` files, not files
  named after the gates:
  - `brokkr sync crates/app/tests/sync-harness/gmail-initial.lua --gate gmail_containers_attach --bench`
  - `brokkr sync crates/app/tests/sync-harness/graph-initial.lua --gate graph_containers_attach --bench`
  - `brokkr sync crates/app/tests/sync-harness/imap-initial.lua --gate imap_containers_attach --bench`
  - `brokkr sync crates/app/tests/sync-harness/jmap-initial.lua --gate jmap_containers_attach --bench`
- `brokkr service-test crates/app/tests/service-harness/parent_sigkill.lua`
  and `respawn_after_sigkill.lua` (step 1a's boot-path edit)
- A fresh `cargo run -p app` dev-seed boot (schema edit smoke; dev-seed
  re-seeds from scratch every launch)

## 7. Keep/revert and ordering

Each brick is one coherent landing kept or reverted on its gates; no env
switches, no probes. Hard edges:

- B15b depends on NOTHING (it is a pure deletion after Obstacle A) and may
  land first. B15c and B15d each depend only on their own side-quest
  promotion. All three are mutually independent, MAY land in any order among
  themselves, and all three precede B15e (Obstacle H).
- B15a precedes B15c only (its gate is B15c's contract). It no longer gates
  B15b, whose subject is deleted rather than ported.
- B15e precedes B15f (the audit walks the post-deletion tree).
- A side-quest is done only when its consuming brick's gates pass against
  the promoted `../bifrost` (B12 rule); a promoted-but-ungated side-quest
  blocks its brick, not the tree (additive surfaces, bifrost green).
- Revert unit = the brick. B15e's revert restores the crates wholesale (git
  revert of one commit); nothing in B15f is reachable before B15e lands.

## 8. Stopping rule

- No app-crate or service-api wire changes. If a deletion appears to demand
  one, the survey missed a consumer - stop and re-survey.
- No calendar, contacts, groups, send, verify, or push behavior changes:
  those seams are done items; B15 touches only what § 2 inventoried.
- The bundling/filters/smart-labels "unwired at sync time" state (B3 note)
  is NOT B15's to change.
- `pending_operations`, `clean_shutdown_cursors`, `shared_mailboxes`,
  `sync_cursors`, `seen_ingest_markers`, the settings cadence counters:
  retained, not in scope.
- bifrost-side: the two side-quests are additive surface only; no
  refactors of bifrost internals ride them. The bifrost-imap keyword-refusal
  fix (Obstacle A) is a behavior change and is explicitly NOT one of them.
- No fix to the two `message_reactions` bugs in § 2.8. They are ported
  faithfully; the instrument pins legacy behavior; the fix is § 9's.
- ONE exception to "no boot behavior changes" is granted, narrowly: B15f
  step 1a renames `clear_account_history_id` and narrows its UPDATE to the
  half that was ever load-bearing. This is required to drop a column whose
  writers are dead, it is gated by the two sigkill service-harness scripts,
  and it changes no observable behavior. Any OTHER boot-path edit that
  surfaces during B15 is a stop-and-re-survey.

## 9. Out of scope, named not deferred

- The bifrost `LiveSupersedes` no-op (the TODO's § 11 lateral finding) - a bifrost
  side-quest of its own.
- The frozen-bifrost Gmail bulk un-spam divergence and bulk-star
  non-coalescing (B4a follow-ups) - separate items.
- The JMAP `$draft`-after-submission bifrost send-fidelity follow-up
  (B5-GATES finding).
- The Gmail multi-message-thread partial-delta sibling gate (B3a-cut-gmail
  accepted gap) - separate follow-up.
- A user-facing folder/label CRUD UI (B6 out-of-scope carry).
- Auto-firing bundling/filters/notifications on new mail - product item.
- Emitting iMIP REPLY for unresolvable invites (B7b carry).

Added by the R1/R2 review reconciliation; each is a direct consequence of a
B15 decision above, so each is named here rather than left implicit:

- BIFROST-IMAP KEYWORD REFUSAL. bifrost-imap hardcodes
  `mutation.set_keyword: true` (`imap/src/account/capabilities.rs:66`) while
  already parsing PERMANENTFLAGS (`imap/src/types/mailbox.rs:438`). It should
  decline a `set_keyword` against a mailbox whose PERMANENTFLAGS lack `\*` -
  reporting it as a per-item `BatchFailure` rather than silently succeeding.
  This is the real successor to the legacy per-mailbox gate at
  `imap/src/client/commands.rs:107,144` that dies with `crates/imap` in
  B15e. A bifrost bugfix side-quest, not a ratatoskr projection, and NOT a
  B15 prerequisite (Obstacle A).
- `message_reactions` DATA-CORRECTNESS FIX (§ 2.8). Two bugs, one item: the
  `__count__` row has no delete branch, and
  `upsert_message_reaction_update_type`'s `ON CONFLICT ... DO UPDATE SET
  reaction_type` can never fire because `reaction_type` is inside the
  conflict key, so changed emojis and changed counts insert duplicate rows
  instead of updating. The fix needs a unique-key change (drop
  `reaction_type` from the key, add `source`) or a delete-then-insert, a
  count-row delete branch, a v100 schema edit under the pre-release policy, a
  one-shot cleanup of accumulated rows, and tests for both value-change and
  removal. B15a's instrument pins the current behavior so the fix shows up as
  a deliberate gate change. Out of B15 because § 8 forbids behavior changes
  in a deletion cut.
- AN ATTACH-WINDOW / AUX-PASS REQUEST BENCH. B15b really does remove N
  SELECTs plus a dedicated LOGIN per 300s aux pass, and B15d really does add
  a `Principal/get` per foreign account at attach. No existing instrument can
  see either: the four `*_steady_state_delta` gates measure a delta kick with
  the aux pass explicitly waited out and cleared (§ 2.7). A gate that
  measures the attach + first-aux window would make both movements visible.
  Out of B15 because building it is its own work with its own flakiness
  surface, and neither movement is a B15 acceptance criterion (Obstacle I).

## 10. Lateral findings surfaced by this survey

Recorded for the orchestrator; none block B15:

- `service/src/actions/provider.rs`'s harness attachment map has been
  write-only since B9: `test.seed_remote_attachment` still stashes bytes
  nobody can read (the only reader sat behind the uncalled
  `create_provider`). Any service-harness script that believed it was
  staging FETCHABLE bytes through that map has been passing for other
  reasons; B15e's suite run will prove the map's deletion is invisible.
- `core/Cargo.toml` has carried unused `gmail`, `jmap`, and old
  `bifrost-jmap` deps for at least one item cycle - dead compile weight on
  every `rtsk` build until B15e.
- `provider-sync/src/graph/sync/` is an empty directory left by B12's
  deletion.
- The `service/Cargo.toml` header comment (Obstacle G) and
  `eviction.rs:102` comment reference modules deleted items ago -
  documentation rot inside the tree, fixed by B15e.
- `graph_folder_delta_tokens` helpers in `sync/src/state.rs` appear
  caller-free already (B12 deleted the last writer); if confirmed during
  B15f they could have been deleted at B12e - a residue-sweep miss worth a
  loop note.

Added by the R1/R2 review reconciliation:

- `accounts.supports_keywords` is a closed write-only loop: probe writes it,
  setter sets it, nothing reads it (§ 2.9). It has narrated a behavior that
  actually lived per-mailbox in `imap/src/client/commands.rs` since it was
  introduced. Worth a loop note on how a column acquires a purpose in prose
  that it never had in code - the earlier draft of this very spec inherited
  the fiction and nearly spent a bifrost API on it.
- `consumer_support.rs`'s doc comments are stale in BOTH directions:
  `replace_message_folders_and_recompute` is marked "reserved" but is live in
  `write.rs`, while `ingest_from_messages` is exported as "the canonical
  entry point" for cut specs that never used it. Comment rot that directly
  mis-informed the first draft's re-homing list (§ 2.3).
- `upsert_message_reaction_update_type`'s conflict clause is dead code -
  `DO UPDATE SET reaction_type = ?4` on a key containing `reaction_type`
  (§ 2.8 bug 2). Any future upsert helper written to this shape has the same
  defect; worth a grep for the pattern across `queries_extra`.
- `OutlookCategory.id` is deserialized and `#[allow(dead_code)]`d in
  `graph/src/label_sync.rs:11-16`. It dies with the crate; noted only so the
  B15c port is not written expecting a category id it never had.
- `resident_aux_loop`'s doc comment (`resident.rs:1040-1043`) says the 300s
  cadence was "chosen to match the legacy per-kick cadence, which was driven
  by the app's 5-minute `SyncTick`". That is correct and is precisely what
  made "every kick" a plausible-sounding error - the two cadences coincided
  historically but no longer share a driver. Worth a note wherever aux
  cadence is documented.

## 11. Review reconciliation: findings NOT folded in

R1 (Opus) and R2 (codex gpt-5.6-sol) were reconciled into the body above.
Their two blocking findings were the same finding twice (`history_id`) plus
one each on `supports_keywords` and the Graph instrument; all are accepted.
What follows is what was rejected or narrowed, with the reason, so a later
reader does not re-litigate it.

1. REJECTED, R2 finding 8, the claim that ALL `consumer_support` exports are
   dead residue. `replace_message_folders_and_recompute` is imported and used
   by `service/src/bifrost/consumer/write.rs:9-14` despite the "reserved"
   comment R2 cites; the comment is stale, the code is live. The accepted
   half - `ingest_from_messages` and
   `replace_thread_membership_from_full_coverage` really are dead - is folded
   into § 2.3 and Obstacle E.

2. REJECTED AS A REMEDY, R2 finding 2, "this needs an explicit production
   bugfix brick" for the `message_reactions` defects. The defects are real
   and are now documented in § 2.8, but fixing them inside B15 contradicts
   § 8: B15 is a deletion and rewire cut, and a data-correctness fix needs a
   unique-key change, a schema edit, a cleanup of already-accumulated rows,
   and its own tests. Folding a behavior change into a brick whose gate is
   "nothing changed" would make the port unprovable. Accepted instead: the
   instrument is written against real legacy behavior (B15a explicitly must
   not assert `__count__` removal), and the fix is a named § 9 follow-up with
   its shape spelled out.

3. NARROWED, R1 finding 3's framing that dropping `history_id` is "arguably
   forbidden" by § 8's stopping rule. § 8 as written forbids app-crate and
   service-api WIRE changes; a boot-path helper rename is neither. The
   substance of the finding - that the column was under-surveyed and the
   drop is a real multi-crate edit touching the invariant pass - is fully
   accepted and rewritten as § 2.5 and B15f step 1a. § 8 now grants this one
   boot-path exception explicitly rather than leaving it arguable.

4. NARROWED, R2 finding 1's reading of `schema/10_sync.sql:132`. That comment
   belongs to `clean_shutdown_cursors` and describes ITSELF as
   "defense-in-depth, not load-bearing", naming the history_id clear as the
   correctness backstop it leans on. So it corroborates the RELIANCE (which
   is why B15f step 1a must update it) but it is not, as R2 implies, an
   independent schema-level claim that the COLUMN is load-bearing. The column
   is not; the `initial_sync_completed = 0` in the same statement is.

5. REJECTED, R1's proposed fallback of keeping `B15-SQ-imap` in reduced form
   if per-mailbox gating is a real requirement. It is a real requirement, but
   projecting a per-mailbox verdict out to ratatoskr so ratatoskr can AND it
   into an account-wide flag and hand it back is the wrong direction of
   travel: bifrost issues the STORE, bifrost knows the mailbox, bifrost
   should decline. R2's independent conclusion (§ 3 of R2: "preferably per
   folder rather than reducing it to an account-wide flag") points the same
   way. Folded as a bifrost follow-up in § 9, not as a B15 side-quest.

6. NOTED, NOT ACTIONED: R1's observation that both reviews' remaining "what
   checks out" claims were verified independently against the tree during
   this reconciliation - `create_provider`'s zero callers, the `ProviderOps`
   implementor set, the absent `use bifrost_jmap::` in service and core, the
   dep-line locations, the empty `provider-sync/src/graph/sync/`, the
   bifrost-side absences (`keywords_supported` / `owner_email` /
   `masterCategories` are nowhere in the frozen tree), and the gate-name and
   command-form citations. No correction needed; recorded so the next
   reviewer can skip re-deriving them.
