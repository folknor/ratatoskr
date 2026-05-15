# Labels Unification: Spec vs. Code Discrepancies

Audit date: 2026-05-15 (supersedes 2026-03-30 audit).

Items are ordered by suggested working order: most-user-visible-with-no-open-design-questions first, cleanup and spec drift last, resolved items at the bottom for the historical record.

---

## Tier 1: Sidebar section 4 is broken end-to-end

These three are one change set - they touch the same builder and the same SQL shape. Fixing them together is cheaper than serializing.

### Section 4 is empty in All-Accounts scope

`get_navigation_state` (`crates/core/src/db/queries_extra/navigation.rs:122-131`) only calls `build_account_labels` in `AccountScope::Single`. In `AccountScope::All`, **no tag-type labels are built at all**. Spec says section 4 must always be the same regardless of scope. `build_all_account_tags` (claimed shipped in Phase 5 at `problem-statement.md:290`) does not exist.

### Cross-account grouping missing trim normalization (partial)

Two new queries that drive the labels-unification surfaces use `LOWER(TRIM(l.name))` correctly: `query_visible_labels` (cross-account grouped) and `query_labels_by_account` (per-account, settings) - both in `crates/core/src/db/queries_extra/navigation.rs`. `search_labels_for_typeahead_sync` (`navigation.rs:624-632`) still uses `COLLATE NOCASE` only; that's now the lone holdout.

### Unread counts are per-account, not cross-account (helper exists, not consumed)

`get_label_unread_counts` (`navigation.rs:345-373`) groups by `tl.label_id` and feeds the current per-account sidebar pills. A new helper `load_cross_account_unread_by_normalized_name` (in `navigation.rs`, alongside `query_visible_labels`) does the spec's required `LOWER(TRIM(l.name))` aggregation. It's used by `query_visible_labels` but **not yet by the sidebar** - sidebar section 4 still routes through `build_account_labels` and the per-account count helper. Wiring the sidebar to `query_visible_labels` is the remaining step (Tier 1 leftover).

---

## Tier 2: Missing user-facing label management

### Cross-account label creation missing (UI scaffolded, no action)

The settings tab (Mail Rules > Labels) has a `+ Add Label` row that opens a `LabelEditorState::new_create()` editor sheet, and the editor's Save button emits `SettingsMessage::LabelEditorSave`. The handler bottoms out at a stub (`crates/app/src/handlers/labels.rs::create_label_async`) that logs a warning and returns `CreatedAck(Err(_))`. No `CreateLabel` action variant, no Service handler, no provider fan-out.

### User-initiated label deletion missing (UI scaffolded, no action)

Same shape as create: editor sheet's Delete button emits `SettingsMessage::LabelEditorConfirmDelete`, but `delete_label_async` is a stub. No `DeleteLabel` action variant, no handler. The settings UI no longer opens the cross-account deletion path described in the spec; deletion is per-`(account_id, label_id)` in the new per-account list, which is a deliberate departure from `problem-statement.md:197-206` (revisit when scoping the action handler).

---

## Tier 3: Correctness and safety gates

### IMAP `supports_keywords` gate missing from action service

`accounts.supports_keywords` is written by sync (`crates/provider-sync/src/imap/{imap_initial,imap_delta}.rs`) but never read by the action service. The label-apply path (`crates/service/src/actions/label.rs`) has no preflight check. Phase 6's claim that the action service rejects unsupported accounts is unmet.

### `label_color_overrides` read path landed; write path still missing

Read path is wired: schema now includes `color_fg` (`crates/db/src/db/schema/02_mail.sql:31-35`). `resolve_label_color` takes an `override_color: Option<(&str, &str)>` and applies it as tier-1 (`crates/label-colors/src/lib.rs:35-58`). The new cross-account and per-account queries (`query_visible_labels`, `query_labels_by_account`) each call `load_label_color_overrides` once and pass the normalized-name override into the resolver per row.

Write path still missing. The settings editor sheet has color preview state and a `LabelEditorColorChanged` message, but `recolor_label_async` is a stub - no INSERT/UPDATE against `label_color_overrides`. Saving an edited color today does nothing.

The pre-existing call sites in `thread_detail.rs` and `navigation.rs::build_account_labels` pass `None` for the override - they still see the synced/hash colors only. Migrating those over is a one-line change per call site once the load-overrides-once pattern is acceptable to spread.

---

## Tier 4: Cleanup, design intent, and spec drift

### `is_folder_based_provider` dead-code trio

Gate removed from `command_resolver.rs` (this was prior audit's #1). Function still defined in three layers with no callers: `crates/app/src/db/palette.rs:53`, `crates/core/src/command_palette_queries.rs:59`, `crates/db/src/db/queries_extra/command_palette.rs:137`. Pure dead code.

### Label dispatch is thread-level, not per-message

Spec § "Applying and Removing Labels" describes per-message dispatch with per-message provider resolution. Code dispatches at thread level: `crates/service/src/actions/label.rs:109` calls `provider.add_label(thread_id, label_id)`. Provider implementations (`crates/gmail/src/ops.rs:124-148`, `crates/imap/src/ops.rs`) accept `thread_id`. Functionally correct today because threads are single-account; would break if threads ever span accounts. Either bring code into line with stated design intent, or amend the spec to commit to thread-level dispatch.

### Spec text drift: `add_tag`/`remove_tag` vs `add_label`/`remove_label`

`ProviderOps` (`crates/common/src/ops.rs:61-72`) uses `add_label`/`remove_label`. Spec text in `problem-statement.md` repeatedly references `add_tag()`/`remove_tag()` (e.g. lines 26, 34, 222, 282). Code is internally consistent; spec needs a sweep.

### Spec text drift: stale `apply_category`/`remove_category` references

Phase 6 removed these methods from ProviderOps, but `problem-statement.md` § Outgoing sync (line ~222) and § Phase 4 description (line ~284) still describe dispatch through them.

---

## Resolved since prior audit

- ~~**#1 Command palette rejects non-Gmail label operations.**~~ Gate removed from `command_resolver.rs`. Dead-code function is now in Tier 4 above.
- ~~**#2 Palette queries use legacy type filtering.**~~ All four palette queries now filter by `label_kind` (`crates/db/src/db/queries_extra/command_palette.rs:32, 57, 86, 116`).
- ~~**#8 `label:` search operator unverified.**~~ Verified working: `crates/smart-folder/src/parser/apply.rs:76` recognizes the operator; `crates/smart-folder/src/sql_builder.rs:443-462` builds `LOWER(l.name) = LOWER(?)` with `label_kind = 'tag'` filter, matching across accounts as the spec requires.
- ~~**#10 `generate-test-db.py` stale shape.**~~ Moot - script no longer exists in the tree.
