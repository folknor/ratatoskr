# Labels & Folders Redesign

## Context

The pre-refactor Ratatoskr label model had two long-standing pain points:

1. **The `labels` table holds both folders and tags**, distinguished by a `label_kind` column. `docs/glossary/folders-labels.md` openly apologises for this in the "Why both are in the `labels` table" section. Every read path branches on the discriminator. Provider sync, sidebar rendering, and the action service all carry it. The cross-account LABELS aggregation bug (`docs/glossary/discrepancies.md`) is partly a consequence: per-account folders and cross-account tags share one query layer but need different semantics.

2. **Cross-account labels auto-collapse by normalised name**. The labels-unification spec made `LOWER(TRIM(name))` the grouping key. A "Work" label on Gmail and a "Work" category on Exchange become one pill. That collapse drives rename/delete fan-out, colour resolution conflicts, and ambiguous identity on partial failures.

This document records two reversals: split the storage, and replace auto-collapse with explicit user-created groups.

## What this supersedes

- `docs/labels-unification/problem-statement.md` (since deleted): the auto-collapse-by-name model it described is reversed here, and the schema phases (1-6) it proposed are superseded by the design below.
- The removed "Why both are in the `labels` table" section of `docs/glossary/folders-labels.md`. Post-split, the code identifier rule is "`folder` always means a row in `folders`; `label` always means a row in `labels`; `label_group` means a row in `label_groups`."
- The removed `label_color_overrides` table. User-recolour intent moves into per-row `user_color_*` columns on `labels` (visible only in Settings) and to the group's `color_bg`/`color_fg` (visible in sidebar and pills).

`docs/glossary/folders-labels.md` remains the source of truth for the user-facing folder/label/message-state classification per provider. None of that classification changes. This document is the implementation design for it.

## Reversal 1: split storage

The split partitions the old unified `labels` table into two tables. Every column on the old table is preserved in one or both of the new tables; no metadata is dropped. The full partition:

```
folders (
    account_id          TEXT    NOT NULL,
    id                  TEXT    NOT NULL,         -- canonical for system roles; graph-{guid}/folder-{path}/jmap-{id} for user; bare native Gmail string for CATEGORY_*/CHAT
    name                TEXT    NOT NULL,
    visible             INTEGER NOT NULL DEFAULT 1,
    sort_order          INTEGER NOT NULL DEFAULT 0,
    imap_folder_path    TEXT,
    imap_special_use    TEXT,
    namespace_type      TEXT,
    parent_id           TEXT,                     -- self-FK; references folders.id on same account_id
    right_read          INTEGER,
    right_add           INTEGER,
    right_remove        INTEGER,
    right_set_seen      INTEGER,
    right_set_keywords  INTEGER,
    right_create_child  INTEGER,
    right_rename        INTEGER,
    right_delete        INTEGER,
    right_submit        INTEGER,
    is_subscribed       INTEGER,
    is_undeletable      INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id, parent_id) REFERENCES folders(account_id, id) ON DELETE CASCADE
)
CREATE INDEX folders_parent ON folders(account_id, parent_id);

labels (
    account_id      TEXT    NOT NULL,
    id              TEXT    NOT NULL,        -- native Gmail label id; cat:{name}; kw:{keyword}; importance:high|low
    name            TEXT    NOT NULL,
    visible         INTEGER NOT NULL DEFAULT 1,
    sort_order      INTEGER NOT NULL DEFAULT 0,
    server_color_bg TEXT,                    -- renamed from color_bg; sync-supplied colour
    server_color_fg TEXT,                    -- renamed from color_fg
    user_color_bg   TEXT,                    -- per-label user override, visible only in Settings
    user_color_fg   TEXT,                    -- per-label user override, visible only in Settings
    is_undeletable  INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, id),
    FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
)
```

Settings colour resolution for an individual label: `user_color_*` first, then `server_color_*`, then deterministic hash fallback. Outside Settings (sidebar pills, message pills), per-label colour is never displayed; the group's colour governs.

Two junctions replace the old unified `thread_labels` table:

