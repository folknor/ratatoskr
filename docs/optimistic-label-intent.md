# Optimistic Label Intent

Design slice for the remaining contract #4 follow-up caveat in `docs/glossary/discrepancies.md`. Spawned from the merge-vs-replace migration; not a sub-task of it.

## Background

Under contract #4, the merge-vs-replace helper selection migration is complete: provider paths with full coverage call provider-local replace wrappers, partial-delta paths call merge wrappers, and `db::queries_extra::thread_persistence` exposes only raw row primitives. One window remains open: local label actions write `thread_labels` (and `thread_label_groups` for group composites) directly before the provider echoes the change. Any provider-truth path that DELETE+INSERTs `thread_labels` for that thread - the IMAP/JMAP keyword recompute, the IMAP full-thread replace, the Gmail full-thread replace - can erase the optimistic row before echo arrives.

This document specifies the slice that closes the window.

## Promise

A label optimistically added by a local action stays visible across every user-facing label-membership query until the action either succeeds and provider truth catches up, or fails permanently. On permanent failure, the rendered state transitions back: the label that was optimistically shown is no longer shown, surfaced through the existing toast/undo path. Sync recompute paths never observe or overwrite optimistic intent; user-facing queries always merge provider truth with the pending-intent overlay. List queries and count queries see the same merged set.

## Non-goals

- `StoredSecret` strictness migration. Sequenced before this slice but tracked separately in `docs/contracts-roadmap.md` § Remaining open questions ("Legacy plaintext credentials").
- Cross-client folder/label move reconciliation. Tracked in `TODO.md`; cured by the per-message membership store pattern from `docs/architecture.md`.
- Tantivy label-token indexing. Tantivy does not index labels today (no `label_id` / `label_name` fields in `SearchDocument`); operator-bearing `label:` filters are SQL-only as a consequence, not as an explicit retirement.

## Data model

### Generation counter

A persisted per-thread counter advances every time **provider-truth** label membership for that thread changes. Local optimistic writes do not bump it. The overlay records the generation observed at intent-write time and clears once the live generation has advanced past that snapshot.

- Storage: `threads.label_membership_generation INTEGER NOT NULL DEFAULT 1`. The migration backfills existing rows to 1 (not 0) so that the default value cannot be mistaken for a captured snapshot from before the column existed. Overlay rows inserted at stage 2 capture the live counter value at insert time; clearance compares strictly greater than that snapshot.
- Single per-thread counter on `thread_labels` provider-truth writes. Group-level rendered membership (`thread_label_groups`) is action-service-only state today and is retired in stage 3 (see below); after stage 3, the only relevant provider-truth write target is `thread_labels`, so one counter is sufficient.
- Relationship to `core::generation::GenerationCounter<T>`: distinct. The in-memory branded counter is a runtime invalidation mechanism. This is a SQL-persisted column with cross-transaction semantics. Adjacent patterns, not the same one.
- Atomicity: the bump must be the last statement before commit in the same transaction as the provider-truth membership write. Sequence: `thread_labels` write, counter increment, commit. Out of order, or split across transactions, reintroduces the current bug at a different layer.

#### `thread_label_groups` and the single-counter claim

