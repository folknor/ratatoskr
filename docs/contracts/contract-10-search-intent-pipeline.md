# Contract #10: Search Intent Pipeline — Design (v1)

## Problem

Search-like features currently share execution paths that do not preserve
intent, source identity, or scope semantics strongly enough. Ad hoc search,
Smart Folder navigation, pinned-search activation, and pinned-search refresh
all flow through overlapping logic, which makes it easy for side effects to
leak across feature boundaries. The bug where clicking a Smart Folder
reactivated a matching pinned search was not an isolated mistake; it exposed
that query text is being treated as if it were object identity.

The architectural problem is that the system knows too much about "a query
string was run" and too little about why it was run. Smart Folders are
first-class, scope-exempt sidebar objects. Pinned searches are persisted
snapshot objects with their own lifecycle. Ad hoc searches are ephemeral. When
those are allowed to collapse into one pipeline without typed distinctions, the
app has to patch behavior back in with flags and suppression logic, which is
exactly the opposite of Ratatoskr's architectural principle of making the right
thing the only thing.

## Current Failure Shape

The current app search flow is centered on `handle_search_execute()`,
`handle_search_results()`, `handle_smart_folder_selected()`,
`handle_select_pinned_search()`, and `handle_refresh_pinned_search()` in
`crates/app/src/handlers/search.rs`. The important problem is not that these
functions all exist; it is that too many of them converge on the same result
handler with different hidden expectations.

Today, successful ad hoc search results flow through `handle_search_results()`,
which both updates the thread list and persists the result set as a pinned
search snapshot. Smart Folder selection also executes through the same search
path and currently relies on `suppress_next_pinned_search_save` to avoid being
mistaken for a pinned-search save/update. That flag is a symptom: the pipeline
does not encode whether the result came from ad hoc search, Smart Folder
navigation, pinned-search activation, or pinned-search refresh. It only knows
that some query ran and some results came back.

Pinned search handling is split in a different direction. Pinned-search
activation loads stored thread IDs and does not re-run the query, while refresh
re-runs the query in stored scope and updates the snapshot. Both of those are
valid behaviors, but they are not represented as first-class search intents in
the type system. Instead they are special cases around the same shared state:
`search_query`, `thread_list.mode`, `sidebar.active_pinned_search`,
`editing_pinned_search`, and the current scope. This is why query equality can
still collapse distinct objects if the surrounding state transitions are not
handled perfectly.

The pipeline also relies on runtime state flags that patch over missing typed
state:

- `suppress_next_pinned_search_save`
- `sidebar.active_pinned_search`
- `editing_pinned_search`
- `was_in_folder_view`

These are not inherently wrong pieces of state. The problem is that they are
mutated ad hoc across multiple paths instead of being consequences of typed
search-intent completion behavior.

## Architecture: Intent → Resolve → Execute → Complete

This contract follows the same design pressure as Contract #9: preserve intent
as a first-class typed value, resolve ambiguity once, execute through a shared
engine, and make completion behavior exhaustive rather than flag-driven.

### Layer 1: SearchIntent (app crate)

The raw user or feature-level search intent. This is not "anything that touches
the search bar." It only includes flows that enter search execution or load a
search result set.

```rust
enum SearchIntent {
    /// User executed an ad hoc search from the search bar.
    AdHoc {
        query: String,
        scope: ViewScope,
    },

    /// User activated a Smart Folder from the sidebar.
    SmartFolder {
        id: String,
        query: String,
    },

    /// User opened an existing pinned search snapshot.
    PinnedActivation {
        id: i64,
    },

    /// User explicitly refreshed an existing pinned search.
    PinnedRefresh {
        id: i64,
    },
}
```

**Not included:** `Search here` / search-bar prefill. That is a UI state change
that sets search-bar text and focus but does not execute a search or load a
result set. It stays outside the search-intent pipeline.

### Layer 2: Resolution (app crate)

Resolution turns a `SearchIntent` into a fully specified `ResolvedSearch`. This
is where scope semantics and source identity are made explicit.

```rust
enum SearchScope {
    /// Derived from the current sidebar scope.
    View(ViewScope),
    /// Scope is intrinsic to the query itself; the current sidebar scope
    /// does not narrow or widen it.
    QueryIntrinsic,
}

enum SearchExecution {
    /// Execute a query through the unified search engine.
    Query {
        query: String,
        scope: SearchScope,
    },

    /// Load a stored pinned-search snapshot by thread IDs.
    Snapshot {
        pinned_search_id: i64,
    },
}

struct ResolvedSearch {
    intent: SearchIntent,
    execution: SearchExecution,
    completion: SearchCompletionBehavior,
}
```

Resolution rules:

- `SearchIntent::AdHoc`
  - remains identified by `SearchIntent::AdHoc`
  - execution uses `SearchScope::View(scope)` with the current sidebar scope
  - resolution chooses between pinned-search creation and pinned-search update
    based on captured editing context