```
thread_folders (
    account_id TEXT NOT NULL,
    thread_id  TEXT NOT NULL,
    folder_id  TEXT NOT NULL,
    PRIMARY KEY (account_id, thread_id, folder_id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE,
    FOREIGN KEY (account_id, folder_id) REFERENCES folders(account_id, id) ON DELETE CASCADE
)
CREATE INDEX thread_folders_by_folder ON thread_folders(account_id, folder_id, thread_id);

thread_labels (
    account_id TEXT NOT NULL,
    thread_id  TEXT NOT NULL,
    label_id   TEXT NOT NULL,
    PRIMARY KEY (account_id, thread_id, label_id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE,
    FOREIGN KEY (account_id, label_id) REFERENCES labels(account_id, id) ON DELETE CASCADE
)
CREATE INDEX thread_labels_by_label ON thread_labels(account_id, label_id, thread_id);
```

Three columns were dropped from the old unified `labels` table:
- `label_kind`: structural now (table-of-origin).
- `label_type`: the old command-palette reader filtered `l.type != 'system'` to scope palette options. Post-split, the equivalent query selects from `labels` directly. System folders are in `folders` by definition, so the filter is structural. `label_type` has no consumers.
- `color_bg` / `color_fg` (old names): renamed to `server_color_bg` / `server_color_fg` on `labels`. Folders have no colour.

### Per-provider mapping

Classification stays exactly as `docs/glossary/folders-labels.md` describes. After the split:

- **Gmail**: system labels (INBOX, SENT, DRAFT, TRASH, SPAM, IMPORTANT, CHAT, CATEGORY_*) go to `folders`. Canonical IDs for the ones in `SYSTEM_FOLDER_ROLES` (INBOX, SENT, DRAFT, TRASH, SPAM, archive, IMPORTANT); bare native Gmail strings for `CHAT` and `CATEGORY_*` (which are not in `SYSTEM_FOLDER_ROLES`). User labels go to `labels`. STARRED, SNOOZED, and the All-Mail view are virtual navigation handles backed by booleans on `threads`; no row in `folders` or `labels` exists for them, per `docs/glossary/folders-labels.md:183`.
- **Graph**: mail folders go to `folders`. `masterCategories` entries and synthesised `importance:high`/`importance:low` go to `labels`.
- **IMAP**: mailboxes go to `folders`. Custom keywords go to `labels` (when `PERMANENTFLAGS \*` is supported).
- **JMAP**: mailboxes go to `folders`. Custom keywords go to `labels`.

The synth path for Graph importance and the prefixed-label escape valve (`crates/service/src/actions/label.rs::ensure_prefixed_tag_label`) writes to `labels`.

### Role identity for system folders

The canonical ID is the role identity. The `SYSTEM_FOLDER_ROLES` map (`crates/db/src/db/folder_roles.rs`) normalises provider-native role names to canonical Ratatoskr IDs at sync ingest, so after ingest a row's `id` is either canonical (`INBOX`, `SENT`, `DRAFT`, `TRASH`, `SPAM`, `archive`, `IMPORTANT`) or provider-prefixed. Queries that need the inbox folder for an account read `WHERE id = 'INBOX'`. `BROAD_INBOX_EXCLUSIONS` (`crates/db/src/db/queries_extra/scoped_queries.rs`) keeps using canonical IDs directly. No separate `role_kind` column.

### ID encoding by origin

This table documents the ID encoding *post-ingest*. The action service receives typed IDs (`FolderId` or `LabelId`) per `crates/common/src/typed_ids.rs`; routing is structural through the type system, not string-prefix-based at the dispatch layer. Ingest is where provider context decides which table a provider-native string lands in.

| Origin                                  | Table     | ID encoding                                                          |
|-----------------------------------------|-----------|----------------------------------------------------------------------|
| System role (any provider)              | `folders` | Canonical: `INBOX`, `SENT`, `DRAFT`, `TRASH`, `SPAM`, `archive`, `IMPORTANT` |
| Gmail `CATEGORY_*`, `CHAT`              | `folders` | Bare native Gmail string                                             |
| Graph user folder                       | `folders` | `graph-{guid}`                                                       |
| IMAP user folder                        | `folders` | `folder-{path}`                                                      |
| JMAP user mailbox                       | `folders` | `jmap-{id}`                                                          |
| Gmail user label                        | `labels`  | Bare native Gmail label ID                                           |
| Exchange category                       | `labels`  | `cat:{name}`                                                         |
| IMAP/JMAP keyword                       | `labels`  | `kw:{keyword}`                                                       |
| Synth (Graph importance)                | `labels`  | `importance:high` / `importance:low`                                 |

