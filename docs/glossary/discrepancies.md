# Codebase Discrepancies

This document tracks a class of bug, not a single bug: code paths "for the same concept" diverge across the codebase and silently produce inconsistent results. The discrepancies fall into a few shapes:

1. **Thread-vs-message filter divergence in query builders.** A list and a count for the "same" saved query pick different SQL aliases (`m.is_read` vs `t.is_read`, `m.date` vs `t.last_message_at`, etc.) and disagree on which threads match.
2. **Duplicate sources of truth at render time.** A stored field exists for some concept (synced color, computed snippet, resolved name) but a downstream renderer re-derives it from a name hash or other ad hoc input and never reads the stored value.

The eventual fix is **compile-time enforced**: the type system must make it impossible to compose a list-and-count pair that disagree on what they're filtering, and impossible to render a property without consulting its canonical source of truth. This is not an "audit-and-fix" item - auditing keeps drifting back to broken six months later. The goal is that "I meant thread-aggregate" and "I meant message-level" are distinct types, and that "the color of a label" has exactly one entry point.

## The motivating example

`crates/smart-folder/src/sql_builder.rs` builds two queries from one `ParsedQuery`:

- `query_threads` (the list view) - `sql_builder.rs:14-43`
- `count_matching` (the sidebar pill) - `sql_builder.rs:46-68`

`count_smart_folder_unread` (`crates/smart-folder/src/lib.rs:41-54`) sets `parsed.is_unread = Some(true)` before calling `count_matching`. `build_read_clauses` (`sql_builder.rs:217-227`) translates that to `m.is_read = 0`, pushed onto `msg_clauses`. The shared SQL skeleton (`build_thread_select_sql` / `build_count_sql`, `sql_builder.rs:468-499`) puts `msg_clauses` *inside* the inner-join messages subquery:

```sql
SELECT ... FROM threads t
INNER JOIN (
  SELECT DISTINCT m.account_id, m.thread_id
  FROM messages m
  WHERE 1=1 {msg_where}    -- m.is_read = 0 lives here
) matched ON ...
WHERE 1=1 {thread_flag_where}
```

So the pill counts "threads where there exists a message satisfying *every* filter simultaneously, including being unread." The list does not enforce unread at all - it just shows whatever the saved query matched and lets the thread-list UI render bold/unread state from `t.is_read` (the thread-aggregate).

The dev-seed symptom: "Starred This Week" (`is:starred after:-7`) shows 24 unread threads when opened and a 0 pill, because the threads have an older unread message and a newer read message. The thread is unread at the aggregate level, satisfies the list query (a recent message exists), but doesn't satisfy the pill (no single message is both recent and unread). Per-glossary aggregate semantics are documented in `docs/glossary/folders-labels.md:118-128`.

## Axes of the discrepancy

Confirmed via grep:

| Axis | Message-level | Thread-aggregate | Status |
|---|---|---|---|
| `is_read` | `m.is_read` (`sql_builder.rs:219, 222`) | `t.is_read` (no current call site in smart-folder; thread-list UI reads it directly) | **Broken.** Pill forces `m.is_read = 0` inside matched-message subquery. |
| `is_starred` | `m.is_starred = 1` (`sql_builder.rs:225` - applies when `parsed.is_starred = Some(true)`) | `t.is_starred = 1` (`sql_builder.rs:133`, via `IN_FLAG_SHORTHANDS` shorthand `is:starred`) | **Two paths, one operator.** The parser routes `is:starred` to the shorthand (thread-level), but the same `parsed.is_starred` field also feeds `build_read_clauses` (message-level). Whichever fires depends on parsing path, not user intent. |
| `date` (`before:` / `after:`) | `m.date < ?` / `m.date > ?` (`sql_builder.rs:230-237`) | `t.last_message_at` (not used in smart-folder) | **Semantically muddy.** "Thread that had recent activity" vs "thread whose latest message is recent" are different questions, conflated. |
| `has:attachment` | `EXISTS … FROM attachments a WHERE a.message_id = m.id` (`sql_builder.rs:207-215`) | (no thread-aggregate column) | **Probably correct as message-level**, but worth confirming - the predicate "thread has any attached message" is what users mean. |

