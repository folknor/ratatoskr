# Codebase Discrepancies

This document is about a class of bug: code paths "for the same concept" diverge across the codebase and silently produce inconsistent results. A list and a count pick different SQL aliases and disagree on what they're filtering. A stored value exists for some property but a downstream renderer re-derives it from a name hash and ignores the stored copy. A composite operation works in the happy path but its per-member fan-out bypasses the preflight on retry. Each one a quiet wrong answer.

The eventual fix is **compile-time enforced**: the type system must make the wrong call impossible to write, not just discouraged by convention. This is not an "audit-and-fix" item - auditing keeps drifting back to broken six months later. The point of this document is to name the contracts being violated precisely enough that the type-level enforcement that fixes one class of bug is the same enforcement that prevents the next one.

The inventory at the bottom is short by design. Most of the original audit has resolved through the contracts work in `docs/contracts-roadmap.md`; the entries that remain are the ones that still warrant active design attention. Resolved findings live in commit history and the roadmap's Status sections, not here.

## The Promise Rule

A discrepancy exists only when two paths claim to answer the same domain question or uphold the same invariant. "Two functions are similar" is not a discrepancy; "two functions promise the same answer and give different ones" is.

The promise rule is operational, not philosophical. Before adding an entry to the inventory, name the shared promise. If no promise can be named - if the two APIs are merely *adjacent* rather than *redundant* - the entry belongs in the parking lot, not the inventory. "Three content stores expose similar `put` / `get` / `delete` methods" is parking-lot material unless the architecture promises substitutability. "List query and count query agree on view membership" is inventory material because the UI breaks when they don't.

## Contract Failures

Five contract failures account for every inventory entry, past or present. The taxonomy is normative: it names what is missing from the type system that allows the discrepancy to be written. New entries should be tagged with the contract failure(s) they exemplify.

### 1. Semantic Grain Untyped

The same primitive shape - `i64`, `String`, `bool`, a struct like `Thread` or `UnifiedSearchResult` - can mean **message**, **thread**, **bundle**, **search-hit**, or any of several aggregate views. The type system does not distinguish them, so a function that operates on one grain can be composed with a function that returns another, and the result silently mixes layers.

The cure splits into two sub-axes, and inventory entries are tagged separately:

- **`grain.vertical`** (message → thread → bundle): grain-branded predicates and result types. A `MessageDate` is a distinct type from a `ThreadLastActivity`; a `ThreadAggregate` is the only output of the canonical recompute helper. A function that filters by `MessageDate` cannot be silently swapped for one that filters by `ThreadLastActivity` even if both wrap an `i64`.
- **`grain.scope`** (personal account vs shared mailbox vs public folder vs all-accounts): exhaustive enum dispatch with no `Option` fallback as the *public* API. The `ViewScope` enum is right; the `to_account_scope() -> Option<AccountScope>` accessor that allows callers to forget non-Account scopes is the failure.

### 2. Canonical Answer Optional

A caller can choose a non-canonical entry point and get a normal-looking result that silently disagrees with the canonical one. `get_draft_threads` returns only synced drafts; `get_draft_count_with_local` returns synced + local. A new call site to `get_draft_threads` undercounts the sidebar silently. The original search example had `search()` returning Tantivy-ranked results while `search_sql_fallback()` returned SQL-only results; that sub-case is now resolved by making fallback private and returning `SearchResults::{FullIndex, Degraded}` from the public entry point.

The cure is one public entry point per question. Narrower paths exist only behind explicit capability or marker types ("I know I want synced drafts only and I accept the undercount"). Doc-comment-enforced contracts are not enforcement.

### 3. Completion State Untyped

A value can exist in a partially-completed state where some fields are real and some are placeholders, and the partial state is observationally equal to the complete state. A `UnifiedSearchResult` built from a Tantivy doc has `is_read: false` and `MatchKind::Body` as placeholders; the *enrichment* pass overwrites them with real values from SQL. Tantivy-only and SQL-only paths skip the enrichment pass entirely, so a partial-state value reaches the renderer indistinguishable from a fully-enriched one.

