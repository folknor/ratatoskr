# Pinned Searches and Smart Folders: Discrepancies

Pinned searches and smart folders are two representations of "a saved search." They share a query string, both live in the sidebar, both produce thread lists, and one can graduate to the other. They also disagree in ways that range from accidental asymmetry to active bugs.

This document catalogues those disagreements so future changes can converge them, accept them on purpose, or - at minimum - stop discovering them one at a time. Background reading: `reference/glossary/pinned-search.md` and `reference/glossary/smart-folders.md`.

## A. Account scope is modelled differently

This is the load-bearing discrepancy; most of the user-visible bugs flow from it.

**Pinned searches use two channels for scope.** The query string in `pinned_searches.query` is whatever the user typed - never rewritten. The sidebar's account scope at execution time is captured into a separate `pinned_searches.scope_account_id` column. Refresh reads the column to build an `AccountScope` and passes that alongside the query to the search pipeline. If the query string itself contains an `account:` operator, the parser-side `account:` filter overrides the column (per `crates/smart-folder/src/sql_builder.rs:13`). So a single pinned-search row can carry scope in either or both channels.

**Smart folders use one channel.** The `smart_folders.account_id` column exists, but neither listing (`query_all_smart_folders_sync`) nor unread counts (`count_smart_folder_unread(_, _, &AccountScope::All)`) reads it. Every click resolves through `SearchIntent::SmartFolder`, which sets `SearchScope::QueryIntrinsic`; that maps to `AccountScope::All` at execution. The only thing that can scope a smart folder query is an `account:` operator inside the saved query text.

This produces three concrete failures:

1. **Graduation drops the sidebar-captured scope.** `handle_save_as_smart_folder` forwards only `self.search_query.text()` to `smart_folder.create`. The Service handler hard-codes `account_id = NULL`. So a pinned search whose scope came from the sidebar dropdown (without a typed `account:`) graduates to a smart folder that runs cross-account.

2. **The pinned-search card label lies whenever scope is in the query text.** `pinned_search_scope_label(sidebar, ps)` reads only `ps.scope_account_id`. A row with `scope_account_id = NULL` and `query = "account:acme from:alice"` renders as "All Accounts" even though the search filters to one account. The display is structurally incapable of representing scope-in-text.

3. **Smart folders have no scope tag at all.** Smart folder cards render name + (default) icon. Because the only legitimate source of scope on a smart folder is the query text, a card created from `account:acme has:attachment` looks visually identical to one created from `has:attachment`. Users have to mouse over or guess.

The convergent fix is to give both features one source of truth. The cleanest direction is to put scope in the query text (synthesize `account:<id>` when the user has the sidebar scoped to a single account at capture time) and retire `pinned_searches.scope_account_id` and `smart_folders.account_id` as live fields. Display labels then derive from the parsed query, which works identically for both features. Refresh on pinned searches becomes "re-run the saved query string" with no separate scope plumbing. Graduation becomes a verbatim text copy.

## B. The display surfaces diverge

Beyond the scope-label issue in §A:

- **Smart folder icons are stored but not rendered.** The schema defaults `icon` to `'Search'`; the three default rows override with `MailOpen`, `Paperclip`, `Star`. The sidebar's `smart.rs` passes `None` for the icon override, so none of these ever surface. Pinned searches don't have an icon column at all, so there's no analog - but the smart-folder side is itself half-built.

- **Smart folder colors are stored but not rendered.** Same pattern as icons. `smart_folders.color` is a column with no read site.

- **Pinned cards show a relative-time line ("12 min ago"); smart folder cards don't.** Pinned cards need this because they're snapshots that may be stale; smart folder cards don't because they're live. This is correct by design, but worth flagging as an intentional asymmetry.

- **Smart folder cards show an unread badge; pinned cards don't.** Smart folders have a meaningful "unread within these results" count (re-evaluated live). Pinned searches have a frozen thread-ID list; "unread" of a snapshot is ambiguous. By design.

