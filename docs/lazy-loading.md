# Lazy Loading: Problem Statement

## Overview

Ratatoskr targets mailboxes with hundreds of thousands of messages and 150+ GB of cached content. The current thread list, sidebar trees, and search-result list all materialise their entire backing dataset up front - the thread list renders every visible card inside a `column![]` wrapped in a `scrollable`, the sidebar's per-account label tree loads all labels eagerly, and search returns a single capped batch. None of these scale to the project's target volume.

The right behaviour is an *endless* lazy-loaded list: the user scrolls, and rows materialise as they come into view, with no end-of-list pagination control and no visible page boundaries. This document captures the surfaces that need it, the constraints, what we already know from a prior attempt, and the open design questions.

## Surfaces

These are the surfaces that need lazy-loading. Listed in rough order of pain.

- **Thread list / inbox.** The dominant case. A 150 GB cached mailbox can hold north of a million threads. The current `column![]` materialises every card in scope; opening a heavily-used `Inbox` against the upper end of the target range freezes the UI for seconds and uses an unreasonable amount of memory. This is the surface that motivates the whole document.
- **Search results.** The unified search pipeline today caps results at engine-specific constants (SQL `DEFAULT_QUERY_LIMIT=500`, Tantivy 200, SQL fallback another value). The "Result limits are fixed and engine-specific" item in `docs/search/implementation-spec.md` is genuinely a lazy-loading problem: once we have an endless list backed by an offset/cursor cursor, the limits collapse into "fetch the next page when the user scrolls within K rows of the end."
- **Sidebar label / folder trees.** Less acute - most accounts have tens to low hundreds of labels - but provider folder hierarchies on Exchange / Graph can be deep and wide. The `browse_public_folders()` flow is already tree-shaped and would benefit from lazy expansion (currently a TODO).
- **Contact lists.** Picker popups and the address book view both render all matching contacts; for very large global address lists this is expensive.

The thread list and search results share infrastructure - both render `Thread` cards via the same component - so a single lazy-list widget addresses both. Sidebar trees and contact lists may share with or specialise off that widget.

## Constraints

- **One window**, no infinite-scroll page-loading indicators. The interaction model is "scroll a long list," not "click load more." Endless, not paginated.
- **Backend already paginates**. `query_threads_read` takes `limit` and `offset`; Tantivy supports `TopDocs::with_limit(N).and_offset(M)`. The bottleneck is UI-side materialisation, not the SQL or index layer.
- **Generational load tracking** has to flow through the page fetcher. Scrolling rapidly through 10k positions must not produce out-of-order page deliveries that flicker the list.
- **Selection and focus** survive page boundaries. Selecting a thread, scrolling away, scrolling back must reproduce the selected state without re-fetching everything in between.
- **Memory bound**. We can't keep every row materialised forever; a sensible windowing strategy needs to evict rows that haven't been visible for a while.

## Prior attempt: the 10k-baseline observation

We previously tried introducing lazy rendering and saw a *regression* against the "just materialise the first 10k threads at startup" baseline. Both the eager 10k and the lazy approach were measured in interactive responsiveness; the lazy approach was worse.

The reason for this is not understood. Working hypothesis (to be verified, not committed to):

- iced's built-in `scrollable` widget allocates layout for every child element regardless of whether it's visible, so wrapping a virtualised list inside it forces layout work on the full backing dataset anyway.
- Building and tearing down child widgets on scroll is more expensive than keeping a fixed pool of 10k pre-built cards and letting iced's renderer cull off-screen ones.
- iced messages produced by scroll events fan out across every widget in the tree; a partial-materialisation approach that swaps child widgets in and out re-triggers diffing far more often than steady-state 10k-row rendering does.

The strong implication is that this work is **not** a pure "swap a `column!` for a virtualised container" exercise - the iced fork as it stands doesn't make virtualisation cheap. The path forward is probably a custom widget that owns its own layout, draw, and event-routing path, exposes only the visible range to iced's diff cycle, and treats the backing dataset as an external store rather than a `Vec` of children.

## Design direction

- **Custom widget.** Not a wrapper around `scrollable + column!`. The widget owns its scroll offset, its visible-range layout, and the diff against a backing `LazyList` model.
- **Endless, not paginated.** No "Load more" affordance, no page count, no fixed limit. The widget asks the model for rows by index; the model fetches pages on demand and caches them.
- **Window the materialised rows.** Keep N rows materialised around the current viewport; evict rows outside the window so the widget tree stays small.
- **Page fetcher is generational.** Each fetch carries a generation token; results land if and only if their token is still current. Scroll-spam doesn't accumulate.
- **Shared infrastructure between thread list and search results.** Both render the same kind of card and consume the same kind of paged backing store.
- **Backend-side cursor or offset.** Initial implementation can be offset-based (every backend query takes `(limit, offset)` already); long-term it may make sense to move to a keyset cursor so paging cost doesn't grow with offset depth.

## Open questions

- Is the iced fork itself the right venue for a virtualised container? If so, this work involves an upstream change to `crates/sluggrs/...`, not just an app-level widget. The build infrastructure has the iced source in-tree so the change is mechanically possible, but it's a bigger commitment than an app-only widget.
- How does the widget interact with iced's `responsive` layout, scrollbar styling, and overlay surfaces (context menus on cards, drag-and-drop)?
- Do we keep a fixed-height card assumption, or support variable-height cards? Fixed-height makes the math trivial; thread cards in Ratatoskr are very nearly fixed-height already but not exactly.
- How do we handle selection-rooted navigation (j/k, page up/down, home/end) when the underlying dataset is paged? `end` in particular has to be a backend-side query, not a "jump to row N-1."
- What's the eviction policy? Time-based, distance-based, fixed pool? The "iced re-diffs everything anyway" hypothesis would steer us toward a fixed pool that never deallocates.

## Related work

- `crates/search/`: search results pipeline. The Tantivy limit and the in-app intersection step both fall away once an endless list is in place.
- `crates/db-read/src/db/queries_extra/`: `query_threads_read`, `query_thread_keys_read`, and friends already take `limit` / `offset`. The plumbing on the read side is in place.
- `crates/types/src/`: any new `LazyList<T>` / page-cursor type lives here so the app, providers, and core all share it.

## Status

Not started. This document supersedes the "Scroll virtualization" line item that previously lived in `TODO.md`. The previous attempt is captured under § Prior attempt; a future round needs to either disprove the 10k-baseline-is-faster observation with measurement, or commit to the custom-widget path described above.