This is the deferred-enrichment shape. Distinct from #2 because the canonical entry point here is the *completion step*, not the *construction step*: the partial value is legitimate as an intermediate, but it must not type-check where a complete value is required.

The cure is two types - `PartialSearchHit` and `EnrichedSearchHit`, `UnvalidatedColorPair` and `LabelStyle`, etc. - with the only transition between them being the enrichment function. Renderers and downstream code accept only the complete type.

### 4. Mutation Capability Untyped

Whether a write operation has full coverage of the entity (replace) or partial coverage (merge) is a *capability* of the provider path, not a helper choice. Gmail full-thread sync has full coverage and calls a provider-local replace wrapper; Graph/JMAP partial delta has partial coverage and calls provider-local merge wrappers. The remaining risk is not helper selection but how optimistic local intent is represented while provider echo is pending, and how partial-delta sync reconstructs full membership when the delta cannot.

Composite operations and per-member operations are similarly capability-distinguished. A composite must not enqueue per-member retries (the composite's own preflight covers the retry); a non-composite member call must enqueue. The current shape is structural: composites call `_no_enqueue` entry points; the public enqueueing entry point is unreachable from inside a composite.

The cure is capability-encoded entry points. Full-thread replace wrappers live inside the Gmail/IMAP provider paths that have complete coverage; partial-delta merge wrappers live inside the Graph/JMAP provider paths that do not. Per-member dispatch goes through a `_no_enqueue` entry point that composites use; the public entry point that enqueues is structurally unreachable from inside a composite.

### 5. Validated Domain Type Missing

The type allows representations that should be impossible. `kw:keyword` / `cat:category` / `importance:high` are domain values modeled as `String` with prefix conventions - a `LabelId` of `"keyword"` (missing the `kw:` prefix) or `"importance:medium"` (not a valid importance) type-checks. `decrypt_or_raw(value)` accepts both encrypted and plaintext at the same call site, so a writer that forgot to encrypt looks identical to a reader that handles legacy. A color override stored as `(Some(bg), None)` for the foreground is half a value; the resolver falls back to hash even though a partial value was supplied.

The cure is parse-at-the-boundary, total types inward. `LabelKind` is an enum whose variants take validated payload types (`Keyword(KeywordName)`, `Category(CategoryName)`, `Importance(ImportanceLevel)`, `GmailUser(GmailLabelId)`, ...) - the payload types are themselves private-fielded and can only be built by their own validating parsers, so the enum is sealed by inclusion. `StoredSecret` is a parsed type - legacy plaintext rows go through an explicit migration boundary, not a tolerant accessor. `LabelStyle { bg, fg }` is a complete pair; partial values do not exist.

## Enforcement Techniques

Three techniques implement all five contract failures.

### Sealed Constructors

A type's privacy boundary is its contract. The type exposes its fields but not its constructors; only one function in the crate can build the value, and that function enforces the derivation rule. `ThreadAggregate` has only the SQL aggregate constructor and `ThreadAggregate::compute_from_messages(&first, rest)`; there is no `ThreadAggregate { is_read, ... }` literal in scope outside the constructor's module. A second derivation rule cannot exist because a second constructor cannot exist.

Covers **#1 (grain)** and **#3 (completion state)**. The grain type is sealed; the partial-to-enriched transition is a sealed constructor on the enriched type.

### Capability Tokens

A function signature or module boundary requires a witness that the caller has the right capability. For thread membership, the public crate-wide API exposes only raw row primitives and shared filtering; replace and merge wrappers are private to the provider paths with the right coverage. Similarly, the public `drafts_list()` returns a `DraftsView` that is the unique type accepted by the renderer; the synced-only function returns a `SyncedDraftsOnly` that does not satisfy that signature.

Covers **#2 (canonical answer)** and **#4 (mutation capability)**. Phantom types, zero-size witnesses, and newtype wrappers are all valid implementations.

Cross-crate enforcement of capability-token contracts is not perfect in Rust - there is no friend-crate mechanism. The roadmap names this as the highest-uncertainty design question and proposes a standing answer (`docs/contracts-roadmap.md` § Fidelity).

### Boundary Parsing

External input - protocol payloads, on-disk values, user strings - is parsed into a total domain type at the boundary. Inward code never sees the raw form. `LabelKind::parse(raw: &str, provider: MailProviderKind) -> Result<LabelKind, ParseError>` is the only constructor from raw external values; `LabelKind` itself is an enum whose variants are sealed by their payload types (validated newtypes that have their own boundary parsers). `StoredSecret::parse(raw: &str)` handles both new and legacy formats but returns a single typed value; readers see only the parsed type.

Covers **#5 (validated domain)**.

## Multi-tag Legend

Inventory entries carry three tags. The interesting bugs sit at intersections, so multi-tagging is the default.

- **`contracts:`** comma-separated list of `grain.vertical` / `grain.scope` / `canonical-entry` / `completion-state` / `mutation-capability` / `validated-domain`. The contract failure(s) the entry violates. The grain contract is sub-tagged because the two cures (newtype branding vs exhaustive dispatch) are distinct migrations.
- **`enforcement:`** comma-separated list of `sealed-constructor` / `capability-token` / `boundary-parse`. The technique(s) that would prevent the discrepancy at compile time.
- **`promise:`** one short sentence naming the shared invariant the two paths *claim* to uphold and don't. This is the operational form of the promise rule - if the promise can't be named, the entry doesn't belong in the inventory.

## The Motivating Example

The framework was developed in response to a concrete bug. The smart-folder pill for "Starred This Week" (`is:starred after:-7`) showed 24 unread threads when opened and a 0 pill, because the threads had an older unread message and a newer read message. The thread was unread at the aggregate level, satisfied the list query (a recent message exists), but did not satisfy the pill (no single message is both recent and unread).

Root cause: `count_smart_folder_unread` set `parsed.is_unread = Some(true)` before calling `count_matching`, and the old read/starred clause builder translated that to `m.is_read = 0` on `msg_clauses`. The shared SQL skeleton put `msg_clauses` *inside* the inner-join messages subquery:

```sql
SELECT ... FROM threads t
INNER JOIN (
  SELECT DISTINCT m.account_id, m.thread_id
  FROM messages m
  WHERE 1=1 {msg_where}    -- m.is_read = 0 lives here
) matched ON ...
WHERE 1=1 {thread_flag_where}
```

So the pill counted "threads where there exists a message satisfying *every* filter simultaneously, including being unread." The list did not enforce unread at all - it just showed whatever the saved query matched and let the thread-list UI render bold/unread state from `t.is_read` (the thread aggregate). Two paths, same domain question (what does this smart folder contain), different answers.

Resolved (historical; the fix predates this document's current revision): `build_thread_state_clauses` emits read, unread, and starred predicates against `threads` through `thread_flag_clauses`, and the list and count builders consume the same thread-flag clause set. The named function (`smart_folder::count_smart_folder_unread`) still exists with the corrected behavior. Per-glossary aggregate semantics are in `docs/glossary/folders-labels.md`.

This is the worked example for **#1 (grain.vertical)**. The framework above generalizes the lesson.

## Inventory

Three issues remain in active inventory. Each names the shared promise, the contract failure(s) being violated, and the cure shape. Resolution work for each is sequenced in `docs/contracts-roadmap.md` or in a dedicated design slice.

### Optimistic Local Label Intent

Local label actions write directly to `thread_labels` (and `thread_label_groups` for group composites) before the provider echoes the change. Any provider-truth path that DELETE+INSERTs `thread_labels` for that thread before the local action has been echoed can erase the optimistic row. The erase surfaces today are:

- `crates/provider-sync/src/keyword_membership.rs::recompute_thread_keyword_labels` (IMAP and JMAP keyword paths).
- `crates/provider-sync/src/imap/thread_store.rs::replace_full_thread_labels` (IMAP `imap_initial` and `imap_delta`).
- `crates/provider-sync/src/gmail/sync/storage.rs` full-thread replace (Gmail).

The window is small but user-visible: a label flickers off and back on as the destructive replace and the action queue race.

Two paths answer the same domain question - "what labels are currently on this thread, as far as the user is concerned" - and give different answers. The local UI sees the optimistic state immediately after the action; sync sees provider truth and overwrites the optimistic row on the next persist.

The cure is a small local overlay (table or queue-derived projection) that holds pending intent until the provider echoes or the action fails permanently. User-facing label-membership reads merge provider truth with the overlay; sync recompute paths never observe the overlay. A persisted per-thread generation counter advances on every provider-truth membership write and drives clearance for async-echo provider paths.

Detailed design and three-stage implementation plan: `docs/optimistic-label-intent.md`.

Tags: contracts=mutation-capability,grain.vertical; enforcement=capability-token,sealed-constructor; promise=optimistic local label state stays visible to user-facing queries until provider echo or permanent failure, and sync recompute paths never observe optimistic intent.

### Drafts Pill Semantics

Every universal-folder pill in the sidebar shows an `is_read = 0` count via `get_unread_counts_by_folder` (`crates/db/src/db/queries_extra/scoped_queries.rs`), except Drafts. Drafts is special-cased in `build_universal_folders` (`crates/core/src/db/queries_extra/navigation.rs`) to show *total* drafts via `get_draft_count_with_local` (synced drafts plus local drafts, no `is_read` filter). With dev-seed, the Personal/Drafts pill says (6) but only 2 of those are unread.

The Promise Rule violation: pill counts across the sidebar use the same visual shape (a number with no qualifier) and silently answer different membership questions. A user reading the Inbox pill and the Drafts pill cannot tell that one is "unread" and the other is "total."

Unread is a weak concept for Drafts, and arguably for Sent, Trash, Spam, and Archive too. Two directions to choose between:

- Collapse to one rule, all pills = unread. Accept that Drafts (and similar folders) will rarely show a pill.
- Per-folder semantics (Inbox/Starred/Snoozed = unread; Drafts/Sent/Trash/Spam/Archive = total) with a visual distinction (different pill style) so users can tell which count they're looking at.

The product decision determines which membership predicate the pill represents. The contract failure is that list query and count query can carry different predicates without the type system noticing - and the canonical-entry pair is what enforces single-predicate-per-folder regardless of which predicate is chosen. `get_drafts_view` and `get_draft_count_with_local` (and any equivalent pairs for other affected folders) migrate together; that pairing is type-level. The predicate they carry is product-level.

Tags: contracts=canonical-entry; enforcement=capability-token; promise=list query and count query for a given folder answer the same membership question, with a single declared predicate per folder.

### Cross-Client Folder/Label Move Reconciliation

Graph delta-sync persistence (`crates/provider-sync/src/graph/sync/persistence.rs:210-211`) calls both `merge_partial_delta_folders` and `merge_partial_delta_labels` for `thread_folders` and `thread_labels`; JMAP delta-sync persistence (`crates/provider-sync/src/jmap/sync/storage.rs:196`) calls `merge_partial_delta_folders` for `thread_folders` only (JMAP keyword-shaped labels flow through `recompute_thread_keyword_labels` separately). Partial-delta pages no longer wipe sibling-message rows. The trade-off is asymmetric: when another client moves a thread (Inbox to Archive, say), the new `thread_folders` row gets added but the old one is never removed, because the delta only reports what the changed message *is* in, not what it's no longer in. Same-client moves are fine - the action service updates `thread_folders` locally before dispatching, so the source row is removed in the same transaction.

The Promise Rule violation: Gmail full-thread sync and Graph/JMAP partial-delta sync both claim to answer "what folders does this thread currently live in." Gmail's answer is correct (full replace); Graph and JMAP under-remove, carrying stale `thread_folders` rows after cross-client moves. The same shape applies to Graph's `thread_labels` partial-delta path, since `merge_partial_delta_labels` is the same merge semantics applied to a different table.

Not a type-system fix. The cure is data-model: a per-message folder/category membership table analogous to the existing `message_keywords` table that the IMAP/JMAP keyword-membership slice already uses. Thread folder/label membership is recomputed from the per-message union on every persist, the same way `kw:*` rows are recomputed from `message_keywords`. The keyword-membership slice (`crates/provider-sync/src/keyword_membership.rs`) is the existing partial implementation of this pattern; broadening it to folders is an architectural slice, not a type-level migration.

Until that pattern is extended to folders and labels broadly, stale rows are an accepted artifact of the cross-client move case. A periodic full-thread reconciler that prunes against the per-message union is the lighter-weight alternative if the per-message table is deferred.

Severity note: post-labels-unification, a stale `thread_labels` row on a member-bearing label now renders the whole group pill via the `thread_labels` JOIN `label_group_members` rendering path (`docs/labels-unification/redesign.md` § "Message pill rendering"). Before that work, a stale row only surfaced as a per-account label the message UI did not foreground. After the unification, the same stale row shapes like a deliberate user "apply group" action and users who never used a group can see it attached to threads they did not touch. This raises the priority of either the per-message membership store or the periodic reconciler.

Tags: contracts=mutation-capability,completion-state; enforcement=sealed-constructor; promise=thread folder/label membership reflects current provider truth across all sync paths, including partial-delta after cross-client mutation.

## Parking Lot

Items that surfaced during audit but do not meet the Promise Rule. Kept here so future passes don't re-discover them as findings. Each cluster names *why* it's parked, so it stays parked.

### Parallel Content Stores

Three content stores - `body_store`, `inline_image_store`, `attachment_pack` (`crates/stores/src/`) - implement similar `put` / `get` / `delete` contracts. Body store and inline image store also expose synchronous variants of the same read methods (`get_batch_sync`) for callers already inside `spawn_blocking`.

"Similar APIs differ" is only a discrepancy if architecture promises substitutability. No shared trait exists, no call site polymorphs over the stores, no documented substitutability contract. The similarity is convergent design, not promised equivalence. The sync/async variants within each store don't violate the Promise Rule either - they promise to return the same value, one inside `spawn_blocking` and one not. That's structural API ergonomics, not a contract violation.

Graduates to inventory only if a `ContentStore` trait lands or a real caller starts polymorphing over the three stores. Until then, future audit passes should skip this cluster.

### Provider `create_label` Color Returns

All three providers (JMAP, Graph, IMAP) hardcode `color_bg: None, color_fg: None` in `ProviderOps::create_label`'s return; the canonical post-creation color is written by sync ingest, not by the create call.

No shared promise. The create-op return answers "did creation succeed and what is the new label's ID"; the post-sync DB row answers "what color does the server canonically store for this label." Different questions, served by different paths intentionally.

Graduates to inventory only if a UX path starts needing the immediate color from the create result (for example, to confirm the user-picked color against what the server actually stored before sync echo). Until then, the `None` returns are correct and not a contract violation.

## See Also

- `docs/contracts-roadmap.md` - implementation order, design sketches per contract failure, migration scope, fidelity ceilings, and the cross-crate capability-construction design decision.
- `docs/optimistic-label-intent.md` - design slice for the first inventory entry.
- `docs/architecture.md` - per-message membership store pattern referenced by the cross-client cure.
- `docs/glossary/folders-labels.md` - canonical folder/label and aggregate-state semantics.