- **Pinned card query strings have no tooltip.** `pinned_search_card` truncates the query to `PINNED_SEARCH_QUERY_MAX_CHARS` with `text(...).wrapping(Wrapping::None)`. There is almost never room for the full query string on a sidebar-width card, and once truncated the user has no way to read the rest short of clicking the card and reading the search bar. Add an iced `tooltip` wrap around the truncated text, anchored to a hover surface, showing the full `ps.query` verbatim. Smart folders don't need this because their `name` is user-supplied and bounded; their `query` is only metadata.

- **If smart folders gain a real account scope (§A), they will need a visual indicator.** The pinned-search card carries scope as a "• Acme Mail" text tag today. For smart folders the cleaner pattern is a colored dot using the account's own color, after the name rather than before, so the dominant scan target stays the folder name. The account color is already on `Account.account_color` and is rendered as a hex string via `theme::hex_to_color` in `sidebar/scope.rs:24` and `:70`. Pinned cards could adopt the same dot in place of (or in addition to) the text tag, which would also resolve the §A.2 "scope label lies when scope is in query text" issue, since a dot is naturally an *additional* signal rather than a load-bearing label.

## C. Lifecycle paths are nearly opposite

| Aspect | Pinned search | Smart folder |
|---|---|---|
| Creation | Automatic, on every executed search | Explicit, only via graduation from a pinned search |
| Cap on count | None | None |
| Auto-expiry | 14 days untouched (`pinned_search.kick`) | Never expires |
| Default rows shipped with DB | None | Three (`sf-unread`, `sf-attachments`, `sf-starred-recent`) |
| Rename | Implicit - edit-in-place rewrites the query and `updated_at` | Not possible (no IPC) |
| Delete | Dismiss button on the card; "Clear All" command in palette | Not possible (no IPC) |
| Re-order | Always ordered by `updated_at DESC` - no manual reorder | `sort_order` column + `db_update_smart_folder_sort_order` helper, but no UI |

The asymmetry that bites users today is the right column: once a smart folder exists, including the three defaults, the user has no in-app way to rename or delete it. The DB helpers (`db_update_smart_folder`, `db_delete_smart_folder`, `db_update_smart_folder_sort_order`) all exist; they just aren't wired through the Service IPC layer yet.

## D. IPC and code-surface coverage

