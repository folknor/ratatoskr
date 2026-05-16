# Optimistic Label Intent

Landed design slice for the contract #4 follow-up caveat in `docs/glossary/discrepancies.md`. Spawned from the merge-vs-replace migration; not a sub-task of it.

## Status

Implemented. `threads.label_membership_generation` and `pending_thread_label_intents` exist in schema v100. Local label and label-group actions write pending intent instead of optimistic `thread_labels` / `thread_label_groups` rows. User-facing label-group queries route through overlay-aware helpers. Provider-sync membership writes bump the generation and clear satisfied intents. A successful same-client label provider call applies the confirmed provider-truth delta through `db::queries_extra::confirmed_provider_label_intents`, then clears matching pending intent; retryable failures attach the pending-operation id so exhausted retries can clear the overlay.

## Background

Under contract #4, the merge-vs-replace helper selection migration is complete: provider paths with full coverage call provider-local replace wrappers, partial-delta paths call merge wrappers, and `db::queries_extra::thread_persistence` exposes only raw row primitives. The remaining window was local label actions writing `thread_labels` (and `thread_label_groups` for group composites) directly before provider truth caught up. That window is closed by the pending-intent overlay described here.

This document specifies the slice that closes the window.

## Promise

A label optimistically added by a local action stays visible across every user-facing label-membership query until the action either succeeds and provider truth catches up, or fails permanently. On permanent failure, the rendered state transitions back: the label that was optimistically shown is no longer shown, surfaced through the existing toast/undo path. Sync recompute paths never observe or overwrite optimistic intent; user-facing queries always merge provider truth with the pending-intent overlay. List queries and count queries see the same merged set.

## Non-goals

- `StoredSecret` strictness migration. Completed separately and tracked in `docs/contracts-roadmap.md`.
- Cross-client folder/label move reconciliation. Tracked in `TODO.md`; cured by the per-message membership store pattern from `docs/architecture.md`.
- Tantivy label-token indexing. Tantivy does not index labels today (no `label_id` / `label_name` fields in `SearchDocument`); operator-bearing `label:` filters are SQL-only as a consequence, not as an explicit retirement.

## Data model

### Generation counter

A persisted per-thread counter advances every time **provider-truth** label membership for that thread changes. Local optimistic writes do not bump it. The overlay records the generation observed at intent-write time and clears once the live generation has advanced past that snapshot.

- Storage: `threads.label_membership_generation INTEGER NOT NULL DEFAULT 1`. The migration backfills existing rows to 1 (not 0) so that the default value cannot be mistaken for a captured snapshot from before the column existed. Overlay rows capture the live counter value at insert time; clearance compares strictly greater than that snapshot.
- Single per-thread counter on `thread_labels` provider-truth writes. Group-level rendered membership derives from overlay-aware label membership through `label_group_members`; `thread_label_groups` is no longer a rendering source, so one counter is sufficient.
- Relationship to `core::generation::GenerationCounter<T>`: distinct. The in-memory branded counter is a runtime invalidation mechanism. This is a SQL-persisted column with cross-transaction semantics. Adjacent patterns, not the same one.
- Atomicity: the bump must be the last statement before commit in the same transaction as the provider-truth membership write. Sequence: `thread_labels` write, counter increment, commit. Out of order, or split across transactions, reintroduces the current bug at a different layer.

#### `thread_label_groups` and the single-counter claim

`thread_label_groups` was previously written only by the action service as an optimistic group-membership shortcut at the start of composite "add label group" / "remove label group" operations. That shortcut is retired. The composite now fans out to per-member pending label intents, and provider-sync never reads or writes `thread_label_groups`.

`label_group_rendered_fragment` in `crates/smart-folder/src/sql_builder.rs` delegates to the DB overlay helper. Group-level rendered membership derives from the overlay-aware label set. The `thread_label_groups` table remains in schema as a structurally-empty legacy table, without a writer.