- `SearchIntent::SmartFolder`
  - remains identified by `SearchIntent::SmartFolder { id, .. }`
  - execution uses the Smart Folder query exactly as written
  - execution scope is `SearchScope::QueryIntrinsic`
  - current sidebar scope does not narrow or widen it
  - completion behavior does not persist a pinned search and keeps the Smart
    Folder as the active sidebar object

- `SearchIntent::PinnedActivation`
  - remains identified by `SearchIntent::PinnedActivation { id }`
  - execution loads stored thread IDs and then threads
  - completion behavior is read-only with respect to pinned-search persistence

- `SearchIntent::PinnedRefresh`
  - remains identified by `SearchIntent::PinnedRefresh { id }`
  - execution re-runs the stored query in the stored pinned-search scope
  - completion behavior explicitly updates the pinned-search snapshot and reloads
    pinned-search metadata

**Scope conversion rule:** the execution layer eventually needs either account
IDs or "all accounts" semantics, not a raw `ViewScope`. Resolution owns this
conversion. `SearchScope::View(ViewScope)` is converted once to the execution
engine's account filter form. `SearchScope::QueryIntrinsic` means "do not apply
sidebar scope narrowing; any scoping comes from the query itself."

### Layer 3: Execution (shared search engine)

The existing search executor can remain mostly unchanged. The important
architectural change is that execution no longer decides completion behavior.

```rust
enum SearchFreshness {
    Query(GenerationToken<Search>),
    Snapshot(GenerationToken<Nav>),
}

struct SearchExecutionResult {
    resolved: ResolvedSearch,
    freshness: SearchFreshness,
    results: Result<Vec<Thread>, String>,
}
```

Execution shape:

- `SearchExecution::Query` uses the current unified search engine
  (`execute_search`)
- `SearchExecution::Snapshot` loads the stored pinned-search thread IDs and then
  the corresponding threads

The executor should return `SearchExecutionResult` directly so completion never
has to infer which path produced the results. Stale-result checks should happen
against the typed `freshness` token before completion logic runs.

**Target message shape:** collapse the current result-carrying message variants
into a single app-level completion message:

```rust
Message::SearchCompleted(SearchExecutionResult)
```

Pinned-search activation may still need an internal two-step load
(`thread_ids -> threads`), but that should be hidden behind the execution
helper. The completion layer should only see the final `SearchExecutionResult`.

### Layer 4: SearchCompletionBehavior (app crate)

Each resolved search source has a typed completion behavior. This replaces the
current overloaded `handle_search_results()` logic and eliminates the need for
runtime suppression flags.

```rust
struct SearchCompletionBehavior {
    persistence: SearchPersistenceBehavior,
    pinned_state: SearchPinnedStateBehavior,
    post_success: SearchPostSuccessEffect,
    folder_restore: FolderRestoreBehavior,
}

enum SearchPersistenceBehavior {
    None,
    CreatePinnedSnapshot,
    UpdatePinnedSnapshot { id: i64 },
    RefreshPinnedSnapshot { id: i64 },
}

enum PinnedSearchRef {
    Existing(i64),
    FromPersistence,
}

enum SearchPinnedStateBehavior {
    /// Clear both `active_pinned_search` and `editing_pinned_search`.
    Clear,
    /// Clear pinned-search state for a Smart Folder result. Sidebar activation
    /// remains owned by the sidebar's own selection flow.
    SmartFolder { id: String },
    /// Activate a pinned search and set editing context.
    PinnedSearch {
        active: PinnedSearchRef,
        editing: PinnedSearchRef,
    },
}

enum FolderRestoreBehavior {
    LeaveAsIs,
    EnterSearchFromFolderView,
}

enum SearchPostSuccessEffect {
    None,
    RefreshPinnedSearchList,
}
```

Completion matrix:

| Source | Persistence | Pinned State | Post Success | Folder Restore |
|---|---|---|---|---|
| AdHoc (new pinned search) | `CreatePinnedSnapshot` | `PinnedSearch { active: FromPersistence, editing: FromPersistence }` | `RefreshPinnedSearchList` | `EnterSearchFromFolderView` if entered from folder mode |
| AdHoc (editing active pinned search) | `UpdatePinnedSnapshot { id }` | `PinnedSearch { active: Existing(id), editing: Existing(id) }` | `RefreshPinnedSearchList` | `EnterSearchFromFolderView` if entered from folder mode |
| SmartFolder | `None` | `SmartFolder { id }` | `None` | `EnterSearchFromFolderView` if entered from folder mode |
| PinnedActivation | `None` | `PinnedSearch { active: Existing(id), editing: Existing(id) }` | `None` | `EnterSearchFromFolderView` if entered from folder mode |
| PinnedRefresh | `RefreshPinnedSnapshot { id }` | `PinnedSearch { active: Existing(id), editing: Existing(id) }` | `RefreshPinnedSearchList` | `LeaveAsIs` |

**Design note:** `was_in_folder_view` may remain as storage for "search session
origin" initially, but it should only be mutated through `FolderRestoreBehavior`
rather than directly by each search path.

**Completion ordering contract:** completion executes in this order:

1. persistence
2. pinned state
3. post-success effects
4. folder-restore state updates