Suspected but not yet grepped for divergent call sites:

| Axis | Notes |
|---|---|
| `is_replied` / `is_forwarded` | Stored on `messages` only (per glossary `folders-labels.md:244-248`); no `threads.*` column exists. Thread-level rendering ORs across messages at read time. Any predicate that wants "thread that has been replied to" needs an explicit `EXISTS` against messages - easy to get wrong, easy to get inconsistent across builders. |
| Folder/label membership | `thread_labels` is already thread-aggregate by construction, so most call sites are consistent. The provider-side merge-vs-replace inconsistency for Graph/JMAP partial-delta sync (`folders-labels.md:119-127`) is a separate issue. |
| `is_snoozed`, `is_pinned`, `is_muted` | Stored only on `threads` (no message-level analogue). These can't suffer the discrepancy; included only to note the asymmetry - the read/starred axes have message-level columns precisely because they were ported from per-message provider primitives. |
| Free-text / `from:` / `to:` | Message-level by nature (each message has its own envelope). The question is whether the matched-message subquery's join semantics introduce surprising count behavior when combined with a thread-aggregate filter elsewhere in the same query. |

## Investigation plan

The audit needs to cover everywhere a thread list or a count is computed:

- `crates/smart-folder/src/sql_builder.rs` - done; this is the motivating example.
- `crates/db/src/db/queries_extra/scoped_queries.rs` - universal-folder unread counts (`get_unread_counts_by_folder`, `get_system_folder_unread_counts`, `get_flag_folder_unread_count`, `broad_inbox_unread_count`, draft count) and per-label unread counts (`get_label_unread_counts`, the source of the sidebar LABELS-section pills). These use `t.is_read = 0` directly against `threads`. Spot-checked consistent with the thread-aggregate side, but the same call sites would silently regress if a future change inner-joined `messages` for "in this folder" / "with this label" and applied `is_read` against `m` - the type system does not prevent this today.
- `crates/db/src/db/queries_extra/thread_detail.rs` - `recompute_thread_read_starred` and `query_thread_state_decorations` are the canonical helpers per glossary. These are the *source* of `t.is_read` and `t.is_starred`; any divergence here is a separate (worse) bug because it desyncs the aggregate from the underlying messages.
- `crates/db/src/db/queries_extra/navigation.rs` - composes the above into `NavigationState`. The current Drafts special case (also tracked separately in `TODO.md`) is part of this surface area.
- Per-provider sync (Gmail/IMAP `store_thread_groups_to_db`, Graph/JMAP `merge_thread_labels`) - these write `thread_labels` and `t.is_read`/`t.is_starred`; their consistency with the read-side query builders is what makes the aggregates trustworthy.
- App-side thread list rendering - anywhere that decides "is this row bold" needs to read the same column the pill counted from. Currently the thread list reads `t.is_read`; any future drift to per-message rendering needs to be paired with a matching pill change.

Each call site needs a written answer to: "is this predicate on a thread or on a message, and is the join structure consistent with that answer?"

## Compile-time enforcement direction

The audit-and-fix approach has failed every time it's been attempted on bug classes like this. The constraint here is:

> Any query builder that emits SQL touching `is_read`, `is_starred`, `date`/`last_message_at`, or any other axis with both message-level and thread-aggregate variants must declare, at the type level, which side it is filtering on. Any consumer that pairs a list with a count must be unable to construct a list/count pair that uses different sides for the same axis.

Sketches (none committed; this section is intentionally directional, not prescriptive):

- **Typed predicate enum.** `ParsedQuery` today is a struct of `Option<bool>` / `Option<i64>` fields that get translated to SQL by a builder that knows which side to use. Replace those fields with an enum whose variants name the side explicitly: `Predicate::ThreadIsUnread`, `Predicate::MessageMatchesText(s)`, `Predicate::ThreadLastMessageBefore(t)`, `Predicate::MessageDateBefore(t)`. The SQL builder pattern-matches and is forced to handle each variant on the correct alias.