`thread_label_groups` is written today only by the action service (`crates/service/src/actions/label_group.rs:97` INSERT, `:146` DELETE) as an optimistic group-membership write at the start of composite "add label group" / "remove label group" operations. The composite then fans out to per-member `add_label_with_provider_no_enqueue` / `remove_label_with_provider_no_enqueue` calls. No provider-sync path writes `thread_label_groups` (verified by inspection; the keyword-membership helper's doc comment explicitly excludes it).

`label_group_rendered_fragment` in `crates/smart-folder/src/sql_builder.rs:443` is `EXISTS thread_label_groups OR EXISTS thread_labels JOIN label_group_members`. The first branch is the optimistic shortcut; the second branch is the derived rendering from individual label membership. Stage 3 retires the direct `thread_label_groups` writes (the composite's `INSERT OR IGNORE` / `DELETE` at lines 97 and 146) along with the action service's direct `thread_labels` writes. After stage 3, group-level rendered membership derives from the overlay-aware label set; the `thread_label_groups` table either becomes dead or is retained as a denormalized cache without a writer. The first OR-branch of `label_group_rendered_fragment` is dropped in the same stage.

This means a single counter on `thread_labels` provider-truth writes is correct *under the post-slice model*. Stage 1 and stage 2 do not depend on the retirement; they only require that the counter not be polluted by local writes (it isn't).

Stage 1 audit produces two enumerated lists.

**Paths that bump (provider-truth `thread_labels` writes):**

- `provider-sync::keyword_membership::recompute_thread_keyword_labels` (IMAP and JMAP keyword paths).
- `provider-sync::imap::thread_store::replace_full_thread_labels` (IMAP `imap_initial` and `imap_delta`).
- `provider-sync::gmail::sync::storage` full-thread replace (Gmail full-thread persistence).
- `provider-sync::graph::sync::persistence::merge_partial_delta_labels` (Graph partial-delta merge).

These are the known instances at design time. The stage 1 audit verifies the list is complete by grepping every direct INSERT/DELETE/UPDATE against `thread_labels` and confirming each one is either in this list or in the next.

**Paths that explicitly do NOT bump (local optimistic writes, slated for removal in stage 3):**

- `crates/service/src/actions/label.rs:46` add path.
- `crates/service/src/actions/label.rs:293` remove path.
- `crates/service/src/actions/label_group.rs:97` group composite INSERT (writes `thread_label_groups`, not `thread_labels`, but retired together).
- `crates/service/src/actions/label_group.rs:146` group composite DELETE (same).

These paths remain unchanged through stage 1 and stage 2; stage 3 removes them and the overlay takes over their UI role. Until then, omitting the bump from them is what keeps the counter a clean provider-truth signal that overlay clearance can rely on.

### Overlay table

Working assumption pending the queue-vs-table artifact below.

```sql
CREATE TABLE pending_thread_label_intents (
  account_id      TEXT    NOT NULL,
  thread_id       TEXT    NOT NULL,
  label_id        TEXT    NOT NULL,
  op              TEXT    NOT NULL CHECK (op IN ('Add', 'Remove')),
  generation_seen INTEGER NOT NULL,
  action_id       TEXT,
  created_at      INTEGER NOT NULL,
  updated_at      INTEGER NOT NULL,
  PRIMARY KEY (account_id, thread_id, label_id)
);

CREATE INDEX idx_pending_intents_action_id  ON pending_thread_label_intents(action_id);
CREATE INDEX idx_pending_intents_updated_at ON pending_thread_label_intents(updated_at);
```

The `action_id` index supports action-failure clearance (`DELETE WHERE action_id = ?`). The `updated_at` index supports the TTL backstop sweep (`DELETE WHERE updated_at < ?`). Both clearance paths must avoid table scans on the row volumes this table will see during heavy multi-account use.

- Unique key: `(account_id, thread_id, label_id)`. `op` is an attribute, not part of the key. Last write wins; an `Add` followed by a `Remove` for the same triple updates the existing row rather than producing two contradictory rows.
- `generation_seen`: the value of `threads.label_membership_generation` at intent-write time. Drives the clearance rule for async-echo provider paths.
- `action_id`: pointer to the *currently winning* action-queue entry for this key. Drives action-failure clearance: a permanent failure for the action that owns the overlay row deletes the row. A permanent failure for an action whose intent was already superseded by a later write (and whose `action_id` is no longer in the overlay) is a no-op for the overlay - the user has moved on, the failure surfaces through the toast/undo path but does not change the rendered state. This is correct UX, not a bug.
- Not a full event log. If history is ever needed, the action queue carries it.

## Lifecycle

### Insert and update

A local optimistic label action upserts the intent row capturing the current `threads.label_membership_generation` as `generation_seen`. The upsert always happens at action attempt; the relationship to the action queue depends on the action outcome (see Lifecycle paths below).

### Lifecycle paths

The current action-service contract is: successful online actions do not enqueue a `pending_operations` row; the queue is reserved for retryable failures. The overlay row's `action_id` column is nullable to accommodate this. Three lifecycle paths:

1. **Immediate provider success.** Overlay row written with `action_id = NULL`. No `pending_operations` row created. The overlay row stays until the generation advances and truth matches (clearance via §Cleanup executor item 1 or 2).
2. **Retryable failure.** Overlay row written with `action_id = <new pending_operations.id>` in the same transaction as the queue row. The queue's retry loop runs. On eventual success, the queue row clears but the overlay row remains until clearance. On exhausted retries, the action transitions to permanent failure and the overlay row clears via `DELETE WHERE action_id = ?`.
3. **Immediate permanent failure** (no retry). Overlay row is never written, or is deleted in the same transaction that surfaces the failure to the user. Renderer sees no optimistic state.

`action_id` is therefore set iff the action took the retryable-failure path. A NULL `action_id` is normal, not an error.

### Exclusive label sets

Some operations are structurally a swap rather than an addition. The action service's current Graph importance handler (`crates/service/src/actions/label.rs:41-46`) removes the opposite level (`high` removes `low` and `normal`, etc.) before inserting the new one. Under the post-slice model, the action handler emits multiple overlay intents atomically with a single shared `action_id`:

- `Add(importance:high)` and `Remove(importance:low)` and `Remove(importance:normal)` for the set member being added.
- All three rows share the same `action_id`. Permanent failure deletes all three via the `action_id` index.
- The renderer's merge algebra handles the rest correctly: `overlay_aware_labels` reflects the intended exclusive state immediately.

The same pattern generalizes to any other exclusive label set encoded by the action service. Stage 2's overlay write path takes a list of `(label_id, op)` pairs per action, not a single pair.

### Clearance rule (uniform across providers)

Under the post-slice model, no path writes `thread_labels` directly from the action service; provider-sync owns every `thread_labels` write. Action success alone never clears an overlay row directly; the bump that signals "provider truth has caught up" comes from sync (whether from a delta poll, a full replace, or an action-triggered recompute - see Provider convergence model below).

All five provider paths follow the same clearance rule: **clear when `threads.label_membership_generation > intent.generation_seen` AND the resulting `thread_labels` row state matches `intent.op`.** Strict inequality on the generation: the row must observe a provider-truth bump that happened *after* the intent was captured. Both conditions must hold; the generation bump alone (without the truth-match check) would clear overlay rows on unrelated label changes on the same thread.

The strict-inequality comparison and the migration backfill to 1 (not 0) interact: a default-valued row cannot be mistaken for a captured snapshot, because any subsequent provider-truth bump produces a generation strictly greater than the snapshot.

### Provider convergence model

The clearance rule depends on provider-truth catching up to the local intent. Not every provider has a recompute path that subtracts; the model below names how each one converges. All five paths still bump the generation counter and route writes through `provider-sync`; the difference is what triggers the write.

- **Gmail full-thread replace**: convergence on the next full-thread persist. The replace handles add and remove symmetrically because it overwrites the whole `thread_labels` row set for the thread.
- **IMAP keyword recompute** (`provider-sync::keyword_membership::recompute_thread_keyword_labels`): the recompute does a DELETE-then-INSERT against `thread_labels` keyed on the per-message `message_keywords` union. Add and remove both converge because both reshape the union.
- **IMAP full-thread replace** (`thread_store::replace_full_thread_labels`): symmetric for the same reason as Gmail.
- **JMAP keyword recompute**: same shape as IMAP keyword.
- **Graph partial-delta merge** (`merge_partial_delta_labels`): **asymmetric** - the merge is INSERT-only and cannot remove. Without intervention, Graph remove intents would never clear via the generation-advance rule, only via TTL. The slice closes this with an action-triggered recompute: on action success, the Graph action handler calls a provider-sync-owned `recompute_thread_truth(account_id, thread_id)` that issues a full-thread refetch (or applies a targeted base-truth delta, depending on what Graph supports cheaply), writes `thread_labels`, and bumps the generation in the same transaction. The action handler does not write `thread_labels` directly; it calls into `provider-sync`. The "provider-sync is the sole writer of `thread_labels`" rule is preserved.

For Graph add intents, the standard `merge_partial_delta_labels` path on the next poll converges naturally; the action-triggered recompute is only needed for removes.

For Graph category and importance operations: same as labels. Importance has additional structure (exclusive set) handled by the action handler emitting multiple overlay intents atomically - see Exclusive label sets below.

### Failure clearance

- Permanent action failure: clear via `DELETE WHERE action_id = ?` (uses the `action_id` index), surface failure through the existing toast and undo path. The delete is no-op if the action has been superseded - see the `action_id` semantics in Overlay table above.
- Retryable or offline failure: retain while the action queue holds the entry as live.

### Cleanup executor

The clearance check is read-by-someone, not a passive property of the table. Provider-sync never reads or writes the overlay (see Provider-sync boundary below), so it cannot run the check itself. The cleanup is layered for durability:

1. **Primary path (signal-driven, low-latency).** After a provider-sync membership transaction commits, it emits a `MembershipChanged { account_id, thread_id }` signal. An overlay-cleanup task subscribes, opens its own transaction, reads the affected thread's current `threads.label_membership_generation` and `thread_labels` state, scans overlay rows for that `(account_id, thread_id)`, and deletes rows whose `generation_seen <` the current generation AND whose intent matches the resulting truth.
2. **Backstop sweep (periodic, durability-providing).** A periodic task (every few minutes) sweeps the overlay table for rows whose `generation_seen <` the current per-thread generation AND whose intent op matches the current `thread_labels` row state, applying the same two-condition clearance rule from §Clearance rule. Generation advance alone is not sufficient - an unrelated label change on the same thread bumps the generation without satisfying any specific intent's truth match. The sweep makes the signal an optimization for latency rather than a correctness dependency: a lost signal extends the optimistic window by at most the sweep interval, not by the TTL.
3. **TTL tripwire (long, alarms on fire).** A long TTL (default 48h) scans by `updated_at` index and deletes stuck rows, emitting a warning log with `action_id`, `account_id`, and `thread_id` on every fire. This catches design bugs that escape both 1 and 2; it is not a normal cleanup mechanism and should be 0 fires per day in steady state.

Provider-sync's only interactions with the overlay are the in-transaction bump (§Generation counter) and the post-commit signal emission. It does not read overlay rows, does not delete them, and does not need to be aware of them existing.

## Read path

### Witness types

```rust
// In the overlay crate. Both types pub(crate); external code never names them.
pub(crate) struct IncludeOverlay(());
pub(crate) struct BaseProviderTruthOnly(());
```

External crates do not see the witness types and cannot construct them. They call per-mode public functions exposed by the overlay crate:

```rust
// Public API of the overlay crate
pub fn read_thread_labels_for_user(account_id: &str, thread_id: &str) -> Vec<LabelId> {
    read_thread_labels_internal(account_id, thread_id, IncludeOverlay(()))
}

pub fn read_thread_labels_for_sync(account_id: &str, thread_id: &str) -> Vec<LabelId> {
    read_thread_labels_internal(account_id, thread_id, BaseProviderTruthOnly(()))
}

// Internal helper, generic over the witness
fn read_thread_labels_internal<W: MembershipReadCapability>(
    account_id: &str, thread_id: &str, witness: W,
) -> Vec<LabelId> { /* ... */ }
```

`MembershipReadCapability` is a crate-private sealed trait that the two witness types impl. The witness never leaves the overlay crate; it is constructed inside the per-mode function and consumed by the internal helper. There is no public way to manufacture a witness, so no external caller can mix modes.

### Merge algebra

For a given `(account_id, thread_id)`, overlay-aware membership is:

```
overlay_aware_labels(t) = provider_truth_labels(t)
                          ∪ { intent.label_id : intent.op = Add }
                          \ { intent.label_id : intent.op = Remove }
```

`provider_truth_labels(t)` is the row set in `thread_labels` for that `(account_id, thread_id)`. The Add/Remove sets come from the overlay rows for that key with the corresponding `op`. The `(account_id, thread_id, label_id)` uniqueness constraint guarantees a given label is in at most one of the Add or Remove sets, never both.

Label-group rendered membership for a thread `t` and group `g`:

```
renders_group(t, g) =
   ∃ label ∈ overlay_aware_labels(t) : label ∈ label_group_members(g)
   ∨ ∃ row ∈ thread_label_groups : row.account=t.account ∧ row.thread=t.id ∧ row.group=g
                                                      ↑ second branch dropped in stage 3
```

The current `label_group_rendered_fragment` in `crates/smart-folder/src/sql_builder.rs:443` is the `EXISTS thread_label_groups OR EXISTS thread_labels JOIN label_group_members` shape, matching both branches. Stage 3 retires the `thread_label_groups`-as-rendering-source branch (along with the action-service writes that populate it), simplifying the fragment to the first branch only, applied against the overlay-aware label set.

Stages 1 and 2 leave the SQL fragment untouched; stage 3 is where the rewrite lands.

### Cross-crate fidelity decision

The contracts roadmap's standing answer (`docs/contracts-roadmap.md:39`) for cross-crate capability tokens on contract #4 is **option 4: restructure ownership**. The capability-token type and its sole sanctioned constructor live in the same crate as the high-level write helper that consumes it; standard within-crate sealing then applies and fidelity is high.

Applied to this slice: the overlay crate exports per-mode public functions (see §Witness types). The witness types themselves are `pub(crate)` and never escape the crate; external callers see only the per-mode functions. The crate is a new module owned by the overlay layer, downstream of `db` (which keeps the raw row primitives) and upstream of every read site.

Why this is structurally hard rather than convention:

- External crates cannot construct either witness because the types are not public.
- External crates cannot call the internal generic helper because it is private.
- The only path that produces a witness value is inside the overlay crate, inside the per-mode public function, which constructs the one witness it is supposed to.
- A future maintainer adding a new read site picks `_for_user` or `_for_sync` at the call site; mis-choosing is visible by grep. There is no third option available.

This is structurally distinct from a "public constructor with private fields" shape (option 1, medium fidelity), where any crate could call `BaseProviderTruthOnly::new()` against intent. Here, the witness type is not nameable from outside the overlay crate; the capability *is* the per-mode function and is enforced by its existence as the sole construction path. The roadmap's "ownership restructuring" is what gives the option-4 fidelity claim its teeth - the helpers and the witnesses must move together, otherwise the fidelity collapses to option 1.

Stage 3 confirms this design holds against the actual read sites (see Read sites that must include overlay) and records the chosen crate name back here.

### Read sites that must include overlay

The stage 3 acceptance test is the literal enumeration of every SQL site that joins `thread_labels` or `thread_label_groups`. Each site is either migrated to the overlay-aware helper or explicitly tagged `BaseProviderTruthOnly` with a one-line justification. Surfaces in scope:

- Reading-pane label pills.
- Thread-list label markers.
- Sidebar label rows, and label counts if implemented.
- Settings label rows.
- Smart-folder `label:` predicate and `is:tagged` predicate (`label_group_rendered_fragment`).
- Smart-folder count queries (list-equals-count under the Promise Rule).
- Search SQL fallback label predicates (verify route through smart-folder).
- Any canonical label-group rendered-membership SQL.

The list is the acceptance test. Stage 3 lands with every entry migrated and this section updated to mark the inventory complete.

### Tantivy

Tantivy does not index labels today (verified: `crates/search/src/lib.rs` defines `SearchDocument` with no `label_id` / `label_name` / category fields). Operator-bearing `label:` filters therefore route through SQL by construction; no explicit retirement is required. Should a future Tantivy schema change add label indexing, the overlay cannot reach index tokens (they are message-level snapshots of provider truth at indexing time), and operator-bearing label filters must remain SQL-routed to preserve overlay correctness. The DateBound resolution under contract #1 is the established precedent.

## Queue versus new table

Working assumption: **new table.** The queue-derivation option fails on two structural points:

1. The action queue's payload is JSON (`pending_operations.params`, defined in `crates/db/src/db/pending_ops.rs`; the SQL table name is `pending_operations`, the Rust module is `pending_ops`). Indexing by `label_id` for the read-path SQL predicates - which need fast lookup `WHERE account_id = ? AND thread_id = ?` returning per-label-id intents - would require either a generated column on the JSON or a parallel index table. Both undo the "single source of truth" benefit.
2. `pending_ops::compact_queue` (`crates/db/src/db/pending_ops.rs:434`, called via the public `db_pending_ops_compact`) actively compacts opposing `addLabel`/`removeLabel` pairs into nothing on a periodic schedule. A queue-derived overlay would be silently emptied by this compaction. Disabling compaction is a regression for the action queue; bypassing it for label ops is an exception that contradicts the queue's own contract.

Before stage 2 starts, a one-paragraph artifact lands in this document that confirms or refutes the two points above against the actual `pending_ops` code as it stands at that moment. If both still hold, the slice proceeds with `pending_thread_label_intents` as specified. If circumstances have changed (queue schema, compaction policy), the artifact lays out the new options.

The artifact is a verification step, not a redesign gate. The working assumption is load-bearing.

## Provider-sync boundary

Provider-sync recompute and merge paths never read the overlay and never write into it. They read `thread_labels` and `thread_label_groups` directly, with `BaseProviderTruthOnly` as the only witness they construct. Their interactions with the overlay system are limited to two side-effects of normal membership writes:

1. The transactional generation bump on `threads.label_membership_generation`, in the same transaction as the membership write.
2. The `MembershipChanged { account_id, thread_id }` signal emitted after the transaction commits.

The overlay-cleanup task consumes the signal and runs clearance in its own transaction; provider-sync does not delete overlay rows itself, does not consult the overlay to decide what to write, and does not need any awareness of overlay rows existing.

## Implementation stages

### Stage 1: Generation infrastructure

Zero behavior change. Establishes the bump signal.

- Add `threads.label_membership_generation INTEGER NOT NULL DEFAULT 1` column. Migration backfills existing rows to 1 (not 0) so that the default cannot be mistaken for a captured snapshot of value 0. Stage 2 overlay rows capture `generation_seen` at insert time from the live counter; clearance is strict greater-than.
- Audit every path that writes to `thread_labels` or `thread_label_groups`. Classify each path as **provider-truth** (sync recompute, replace, merge) or **local optimistic** (action service). Add the transactional bump to the provider-truth `thread_labels` writes only. Local optimistic paths remain unchanged.
- The audit records the local optimistic paths as "no bump, slated for removal in stage 3" so future maintenance does not accidentally add a bump to them.
- Tests assert the bump fires in every provider-truth `thread_labels` membership transaction and does not fire from local optimistic writes.
- No new read paths consult the counter yet.

Acceptance: the counter advances on every provider-truth `thread_labels` membership write and only on those writes. Local action-service writes to `thread_labels` or `thread_label_groups` complete without bumping the counter. Migration backfill leaves no row at the default-0 value.

### Stage 2: Overlay shadow-write lifecycle

Overlay table exists; rows are written and cleared correctly. **Current optimistic UI flow continues unchanged**: the action service still writes `thread_labels` directly, the renderer still reads `thread_labels`, the visible behavior does not regress. The overlay runs in shadow alongside.

Stage 2 is independently safe by design. It adds infrastructure, it does not change any UI surface, and reverting it removes the overlay without changing visible behavior. The overlay cannot close the erase window in this stage - that requires stage 3's read-side migration - but it must not regress UX either.

- Queue-versus-table verification artifact lands in this document first (working assumption is new-table, see §Queue versus new table).
- Add `pending_thread_label_intents` with both indexes.
- Action paths upsert intents with `generation_seen` captured at insert time, in the same transaction as the action-queue entry. The action service continues to write `thread_labels` and `thread_label_groups` directly in the same transaction; the overlay write is an addition, not a replacement.
- Provider-sync emits a `MembershipChanged { account_id, thread_id }` signal after every membership transaction commits. Carrier mechanism (in-memory channel preferred for latency, with the periodic backstop sweep providing durability) decided in this stage.
- Overlay-cleanup task subscribes to the signal and applies the generation-advance clearance rule in its own transaction (§Cleanup executor item 1).
- Periodic backstop sweep runs every few minutes and applies the same clearance rule regardless of signal arrival (§Cleanup executor item 2). This makes the signal a latency optimization, not a correctness dependency.
- Action-failure clearance: `DELETE WHERE action_id = ?` on permanent failure, no-op if the action has been superseded.
- Long-TTL tripwire runs on a coarse schedule; firing emits a warning log (§Cleanup executor item 3).

Acceptance: overlay rows appear when actions queue, clear correctly under the generation-advance rule, clear immediately on permanent failure, and the TTL warning does not fire under representative test loads. Current UI continues to work exactly as before; the overlay is observable in the database but has no rendered effect.

### Stage 3: Read migration and local-write removal

The visible behavior change. Four paired changes that must land together because any alone breaks the UI:

1. **Reads migrate**: the overlay-crate per-mode helpers (`read_thread_labels_for_user` and friends) land. Every site in the read-path inventory is migrated to the user-facing helper, or explicitly routed through the sync helper with a one-line justification.
2. **Local `thread_labels` writes stop**: the action service's per-label add and remove paths stop writing `thread_labels`. They write only the overlay (plus enqueue the action where applicable). Provider-sync becomes the sole writer of `thread_labels`.
3. **Local `thread_label_groups` writes stop, table is retired**: the action service's group-composite add and remove paths (`crates/service/src/actions/label_group.rs:97, 146`) stop writing `thread_label_groups`. The composite still fans out to per-member overlay intents. `label_group_rendered_fragment` is rewritten to drop the `EXISTS thread_label_groups` branch and read the overlay-aware label set through `label_group_members` only. Existing `thread_label_groups` rows are discarded as part of this stage's migration (`DELETE FROM thread_label_groups`); the table either drops entirely or stays as a structurally-empty leftover. Dev-seed and tests that seed `thread_label_groups` directly are updated to seed `thread_labels` rows that join through `label_group_members` instead, so the rendered-group state under test matches the post-slice derivation.
4. **Graph action-triggered recompute lands**: provider-sync exposes `recompute_thread_truth(account_id, thread_id)` (Graph-specific; Gmail/IMAP/JMAP do not need it because their normal sync paths converge symmetrically). Graph remove, category-remove, and importance-set action handlers call this on confirmed action success. The recompute writes `thread_labels` and bumps the generation atomically inside `provider-sync`; the action handler never touches `thread_labels` directly. This closes the Graph-merge asymmetry described in §Provider convergence model.

All four changes ship in one PR. Any subset breaks an axis: (1) without (2)/(3) → reads see overlay but local writes still race recomputes (no fix); (2)/(3) without (1) → optimistic UI disappears until sync echoes (regression); (1)–(3) without (4) → Graph remove intents never clear except by TTL.

- Cross-crate Fidelity option 4 wiring confirmed by implementation: the overlay-crate name is recorded back in §Cross-crate fidelity decision.
- Smart-folder list and count queries land in the same wave to preserve list-equals-count.
- Search SQL fallback label predicates inherit through smart-folder; verify and record.
- `crates/service/src/actions/label.rs` and `crates/service/src/actions/label_group.rs` no longer mutate `thread_labels` or `thread_label_groups` directly; they write only the overlay (multi-intent for exclusive sets like Graph importance) and the action queue where applicable.
- `label_group_rendered_fragment` simplifies to its second branch (overlay-aware `thread_labels JOIN label_group_members`).
- `provider-sync::graph::recompute_thread_truth` (or equivalent name) exists and is called by Graph action handlers on confirmed remove/category-remove/importance-set success.
- `docs/glossary/discrepancies.md` "Optimistic Local Label Intent" entry is replaced with a "resolved by optimistic-intent slice" note.

Acceptance: the read-path inventory is fully migrated; list and count agree across every label-membership surface; the action service no longer mutates `thread_labels` or `thread_label_groups`; the IMAP/JMAP keyword recompute, IMAP full-thread replace, Gmail full-thread replace, and Graph partial-delta merge windows can no longer flicker an optimistic label, because the optimistic row no longer exists in `thread_labels` until provider truth catches up.

## Open decisions

Most have closed; what remains is verification rather than design.

1. **Queue-versus-table verification artifact**: confirm that the two structural blockers in §Queue versus new table still hold against current `pending_ops` code at the time stage 2 starts. Working assumption (new table) stands unless the verification refutes a blocker.
2. **`MembershipChanged` carrier choice**: in-memory channel (preferred for latency) plus the periodic backstop sweep, or a notification table if there is a reason to prefer durability over latency on the primary path. Either is correct because the backstop sweep is the durability layer. Decided in stage 2 wiring.
3. **Overlay crate name and location**: a new crate (downstream of `db`, upstream of every read site), or a new module inside `core`. Decided at the start of stage 3 against the read-site inventory; recorded back here.

## Settled decisions

These were open in earlier drafts of this document and are now closed.

- **Generation counter scope**: provider-truth `thread_labels` writes only; local optimistic writes do not bump. Stage 1 audit enumerates both lists.
- **Per-provider clearance rule**: a single rule (generation-advance + truth-match, strict inequality) applies to all five provider paths. Under the post-slice model no action path writes `thread_labels` directly, so action success alone never clears the overlay (it only triggers `recompute_thread_truth` on Graph, which writes via provider-sync and bumps the generation, leading to standard clearance).
- **Generation column shape**: one counter per thread on `thread_labels` provider-truth writes. `thread_label_groups` is action-service-only state today and is retired in stage 3, so no separate counter is needed for it.
- **`thread_label_groups` retirement**: stage 3 removes the direct action-service writes to `thread_label_groups` and rewrites `label_group_rendered_fragment` to read overlay-aware `thread_labels JOIN label_group_members` only.
- **Cross-crate Fidelity option**: option 4 (restructure ownership), per the roadmap's standing answer for contract #4. Witness types and overlay-aware helpers live in the same crate; within-crate sealing applies.
- **Bootstrap collision**: avoided by migrating the counter default to 1 and using strict-greater-than for clearance comparison.
- **Cleanup durability**: layered (signal + periodic sweep + TTL); signal carrier choice is latency-only, not correctness-critical.
- **Provider convergence**: Gmail full-thread, IMAP keyword recompute, IMAP full-thread replace, and JMAP keyword recompute converge symmetrically via existing sync paths. Graph partial-delta merge is INSERT-only, so Graph removes (labels, categories, importance opposite) trigger a `provider-sync::graph::recompute_thread_truth` call from the action handler on confirmed success. Provider-sync remains the sole writer of `thread_labels`.
- **Action-overlay lifecycle**: three paths - immediate success (no queue row, `action_id = NULL`), retryable failure (queue row, `action_id` set), immediate permanent failure (no overlay row persists). Nullable `action_id` is normal, not an error.
- **Exclusive label sets**: action handlers emit multiple overlay intents atomically with shared `action_id`. Graph importance is the current case (`high` adds with `low`/`normal` removes); future exclusive sets follow the same pattern.

## References

- `docs/glossary/discrepancies.md` "Optimistic Local Label Intent" inventory entry (the open window this slice closes).
- `docs/contracts-roadmap.md` migration #4 (the parent contract this slice descends from).
- `docs/glossary/folders-labels.md` (canonical label-membership semantics).
- `docs/architecture.md` (per-message membership store pattern; cross-client moves).
