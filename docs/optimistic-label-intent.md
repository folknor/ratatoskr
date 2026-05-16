# Optimistic Label Intent

Design slice for the remaining contract #4 follow-up caveat in `docs/glossary/discrepancies.md`. Spawned from the merge-vs-replace migration; not a sub-task of it.

## Background

Under contract #4, the merge-vs-replace helper selection migration is complete: provider paths with full coverage call provider-local replace wrappers, partial-delta paths call merge wrappers, and `db::queries_extra::thread_persistence` exposes only raw row primitives. One window remains open: optimistic local label actions write directly to `thread_labels` before the provider echoes the change. A concurrent IMAP/JMAP keyword recompute can temporarily erase that optimistic `kw:*` row, because the per-message `message_keywords` union has not observed the intent yet.

This document specifies the slice that closes the window.

## Promise

A label optimistically added by a local action stays visible across every user-facing label-membership query until the action either succeeds and provider truth catches up, or fails permanently. Sync recompute paths never observe or overwrite optimistic intent; user-facing queries always merge provider truth with the pending-intent overlay. List queries and count queries see the same merged set.

## Non-goals

- `EncryptedSecret` strictness migration. Sequenced before this slice but tracked separately.
- Cross-client folder/label move reconciliation. Tracked in `TODO.md`; cured by the per-message membership store pattern from `docs/architecture.md`.
- Tantivy label-token reindexing. Operator-bearing label filters move to SQL; see Read path.

## Data model

### Generation counter

A persisted per-thread counter advances every time provider-truth label membership for that thread changes. The overlay records the generation observed at intent-write time and clears once the live generation has advanced past that snapshot.

- Storage: `threads.label_membership_generation INTEGER NOT NULL DEFAULT 0`. A single per-thread counter unless the stage 1 audit finds a concrete need for separate folder, label, or provider counters.
- Relationship to `core::generation::GenerationCounter<T>`: distinct. The in-memory branded counter is a runtime invalidation mechanism. This is a SQL-persisted column with cross-transaction semantics. Adjacent patterns, not the same one.
- Atomicity: the bump must be the last statement before commit in the same transaction as the membership write. Sequence: `thread_labels` write, counter increment, commit. Out of order, or split across transactions, reintroduces the current bug at a different layer.

Bump points (stage 1 audit produces the canonical list):