- **Two-context builder with phantom-typed clauses.** `QueryContext` already separates `msg_clauses` from `thread_flag_clauses`. Make `push_msg_clause` / `push_thread_clause` take strongly-typed inputs (e.g. `MessageClause` and `ThreadClause` newtypes), and make the `build_*_clauses` functions return one or the other rather than mutating the context directly. Any function that wants to push to both gets two return values and the caller has to dispatch them.

- **Shared filter spec.** `count_smart_folder_unread` forces unread on a *shared* `ParsedQuery`. Replace the forced-mutation pattern with an explicit `UnreadScope::Thread` / `UnreadScope::Message` parameter that both `query_threads` and `count_matching` consume identically. The motivating bug exists precisely because the forcing happens in one place and the list never sees the same forcing.

- **Single source of truth at the type level for "aggregate-or-not."** Define `ThreadColumn` and `MessageColumn` as zero-sized newtypes around column names, and have the SQL emitter take `&dyn Column` with a method that picks the right alias. Builders then can't accidentally hand a `MessageColumn` to a thread-flag emitter.

The right design depends on how invasive the change should be (smart-folder-local vs workspace-wide) and how much existing call-site churn is tolerable. The investigation should produce that answer before any code lands.

## Duplicate sources of truth at render time

### Label color in the sidebar (fixed 2026-05-15)

`crates/label-colors/src/lib.rs::resolve_label_color` is the canonical resolver: it returns the synced `color_bg`/`color_fg` from the `labels` table if present, else a deterministic fallback from the 25-preset palette via `color_for_label` (hash of label name + namespace). The Gmail sync writes real colors into `color_bg`/`color_fg`; dev-seed does the same for user labels (`crates/dev-seed/src/accounts.rs:30-89` - `PERSONAL_LABELS` and friends each ship a distinct hex pair).

Before the fix, `crates/app/src/ui/sidebar/labels.rs` rendered the label dot via `theme::avatar_color(&f.name)` - a separate hash over just the name, using a different palette baked into `crates/app/src/ui/theme/avatar.rs:16`. Stored colors were silently ignored. Users with seeded "Personal" labels saw avatar-palette colors with no relationship to the configured `color_bg`.

Fix: `NavigationFolder` now carries `color_bg` / `color_fg`; `build_account_labels` populates them via `label_colors::resolve_label_color` (synced or preset); sidebar parses `f.color_bg` with `theme::hex_to_color`. `theme::avatar_color` remains in place as a defensive fallback if `color_bg` is unexpectedly `None`, but the resolver always returns `Some` for `AccountLabel` rows.

Compile-time goal: any iced widget that draws a label-shaped surface (sidebar dot, reading-pane chip, thread-list chip, picker swatch) should take a typed `LabelStyle { bg: Color, fg: Color }` produced exclusively by one constructor that consults the resolver. Other inputs (a raw name, a raw hex string) should not type-check as a label color. Today the relevant call sites (`reading_pane.rs:746-747`, `widgets/pickers.rs:213`, `thread_list.rs:661`, `sidebar/labels.rs`) each parse hex independently, so the discipline is by convention only.

### Mixed drafts list merged at the app layer, not the query layer

The sidebar's Drafts view must show server-synced drafts (threads with the `DRAFT` label) and local-only drafts (rows in `local_drafts` with `sync_status != 'synced'`) in one chronological list. The count is unified at the core query layer via `get_draft_count_with_local` (`crates/db/src/db/queries_extra/scoped_queries.rs:590`). The list is unified at the app layer in `crates/app/src/helpers.rs:167-175`: it calls `get_threads_scoped(label="DRAFT")` for the synced subset, `get_local_draft_summaries` for the local subset, runs each through `local_draft_to_app_thread` (`helpers.rs:361`) to coerce a `LocalDraftSummary` into the app's `Thread` shape, concatenates, and sorts by `last_message_at` desc.