Any `PinnedSearchRef::FromPersistence` value is resolved using the pinned-search
ID returned by the persistence step. If persistence is `None`, then
`FromPersistence` is a contract violation and should be unreachable by
construction from the resolver.

For ad hoc execution, create-vs-update must also be unreachable by construction
from completion. The resolver captures pinned-search editing context once and
chooses either `CreatePinnedSnapshot` or `UpdatePinnedSnapshot { id }`. The
completion layer must not re-read `editing_pinned_search` to decide which one
to do.

**Error handling contract:** on execution error, completion behavior does not
run. No persistence, pinned-state mutation, or post-success effect is applied.
The app records error status and leaves the current search-session origin state
unchanged; it does not attempt to restore folder view automatically.

### Layer 5: Search Lifecycle Events Outside Execution

Some events are related to search but are not search execution intents:

- search-bar prefill / `Search here`
- clearing search
- dismissing a pinned search
- saving a pinned search as a Smart Folder

These should not be forced into `SearchIntent`, but they still must respect the
same identity and lifecycle rules. In particular, pinned-search graduation to a
Smart Folder should dismiss the pinned search as part of the same lifecycle if
the product spec says graduation consumes it (see discrepancy #17).

## What This Eliminates

| Current | After |
|---------|-------|
| `handle_search_results()` infers meaning from runtime flags | Completion reads typed `SearchCompletionBehavior` |
| `suppress_next_pinned_search_save` | Gone — Smart Folder completion uses `SearchPersistenceBehavior::None` |
| Ad hoc search decides create-vs-update by reading `editing_pinned_search` at completion time | Resolver emits `CreatePinnedSnapshot` or `UpdatePinnedSnapshot { id }` up front |
| Ad hoc `clear_pinned_search_context()` calls | Replaced by typed `SearchPinnedStateBehavior` |
| Query text used as de facto identity | Source identity carried in `SearchIntent` |
| Scope semantics split across call sites | Scope resolved once in `ResolvedSearch` |
| Pinned activation vs refresh as special cases around shared state | Distinct `SearchIntent` + `SearchExecution` + completion behavior |
| Result routing split across `SearchResultsLoaded`, `PinnedSearchThreadsLoaded`, `PinnedSearchRefreshed`, etc. | Single `Message::SearchCompleted(SearchExecutionResult)` |

## Implementation Phases

### Phase A: Introduce SearchIntent + resolution

- Define `SearchIntent`, `SearchScope`, `SearchExecution`, `ResolvedSearch`
- Add a single pure resolution function:

  ```rust
  fn resolve_search_intent(intent: SearchIntent, ctx: &UiSearchContext) -> ResolvedSearch
  ```

- Call that resolver exactly once per search intent
- Keep the existing execution helpers, but route them through the resolver first
- **Pitfalls:**
  - do not let `SearchIntent::SmartFolder` inherit current sidebar scope
    implicitly
  - ad hoc search must resolve pinned-search create-vs-update at this stage,
    not later in completion
  - do not spread resolution across multiple handler functions; if resolution is
    not single-entry and pure, the contract layering is cosmetic

### Phase B: Introduce typed completion behavior

- Define `SearchCompletionBehavior` and its sub-enums
- Introduce `SearchExecutionResult { resolved, freshness, results }`
- Replace the current result-specific handlers with a single completion path
  that receives `SearchExecutionResult`
- Remove `suppress_next_pinned_search_save`
- Route Smart Folder, ad hoc, pinned activation, and pinned refresh through the
  same typed completion entry point
- **Pitfall:** pinned activation may still require an internal two-step load
  (`thread_ids -> threads`), but that chaining must be encapsulated inside the
  execution helper's async block. `PinnedSearchThreadIdsLoaded` must not remain
  a user-visible completion-stage message in the app-level message flow.

### Phase C: Clean up surrounding lifecycle state

- Move `active_pinned_search` / `editing_pinned_search` mutation behind typed
  completion behavior
- Restrict `was_in_folder_view` writes to explicit folder-restore behavior
- Reconcile pinned-search graduation / dismissal lifecycle with the new source
  identity model
- Reload pinned-search sidebar metadata only through typed `SearchPostSuccessEffect`
- Delete leftover compatibility helpers that only existed for the pre-contract
  flow

## Design Review Notes

- `Search here` is intentionally excluded from `SearchIntent` because it is a
  UI prefill/focus action, not search execution.
- Smart Folders are scope-exempt first-class sidebar objects. Their search
  semantics come from their stored query, not from the current sidebar scope.
- `SearchPinnedStateBehavior::SmartFolder { id }` is intentionally not a
  sidebar-selection command. It means "clear pinned-search state for this smart
  folder result"; the sidebar owns its own active Smart Folder selection state.
- Pinned search activation and pinned search refresh are intentionally separate
  intents because they have different execution and persistence semantics.
- Editing an active pinned search and re-executing it is still `SearchIntent::AdHoc`.
  The distinction is not intent-level; it is persistence-level, resolved once
  into `CreatePinnedSnapshot` vs `UpdatePinnedSnapshot { id }`.
