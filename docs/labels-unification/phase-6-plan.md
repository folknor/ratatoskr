# Labels Unification: Phase 6 Plan

## Goal

Drop the legacy `categories` and `message_categories` tables, consolidate `apply_category`/`remove_category` into `add_tag`/`remove_tag` on `ProviderOps`, and persist IMAP keyword capability for graceful degradation.

After this phase: no legacy category tables, no `apply_category`/`remove_category` on the provider trait, the tag/container boundary is enforced in the action service, and IMAP accounts that don't support keywords get an explicit failure instead of silent success.

## Current State

All sync pipelines write to both the old system (`categories` + `message_categories`) and the unified system (`labels` + `thread_labels`) in parallel. The action service dispatches label operations by branching on `label_kind`: tags go through `apply_category`/`remove_category` (name-based), containers go through `add_tag`/`remove_tag` (ID-based). Four providers implement both method pairs.

## Prerequisites Found During Review

### INSERT OR REPLACE data corruption bug

Gmail's `persist_labels()` (`crates/gmail/src/sync/labels.rs`) uses `INSERT OR REPLACE INTO labels` without specifying `label_kind`. Every sync resets user labels to `label_kind = 'container'` (the DEFAULT). Migration 67's backfill only ran once. IMAP and JMAP folder sync paths have the same pattern — the DEFAULT happens to be correct for containers, but it's fragile.

This must be fixed before any legacy writes are removed.

### JMAP writes keywords to AI bundling table

`sync_keyword_categories()` (`crates/jmap/src/sync/storage.rs:304-314`) inserts keywords into `thread_categories` (the AI inbox bundling table). Keywords are user labels, not AI classifications. This write should be removed.

### Graph legacy write missing from original plan

`store_thread_to_db()` in `crates/graph/src/sync/persistence.rs` calls `insert_message_categories()` — an additional legacy write site.

## Design Decisions

### Tag/container boundary is enforced, not inferred

`add_tag`/`remove_tag` are the only provider methods for additive tag-kind labels. Container labels (folders) are outside this action path entirely — they use move operations. The action service validates `label_kind = 'tag'` and rejects container labels with a hard failure.

### Prefix detection is provider-specific, not a universal contract

Some providers infer subtype from ID encoding (`cat:` for Graph, `kw:` for IMAP/JMAP). Gmail has no prefix — native label IDs work directly with `modify_thread()`. The routing contract is: "these methods handle additive tag-kind labels only." How each provider resolves the label ID internally is an implementation detail.

### Validation stays in the action service

The `(account_id, label_id)` lookup in `core::actions::label` is kept. It validates that the label exists and is `label_kind = 'tag'`. The `label_name` return is dropped (no longer needed for dispatch). The `label_kind` return is kept for the guard.

### IMAP keyword failure is permanent, not transient

When an IMAP account doesn't support custom keywords, the action returns `Failed` — not `LocalOnly`. This is a capability gap, not a transient sync miss. The local label should not be written either (it would create drift that can never reconcile).

### IMAP capability check lives in the action service, not the provider

The capability preflight must happen in `add_label_local`/`remove_label_local` — before the `thread_labels` mutation. If the check only lived in the IMAP provider's `add_tag`, it would be too late: the local DB write would already have happened. The action service validation query (which already runs in `_local` to check `label_kind = 'tag'`) is extended to also check the account's `supports_keywords` flag when the label_id has a `kw:` prefix. `Failed` is returned before any DB write.

### IMAP capability is an account-level approximation

Per-folder PERMANENTFLAGS capability exists but is not tracked individually. The account-level flag is conservative: set to supported only if all synced folders support it. The action layer must still tolerate server-side rejection gracefully even when cached capability says "supported."

## Implementation Steps

### Step 0 — Fix INSERT OR REPLACE label_kind bug

**Prerequisite for all subsequent steps.**