Bare native strings appear in both tables. Pre-ingest, a Gmail label arriving as a bare string is classified by provider context (Gmail API's `type: "system"` vs `type: "user"`) plus `SYSTEM_FOLDER_ROLES`. Post-ingest, the row exists in exactly one table; the type system carries that knowledge from there.

### Folder hierarchy insertion order

`folders.parent_id` is a self-FK with `ON DELETE CASCADE`, which means INSERTs must land parent-before-child. Three of four providers can present folders in arbitrary order during sync:

- **IMAP**: path delimiters carry hierarchy; child paths can arrive before parent paths in `LIST` responses or piecemeal during incremental updates.
- **Graph**: `parentFolderId` is explicit but delta-token responses return folders individually without ordering guarantees.
- **JMAP**: `Mailbox/get` / `Mailbox/changes` returns mailboxes in arbitrary order.
- **Gmail**: no folder hierarchy. `parent_id` is never populated.

To make the FK safe by construction (the "make the right thing the only thing" principle in `docs/architecture.md`), all folder inserts route through a single `db` helper `insert_folders_batch(account_id, rows)` which topologically sorts by `parent_id` (Kahn's algorithm or BFS from roots) before issuing INSERTs. Provider sync paths must use this helper; direct INSERTs against `folders` are not part of the public surface. The fan-out is one call site per provider's folder-sync helper (three providers), plus the helper itself. Gmail's folder-sync path uses the same helper but always passes flat lists with `parent_id = NULL`.

### `message_keywords`

The per-message keyword store (`crates/db/src/db/schema/02_mail.sql`) keeps its current shape. Its `label_id` column retargets to the new labels-only `labels` table. `recompute_thread_keyword_labels` (`crates/provider-sync/src/imap/sync_pipeline.rs`) keeps writing thread-level rollups into the new labels-only `thread_labels`. The old `WHERE label_id LIKE 'kw:%'` filter, which scoped the delete to keyword rows within the unified junction, is unnecessary post-split because the junction is labels-only.

### Junction-helper duplication

Before the split, two helpers maintained `thread_labels`: `replace_thread_labels` (full-thread replace, used by Gmail and IMAP via `crates/sync/src/pipeline.rs::store_thread_groups_to_db`) and `merge_thread_labels` (partial-delta merge, used by Graph and JMAP). Post-split, four helpers exist: `replace_thread_folders`, `merge_thread_folders`, `replace_thread_labels`, `merge_thread_labels`. Each owns one junction.

Mechanical duplication, no genericisation. The two junctions have different FK targets, different prefix vocabularies, and different downstream consumers; generics here obscure more than they save. Provider-sync code already has to bifurcate incoming label sets into "this one is a folder" / "this one is a label" at parse time, so call sites typically invoke folder-side and label-side helpers in the same transaction.

On the action side, `crates/service/src/actions/move_to_folder.rs::move_local` (the largest action by volume) retargets its INSERT/DELETE from `thread_labels` to `thread_folders`. The action-side helpers it relies on follow the same renaming: folder-targeting actions hit `thread_folders`; label-targeting actions hit `thread_labels`.

### `is_undeletable`

Set on rows the user must not be allowed to delete from the Ratatoskr UI:

- **Any provider-classified system entity**, set at sync ingest. Includes canonical roles in `SYSTEM_FOLDER_ROLES` (INBOX, SENT, DRAFT, TRASH, SPAM, archive, IMPORTANT) and Gmail's other `type: "system"` labels (`CATEGORY_*`, `CHAT`). Graph well-known folders, IMAP special-use mailboxes, and JMAP system-role mailboxes are also flagged. The classification source is the provider's own system flag at ingest, not Ratatoskr's role map.
- **Synthesised labels** (`importance:high`, `importance:low`).

Default Outlook category presets ("Red Category" etc.) are not flagged. They are the `type: "user"` analog on Exchange (regular user-created categories that ship preloaded) and are deletable like any user-created entity, matching Outlook's own behaviour.

**Invariant**: any `importance:*` row in `labels` carries `is_undeletable = 1`, regardless of which writer produced it. The bootstrap synth path and the on-demand escape valve (`crates/service/src/actions/label.rs::ensure_prefixed_tag_label`) both set the flag.

## Reversal 2: cross-account labels are explicit groups

Auto-collapse by normalised name is removed. Cross-account label identity is an explicit user-created entity.

### New tables

```
label_groups (
    id       INTEGER PRIMARY KEY,
    name     TEXT    NOT NULL,
    color_bg TEXT    NOT NULL,
    color_fg TEXT    NOT NULL,
    UNIQUE (name COLLATE NOCASE)
)

label_group_members (
    group_id   INTEGER NOT NULL,
    account_id TEXT    NOT NULL,
    label_id   TEXT    NOT NULL,
    PRIMARY KEY (group_id, account_id, label_id),
    UNIQUE (account_id, label_id),                                                       -- a per-account label belongs to at most one group
    FOREIGN KEY (group_id) REFERENCES label_groups(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id, label_id) REFERENCES labels(account_id, id) ON DELETE CASCADE
)

thread_label_groups (
    account_id TEXT    NOT NULL,
    thread_id  TEXT    NOT NULL,
    group_id   INTEGER NOT NULL,
    PRIMARY KEY (account_id, thread_id, group_id),
    FOREIGN KEY (account_id, thread_id) REFERENCES threads(account_id, id) ON DELETE CASCADE,
    FOREIGN KEY (group_id) REFERENCES label_groups(id) ON DELETE CASCADE
)
CREATE INDEX thread_label_groups_by_group ON thread_label_groups(group_id);
```

`UNIQUE(name COLLATE NOCASE)` on `label_groups` is load-bearing: smart-folder and search resolve `label:Work` by name, so duplicate names would be ambiguous. The constraint also covers user typos that would otherwise create near-duplicate groups ("Work" vs "work").

`UNIQUE(account_id, label_id)` on `label_group_members` is load-bearing: it makes shared membership impossible. The reason is operational, not philosophical: if a per-account label X were a member of two groups G1 and G2, removing G1 from a thread would dispatch a `RemoveLabel` for X, which would also clear G2's `thread_labels`-path rendering on that thread. That is the wrong behaviour but cannot be detected without provenance tracking. Forbidding shared membership eliminates the case structurally. UX consequence: adding a label to a group it is not yet in fails if the label is already in another group; the picker surfaces "this label is in <other group>; move it?" and on confirmation does the swap.

`thread_label_groups.account_id` is not redundant. Groups are cross-account, but the *thread* being attached to is single-account, and `threads` PK is `(account_id, id)`. `account_id` here is the join key for the FK back to `threads`, not a per-account scoping axis on groups. Since threads are single-account in Ratatoskr, `(account_id, thread_id)` uniquely identifies the thread; group attachment is conceptually per-thread, with `account_id` determined by `thread_id`.

### Principle

> `labels` and `thread_labels` are ratatoskr's view of provider-observable state. Writes come from sync, the escape valve (`crates/service/src/actions/label.rs::ensure_prefixed_tag_label`), and the synth path for derived primitives (Graph `importance`).
> `label_groups` and `thread_label_groups` are what the user has done.

The two pairs are independent. No row in `label_groups` is system-managed. The sidebar's LABELS section starts empty on a fresh install and remains empty until the user creates a group.

### New types

- `LabelGroupId(i64)` newtype in `crates/types/`.
- `SidebarSelection::LabelGroup(LabelGroupId)` variant.
- `get_threads_for_label_group(group_id, scope)` in `crates/db/src/db/queries_extra/scoped_queries.rs`: returns threads matching either rendering path (see below).

### Sidebar rendering

The LABELS section renders one entry per `label_groups` row, sorted alphabetically by name. The unread count for each group is the count of distinct threads matching either:

- A row in `thread_label_groups` for that group, **or**
- A row in `thread_labels` whose `(account_id, label_id)` is in `label_group_members` for that group.

The two conditions are unioned.

### Message pill rendering

A pill for group G renders on thread T if:

- A row in `thread_label_groups` exists for (T, G), **or**
- T has any `thread_labels` row whose `(account_id, label_id)` is in `label_group_members` for G.

A thread can render two pills for two groups by satisfying both conditions independently. Because `label_group_members` enforces single-group membership per per-account label, a single `thread_labels` row contributes to at most one group pill.

### Member-incompatibility rules

Some per-account labels are mutually exclusive at the provider level and must not coexist as members of the same group. The only such pair today is Graph's `importance:high` / `importance:low` (`docs/glossary/folders-labels.md:67`, "Mutually exclusive at write time"). The Settings member picker enforces this rule at the UI layer: adding `importance:high` to a group that already contains `importance:low` is rejected with an explanation.

The constraint is provider-specific and small; not encoded in the schema. Future provider-specific mutex pairs extend this picker rule.

## Action pipeline integration

The message UI never operates on per-account labels directly. Apply/remove targets `MailActionIntent::ApplyLabelGroup` / `RemoveLabelGroup` only; there is no equivalent intent for raw `AddLabel`/`RemoveLabel` from the message-level UI. Per-account label create/rename/delete affordances live in Settings only.

Two new `MailOperation` variants:

- `MailOperation::ApplyLabelGroup { group_id }`
- `MailOperation::RemoveLabelGroup { group_id }`

Thread identity is carried on `ActionWireOperation`'s envelope (`crates/service/src/actions/operation.rs:19`, `crates/service-api/src/action.rs:116`) and is not duplicated on the variants. Each is a **single composite operation** at the action-planner level. Internal fan-out (per-member provider dispatches) happens inside the service-side action function, hidden from the planner. This matches the planner's existing assumption that `ResolvedIntent` derives behaviour and undo from one operation per target (`crates/app/src/action_resolve.rs:68`, `:515`, `:681`).

Both must follow the full checklist in `docs/architecture.md` section "Adding a New Email Action":
- Variant in `MailActionIntent` (`ApplyLabelGroup` / `RemoveLabelGroup`).
- Variant in `MailOperation` (the two composite variants above).
- Variant in `WireMailOperation` (`crates/service-api/src/action.rs`), 1:1 with `MailOperation`.
- Arms in `to_wire_op` / `wire_to_mail`.
- Arms in `resolve_intent()` (intent -> operation, 1:1).
- Arms in `completion_behavior()` (view effect, toast, undo).
- Service-side action functions (`crates/service/src/actions/label_group.rs`) for apply and remove.
- Arms in `batch.rs` routing.
- `MailUndoPayload` variants: `ApplyLabelGroup` pairs with `RemoveLabelGroup` as composite inverses.

### Apply group to thread

`MailOperation::ApplyLabelGroup { group_id }` on thread T (account A). The service-side action function:

1. Inserts a row in `thread_label_groups` for (T, G). Idempotent (PK conflict is a no-op).
2. Reads `label_group_members` for G filtered to `account_id = A`. For each member label, dispatches the provider `AddLabel` for (T, label) and inserts the corresponding `thread_labels` row.
3. Tracks per-member outcomes internally. Returns `OperationOutcome::Success` if all member dispatches succeeded; returns `OperationOutcome::LocalOnly` if any failed, with the failing member IDs persisted in a service-side per-op retry queue.

If no member is on account A, step 2 has nothing to dispatch and the operation succeeds purely from local intent.

### Remove group from thread

`MailOperation::RemoveLabelGroup { group_id }` on thread T (account A). The service-side action function:

1. Deletes the `thread_label_groups` row for (T, G), if present. Idempotent.
2. Reads existing `thread_labels` rows on T whose `(account_id, label_id)` is in `label_group_members` for G. For each, dispatches `RemoveLabel` and deletes the local row.
3. Tracks per-member outcomes. Returns `Success` or `LocalOnly` per the same rule as apply.

### Retry preflight

When the pending-ops mechanism retries a queued `ApplyLabelGroup` or `RemoveLabelGroup`, the service-side function reads the current `thread_label_groups` state for (T, G) before dispatching any member writes:

- **Apply retry**: if `thread_label_groups(T, G)` no longer exists, the user has since removed the group. The retry skips all queued member `AddLabel` dispatches and resolves successfully; the local state already matches the user's current intent.
- **Remove retry**: if `thread_label_groups(T, G)` exists again (user re-applied), the retry skips queued member `RemoveLabel` dispatches.

This prevents stale retries from resurrecting or re-clearing a pill against current user intent.

### Failure semantics

Provider `AddLabel` / `RemoveLabel` dispatches follow the existing label-action contract (`crates/service/src/actions/label.rs:111`, `crates/service-api/src/action.rs:186`): local writes commit optimistically and do not roll back on provider failure. A failed dispatch returns to the composite action function as a per-member error and lands in the service-side retry queue. The local `thread_label_groups` and `thread_labels` writes from steps 1 and 2 stay committed regardless of provider outcomes.

The composite op reports a single `OperationOutcome` to the planner. Internal partial failures do not surface as separate outcomes; they are reconciled by the service-side retry mechanism with the preflight described above.

### Sync precedence vs local intent

Five concrete cases:

- **Sync adds a member label to a thread.** Inserts a `thread_labels` row. If `thread_label_groups` already held a row for that group, no visible change. If it did not, the pill now renders via the `thread_labels` path.
- **Sync removes a member label from a thread.** Deletes the `thread_labels` row. If `thread_label_groups` row still holds, the pill stays (local intent wins). If not, and no other member is on the thread, the pill disappears.
- **Sync deletes a label entirely** (provider deleted it, or it dropped out of the master list). CASCADE wipes the `labels` row, its `thread_labels` rows, and its `label_group_members` row (unique per label). The group's member count drops by one. If the group hits zero members, the group still exists, still renders in the sidebar, still attaches to threads via `thread_label_groups` rows that survived.
- **Sync renames a label.** `UPDATE labels.name`. The group's `name` is independent of any member's name, so the group is unaffected.
- **Account is removed.** CASCADE on `account_id` wipes that account's `folders`, `labels`, `thread_folders`, `thread_labels`, and any `label_group_members` rows for those labels. `label_groups` rows have no `account_id` and persist. Groups may become zero-member.

## Smart-folder and search semantics

The `label:` operator in smart-folder queries targets *groups*, not raw provider labels. The current SQL resolves `label:Work` against `label_groups.name` with case-insensitive comparison and matches the resolved group through either rendering path. Persisted smart folders store the textual query, so a group rename changes which group a name-based query resolves to.

`label:Work` matches threads attached to the group via either rendering path (`thread_label_groups` or `thread_labels`-via-members). Raw provider labels are not user-facing and have no operator.

`is:tagged` returns threads attached to any group via either path.

**User-visible behaviour change**: threads carrying labels that are not members of any group stop matching `is:tagged`. Those labels remain in the per-account view in Settings but are not represented in any group-based query. Users who want a label to participate in `is:tagged` add it to a group.

Thread pill decorations (`crates/db/src/db/queries_extra/thread_detail.rs`) read groups, not raw labels. The decoration query reads from `thread_label_groups` joined to `label_groups`, unioned with `thread_labels` joined to `label_group_members` joined to `label_groups`, deduped by `group_id`.

This is a behavioural change from the old `label:` operator, which hit raw `thread_labels` joined to `labels`. The smart-folder SQL builder (`crates/smart-folder/src/sql_builder.rs`) now routes `label:` and `is:tagged` through label groups; other operators are unaffected.

## Performance

The sidebar's LABELS counts and the per-message pill decoration both UNION two tables (`thread_label_groups` and `thread_labels` joined through `label_group_members`). At 150 GB cached mailbox sizes (per `AGENTS.md`), this is a hot path.

- **Canonical aggregate function**: one query function (`get_label_group_unread_counts` in `crates/db/src/db/queries_extra/scoped_queries.rs`) is the single source of truth for sidebar counts. All consumers route through it. No ad-hoc UNION at call sites.
- **Indexes**: `thread_labels_by_label(account_id, label_id, thread_id)` covers the join from members to threads. `thread_label_groups_by_group(group_id)` covers the local-intent side. `label_group_members.UNIQUE(account_id, label_id)` provides the reverse index for member lookups.
- **Distinct counting**: dedup on `(account_id, thread_id)`, not `thread_id` alone, since threads are scoped by account. Cross-account counts sum over distinct `(account_id, thread_id)` pairs.
- **Harness coverage**: Lua service-harness scripts (`docs/glossary/harness.md`) should exercise the count path at fixture scale, with at least one cohort that stresses the UNION across thousands of threads with many groups.

## Group lifecycle

### Group creation

Groups are created exclusively by the user, in Settings. Two views:

1. **Per-account labels**: a read-mostly view of `labels` filtered by account. Shows every per-account entity. Each row indicates which group (if any) it belongs to. The view is generation-counter-protected (`docs/architecture.md` section "Generation counters") and invalidates on `labels` writes from sync, so a stale add-to-group click that races a sync delete fails cleanly on FK violation and the picker refetches.
2. **Label groups**: list of `label_groups`. Each group has name, colour, and a member list. The member picker reads from view 1 and enforces the member-incompatibility rules described above.

A group with zero members is valid. It renders in the sidebar and can be applied to threads via local intent.

### Group membership changes

Adding or removing a member to/from a group is a pure local mutation against `label_group_members`. No provider writes. No backfill of `thread_labels`. Pill rendering recomputes at read time:

- **Adding a member to a group**: threads carrying that label start rendering the group pill via the `thread_labels` path on the next decoration read. No data write to `thread_labels` or `thread_label_groups`.
- **Removing a member from a group**: threads whose only backing for the group was via this member stop rendering the group pill, unless `thread_label_groups` row exists for that thread+group.

The action service has no role in membership changes. Settings dispatches local INSERT/DELETE on `label_group_members` directly (a Service-side IPC for the mutation, per the standard write split).

### Group deletion

User deletes group G:

1. Delete the `label_groups` row. CASCADE wipes `label_group_members` and `thread_label_groups`.
2. No provider writes are dispatched. Per-account labels stay where they are.

### Group rename

User renames group G:

1. UPDATE `label_groups.name`. That is all.

The `UNIQUE(name COLLATE NOCASE)` constraint rejects renames that collide with an existing group. Smart-folder references to G continue to work because they bind to `group_id`, not name.

## Bootstrap behaviour

### Adding an account (any provider)

1. Sync the folder tree, write `folders` rows via `insert_folders_batch`. Canonical IDs for system roles; provider-specific IDs for user folders.
2. Sync whatever the provider exposes as label-shaped entities, write `labels` rows. For Graph this includes `masterCategories` (the 6 presets if fresh) and synthesised `importance:high`/`importance:low`.
3. Sidebar's LABELS section stays empty. The user can open Settings to see what got synced and create groups.

### Synthesised rows for Graph

On Graph account add, ratatoskr writes two `labels` rows per account: `importance:high` and `importance:low`, both `is_undeletable = 1`. They have no `server_color_*`. They are invisible until the user creates a group containing them.

### Dev-seed

Dev-seed may ship pre-populated groups for the demo state. Seed data, not a system-managed feature. The schema knows nothing about it.

## Settings UI

Sketch only; details are presentation, not schema-affecting.

- **Labels per account**: one collapsible panel per account, listing that account's `labels` rows. Each row shows name, resolved colour (`user_color_*` -> `server_color_*` -> hash fallback), and a "in group: <group or none>" affordance. Per-row ordering uses `sort_order` (preserved from current schema), with optional `visible` filter. Per-label rename, recolour (writes `user_color_*`), and delete affordances live here.
- **Label groups**: list of all `label_groups`. Each group expands to show members. Add/remove members via a picker that reads from the labels-per-account view and enforces member-incompatibility rules.

Per-account-label rename/delete affordances dispatch provider writes. Presented as advanced; the common-path user does not need to touch them.

## Code identifier rule

After this split:
- `folder` in code refers to rows in `folders`. `folder_id` is the FK.
- `label` in code refers to rows in `labels`. `label_id` is the FK.
- `label_group` in code refers to rows in `label_groups`. `group_id` is the FK.

`LabelId` in `crates/types/` retains its meaning (per-account label ID) but is now strictly tag-only; no folder IDs flow through it. `FolderId` is unchanged. `LabelGroupId` is new.

## Open

- **Default colours for `importance:high` / `importance:low`** when included in a user group. Synth rows have no server colour, so something has to seed the group's colour on first add.
- **Resync cadence for Graph `masterCategories`**. Full fetch, no delta endpoint.
- **Stable smart-folder group binding**: the landed `label:` SQL resolves by group name at execution time. Binding by `group_id` would preserve a saved smart folder across group renames, but requires changing the persisted smart-folder representation away from plain text.