This means a single counter on `thread_labels` provider-truth writes is correct under the post-slice model. Local optimistic writes never bump it.

The audit produced two enumerated lists.

**Paths that bump (provider-truth `thread_labels` writes):**

- `provider-sync::keyword_membership::recompute_thread_keyword_labels` (IMAP and JMAP keyword paths).
- `provider-sync::imap::thread_store::replace_full_thread_labels` (IMAP `imap_initial` and `imap_delta`).
- `provider-sync::gmail::sync::storage` full-thread replace (Gmail full-thread persistence).
- `provider-sync::graph::sync::persistence::merge_partial_delta_labels` (Graph partial-delta merge).

These are the provider-truth writers. Each calls `finalize_provider_truth_label_membership` after the `thread_labels` write.

**Paths that explicitly do NOT bump (local optimistic writes):**

- `crates/service/src/actions/label.rs` add path.
- `crates/service/src/actions/label.rs` remove path.
- `crates/service/src/actions/label_group.rs` apply composite.
- `crates/service/src/actions/label_group.rs` remove composite.

These paths no longer write `thread_labels` or `thread_label_groups`; they upsert `pending_thread_label_intents` and do not bump the provider-truth counter.

### Overlay table

Landed table.

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

The `action_id` index supports action-failure clearance (`DELETE WHERE action_id = ?`). The `updated_at` index supports the stale-intent backstop sweep for unattached rows and rows whose queue entry is no longer live. Both clearance paths avoid table scans on the row volumes this table will see during heavy multi-account use.

- Unique key: `(account_id, thread_id, label_id)`. `op` is an attribute, not part of the key. Last write wins; an `Add` followed by a `Remove` for the same triple updates the existing row rather than producing two contradictory rows.
- `generation_seen`: the value of `threads.label_membership_generation` at intent-write time. Drives the clearance rule for async-echo provider paths, and is also the snapshot identity used by attach (see below) and immediate-permanent-failure clear.
- `action_id`: pointer to the *currently winning* action-queue entry for this key. Drives action-failure clearance: a permanent failure for the action that owns the overlay row deletes the row. A permanent failure for an action whose intent was already superseded by a later write (and whose `action_id` is no longer in the overlay) is a no-op for the overlay - the user has moved on, the failure surfaces through the toast/undo path but does not change the rendered state. This is correct UX, not a bug.
- **Attach is keyed on `(label_id, op, generation_seen)`, not on the primary key alone.** A same-`op` overwrite by a concurrent action refreshes `generation_seen` to a new snapshot, so a late attach for the original action no-ops instead of clobbering the newer action's `action_id`. The upsert helper returns the captured generation so the caller can pass that exact snapshot back to attach.
- Not a full event log. If history is ever needed, the action queue carries it.

## Lifecycle

### Insert and update

A local optimistic label action upserts the intent row capturing the current `threads.label_membership_generation` as `generation_seen`. The upsert always happens at action attempt; the relationship to the action queue depends on the action outcome (see Lifecycle paths below).

### Lifecycle paths

The current action-service contract is: successful online actions do not enqueue a `pending_operations` row; the queue is reserved for retryable failures. The overlay row's `action_id` column is nullable to accommodate this. Three lifecycle paths:

1. **Immediate provider success.** Overlay row written with `action_id = NULL`. No `pending_operations` row created. The action service applies the confirmed provider-truth delta through `db::queries_extra::confirmed_provider_label_intents`, which mutates `thread_labels`, bumps `threads.label_membership_generation`, and clears matching overlay rows in one caller-owned transaction.
2. **Retryable failure.** Overlay row written first with `action_id = NULL`, then updated to the new `pending_operations.id` after the retry row is enqueued. The attach UPDATE matches on `(label_id, op, generation_seen)`, so a concurrent same-`op` overwrite that refreshed `generation_seen` makes the late attach a no-op rather than letting it clobber the newer action's `action_id`. The queue's retry loop runs. On eventual success, the same confirmed-provider-truth path clears the overlay row. On exhausted retries, the action transitions to permanent failure and the overlay row clears via `delete_pending_thread_label_intents_for_action`.
3. **Confirmed-truth failure** (provider call returned `Ok` but the local finalize-truth-write failed). The action returns `LocalOnly { retryable: true }` so the retry queue re-drives the provider call. Provider label add/remove are idempotent, so the redrive is safe; eventually either the finalize succeeds or the next provider sync converges truth and clears the overlay.
4. **Immediate permanent failure** (provider call returned a classified-permanent error such as `unknown provider` / `decrypt credential` / `malformed stored secret`, or `create_provider` failed permanently). Overlay rows for the action's intents are deleted in the same dispatch path via `delete_pending_thread_label_intents_for_labels`, keyed on `(label_id, op, generation_seen)`. A same-`op` overwrite by a concurrent later action makes the delete a no-op for the now-owned row. The renderer no longer shows the optimistic state and the failure surfaces through the toast/undo path.

`action_id` is therefore set iff the action took the retryable-failure path and enqueue succeeded. A NULL `action_id` is normal for immediate success and for the short window before a retryable failure has been attached to its queue row.

### Exclusive label sets

Some operations are structurally a swap rather than an addition. The action service's current Graph importance handler removes the opposite level before inserting the new one. Under the post-slice model, the action handler emits multiple overlay intents:

- `Add(importance:high)` and `Remove(importance:low)` for the set member being added.
- Retryable failure attaches the same `action_id` to both rows. Permanent failure deletes both via the `action_id` index.
- The renderer's merge algebra handles the rest correctly: `overlay_aware_labels` reflects the intended exclusive state immediately.

The same pattern generalizes to any other exclusive label set encoded by the action service. The overlay write path takes a list of `(label_id, op)` pairs per action, not a single pair.

### Clearance rule (uniform across providers)

Under the post-slice model, no path writes optimistic state into `thread_labels`. Provider-sync writes provider truth during sync. The action service also writes provider truth after a provider call has succeeded, using the same DB finalization helper; this is a confirmed provider-truth delta, not an optimistic local row.

All five provider paths follow the same clearance rule: **clear when `threads.label_membership_generation > intent.generation_seen` AND the resulting `thread_labels` row state matches `intent.op`.** Strict inequality on the generation: the row must observe a provider-truth bump that happened *after* the intent was captured. Both conditions must hold; the generation bump alone (without the truth-match check) would clear overlay rows on unrelated label changes on the same thread.

The strict-inequality comparison and the migration backfill to 1 (not 0) interact: a default-valued row cannot be mistaken for a captured snapshot, because any subsequent provider-truth bump produces a generation strictly greater than the snapshot.

### Provider convergence model

The clearance rule depends on provider-truth catching up to the local intent. Sync paths converge through provider-sync; same-client provider success converges immediately through `confirmed_provider_label_intents`.

- **Same-client action success**: convergence immediately through the confirmed-provider-truth delta. Later sync is idempotent with that local truth.
- **Gmail full-thread replace**: convergence on the next full-thread persist. The replace handles add and remove symmetrically because it overwrites the whole `thread_labels` row set for the thread.
- **IMAP keyword recompute** (`provider-sync::keyword_membership::recompute_thread_keyword_labels`): the recompute does a DELETE-then-INSERT against `thread_labels` keyed on the per-message `message_keywords` union. Add and remove both converge because both reshape the union.
- **IMAP full-thread replace** (`thread_store::replace_full_thread_labels`): symmetric for the same reason as Gmail.
- **JMAP keyword recompute**: same shape as IMAP keyword.
- **Graph partial-delta merge** (`merge_partial_delta_labels`): asymmetric for cross-client changes because the merge is INSERT-only and cannot remove. Same-client Graph remove/category/importance actions converge immediately through the confirmed-provider-truth delta after the Graph API call succeeds.