| Operation | Pinned search | Smart folder |
|---|---|---|
| Create | `pinned_search.create_or_update` | `smart_folder.create` |
| Update query / metadata | `pinned_search.update` | (none) |
| Delete one | `pinned_search.delete` | (none) |
| Delete all | `pinned_search.delete_all` | (none) |
| Expire stale rows | `pinned_search.kick` (Service-side cadence) | (none - smart folders don't expire) |

Pinned searches have a full CRUD surface plus a Service-triggered self-heal. Smart folders have just create. This mirrors the lifecycle asymmetry in §C: smart folders weren't given write paths because the original spec assumed they'd be edited via a future settings UI that never landed.

## E. State and selection model in the sidebar

Pinned searches and smart folders are tracked in different state slots:

- **Pinned search "active" state** lives in `Sidebar.active_pinned_search: Option<i64>`. It is *not* a `SidebarSelection` variant.
- **Smart folder selection** lives in `Sidebar.selection: SidebarSelection::SmartFolder { id }`.

This means the smart-folder card's `is_active` check has to combine both signals: `sidebar.active_pinned_search.is_none() && matches!(&sidebar.selection, SidebarSelection::SmartFolder { id } if id == &f.id)`. Activating a pinned search does not touch `sidebar.selection`, so a smart folder can remain in `sidebar.selection` while the user is actually viewing a pinned search - the card's `is_active` predicate is the only thing preventing both from rendering highlighted at once.

The two slots are not strictly redundant - pinned-search activation needs to *not* be a sidebar selection (because it doesn't change navigation in the folder sense) - but the asymmetry means every "what is the user looking at right now?" predicate has to consult both fields.

## F. Execution paths

A pinned search and a smart folder representing the same query do not reach the search pipeline the same way:

- Pinned activation: `SearchIntent::PinnedActivation` -> `SearchExecution::Snapshot { pinned_search_id }`. The handler reads `pinned_search_threads` and shows the stored thread IDs verbatim, joined against live `threads` for metadata.
- Smart folder click: `SearchIntent::SmartFolder` -> `SearchExecution::Query { query, scope: QueryIntrinsic }`. The handler re-parses the query and runs it through the full search pipeline.

This is the snapshot-vs-live distinction at the code level, and it is the principled core difference between the two features. Everything else should converge; this should not.

## G. Naming and identity

- Smart folders have a user-supplied `name` separate from the `query`. Two smart folders can share a query but have different names (and vice versa).
- Pinned searches have only `query`. The card labels itself by the query string; "rename" means "edit the query". The `idx_pinned_searches_query` unique index enforces that the query *is* the identity - a second pinned search with the same text updates the existing row.

This is intentional - the snapshot model treats the query as a key, the persistent model treats it as a field. But it does mean graduation requires a name input (the new identity), and that name is the first time the search acquires a label distinct from its text.

## H. Visibility under the current sidebar scope

Both features list every row regardless of the current sidebar account scope. Consistent. A pinned search card from "Acme Mail" appears even when the user is scoped to "Personal"; a smart folder appears in every scope. The pinned card uses its scope tag to disambiguate (modulo §A.2); the smart folder card has no such tag (modulo §A.3).

## I. Edit-in-place semantics are silently different and probably wrong on both sides

When a saved search is "active" (sidebar-selected) and the user edits the search bar, the two features disagree on what that means - and neither answer is obviously right.

**Pinned search active, user edits and submits.** The `editing_pinned_search` flag is `Some(id)`, so the resolver routes through `UpdatePinnedSnapshot` and rewrites the row in place: new query, refreshed `updated_at`, new `pinned_search_threads` snapshot. The card in the sidebar updates. No prompt, no undo, no warning.

**Smart folder active, user edits and submits.** `handle_smart_folder_selected` cleared `editing_pinned_search` on the way in, so the resolver routes through `CreatePinnedSnapshot`. A new pinned search appears at the top of the sidebar; the smart folder is untouched and remains the selected sidebar item.

Both behaviors are plausible. Both are arguably wrong:

- The pinned-search case is fast and friction-free, but it means there is no way to "save current pinned search, branch off into a new one" without first explicitly clicking somewhere else. The current shape is a cliff: type a character and the previous version is gone.

- The smart-folder case never lets the user actually edit the smart folder. The DB has no IPC for it today (§D), but even if it did, there is no UI pathway: editing the search bar silently spawns a pinned search and abandons the smart folder. The only way to "update a smart folder" would be: delete it (also no IPC), re-graduate a pinned search with the same name.

The unanswered question is **when** to disambiguate. Options:

1. **Immediately on first keystroke.** Show an inline affordance next to the search bar - two pills, "Update [foo]" and "New search" - as soon as the text diverges from the saved query. Costs a UI element on every edit; benefits from being present before the user commits anything irreversible.

2. **On submit.** Hold a modal-ish confirmation when the user hits Enter. Costs friction on every search; benefits from being explicit.

3. **Asymmetric defaults.** Keep pinned at silent-rewrite (matches its ephemeral model) and make smart folder ask. This codifies "pinned = scratch, smart folder = committed."

4. **Status quo + escape hatch.** Keep the current silent behaviors but add a command-palette "Save changes to active smart folder" / "Save changes to active pinned search" that the user invokes when they want the commit semantics. Cheapest to build; lowest discoverability.

5. **A "release" affordance.** Treat the active saved search as a held context. Editing the search bar requires the user to explicitly release the context first (click an X, press Escape, something). The release converts the current saved search back to "view-only" and any new typing starts a fresh search. This is the most theoretically pure but adds a mode the user has to learn.

None of these is obviously right. The doc-level resolution is to pick one model and apply it to *both* features consistently, since today they each pick a different unprincipled answer. The underlying choice is whether saved searches behave like documents (option 1, 2, or 4) or like queries (option 3, 5).

## J. Typeahead suggestions render inline instead of as an overlay

When the user types an operator with completions (`from:`, `in:`, `label:`, `has:`, `account:`, etc.), the search bar's typeahead state populates and renders a list of suggestions. The list works - arrow keys navigate it, Enter selects - but it renders as an *inline* element in the thread-list panel's header column, not as an overlay anchored to the search input.

Code location: `crates/app/src/ui/thread_list.rs:636-672`. The typeahead `column!` is `.push`ed onto `header_col`, which is then the contents of the thread-list panel header. Adding items to that column visibly pushes the thread list downward by the height of the suggestion strip.

The right pattern per `reference/glossary/overlay-surfaces.md` is `Dropdown` (an anchored selection surface using `AnchoredOverlay`): the suggestion list should float on top of the thread list, anchored to the search input's bottom edge, dismissable on outside click and Escape, and have no effect on the thread-list panel's layout. Adopting that pattern also makes the dropdown work correctly when the thread list is short - currently a near-empty thread list shifts under the suggestions, exaggerating the displacement.

This is a search-UX issue rather than a pinned-vs-smart discrepancy, but it affects every query that uses an operator, so it sits next to the other items here.

## K. `after:-N` is regressed

The smart folder `sf-starred-recent` ships with `query = "is:starred after:-7"`, advertised as "Starred This Week." This used to work via a magic token in the query string (something like `__LAST_7_DAYS__`) that was expanded at execution time. The current architecture parses `after:-N` directly: `crates/smart-folder/src/parser/tests.rs:213` asserts that `parse_query("after:-7")` yields `Some(DateBound)` pointing at today minus seven days.

The parser test still passes, but the smart folder no longer filters correctly in the app - "Starred This Week" returns results that should not be in the window, or fails to return results that should be. The regression is somewhere downstream of the parser (likely the SQL builder's clause emission, the date-bound timestamp comparison, or the post-filter applied after Tantivy results land). The parser-level unit test isn't enough to catch the failure shape.

To investigate: thread a `parse_query("is:starred after:-7")` through `count_smart_folder_unread` and `query_threads_read` with a fixture database that has known starred-and-recent threads, and assert on the row IDs returned. Today's coverage stops at the parser boundary, which is why the regression slipped in. Once the failing layer is identified, fix it and add a runtime-shaped test there.

This is unambiguously a bug, not a discrepancy - but it sits next to the discrepancies because it affects one of the three default smart folders out of the box and any user who graduates a "last 7 days" pinned search.

## What to fix

In rough priority order, and grouped:

1. **`after:-N` regression (§K).** Functional bug that breaks a default smart folder. Diagnose the layer (parser vs SQL builder vs post-filter) and patch + add a runtime-shaped test that the unit test couldn't catch.

2. **Scope unification (§A).** Pick one channel - put scope into the query text on capture, retire the column-based path. Then graduation is a verbatim copy, card labels can come from the parsed query, and both features render scope information from the same source.

3. **Edit-in-place semantics (§I).** Pick one model (documents-with-commit-step vs queries-as-scratch) and apply it to both pinned searches and smart folders consistently. Today's split-by-accident is the worst outcome.

4. **Smart folder write IPC (§C, §D).** Add `smart_folder.update`, `smart_folder.delete`, and `smart_folder.sort_order` IPC methods on top of the existing DB helpers, then expose them through the sidebar (rename / delete / drag-reorder). Today's "graduation is one-way" is a real product hole. Pairs with §I option 1/2/4 (any "save changes to active smart folder" command needs the update IPC behind it).

5. **Typeahead overlay (§J).** Move the typeahead suggestion list from inline-in-header to an `AnchoredOverlay` anchored to the search input. Stop pushing the thread list around as the user types operators.

6. **Pinned-card tooltip (§B).** Wrap the truncated query string in an iced `tooltip` so the user can see the full text on hover. Cheap, mechanical.

7. **Smart folder icon, color, and account-scope dot (§B).** Plumb `icon` and `color` through `smart.rs` so the defaults look like the schema says they should. If §A lands and scope ends up visible on smart folders, render a colored dot after the name using `Account.account_color` rather than a textual "• Acme Mail" tag. Pinned cards could adopt the same dot.

8. **Selection-state cleanup (§E).** Either move pinned-search active state into `SidebarSelection`, or be deliberate that it lives outside and document the rule. Today's split is workable but every new sidebar predicate has to remember it.

The asymmetries that should *not* be unified: the snapshot-vs-live execution paths in §F, and the lifecycle differences in §C that follow from them (auto-create, auto-expire, refresh button, no unread badge). Those are the principled difference between the two features.