This is the same shape as the thread-vs-message bugs: the canonical answer to "what does the Drafts folder contain?" has two definitions in two places, and any consumer that picks the wrong entry point silently disagrees with the rest of the system. Specifically, anyone calling `get_draft_threads` directly gets the synced subset only and will report a smaller list than `get_draft_count_with_local` counts. A doc comment on `get_draft_threads` now flags this for future callers, but the type system does not enforce it.

`local_draft_to_app_thread` itself is a quiet second source of truth for "what a `Thread` looks like" alongside the canonical `db_thread_to_app_thread`. It hardcodes `is_read: true`, `is_starred: false`, empty labels, `message_count: 1`, no decorations. If the canonical converter grows new fields or behavior (e.g. label-color resolution, decoration application), the local-draft converter has to be kept in lockstep manually.

Compile-time goal: there should be one entry point that returns "everything that belongs in the Drafts list for this scope," with the merge done where the data is - i.e. at the query layer, returning a typed `DraftItem` that the app projects into its `Thread`. Direct access to the synced-only path should require an explicit opt-in (different function name, or a marker type) so it can't be reached by autocomplete-driven mistake.

### Per-account vs cross-account label aggregation (labels-unification)

The labels-unification spec (`docs/labels-unification/problem-statement.md`) requires section 4 of the sidebar to be a cross-account view: all tag-type labels from all accounts, grouped by normalized name (`LOWER(TRIM(l.name))`), with unread counts summed across accounts. The actual builder does it per-account.

- `get_navigation_state` (`crates/core/src/db/queries_extra/navigation.rs:122-131`) calls `build_account_labels` only in `AccountScope::Single`. In `AccountScope::All`, no tag-type labels are built at all - section 4 is empty.
- `get_label_unread_counts` (`navigation.rs:345-373`) groups by `tl.label_id`, never by name; cross-account sum is not computed anywhere.
- `search_labels_for_typeahead_sync` (`navigation.rs:624-632`) does case-insensitive comparison via `COLLATE NOCASE` but not whitespace trim, so `"Work"` and `"Work "` produce two entries.

This is the same shape as the Drafts-list bug: the question "what labels exist for this scope" has one answer in the per-account branch and a different (missing) answer in the cross-account branch. The per-account builder is the only entry point, so anyone querying labels for a unified view will either get nothing or have to re-implement the cross-account aggregation locally. The compile-time goal: a single `query_visible_labels(scope) -> Vec<Label>` function that returns the same logical set regardless of scope, with cross-account grouping baked in. The current `AccountScope::All` arm not calling any label builder is a class of bug the type system could prevent (exhaustive match returning the same type, instead of returning early with an empty set when scope is All).

### Label color override schema vs resolver shape mismatch

`label_color_overrides` (`crates/db/src/db/schema/02_mail.sql:31-34`) stores `(label_name COLLATE NOCASE, color_bg)` - background only. `resolve_label_color` (`crates/label-colors/src/lib.rs:35-51`) returns `(bg, fg)` pairs. Two consequences:

1. Even if override lookups were wired in (they are not - see labels-unification discrepancies "label_color_overrides is write-never, read-never"), an override would supply `color_bg` but `color_fg` would come from the hash fallback, producing a coherence break: the user picked the bg, the fg is a stable-but-unrelated palette pick.
2. The override key is normalized by `COLLATE NOCASE` only, not `TRIM`. The labels-unification spec requires both. `"Work"` and `"Work "` would be different overrides.

Compile-time goal: the override store should return a typed `LabelStyle { bg: Color, fg: Color }` matching what the resolver and renderers consume. A `Bg`-only override should not type-check as a complete label style.

## Out of scope for this document

- Fixing the motivating bug. The "Starred This Week" pill discrepancy is real and shippable as a one-line fix (move the forced unread from `msg_clauses` to `thread_flag_clauses`), but doing so without the systemic fix just hides the next instance.
- The Drafts pill semantics question (total-vs-unread contract). Tracked separately in `TODO.md`. That is a product decision about what the pill *should* count; this document is about ensuring the count means what the matching list says it means, whatever the product answer is.
- Provider-side merge-vs-replace inconsistencies on `thread_labels`. Tracked in `TODO.md` under "cross-client folder/label moves."