For Graph category and importance operations: same as labels. Importance has additional structure handled by the action handler emitting multiple overlay intents - see Exclusive label sets above.

### Failure clearance

- Immediate permanent provider failure: clear via `delete_pending_thread_label_intents_for_labels` with the captured `generation_seen`. Same-`op` overwrites by a later action make the delete a no-op for the new row. Surfaces through the existing toast / undo path.
- Exhausted retry: clear via `delete_pending_thread_label_intents_for_action` (uses the `action_id` index). The delete is no-op if the action has been superseded - see the `action_id` semantics in Overlay table above.
- Retryable or offline failure: retain while the action queue holds the entry as live.

### Cleanup

Cleanup is synchronous with the writes that can satisfy or fail an intent:

1. **Provider-truth finalization.** `finalize_provider_truth_label_membership` bumps `threads.label_membership_generation` and deletes overlay rows whose generation is older than the new truth and whose op matches the resulting `thread_labels` state. Provider-sync replace/merge/recompute paths call this after their `thread_labels` write. Same-client provider success calls `confirmed_provider_label_intents` inside a caller-owned transaction, which applies the confirmed delta and then calls the same finalizer.
2. **Immediate permanent provider failure.** The dispatch path classifies the provider error via `classify_provider_error`. A `Permanent` classification marks the outcome as `LocalOnly { retryable: false }` and deletes the action's overlay rows via `delete_pending_thread_label_intents_for_labels`, keyed on `(label_id, op, generation_seen)` so a same-`op` overwrite by a concurrent later action is left intact.
3. **Permanent retry failure.** `db_pending_ops_increment_retry` deletes `pending_thread_label_intents` rows by `action_id` (via `delete_pending_thread_label_intents_for_action`) when a pending operation exhausts retries and becomes permanently failed.
4. **Stale-intent backstop.** The pending-op worker and boot recovery delete very old overlay rows only if they are unattached or their `action_id` no longer points at a live `pending` / `executing` queue row. A warning log is emitted when this path deletes anything; it is a consistency backstop, not the normal lifecycle.

There is no signal carrier in the landed implementation. Normal cleanup runs in the transaction that changed provider truth or marked the retry permanently failed; the stale sweep exists only for crash or queue-corruption residue.

## Read path

### Helper boundary

The landed read boundary is `db::queries_extra::label_intent`. It exposes two overlay-aware SQL fragment helpers:

- `user_visible_label_exists_fragment(account_column, thread_column, label_expr)`.
- `user_visible_label_group_rendered_fragment(account_column, thread_column, group_predicate)`.