**Gmail** (`crates/gmail/src/sync/labels.rs`, `persist_labels()`):
Change `INSERT OR REPLACE INTO labels (id, account_id, name, type, color_bg, color_fg)` to `INSERT INTO labels ... ON CONFLICT(account_id, id) DO UPDATE SET name=excluded.name, type=excluded.type, color_bg=excluded.color_bg, color_fg=excluded.color_fg`. This preserves `label_kind` on conflict instead of resetting to DEFAULT.

**IMAP** (`crates/imap/src/sync_pipeline.rs`) and **JMAP** (`crates/jmap/src/sync/mailbox.rs`): same conversion for their `INSERT OR REPLACE` into `labels`. The DEFAULT `'container'` is correct for folders but the `ON CONFLICT DO UPDATE` pattern is safer.

### Step 1 — Callsite audit + remove legacy writes

Before deleting anything, verify each old write has a unified replacement with the same semantics:

| Old write | Location | Unified replacement | Same scope? |
|-----------|----------|-------------------|-------------|
| Gmail `sync_labels_to_categories()` | `gmail/sync/labels.rs:61-111` | `persist_labels()` writes to `labels` with `label_kind` | Yes — same label set |
| Gmail `sync_message_categories()` | `gmail/sync/storage.rs:276-314` | `thread_labels` written during thread persistence | **Exit gate** — see below |
| Graph `upsert_category()` | `graph/category_sync.rs:55-69` | `cat:` prefix labels in same function (lines 74-92) | Yes — same categories |
| Graph `insert_message_categories()` | `graph/sync/persistence.rs:118` | `thread_labels` written in same `store_thread_to_db` | **Exit gate** — see below |
| JMAP `upsert_category()` + `insert_message_categories()` | `jmap/sync/storage.rs:272-325` | `kw:` prefix labels + `thread_labels` in same function | Yes |
| JMAP mailbox `upsert_category()` | `jmap/sync/mailbox.rs:138-155` | `labels` table inserts in same function | Yes — these are containers |
| IMAP folder `upsert_category()` | `imap/sync_pipeline.rs:430-451` | `labels` table inserts in same function | Yes — containers |

#### Exit gate: message-level vs thread-level equivalence

Gmail `sync_message_categories()` and Graph `insert_message_categories()` write message-granular category evidence into `message_categories`. The unified path writes to `thread_labels`, which is a thread-level rollup. These two call sites must NOT be deleted until it is demonstrated that:

1. **Removal recalculation is equivalent.** When a label is removed from one message in a multi-message thread, the unified `thread_labels` path must recalculate whether the thread still has that label (i.e., check remaining messages). If the unified path only inserts and never recalculates on partial removal, deleting the message-level table loses the evidence needed to do so later.
2. **Partial-thread membership is preserved.** If only some messages in a thread have a given category, the thread-level rollup must still show the label. Verify the unified path inserts a `thread_labels` row whenever *any* message in the thread carries the category — not only when all messages do.

The implementation must verify both properties by reading the unified sync code paths for Gmail and Graph before deleting these two call sites. If the unified path does not satisfy both, it must be fixed first.

**After audit confirms parity (including the exit gate above)**, remove:

- **Gmail**: delete `sync_labels_to_categories()` + call site, delete `sync_message_categories()` + call site
- **Graph**: remove `upsert_category()` calls in `graph_categories_sync()`, remove `insert_message_categories()` call in `store_thread_to_db()`
- **JMAP storage**: remove `upsert_category()` + `insert_message_categories()` calls in `sync_keyword_categories()`, remove the incorrect `thread_categories` INSERT (keywords are not AI bundles)
- **JMAP mailbox**: remove `upsert_category()` loop
- **IMAP**: remove `upsert_category()` loop

If `graph_categories_sync()` becomes just the unified label writes after removing `upsert_category()`, rename to `graph_label_sync()`. Similarly rename `sync_keyword_categories()` to `sync_keyword_labels()`.

### Step 2 — Delete dead code

- `upsert_category()`, `CategoryColors`, `CategorySortOnConflict` from `crates/db/src/db/queries.rs`
- `insert_message_categories()` from `crates/sync/src/persistence.rs`
- `db_get_categories()` from `crates/core/src/db/queries_extra/misc.rs`
- `DbCategory` from `crates/db/src/db/types.rs`
- `DbCategory` FromRow impl from `crates/db/src/db/from_row_impls.rs`
- Dead imports in all modified files
- Hold on `find_label_id_by_name()` from `crates/gmail/src/ops.rs` — deleted in Step 3b