- `provider-sync::keyword_membership` recompute helper (IMAP and JMAP).
- Gmail full-thread replace wrappers (Gmail-local).
- IMAP thread-store replace wrappers.
- Graph and JMAP merge wrappers.
- Any other path that mutates `thread_labels` or `thread_label_groups`.

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
```

- Unique key: `(account_id, thread_id, label_id)`. `op` is an attribute, not part of the key. Last write wins; an `Add` followed by a `Remove` for the same triple updates the existing row rather than producing two contradictory rows.
- `generation_seen`: the value of `threads.label_membership_generation` at intent-write time. Drives the clearance rule for async-echo provider paths.
- `action_id`: optional pointer to the action-queue entry. Drives action-failure clearance.
- Not a full event log. If history is ever needed, the action queue carries it.

## Lifecycle

### Insert and update

A local optimistic label action upserts the intent row in the same transaction as the action-queue entry, capturing the current `threads.label_membership_generation` as `generation_seen`.

### Clearance by provider path

"Clear on echo" is not one rule. It splits by whether the action wrote durable provider truth synchronously.

- Gmail full-thread replace: the action is the durable write. Clear on action success.
- IMAP keyword paths: provider returns success, but `thread_labels` only catches up on the next changes-poll keyword recompute. Clear when `threads.label_membership_generation > intent.generation_seen` AND the resulting `thread_labels` row state matches `intent.op`.
- JMAP keyword paths: identical to IMAP.
- Graph categories and importance: same generation-comparison rule. Action success records pending state; sync echo bumps the generation and the comparison clears the row.

### Failure clearance

- Permanent action failure: clear immediately, surface failure through the existing toast and undo path.
- Retryable or offline failure: retain while the action queue holds the entry as live.

### TTL backstop

A long TTL (default 48h) clears stuck rows as last resort. Firing the TTL emits a warning log with `action_id`, `account_id`, and `thread_id`. The TTL is not a normal cleanup mechanism; every TTL fire is a bug to investigate, not a routine event.

## Read path

### Witness types

```rust
pub struct IncludeOverlay(());          // overlay-aware reads
pub struct BaseProviderTruthOnly(());   // provider-sync reads, never include overlay
```

Label-membership query helpers take one or the other as a marker. Renderer, smart-folder, sidebar, settings, and search SQL paths construct `IncludeOverlay`. Provider-sync recompute and merge paths construct `BaseProviderTruthOnly`.

### Cross-crate fidelity decision

Stage 3 must pick one. Both are valid; the document records the choice once made.

- Hard: `BaseProviderTruthOnly` is constructible only inside `provider-sync` via a private constructor. Closes the boundary at compile time within the slice, but Rust has no friend-crate mechanism, so this requires routing every provider-sync membership read through a single helper that owns the constructor.
- Soft: both witnesses are `pub`, contract enforced by doc-comment and the read-path inventory. Drift-prone but unblocks shipping.

The contracts roadmap's standing Fidelity question applies here. Whichever option this slice picks should be consistent with the wider answer, not an exception.

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

Tantivy label-token filters cannot observe the overlay; index tokens are message-level snapshots of provider truth at indexing time. Decision: operator-bearing label filters route through SQL exclusively, matching the `DateBound` resolution under contract #1. Tantivy retains label tokens in the index for free-text queries that mention label names; explicit `label:` operators are SQL-only.

## Queue versus new table

Before stage 2 starts, a one-paragraph artifact lands in this document that enumerates the pending-action queue's schema and answers: can the queue carry `(account_id, thread_id, label_id, op, generation_seen)` indexed for the read-path SQL predicates? If yes, the overlay derives from the queue and `pending_thread_label_intents` is not created. If no, the artifact names the missing column or index that forces a projection table.

Working assumption until the artifact lands: new table. The schema above is written against that assumption; if the artifact concludes otherwise, the schema section becomes a derivation view and the data-model section is rewritten.

## Provider-sync boundary

Provider-sync recompute and merge paths never read the overlay and never write into it. They read `thread_labels` and `thread_label_groups` directly, with `BaseProviderTruthOnly` as the only witness they construct. The generation bump is their sole interaction with overlay clearance, and the bump is a side-effect of normal membership writes, not a separate overlay-aware code path.

## Implementation stages

### Stage 1: Generation infrastructure

Zero behavior change. Establishes the bump signal.

- Add `threads.label_membership_generation` column. Migration default 0.
- Audit every path that writes to `thread_labels` or `thread_label_groups` and add the transactional bump.
- Tests assert the bump fires in every recompute, replace, and merge path, and never fires outside one.
- No new read paths consult the counter yet.

Acceptance: the counter advances on every membership write and only on membership writes.

### Stage 2: Overlay write and clear lifecycle

Overlay table exists; rows are written and cleared correctly. No read path consults the overlay yet.

- Queue-versus-table artifact lands in this document first.
- Under the new-table outcome: add `pending_thread_label_intents`.
- Action paths upsert intents with `generation_seen` captured at insert time, in the same transaction as the action-queue entry.
- Per-provider clearance wired:
  - Gmail: clear on action success.
  - IMAP and JMAP keyword: clear when generation advances and `thread_labels` state matches intent op.
  - Graph: same generation-comparison rule.
- Permanent-failure path clears immediately.
- Long-TTL warning fires and logs as a tripwire.

Acceptance: rows appear when actions queue, clear correctly under each provider model, clear immediately on permanent failure, and the TTL does not fire under representative test loads.

### Stage 3: Overlay-aware reads

The visible behavior change.

- Witness types land. Cross-crate fidelity choice (hard or soft) recorded back here.
- Every site in the read-path inventory is migrated to the overlay-aware helper, or explicitly tagged `BaseProviderTruthOnly` with a one-line justification.
- Smart-folder list and count queries land in the same wave to preserve list-equals-count.
- Search SQL fallback label predicates inherit through smart-folder; verify and record.
- Tantivy operator-bearing label filters route through SQL.
- `docs/glossary/discrepancies.md` Shape 4 follow-up caveat is replaced with a "resolved by optimistic-intent slice" note.

Acceptance: the read-path inventory is fully migrated; list and count agree across every label-membership surface; the IMAP/JMAP recompute window can no longer flicker an optimistic `kw:*` row.

## Open decisions

These answers land back in this document before the corresponding stage begins.

1. Cross-crate witness fidelity: hard or soft. Decided at the start of stage 3.
2. Queue versus new table: decided by the artifact, before stage 2.
3. Generation: one column versus per-kind counters. Decided by the stage 1 audit. Working assumption is one column; multiple counters only on concrete evidence.

## References

- `docs/glossary/discrepancies.md` Shape 4, "#4 follow-up caveat" (the open window this slice closes).
- `docs/contracts-roadmap.md` migration #4 (the parent contract this slice descends from).
- `docs/glossary/folders-labels.md` (canonical label-membership semantics).
- `docs/architecture.md` (per-message membership store pattern; cross-client moves).