These helpers centralize the merge algebra for user-facing membership reads. Sync code does not call them; provider-sync reads and writes provider truth directly.

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
```

`label_group_rendered_fragment` in `crates/smart-folder/src/sql_builder.rs` now delegates to the overlay-aware helper and no longer consults `thread_label_groups`.

### Cross-crate fidelity decision

The provider-truth write side keeps the contracts roadmap's high-fidelity answer for contract #4: provider-sync owns the provider-local replace/merge wrappers, and `db` exposes only raw row primitives.

The read side is a helper boundary, not a capability-token boundary. That is deliberate for this slice. The correctness rule is enforced by inventory plus central SQL helpers: every user-facing membership predicate uses `user_visible_label_exists_fragment` or `user_visible_label_group_rendered_fragment`, while provider-sync is the explicit base-truth exception. A future hard witness type can still be introduced if this surface grows; the current helper shape keeps the SQL close to schema ownership without pretending Rust can forbid hand-written SQL strings across crates.

### Read sites that include overlay

The acceptance test is the literal enumeration of every SQL site that joins `thread_labels` or `thread_label_groups`. Each user-facing site is migrated to the overlay-aware helper; provider-sync remains the explicit base-provider-truth exception. Surfaces in scope:

- Reading-pane label pills.
- Thread-list label markers.
- Sidebar label rows, and label counts if implemented.
- Settings label rows.
- Smart-folder `label:` predicate and `is:tagged` predicate (`label_group_rendered_fragment`).
- Smart-folder count queries (list-equals-count under the Promise Rule).
- Search SQL fallback label predicates (verify route through smart-folder).
- Any canonical label-group rendered-membership SQL.

The inventory is complete for the landed slice: reading-pane and thread-list decorations, command palette label groups, scoped label-group lists, sidebar counts, and smart-folder `label:` / `is:tagged` predicates use the overlay-aware helpers.

### Tantivy

Tantivy does not index labels today (verified: `crates/search/src/lib.rs` defines `SearchDocument` with no `label_id` / `label_name` / category fields). Operator-bearing `label:` filters therefore route through SQL by construction; no explicit retirement is required. Should a future Tantivy schema change add label indexing, the overlay cannot reach index tokens (they are message-level snapshots of provider truth at indexing time), and operator-bearing label filters must remain SQL-routed to preserve overlay correctness. The DateBound resolution under contract #1 is the established precedent.

## Queue versus new table

Decision: **new table.** The queue-derivation option fails on two structural points:

1. The action queue's payload is JSON (`pending_operations.params`, defined in `crates/db/src/db/pending_ops.rs`; the SQL table name is `pending_operations`, the Rust module is `pending_ops`). Indexing by `label_id` for the read-path SQL predicates - which need fast lookup `WHERE account_id = ? AND thread_id = ?` returning per-label-id intents - would require either a generated column on the JSON or a parallel index table. Both undo the "single source of truth" benefit.
2. `pending_ops::compact_queue` (`crates/db/src/db/pending_ops.rs:434`, called via the public `db_pending_ops_compact`) actively compacts opposing `addLabel`/`removeLabel` pairs into nothing on a periodic schedule. A queue-derived overlay would be silently emptied by this compaction. Disabling compaction is a regression for the action queue; bypassing it for label ops is an exception that contradicts the queue's own contract.

This is now the recorded artifact: the queue was inspected, both structural blockers still hold, and the overlay table is the landed source for user-visible intent.

## Provider-sync boundary

Provider-sync recompute and merge paths never consult the overlay to decide provider truth. They read and write base `thread_labels` directly. Their interaction with the overlay system is the finalization helper after normal membership writes:

1. The transactional generation bump on `threads.label_membership_generation`, in the same transaction as the membership write.
2. Deletion of overlay rows satisfied by the resulting provider truth.

## Landed implementation

- Schema: `threads.label_membership_generation` and `pending_thread_label_intents`, with indexes on `action_id` and `updated_at`.
- Writes: `crates/service/src/actions/label.rs` and `crates/service/src/actions/label_group.rs` write pending intents instead of optimistic `thread_labels` / `thread_label_groups` rows. The upsert helper returns the captured `generation_seen` so the same dispatch path can attach / clear keyed on `(label_id, op, generation_seen)`. Composite member dispatch reuses the composite-captured snapshot and never re-upserts. Composite retry preflight reads the overlay-aware rendered group state.
- Confirmed truth: successful same-client provider calls apply their confirmed delta through `confirmed_provider_label_intents`, which takes a caller-owned `&Transaction` so it composes with other writes in one atomic step. Provider-sync replace/merge/recompute paths call `finalize_provider_truth_label_membership` after their membership writes.
- Failure handling: `classify_provider_error` is consulted at the dispatch site. A permanent classification produces `LocalOnly { retryable: false }` and deletes the action's overlay rows via `delete_pending_thread_label_intents_for_labels` immediately, so a permanent provider failure no longer leaves the optimistic state in place until the 48 h sweep. A confirm-finalize failure on an otherwise-successful provider call falls back to `LocalOnly { retryable: true }`; the queue re-drives the idempotent provider operation.
- Reads: `user_visible_label_group_rendered_fragment` and `user_visible_label_exists_fragment` are the shared SQL helpers. Reading-pane/thread-list decorations, command palette label groups, scoped label-group lists, sidebar counts, and smart-folder `label:` / `is:tagged` predicates use the overlay-aware path.
- Dev/test data: dev-seed no longer creates sample `thread_label_groups` rows; sample group rendering comes from member `thread_labels` rows.

Acceptance: list and count agree across label-membership surfaces; the action service no longer writes optimistic rows into `thread_labels` or `thread_label_groups`; provider-truth writes clear satisfied overlay rows; exhausted label retries clear overlay rows by `action_id`; immediate permanent provider failures clear overlay rows in the dispatch path.

## Open decisions

Closed by the landed implementation.

1. **Queue-versus-table**: new table. `pending_operations.params` is JSON and `pending_ops::compact_queue` can erase opposing label operations, so deriving the overlay from the queue is not a stable read source.
2. **`MembershipChanged` carrier**: not needed. Cleanup is synchronous with provider-truth finalization and permanent retry failure.
3. **Overlay location**: `db::queries_extra::label_intent`. The SQL helper lives with the schema and row primitives; smart-folder and user-facing DB queries call into it.

## Settled decisions

These were open in earlier drafts of this document and are now closed.

- **Generation counter scope**: provider-truth `thread_labels` writes only; local optimistic writes do not bump. The audit enumerates both lists.
- **Per-provider clearance rule**: a single rule (generation-advance + truth-match, strict inequality) applies to provider-truth finalization. Same-client provider success first applies the confirmed truth delta, then runs the same finalizer.
- **Generation column shape**: one counter per thread on `thread_labels` provider-truth writes. `thread_label_groups` is retired as a rendering source, so no separate counter is needed for it.
- **`thread_label_groups` retirement**: action-service writes to `thread_label_groups` are removed and `label_group_rendered_fragment` reads overlay-aware `thread_labels JOIN label_group_members` only.
- **Cross-crate Fidelity option**: the landed helper shape is `db::queries_extra::label_intent`, with private SQL construction behind public user-facing helper functions. Provider-sync still owns provider-delta semantics; `db` owns schema and finalization.
- **Bootstrap collision**: avoided by migrating the counter default to 1 and using strict-greater-than for clearance comparison.
- **Cleanup durability**: synchronous finalization on provider-truth writes plus action-id deletion on permanent retry failure.
- **Provider convergence**: Gmail full-thread, IMAP keyword recompute, IMAP full-thread replace, and JMAP keyword recompute converge symmetrically via existing sync paths. Same-client actions for all providers converge on confirmed provider success through `confirmed_provider_label_intents`, so Graph remove/category/importance actions do not wait for partial-delta subtraction.
- **Action-overlay lifecycle**: four paths - immediate provider success (no queue row, `action_id = NULL`, overlay cleared by confirmed-truth finalize), retryable failure (queue row, `action_id` set by an attach that matches on `(op, generation_seen)`), confirmed-truth failure (retryable LocalOnly so the queue re-drives the idempotent provider call), and immediate permanent provider failure (overlay rows deleted in the dispatch path). Nullable `action_id` is normal, not an error.
- **Exclusive label sets**: action handlers emit multiple overlay intents for one action. Graph importance is the current case (`high` adds with `low` remove, and the reverse); future exclusive sets follow the same pattern.

## References

- `docs/glossary/discrepancies.md` "Optimistic Local Label Intent" inventory entry (the open window this slice closes).
- `docs/contracts-roadmap.md` migration #4 (the parent contract this slice descends from).
- `docs/glossary/folders-labels.md` (canonical label-membership semantics).
- `docs/architecture.md` (per-message membership store pattern; cross-client moves).