### Step 3 — Consolidate provider trait methods

#### 3a — Action dispatch (`crates/core/src/actions/label.rs`)

- Keep the `(account_id, label_id)` validation query
- **Add `label_kind = 'tag'` guard**: if the label is a container, return `Failed { error: "container labels use move operations, not add/remove" }`
- Remove `label_name` from the dispatch path (no longer needed for provider routing)
- Remove `label_kind` branching — always call `add_tag`/`remove_tag` (the guard has validated it's a tag)
- Simplify `_local` function signatures: the DB query reduces to validating `(account_id, label_id, label_kind='tag')` exists + the `INSERT`/`DELETE` on `thread_labels`

#### 3b — Gmail (`crates/gmail/src/ops.rs`)

`add_tag`/`remove_tag` already work — native label IDs passed to `modify_thread()`. Gmail user labels are stored with their native Gmail ID (no prefix) in the `labels` table.

- Delete `apply_category` and `remove_category` implementations
- Delete `find_label_id_by_name()` helper

Note: this changes the operation from per-message (`modify_message` in `apply_category`) to per-thread (`modify_thread` in `add_tag`). For Gmail, `modify_thread` with a label applies it to all messages in the thread, which matches the spec: "applies it to all existing messages in that thread."

#### 3c — Graph (`crates/graph/src/ops/mod.rs`)

`add_tag`/`remove_tag` already detect `cat:` prefix and do category array read-modify-write with lock.

- Delete `apply_category` and `remove_category` implementations

#### 3d — JMAP (`crates/jmap/src/ops.rs`)

`add_tag`/`remove_tag` currently do mailbox operations via `resolve_mailbox_id()`. `resolve_mailbox_id()` has no `kw:` handling and will fail on keyword label IDs.

- Add `kw:` prefix detection BEFORE `resolve_mailbox_id()`: strip prefix, use `email_set.keyword(name, true/false)` (same logic as current `apply_category`/`remove_category`)
- Non-`kw:` IDs fall through to existing mailbox path
- Delete `apply_category` and `remove_category` implementations

No prefix collision risk: `kw:` cannot collide with `jmap-` (user mailbox prefix) or system folder IDs (`INBOX`, `TRASH`).

#### 3e — IMAP (`crates/imap/src/ops.rs`)

`add_tag` is currently a no-op. `apply_category` does the actual keyword work via `parse_imap_message_id` + `set_keyword_if_supported`.

- `add_tag`: detect `kw:` prefix → strip to get keyword → query DB for thread's messages via `get_thread_message_refs()` (same pattern as `mark_read`/`star`) → call `set_keyword_if_supported()` per message with `+FLAGS`
- `remove_tag`: same pattern with `-FLAGS`
- Non-`kw:` IDs → no-op (correct: IMAP has no native non-keyword tags)
- Delete `apply_category` and `remove_category` implementations

#### 3f — Trait (`crates/provider-utils/src/ops.rs`)

- Remove `apply_category` and `remove_category` method definitions and default implementations

### Step 4 — Migration to drop tables

New migration (next number in sequence):

```sql
DROP TABLE IF EXISTS message_categories;
DROP TABLE IF EXISTS categories;
```

Order: `message_categories` first (references `messages` via FK, though no other table references either of these).

This step runs AFTER Steps 1-3 are complete and verified. No code references to these tables should remain.

### Step 5 — IMAP PERMANENTFLAGS capability persistence (independent)

Can run anytime after Step 3.

#### 5a — Schema

Add `supports_keywords INTEGER DEFAULT NULL` to the `accounts` table (new migration). NULL = unknown, 0 = not supported, 1 = supported. Explicitly an account-level approximation.

#### 5b — Populate

During IMAP sync, after SELECT returns folder status, update the account's flag. Conservative bias: set to `1` only if all synced folders support it. Set to `0` if any folder doesn't. One write per sync cycle.

#### 5c — Action service: no hard-reject preflight

**Revised decision:** The action service does NOT hard-reject `kw:` label operations based on `supports_keywords = 0`. The `supports_keywords` flag tracks whether PERMANENTFLAGS includes `\*` (arbitrary keywords), but servers with fixed permanent flags (e.g., `$label1` through `$label5`) can still accept those specific keywords even without `\*`. Hard-rejecting at the action layer would break fixed-keyword servers — a real behavior regression.

Instead: the local DB write always proceeds (the label exists in the `labels` table because sync created it from server-reported flags), and the IMAP provider's `set_keyword_if_supported` handles per-folder rejection gracefully at the protocol level. If the server rejects the keyword STORE, the action gets `LocalOnly` — the same outcome as any other provider failure.

The `supports_keywords` column is advisory metadata for future UI gating only (e.g., graying out "create new keyword label" in the label picker for accounts without `\*` support). It does NOT gate individual keyword apply/remove operations.

#### 5d — Expose

Add `supports_keywords: Option<bool>` to account info returned in navigation state, so a future label picker can gray out keyword-apply for IMAP accounts without support.

## Execution Order

```
Step 0 (fix INSERT OR REPLACE bug)
  ↓
Step 1 (audit + remove legacy writes)
  ↓
Step 2 (delete dead code)
  ↓
Step 3 (consolidate trait methods)
  ↓
Step 4 (migration to drop tables)
  ↓
Step 5 (IMAP capability — independent, can run after Step 3)
```

Each step compiles independently. Steps 1-3 can each be verified in isolation before proceeding. Step 4 is the last code change before the tables are dropped. Step 5 is an enhancement with no dependency on Step 4.

## Verification Checklist

After each step:

- [ ] `cargo check --workspace`
- [ ] `cargo clippy -p app -p ratatoskr-core -p ratatoskr-sync -p gmail -p graph -p jmap -p imap`

After Step 1:
- [ ] Grep: zero calls to `upsert_category`, `insert_message_categories`, `sync_labels_to_categories`, `sync_message_categories`
- [ ] Unified `labels`/`thread_labels` still populated for all four providers

After Step 2:
- [ ] Grep: zero definitions of `upsert_category`, `CategoryColors`, `CategorySortOnConflict`, `insert_message_categories`, `db_get_categories`, `DbCategory`

After Step 3:
- [ ] Grep: zero references to `apply_category`, `remove_category` across all crates
- [ ] Grep: zero references to `find_label_id_by_name`
- [ ] Label apply/remove works for: Gmail user labels, Graph categories (`cat:`), JMAP keywords (`kw:`), IMAP keywords (`kw:`)
- [ ] Container labels rejected by action service (returns `Failed`, not routed to provider)

After Step 4:
- [ ] Grep: zero SQL references to `categories` table (distinguish from `thread_categories`)
- [ ] Grep: zero SQL references to `message_categories` table

After Step 5:
- [ ] IMAP accounts without keyword support: action returns `Failed`
- [ ] IMAP accounts with keyword support: action returns `Success` on provider success

## Documentation Updates

After all steps:
- [ ] `TODO.md`: mark Phase 6 item complete, update glossary cleanup checklist items
- [ ] `docs/glossary.md`: remove "Known Terminology Debt" items for `apply_category`/`remove_category` and `categories`/`message_categories` tables; update `category_sync.rs` → `label_sync.rs` item
- [ ] `docs/labels-unification/problem-statement.md`: mark Phase 6 complete

## What Phase 6 Does NOT Do

- **Rename `thread_categories` → `thread_bundles`**: AI bundling table rename is independent cleanup (glossary item).
- **Rename `category_colors.rs` → `preset_colors.rs`**: Exchange color preset module rename is independent cleanup (glossary item).
- **Rename `CATEGORY_PRIMARY` constants**: Bundle constant rename is independent cleanup (glossary item).
- **Batch optimization**: Per-thread label metadata lookup is redundant across batches. Hoisting deferred to future work.
