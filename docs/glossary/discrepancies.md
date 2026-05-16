# Codebase Discrepancies

This document is about a class of bug: code paths "for the same concept" diverge across the codebase and silently produce inconsistent results. A list and a count pick different SQL aliases and disagree on what they're filtering. A stored value exists for some property but a downstream renderer re-derives it from a name hash and ignores the stored copy. A composite operation works in the happy path but its per-member fan-out bypasses the preflight on retry. Each one a quiet wrong answer.

The eventual fix is **compile-time enforced**: the type system must make the wrong call impossible to write, not just discouraged by convention. This is not an "audit-and-fix" item - auditing keeps drifting back to broken six months later. The point of this document is to name the contracts being violated precisely enough that the type-level enforcement that fixes one class of bug is the same enforcement that prevents the next one.

## The Promise Rule

A discrepancy exists only when two paths claim to answer the same domain question or uphold the same invariant. "Two functions are similar" is not a discrepancy; "two functions promise the same answer and give different ones" is.

The promise rule is operational, not philosophical. Before adding an entry to the inventory, name the shared promise. If no promise can be named - if the two APIs are merely *adjacent* rather than *redundant* - the entry belongs in the parking lot, not the inventory. "Three content stores expose similar `put` / `get` / `delete` methods" is parking-lot material unless the architecture promises substitutability. "List query and count query agree on view membership" is inventory material because the UI breaks when they don't.

## Contract Failures

Five contract failures account for every inventory entry. The taxonomy is normative: it names what is missing from the type system that allows the discrepancy to be written. New entries should be tagged with the contract failure(s) they exemplify.

### 1. Semantic Grain Untyped

The same primitive shape - `i64`, `String`, `bool`, a struct like `Thread` or `UnifiedSearchResult` - can mean **message**, **thread**, **bundle**, **search-hit**, or any of several aggregate views. The type system does not distinguish them, so a function that operates on one grain can be composed with a function that returns another, and the result silently mixes layers.

The cure splits into two sub-axes, and inventory entries are tagged separately:

- **`grain.vertical`** (message → thread → bundle): grain-branded predicates and result types. A `MessageDate` is a distinct type from a `ThreadLastActivity`; a `ThreadAggregate` is the only output of the canonical recompute helper. A function that filters by `MessageDate` cannot be silently swapped for one that filters by `ThreadLastActivity` even if both wrap an `i64`.
- **`grain.scope`** (personal account vs shared mailbox vs public folder vs all-accounts): exhaustive enum dispatch with no `Option` fallback as the *public* API. The `ViewScope` enum is right; the `to_account_scope() -> Option<AccountScope>` accessor that allows callers to forget non-Account scopes is the failure.

### 2. Canonical Answer Optional

A caller can choose a non-canonical entry point and get a normal-looking result that silently disagrees with the canonical one. `get_draft_threads` returns only synced drafts; `get_draft_count_with_local` returns synced + local. A new call site to `get_draft_threads` undercounts the sidebar silently. `search()` returns Tantivy-ranked results; `search_sql_fallback()` returns SQL-only results. A call site that picks the wrong one when Tantivy is available gets a degraded result set with no indication.

The cure is one public entry point per question. Narrower paths exist only behind explicit capability or marker types ("I know I want synced drafts only and I accept the undercount"). Doc-comment-enforced contracts are not enforcement.

### 3. Completion State Untyped

A value can exist in a partially-completed state where some fields are real and some are placeholders, and the partial state is observationally equal to the complete state. A `UnifiedSearchResult` built from a Tantivy doc has `is_read: false` and `MatchKind::Body` as placeholders; the *enrichment* pass overwrites them with real values from SQL. Tantivy-only and SQL-only paths skip the enrichment pass entirely, so a partial-state value reaches the renderer indistinguishable from a fully-enriched one.

This is the deferred-enrichment shape. Distinct from #2 because the canonical entry point here is the *completion step*, not the *construction step*: the partial value is legitimate as an intermediate, but it must not type-check where a complete value is required.

The cure is two types - `PartialSearchHit` and `EnrichedSearchHit`, `UnvalidatedColorPair` and `LabelStyle`, etc. - with the only transition between them being the enrichment function. Renderers and downstream code accept only the complete type.

### 4. Mutation Capability Untyped

Whether a write operation has full coverage of the entity (replace) or partial coverage (merge) is a *capability* of the input, not a helper choice. Gmail full-thread sync has full coverage and calls `replace_thread_labels`; Graph/JMAP partial delta has partial coverage and calls `merge_thread_labels`. Today the choice is by convention: a future JMAP path that calls `replace_thread_labels` instead of merging would compile and silently drop labels.

Composite operations and per-member operations are similarly capability-distinguished. A composite must not enqueue per-member retries (the composite's own preflight covers the retry); a non-composite member call must enqueue. Today the distinction is a `suppress_pending_enqueue: bool` flag stashed on `ActionContext` that the composite remembers to set. A new composite that forgets re-introduces the bug.

The cure is capability-encoded inputs and entry points. `MergeInput` accepts partial-delta and only `merge_*` helpers take it; `ReplaceInput` requires full-thread coverage and only `replace_*` helpers take it. Per-member dispatch goes through a `_no_enqueue` entry point that composites use; the public entry point that enqueues is structurally unreachable from inside a composite.

### 5. Validated Domain Type Missing

The type allows representations that should be impossible. `kw:keyword` / `cat:category` / `importance:high` are domain values modeled as `String` with prefix conventions - a `LabelId` of `"keyword"` (missing the `kw:` prefix) or `"importance:medium"` (not a valid importance) type-checks. `decrypt_or_raw(value)` accepts both encrypted and plaintext at the same call site, so a writer that forgot to encrypt looks identical to a reader that handles legacy. A color override stored as `(Some(bg), None)` for the foreground is half a value; the resolver falls back to hash even though a partial value was supplied.

The cure is parse-at-the-boundary, total types inward. `LabelKind` is an enum whose variants take validated payload types (`Keyword(KeywordName)`, `Category(CategoryName)`, `Importance(ImportanceLevel)`, `GmailUser(GmailLabelId)`, …) - the payload types are themselves private-fielded and can only be built by their own validating parsers, so the enum is sealed by inclusion. `StoredSecret` is a parsed type - legacy plaintext rows go through an explicit migration boundary, not a tolerant accessor. `LabelStyle { bg, fg }` is a complete pair; partial values do not exist.

This is one contract failure but several migrations, with very different scope:

- **Credentials** (`decrypt_or_raw`, `decrypt_if_needed`): small, local - two functions in `crates/common/src/crypto.rs`, a handful of callers.
- **Color pairs** (label color resolver, palette swatches): moderate - a constructor change and the call sites that parse hex.
- **Provider kinds** (`ProviderKind`, `FolderKind`, `LabelKind`, `SystemFolderId`): broad, multi-crate. Touches every provider crate, the action service, dev-seed, smart-folder, and the harness. The most substantial #5 migration; worth scoping as a standalone design pass. See `docs/contracts-roadmap.md` for the staged plan.

## Enforcement Techniques

Three techniques implement all five contract failures.

### Sealed Constructors

A type's privacy boundary is its contract. The type exposes its fields but not its constructors; only one function in the crate can build the value, and that function enforces the derivation rule. `ThreadAggregate` has only `ThreadAggregate::compute_in_tx(tx, key)` and `ThreadAggregate::compute_from_messages(&[NonReactionMessage])`; there is no `ThreadAggregate { is_read, ... }` literal in scope outside the constructor's module. A second derivation rule cannot exist because a second constructor cannot exist.

Covers **#1 (grain)** and **#3 (completion state)**. The grain type is sealed; the partial-to-enriched transition is a sealed constructor on the enriched type.

### Capability Tokens

A function signature requires a witness that the caller has the right capability. `replace_thread_labels(input: ReplaceInput, ...)` cannot be called without a `ReplaceInput`, and `ReplaceInput` is only built by code paths that have full-thread coverage. Similarly, the public `drafts_list()` returns a `DraftsView` that is the unique type accepted by the renderer; the synced-only function returns a `SyncedDraftsOnly` that does not satisfy that signature.

Covers **#2 (canonical answer)** and **#4 (mutation capability)**. Phantom types, zero-size witnesses, and newtype wrappers are all valid implementations.

Cross-crate enforcement of capability-token contracts is not perfect in Rust - there is no friend-crate mechanism. The roadmap names this as the highest-uncertainty design question and proposes a standing answer (`docs/contracts-roadmap.md` §Fidelity).

### Boundary Parsing

External input - protocol payloads, on-disk values, user strings - is parsed into a total domain type at the boundary. Inward code never sees the raw form. `LabelKind::parse(raw: &str, provider: MailProviderKind) -> Result<LabelKind, ParseError>` is the only constructor from raw external values; `LabelKind` itself is an enum whose variants are sealed by their payload types (validated newtypes that have their own boundary parsers). `StoredSecret::parse(raw: &str)` handles both new and legacy formats but returns a single typed value; readers see only the parsed type.

Covers **#5 (validated domain)**.

## Multi-tag Legend

Inventory entries carry three tags. The interesting bugs sit at intersections, so multi-tagging is the default.

- **`contracts:`** comma-separated list of `grain.vertical` / `grain.scope` / `canonical-entry` / `completion-state` / `mutation-capability` / `validated-domain`. The contract failure(s) the entry violates. The grain contract is sub-tagged because the two cures (newtype branding vs exhaustive dispatch) are distinct migrations.
- **`enforcement:`** comma-separated list of `sealed-constructor` / `capability-token` / `boundary-parse`. The technique(s) that would prevent the discrepancy at compile time.
- **`promise:`** one short sentence naming the shared invariant the two paths *claim* to uphold and don't. This is the operational form of the promise rule - if the promise can't be named, the entry doesn't belong in the inventory.

## The Motivating Example

`crates/smart-folder/src/sql_builder.rs` builds two queries from one `ParsedQuery`:

- `query_threads` (the list view) - `sql_builder.rs:14-43`
- `count_matching` (the sidebar pill) - `sql_builder.rs:46-68`

Before the fix, `count_smart_folder_unread` set `parsed.is_unread = Some(true)` before calling `count_matching`. The old read/starred clause builder translated that to `m.is_read = 0`, pushed onto `msg_clauses`. The shared SQL skeleton (`build_thread_select_sql` / `build_count_sql`) puts `msg_clauses` *inside* the inner-join messages subquery:

```sql
SELECT ... FROM threads t
INNER JOIN (
  SELECT DISTINCT m.account_id, m.thread_id
  FROM messages m
  WHERE 1=1 {msg_where}    -- m.is_read = 0 lives here
) matched ON ...
WHERE 1=1 {thread_flag_where}
```

So the pill counted "threads where there exists a message satisfying *every* filter simultaneously, including being unread." The list did not enforce unread at all - it just showed whatever the saved query matched and let the thread-list UI render bold/unread state from `t.is_read` (the thread aggregate).

The dev-seed symptom was: "Starred This Week" (`is:starred after:-7`) showed 24 unread threads when opened and a 0 pill, because the threads had an older unread message and a newer read message. The thread was unread at the aggregate level, satisfied the list query (a recent message exists), but did not satisfy the pill (no single message is both recent and unread).

Current status: `crates/smart-folder/src/sql_builder.rs::build_thread_state_clauses` emits read, unread, and starred predicates against `threads` (`t.is_read`, `t.is_starred`) through `thread_flag_clauses`. The list and count builders consume the same thread-flag clause set. Per-glossary aggregate semantics are documented in `docs/glossary/folders-labels.md`.

Tags: contracts=grain.vertical,canonical-entry; enforcement=sealed-constructor,capability-token; promise=list query and count query answer the same view-membership question.

This is the worked example for **#1 (grain.vertical)** - the smart-folder thread-state predicates are now grain-branded to the thread level. The remaining inventory entries below are the call sites where the same shape recurs, often crossed with other contract failures.

## Inventory

Findings from a slice-by-slice audit. Each entry preserves the auditing agent's evidence verbatim; the tag lines at the end of each entry are the only addition. Slice attribution is kept so overlap across slices is visible as a confidence signal. Shape 1-12 are evidence buckets keyed into the contract failures above. Shape 13 has moved to the parking lot.

### Shape 1 - Predicate alias divergence

- `crates/db/src/db/queries_extra/chat.rs:235` *(slice 1)* - `WHERE t.is_chat_thread = 1 AND tp.email = ?1 AND m.is_read = 0` in unread-affected count query. Current convention: chat unread is computed against `m.is_read` (message-level) in the affected-count fetch (line 235) but also stored to and read from `chat_contacts.unread_count`. Divergence risk: if a future refactor moves the affected-count query to join `threads` and filter `t.is_read` instead, the recompute at line 413-421 would still use `m.is_read`, causing the stored aggregate to desync.

  Tags: contracts=grain.vertical; enforcement=sealed-constructor; promise=chat unread count and chat unread recompute aggregate the same per-message predicate.

- `crates/db/src/db/queries_extra/chat.rs:417` *(slice 1)* - `WHERE t.is_chat_thread = 1 AND m.is_read = 0 AND LOWER(m.from_address) = ?1` in the unread-recompute query (lines 413-421). Message-level predicate on `m.is_read`. The affected-count query (235) also uses `m.is_read`, so currently consistent. However, `chat_contacts.unread_count` is a thread-aggregate column (summarizes unread state per contact). Convention only prevents divergence here: the two query sites must both use `m.is_read` or both switch to `t.is_read` in lockstep.

  Tags: contracts=grain.vertical; enforcement=sealed-constructor; promise=chat unread count and chat unread recompute aggregate the same per-message predicate.

- `crates/db/src/db/queries_extra/thread_detail.rs:147-151` *(slice 2)* - `query_thread_state_decorations` queries `MAX(is_replied)`, `MAX(is_forwarded)` from `messages` without filtering `is_reaction = 0`. Current convention: `compute_thread_aggregate` (thread_persistence.rs) filters `is_reaction = 0` in all its message-level queries (lines 62, 71, 81, 92, 102, 112). The divergence means thread decorations could include reactions when computing aggregate state would exclude them.

  Tags: contracts=grain.vertical,completion-state; enforcement=sealed-constructor; promise=thread-level glyphs reflect non-reaction message state.

- `crates/smart-folder/src/sql_builder.rs:219-223` *(deep slice: smart-folder + search)* - `m.date < ?{idx}` / `m.date > ?{idx}` in `build_date_clauses`. Current convention: date predicates operate at message-level in smart-folder's SQL path. However, `crates/core/src/search_pipeline.rs:345-362` converts Tantivy results to unified results using `t.last_message_at` when enriching from SQL results (line 354, 411). The two paths use different columns: smart-folder filters on `m.date` (message-specific), search pipeline's SQL-fallback path (lines 120) uses `t.last_message_at` (thread-aggregate). A thread with a recent read message and an older unread message would match `after:-7 m.date` in smart-folder but might diverge on `t.last_message_at` depending on which message was inserted most recently to the thread.

  Tags: contracts=grain.vertical; enforcement=sealed-constructor; promise=date predicates across smart-folder and search return the same threads.

### Shape 2 - Duplicate source of truth at render time

*(The provider `create_label` color-return entries and the store sync/async entries previously listed here have moved to the parking lot - neither cluster meets the promise rule.)*

- `crates/app/src/helpers.rs:530-551` *(slice 1)* - `local_draft_to_app_thread` is a second constructor for the app's `Thread` type, parallel to `db_thread_to_app_thread` (line 505-528). It hardcodes `is_read: true`, `is_starred: false`, `is_replied: false`, `is_forwarded: false`, `has_attachments: false`, `label_color_bgs: Vec::new()`. Current convention: app-layer merge in `load_threads_for_current_view` (crates/app/src/helpers.rs:167-175 per glossary) concatenates synced drafts from `get_draft_threads` with local drafts promoted via `local_draft_to_app_thread`. If the canonical `db_thread_to_app_thread` gains new fields or default logic (e.g. label-color resolution per glossary § 87-98), the local-draft constructor will silently lack them, and the sidebar's draft list will show a subset of decorations for local drafts only.

  Tags: contracts=canonical-entry,completion-state; enforcement=sealed-constructor,capability-token; promise=Drafts list rows have a single value shape regardless of sync state.

- `crates/label-colors/src/lib.rs:40-50` *(slice 4)* - `resolve_label_color` is the single entry point that returns `(bg_hex, fg_hex)` tuples with priority: user_color > server_color > hash fallback via `color_for_label`. Current convention: callers must invoke this one resolver; the type system does not prevent a hypothetical call site from re-deriving color via `color_for_label` alone and ignoring synced values.

  Tags: contracts=canonical-entry,validated-domain; enforcement=sealed-constructor,boundary-parse; promise=label rendering consults the synced color before falling back to hash.

- `crates/app/src/db/pinned_searches.rs:33-55` *(slice 8)* - Duplicate `db_thread_to_app_thread` converter: initializes `is_replied: false`, `is_forwarded: false`, `label_color_bgs: Vec::new()` identically to the helpers.rs copy (line 506). Convention: both feed through `apply_thread_decorations` (line 86) to fill in the actual values, but duplication creates divergence risk if decorations behavior changes and one site is missed. Current convention enforces consistency through shared decorator call, but the type system does not prevent a future copy from skipping the call.

  Tags: contracts=completion-state; enforcement=sealed-constructor; promise=thread values across views have a single derivation.

- `crates/app/src/handlers/search.rs:927-949` *(slice 8)* - `unified_result_to_thread` inline converter: initializes `is_replied: false`, `is_forwarded: false`, `label_color_bgs: Vec::new()`, then calls `apply_thread_decorations` (line 920). Same pattern as other converters, but inlined at call site rather than factored into a shared helper.

  Tags: contracts=completion-state; enforcement=sealed-constructor; promise=thread values across views have a single derivation.

- `crates/app/src/helpers.rs:478-498` *(slice 8)* - Public folder items inline converter: hardcodes the same fields and calls `apply_thread_decorations` (line 500). Initialization values identical to the other converters.

  Tags: contracts=completion-state; enforcement=sealed-constructor; promise=thread values across views have a single derivation.

- `crates/search/src/lib.rs:915` *(deep slice: smart-folder + search)* - `MatchKind::Body` hardcoded as default in `collect_results`. Tantivy-path search results all default to `Body` unless `enrich_match_kinds` (lines 952-1063) is called. Current convention: `crates/core/src/search_pipeline.rs:305` calls `enrich_match_kinds` only in Tantivy-with-free-text paths (lines 155, 204). SQL-only search (lines 72, 135-141) never calls enrichment, leaving all results as `MatchKind::Body` regardless of which field actually matched. The canonical match-kind determination lives in `enrich_match_kinds`'s per-field snippet generation (lines 1017-1061); SQL-only results silently bypass it.

  Tags: contracts=completion-state,canonical-entry; enforcement=sealed-constructor; promise=search results report which field actually matched.

- `crates/core/src/search_pipeline.rs:388` *(deep slice: smart-folder + search)* - `is_read: false, is_starred: false` hardcoded in `tantivy_result_to_unified`. Tantivy results (line 388-389) never populate `is_read`/`is_starred` from the Tantivy doc's stored fields; they remain `false`. In the combined path (line 407), SQL enrichment overwrites them with correct values. In Tantivy-only (line 147-162), they remain `false` even though the Tantivy doc stored these values. Current convention: Tantivy-only results show stale read/starred state unless the caller also performs SQL enrichment (which Tantivy-only does not).

  Tags: contracts=completion-state,canonical-entry; enforcement=sealed-constructor; promise=search results show the thread's true read/starred state.

- `crates/core/src/search_pipeline.rs:388-394` *(deep slice: core + seen + label-colors)* - `tantivy_result_to_unified()` hardcodes `is_read: false, is_starred: false` when converting Tantivy results, even though the TantivyResult struct has populated `match_kind` and `also_matched`. In the combined path (line 407), `enrich_from_sql()` overwrites these with correct values from the thread aggregate. In Tantivy-only (line 73), results remain `false` even though SQL enrichment is never called. Current convention: Tantivy-only search shows stale read/starred state until caller performs SQL enrichment. Current code at line 388-389 documents this as intentional ("false" initial state), but consumers of `search_tantivy_only` path (line 147-163) never overwrite these fields. *(Corroborates the prior smart-folder+search deep-slice entry above.)*

  Tags: contracts=completion-state,canonical-entry; enforcement=sealed-constructor; promise=search results show the thread's true read/starred state.

### Shape 3 - Aggregate-vs-input drift

- `crates/db/src/db/queries_extra/thread_persistence.rs:79-93` *(slice 2)* - `recompute_thread_read_starred` is the canonical write path for `threads.is_read` and `threads.is_starred` (provider_sync_writes.rs also names it as canonical per discrepancies.md). It uses `MIN(is_read)` and `MAX(is_starred)` across all messages without an `is_reaction = 0` filter. This differs from `compute_thread_aggregate` (same file, lines 68-86) which does filter reactions when reading message state during ingest. If reactions ever gain `is_read` or `is_starred` columns that differ from the containing message, the recompute and ingest paths will write different values to `threads`.

  Tags: contracts=grain.vertical,completion-state; enforcement=sealed-constructor; promise=thread aggregate uses the per-field reducer (MIN for is_read, MAX for is_starred) over non-reaction messages.

- `crates/db/src/db/queries_extra/bundles.rs:68-93` *(slice 3)* - `HAVING t.last_message_at = MAX(t.last_message_at)` in the latest-message query: `t.last_message_at` is a thread aggregate, but `MAX(t.last_message_at)` inside a `GROUP BY tc.bundle` can only reference the grouped rows' aggregates. If `t.last_message_at` falls out of sync with the actual max message date for the thread, the predicate silently drifts. Current convention: `recompute_thread_read_starred()` in provider_sync_writes.rs is the canonical recompute path, but bundles query does not call it-it assumes staleness-free aggregates.

  Tags: contracts=grain.vertical,completion-state; enforcement=sealed-constructor; promise=thread `last_message_at` is the max message date for the thread.

- `crates/sync/src/pipeline.rs:93-95` *(slice 5)* - Hardcoded thread aggregate computed in-memory via `.all(m.is_read)` / `.any(m.is_starred)` / `.any(m.has_attachments)` on the `messages` Vec. Current convention: Provider-sync callers (`crates/provider-sync/src/graph/sync/persistence.rs:178`, Gmail `storage.rs:118`, JMAP `storage.rs:169`) instead call `compute_thread_aggregate()` from `db`, which filters `WHERE is_reaction = 0` in all message-level queries. If a reaction message with different `is_read`/`is_starred` state arrives via the sync/pipeline path (JWZ threading), the aggregate will be computed differently from the DB recompute path that fires on provider sync.

  Tags: contracts=grain.vertical,completion-state; enforcement=sealed-constructor; promise=thread aggregate uses the canonical per-field reducer over non-reaction messages.

- `crates/dev-seed/src/threads.rs:246-378` *(slice 10)* - Thread aggregate `is_read` is randomly sampled per-thread (line 246: `rng.random::<f64>() < 0.7`), but per-message `is_read` values are computed deterministically based on position in thread (line 378: `if mi < num_msgs - 1 { true } else { is_read }`). The thread's initial `is_read` random value (line 289) is not recomputed from `MIN(m.is_read)` after messages are inserted; the final state at line 517 does not touch `is_read`. Current convention: dev-seed seeding is one-time setup with no recompute contract, but the divergence between independently-randomized thread state and position-derived message state creates a database where `MIN(m.is_read) != t.is_read` can drift immediately. The same pattern applies to `is_starred`: thread-level random sample (line 247) vs message-level (line 465: `is_starred && mi == 0`), where only the first message carries the starred flag. Thread aggregate is initialized with the random boolean but never recomputed; the canonical recompute helper (`db::compute_thread_aggregate::MIN/MAX`) is not called.

  Tags: contracts=grain.vertical,completion-state; enforcement=sealed-constructor; promise=seeded thread aggregate derives from seeded message state via the canonical reducer.

- `crates/dev-seed/src/chats.rs:323-484` *(slice 10)* - Chat thread seeding computes `thread_is_read` in-loop by setting `thread_is_read = false` whenever an unread message is encountered (lines 369-371). Final thread `is_read` at line 474 is `UPDATE threads SET is_read = ?3` using this loop-computed value. Current convention: the loop correctly computes the aggregate via `if !msg_is_read { thread_is_read = false }`. Consistency check: this matches the canonical `MIN(is_read)` semantics (all-must-be-read). Unlike `threads.rs` which initializes thread state independently of messages, `chats.rs` derives the aggregate from the messages during seeding, which is correct. No divergence detected here; included as a reference showing correct seeding pattern.

  Tags: *(counter-example; no violation. Retained as reference for the correct seeding shape.)*

- `crates/search/src/lib.rs:493-524` *(deep slice: smart-folder + search)* - `build_search_doc` constructs Tantivy documents with message-level `date` field (line 492-495), but the search pipeline materializes thread-aggregate results and sorts by `thread.last_message_at` (search_pipeline.rs:157). When a thread has multiple messages with different dates, the per-message doc carries the message's date, but thread-level sorting uses the thread-aggregate. A query `before:2025-01-01` against Tantivy would match a message with `date < threshold`, but thread-sort order is determined by `t.last_message_at` which might be a newer message's date. The result is correct filtering but potentially surprising ordering.

  Tags: contracts=grain.vertical; enforcement=sealed-constructor; promise=search filter grain and search sort grain answer the same question.

### Shape 4 - Merge-vs-replace asymmetry

- `crates/db/src/db/queries_extra/thread_persistence.rs:505-555` *(slice 2)* - Dual helper families `replace_thread_labels` / `merge_thread_labels` exist with no type-level enforcement of which call site uses which. `crates/provider-sync/src/gmail/sync/storage.rs` and `crates/sync/src/pipeline.rs` call `replace_thread_labels` (Gmail + full-thread sync); `crates/provider-sync/src/graph/sync/persistence.rs` calls `merge_thread_labels` (Graph partial-delta). JMAP calls `merge_thread_folders` but the equivalent label path does not use an explicit helper - the label IDs are mixed into the `merge_thread_folders` call directly (labels_attachments.rs:209-215 in Graph). No type prevents a provider from picking the wrong helper; a future JMAP label sync that calls `replace_thread_labels` instead of merging would silently drop existing labels.

  Tags: contracts=mutation-capability; enforcement=capability-token; promise=providers use the merge/replace helper that matches their delta semantics.

- `crates/db/src/db/queries_extra/bundles.rs:85-106` *(slice 3)* - `db_get_bundle_summaries()` at line 85 (latest query): uses `JOIN messages m` without deduplication on per-bundle row, then groups by `tc.bundle` and selects via `HAVING t.last_message_at = MAX(t.last_message_at)`. The count query at line 68 uses `COUNT(DISTINCT t.id)` and `GROUP BY tc.bundle`. Pattern suggests bundle aggregate (per-thread summary) but aggregate-vs-message-query divergence on whether the latest sender/subject come from the message that matches the max timestamp per bundle or an arbitrary message-group combination. Current convention: raw SQL without intermediate summary table.

  Tags: contracts=grain.vertical,completion-state; enforcement=sealed-constructor; promise=bundle summary picks consistent metadata from the canonical latest message.

- `crates/db/src/db/queries_extra/misc.rs:84-105` *(slice 3)* - `db_get_subscriptions()`: uses `MAX(m.date)` and `MAX(m.from_name)` alongside `GROUP BY LOWER(m.from_address)`. Query aggregates sender metadata across messages but picks the message with max date for the latest_unsubscribe headers. If a message has an older date but newer unsubscribe header, the mismatch is silent. Current convention: MAX() functions rely on message ordering, no explicit subquery for canonical latest-message.

  Tags: contracts=grain.vertical; enforcement=sealed-constructor; promise=subscription summary picks all metadata from the same canonical message.

- `crates/provider-sync/src/jmap/sync/storage.rs:180-197` *(slice 5)* - `set_thread_labels()` calls `merge_thread_folders()` only, never `merge_thread_labels()`. JMAP partial-delta label changes are handled only via `sync_keyword_labels()` (lines 286-329) which does `INSERT OR IGNORE` without any explicit delete or merge. Current convention: JMAP folders merge correctly; `kw:*` label changes INSERT-only into thread_labels. Non-keyword JMAP labels (if any) flow through `message.base.label_ids` but are never deleted from thread_labels on per-message removal, creating asymmetry with the Gmail/Graph `merge_thread_labels()` pattern. See also gmail/storage.rs:71 comment: `// messages. replace_thread_labels inserts FK-constrained rows` indicating replace semantics are used by Gmail full-thread paths.

  Tags: contracts=mutation-capability; enforcement=capability-token; promise=partial-delta paths handle label removal symmetrically with folder removal.

- `crates/provider-sync/src/imap/sync_pipeline.rs:323-346` *(slice 5)* - `recompute_thread_keyword_labels()` destructively `DELETE FROM thread_labels` then re-inserts from `message_keywords` union. IMAP account threads carry only `kw:*` labels; the DELETE is safe (lines 329-333). However, the per-message `replace_message_keywords()` helper (lines 299-314) first deletes all keywords for a message, then inserts the new set. If a message is removed from a thread without calling `recompute_thread_keyword_labels()` after, stale `message_keywords` rows remain and the thread-level union grows stale until recompute fires.

  Tags: contracts=mutation-capability,completion-state; enforcement=capability-token; promise=thread_labels `kw:%` rows = union of `message_keywords` for thread's messages.

- `crates/provider-sync/src/gmail/sync/storage.rs` (implicit via call to sync_persistence) *(slice 6)* - Gmail calls `replace_thread_folders` + `replace_thread_labels` (per glossary § 175-177). JMAP (crates/provider-sync/src/jmap/sync/storage.rs:189-196) calls `merge_thread_folders` + no explicit label-merge call documented. Graph (crates/provider-sync/src/graph/sync/persistence.rs:209-215) calls `merge_thread_folders` + `merge_thread_labels`. Current convention: Gmail full-sync = replace; JMAP/Graph partial-delta = merge. No type prevents a provider from picking wrong helper if calling sync_persistence directly (though provider-sync crates own the call sites). (1 more elided)

  Tags: contracts=mutation-capability; enforcement=capability-token; promise=providers use the merge/replace helper that matches their delta semantics.

### Shape 5 - Format-tolerant accessor

Credential accessor entries resolved by contract #5a: `common::crypto::StoredSecret` is now the only raw storage parser, `decrypt_or_raw` and `decrypt_if_needed` are deleted, and Gmail, Graph, JMAP, and IMAP decrypt through the typed parse product. Legacy plaintext remains accepted only inside `StoredSecret`; the strict rejection or one-shot migration decision remains open in `docs/contracts-roadmap.md`.

- `crates/label-colors/src/lib.rs:40-58` *(deep slice: core + seen + label-colors)* - `resolve_label_color()` accepts both complete user-color pairs `Option<(&str, &str)>` and partial synced colors `Option<&str>, Option<&str>` as separate arguments. The function enforces that both `server_color_bg` AND `server_color_fg` are present together (line 49: `(None, Some(bg), Some(fg))`), but does not validate that if either is present, the other is also present. Current convention: callers must ensure synced colors are always paired. No type prevents a call site from passing `Some(bg)` with `None` for `fg`, which would silently fall through to the hash fallback despite a synced value being partially available.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=a label style is a complete (bg, fg) pair or nothing.

### Shape 6 - Kind-encoded-in-string

- `crates/core/src/db/queries.rs:38-39` *(slice 4)* - `is_replied` and `is_forwarded` are read from message rows as raw `i64` and cast to bool. Current convention: these columns exist on `messages` only (per glossary § 244-248); no per-message membership table yet. Any future call site that needs "thread that has been replied to" must use an explicit `EXISTS` subquery or risk silently omitting threads whose reply marker exists only on a subset of messages.

  Tags: contracts=grain.vertical,validated-domain; enforcement=sealed-constructor; promise=per-message boolean reads carry their grain explicitly.

- `crates/provider-sync/src/jmap/sync/storage.rs:304` *(slice 5)* - JMAP keyword prefix hardcoded as `format!("kw:{keyword}")` (line 304). If a future non-keyword JMAP label arrives and is processed through the standard label_ids path without prefix sniffing, the thread_labels row will land without the `kw:` prefix, breaking the invariant that JMAP IMAP-only keywords are prefixed. Current convention: IMAP upserts `kw:` rows via `replace_message_keywords()` in sync_pipeline.rs; JMAP upserts via `sync_keyword_labels()` format!. No call site sniffs the prefix to validate correctness-the assumption is that all callers know their category.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=JMAP keyword labels carry the `kw:` prefix.

- `crates/provider-sync/src/graph/sync/persistence.rs:227-228` *(slice 5)* - Graph category prefix hardcoded as `format!("cat:{category}")` (line 228). If a future Graph sync path processes categories without this prefix, the category label will misidentify as a standard label. Current convention: `upsert_graph_label_rows()` is the sole call site that synthesizes `cat:` rows; only called from `store_thread_to_db()` on every thread persist. If a new Graph endpoint or refactor bypasses `upsert_graph_label_rows()`, stale category labels land unprefixed.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=Graph categories carry the `cat:` prefix.

- `crates/provider-sync/src/graph/sync/persistence.rs:246-261` *(slice 5)* - Graph importance prefix hardcoded as the match arm in `graph_importance_label()` (line 273-278). The strings `"importance:high"` and `"importance:low"` are recognized as special and synthesized into label rows only inside `upsert_graph_label_rows()`. If importance labels flow through a different provider label sync path (e.g., a hypothetical direct label merge), they will not be recognized as system labels and will land as user-visible rows without `is_undeletable=true`.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=Graph importance labels carry the `importance:` prefix and `is_undeletable`.

- `crates/jmap/src/ops.rs:295` *(slice 6)* - `label_id.as_str().strip_prefix("kw:")` extracts keyword from label ID. If a call site passes a non-kw: label, the conditional silently takes the false branch and treats it as a mailbox operation. Current convention: label IDs are strings; callers must know `kw:` implies keyword mutation vs mailbox. (2 more elided)

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=label routing dispatches by typed kind, not string prefix.

- `crates/graph/src/ops/mod.rs:169` *(slice 6)* - `label_id_str.strip_prefix("cat:").unwrap_or(label_id_str)` extracts category name or defaults to the full string. If the caller passes `"importance:high"`, the strip_prefix returns None and the category name becomes `"importance:high"` (wrong). Current convention: call site should have already routed `importance:*` labels via `graph_importance_for_label` (line 164); if a future call site adds a label without the importance check, it silently misclassifies.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=label routing dispatches by typed kind, not string prefix.

- `crates/imap/src/ops.rs:379` *(slice 6)* - `label_id.strip_prefix("kw:")` with `let Some(keyword) = ... else { ... invalid IMAP label ... }`. If a caller accidentally passes a `"folder-*"` label (which is valid folder syntax) to this remove path, the strip_prefix fails and the function errors instead of routing to folder removal. Current convention: remove_label must check the prefix before dispatch; a missing check silently rejects valid label IDs.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=label routing dispatches by typed kind, not string prefix.

- `crates/service/src/actions/label.rs:57-66` *(slice 7)* - `label_id.strip_prefix("kw:") / "cat:" / "importance:"` in `ensure_prefixed_tag_label`. Current convention: `add_label_local` sniffs the prefix to determine label kind and synthesize rows; `opposite_importance_label` (line 313-318) pattern-matches hardcoded strings `"importance:high"` / `"importance:low"` to implement mutual exclusion. If a new action path (or retry dispatch) calls `add_label` directly without verifying prefix format, importance labels bypass the mutual-exclusion logic and both rows land. See also `ensure_prefixed_tag_label` comments (line 70-72): the OR semantic on `is_undeletable` repairs synced-before-invariant rows, but the write path doesn't validate that an importance row with `is_undeletable=false` already landed and needs repair before insertion.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=label kind dispatch is total over typed kinds, not partial over string prefixes.

- `crates/dev-seed/src/accounts.rs:256-263` *(slice 10)* - `seeded_user_label_id()` synthesizes label IDs with provider-specific prefixes: `kw:` for IMAP/JMAP (line 260), `cat:` for Graph (line 259), bare string for Gmail. Current convention: the prefix encoding is documented inline (lines 256-262 comment), and the function is internal to seed-only. However, any future call site that needs to synthesize labels without going through this function risks generating rows with wrong prefix format. Type system does not prevent a hypothetical `create_label_for_provider(name: &str)` call from skipping the prefix injection.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=provider label IDs carry the right prefix for their provider.

- `crates/dev-seed/src/accounts.rs:348-364` *(slice 10)* - Graph accounts bootstrap synthesised importance labels (lines 353-354) with hardcoded `"importance:high"` / `"importance:low"` ID strings and `is_undeletable = 1`. Current convention: per `docs/architecture.md` "Pre-create rows for any label_id that lands in `thread_labels`," the importance rows must always carry both the prefix and the undeletable flag. Dev-seed correctly synthesizes them here, but the contract is by convention: a future ingest path that treats importance labels as raw categories and injects them without the prefix / undeletable invariant would silently misclassify. The type system does not enforce that `importance:*` labels are a closed enum and never bare strings.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=importance labels are a closed enum, not bare strings.

- `crates/smart-folder/src/sql_builder.rs:302-336` *(deep slice: smart-folder + search)* - `in_folder` clause uses hardcoded uppercase folder IDs (`INBOX`, `SENT`, `DRAFT`, `TRASH`, `SPAM`, `archive`, `IMPORTANT`) mapped from lowercase input (line 312-315). Current convention: the `IN_FOLDER_SHORTHANDS` constant defines the mapping. If a future refactor adds a new system folder without updating this map, queries with `in:newfolder` silently fail to match (taken as free text). Type system does not prevent a folder ID lookup from using the wrong case or missing entries.

  Tags: contracts=validated-domain; enforcement=boundary-parse; promise=system folder IDs are a closed enum.

- `crates/smart-folder/src/sql_builder.rs:425-443` *(deep slice: smart-folder + search)* - `label_group_rendered_fragment` uses string-formatted SQL with `account_alias`, `thread_alias`, and `group_predicate` parameters. Two call sites: `build_is_tagged_clause` (line 448-452) uses `"t.account_id"` / `"t.id"`, while `build_label_clause` (line 484-488) uses `"m.account_id"` / `"m.thread_id"`. Current convention: both produce syntactically valid SQL but filter on different table aliases. If `build_label_clause` were to receive a predicate meant for thread-level filtering (e.g., written by a maintainer assuming message-level), the divergence would silently propagate to the query. The helper factory should require typed alias parameters rather than strings.

  Tags: contracts=grain.vertical,validated-domain; enforcement=sealed-constructor; promise=label-group query alias is grain-correct.

- `crates/core/src/scope.rs:31-36` *(deep slice: core + seen + label-colors)* - `ViewScope::to_account_scope()` exhaustively matches the four variants and returns `None` for scope types that need different query paths (SharedMailbox, PublicFolder). Current convention: callers that receive `None` must dispatch to dedicated query paths. However, the function is not forced to exhaustively handle both arms: a hypothetical refactor that adds a new `ViewScope` variant would type-check but `to_account_scope()` would still compile if the match statement misses the new variant (relying on catch-all `_ => None` if one existed). Current code has no catch-all (correctly), but the pattern could be fragile across future schema changes.

  Tags: contracts=grain.scope,canonical-entry; enforcement=capability-token; promise=ViewScope dispatch covers all four scope variants without `Option` escape.

- `crates/common/src/html_sanitizer.rs:332-353` *(deep slice: stores + crypto-key + common)* - `sanitize_html_body_with_image_policy(html, block_remote_images: bool, sender_is_allowlisted: bool)` branches on a boolean `block_remote_images` to decide whether to call `strip_remote_images`. Current convention: the decision is made by the caller before dispatch. If a call site fails to set `block_remote_images` correctly, the same HTML receives different treatment at different times silently. The two entry points (`sanitize_html_body` always passes through, `sanitize_html_body_with_image_policy` branches) mean a future caller that needs image blocking must remember to use the second entry point and pass the two booleans in the right positions.

  Tags: contracts=validated-domain,canonical-entry; enforcement=boundary-parse,capability-token; promise=HTML sanitization image policy is a single typed decision per call site.

### Shape 7 - Composite/global-flag contract

- `crates/service/src/actions/label_group.rs:445` *(slice 7)* - `member_ctx.suppress_pending_enqueue = true` inside `dispatch_member_ops`. The composite applies the flag to a cloned context before dispatching per-member label additions/removals (line 451-461). Current convention: comments at line 422-429 flag the design; the contract is "any composite must clone and set this flag before dispatching member writes." A future composite action that forgets the clone-and-set re-introduces the bug: the member writes enqueue individual pending ops, the composite returns success, but the drainer later re-executes those per-member ops without the composite's preflight (lines 78-94 in `apply_label_group_local`), violating the invariant that user-reversed intent between apply and retry drains correctly. Compile-time direction: per-member dispatches should go through a `_no_enqueue` entry point that both composites and non-composite callers route through uniformly.

  Tags: contracts=mutation-capability; enforcement=capability-token; promise=composite operations do not enqueue per-member retries.

### Shape 8 - List/count entry-point split

- `crates/db/src/db/queries_extra/scoped_queries.rs:621-664` *(slice 1)* - `get_draft_threads` returns only synced drafts (threads with DRAFT folder). `get_draft_count_with_local` (line 708) is the canonical count entry point, combining synced threads from `get_thread_count_scoped(..., Some("DRAFT"))` with local drafts from `count_local_drafts`. Current convention: the function docstring (621-636) explicitly flags that callers should use `get_draft_count_with_local` for accurate counts, and the app layer handles the merge at line 167-175 of helpers.rs. However, the type system does not prevent direct calls to `get_draft_threads` from a new call site, which would silently undercount. Per glossary § 101-108, this is the motivating example for the compile-time goal: one entry point for "everything in the Drafts list," with direct access to the synced-only path requiring an explicit opt-in (different function name or marker type).

  Tags: contracts=canonical-entry; enforcement=capability-token; promise=Drafts list and Drafts count answer the same membership question.

- `crates/db/src/db/queries_extra/scoped_queries.rs:638-664` *(slice 3)* - `get_draft_threads()` returns only server-synced drafts; `get_draft_count_with_local()` at line 708 returns synced + local. Doc comment at line 634: "Any consumer that calls `get_draft_threads` directly will silently disagree with `get_draft_count_with_local()` and with what the sidebar shows." Current convention: doc comment + app-layer merge in `crates/app/src/helpers.rs:167-175` (not enforced by type system).

  Tags: contracts=canonical-entry; enforcement=capability-token; promise=Drafts list and Drafts count answer the same membership question.

- `crates/core/src/db/queries_extra/navigation.rs` *(slice 4)* - The drafted sidebar count uses the unified entry point per glossary § 101-108. The merged list in `crates/app/src/helpers.rs` (outside this slice) invokes the same count path. Current convention: app-layer docstring flags direct `get_draft_threads` callers, but the type system does not enforce the unified entry point.

  Tags: contracts=canonical-entry; enforcement=capability-token; promise=Drafts list and Drafts count answer the same membership question.

- `crates/core/src/search_pipeline.rs:51-90` *(deep slice: smart-folder + search)* - `search()` is the unified entry point, but internally dispatches to `search_sql_only`, `search_tantivy_only`, `search_combined`, each with distinct query semantics. The `search_sql_fallback` function (lines 95-130) exists as a parallel entry point that routes differently: it returns empty results for free-text-only queries (line 106) instead of consulting Tantivy. Current convention: callers must use `search()` for full-feature behavior and `search_sql_fallback()` for degraded-mode-only. Type system does not prevent a call site from accidentally using the fallback path when Tantivy is available, leading to silent result-set discrepancy (no-results vs Tantivy-ranked results).

  Tags: contracts=canonical-entry,completion-state; enforcement=capability-token; promise=the public search result declares whether it is full-index or degraded, and the renderer must handle both - no silent fallback.

### Shape 9 - Implicit bundle/label semantics divergence

- `crates/search/src/lib.rs:1142-1162` *(deep slice: smart-folder + search)* - `group_by_thread` keeps the highest-scoring result per thread_id, but the threading decision is message-level (line 1145-1152). When multiple messages in a thread match the query, only the highest-scoring one becomes the thread-group representative. However, thread-metadata enrichment (search_pipeline.rs:402-413) fills `subject`, `from_name`, `from_address` from the SQL thread row, not from the highest-scoring message. Current convention: Tantivy scores by message, grouping reduces to one message per thread, but metadata comes from the thread aggregate. If the best-matching message is old and a newer message with different sender is recent, the result displays the recent sender (from thread aggregate) paired with the old message's relevance score.

  Tags: contracts=grain.vertical; enforcement=sealed-constructor; promise=thread search results have a single declared metadata source.

- `crates/core/src/search_pipeline.rs:154-163` *(deep slice: core + seen + label-colors)* - In Tantivy-only search, `search_state.search_with_filters()` returns Tantivy-ranked results keyed by message (line 154). `group_by_thread_unified()` (line 156) reduces to one message per thread by taking the highest score. The result's metadata (subject, from_name, from_address) comes from the highest-scoring message's stored Tantivy fields (lines 384-386), but in the combined path (line 212), `enrich_from_sql()` overwrites these with thread-aggregate values (lines 403-406). Current convention: Tantivy-only and combined paths use different metadata sources (message-level vs thread-aggregate), but the result type makes this decision invisible. A search that matches old and new messages in the same thread will show the old message's rank but the thread-aggregate's sender/subject in Tantivy-only, vs the highest-scoring message's metadata in combined.

  Tags: contracts=grain.vertical,completion-state; enforcement=sealed-constructor; promise=thread search results have a single declared metadata source across all internal paths.

### Shape 10 - Partial-delta keyword label loss

*(Instance of Shape 4 with a specific symptom: silent data loss rather than stale row.)*

- JMAP's `sync_keyword_labels()` and IMAP's `recompute_thread_keyword_labels()` handle deletions differently *(slice 5)*. IMAP deletes and re-inserts the entire thread_labels `kw:*` set from message_keywords. JMAP only INSERTs new keywords; if a message with a keyword is removed from a thread via partial delta without the keyword being present in the current delta page, the thread_labels row persists orphaned. The invariant "thread_labels `kw:*` rows = union of message_keywords rows for messages in the thread" can drift between IMAP (enforced by recompute) and JMAP (enforcement is implicit in no-remove behavior).

  Tags: contracts=mutation-capability,completion-state; enforcement=capability-token; promise=`thread_labels.kw:%` rows = union of `message_keywords` for thread's messages, across all providers.

### Shape 11 - Divergent date semantics in date-range construction

*(Sub-case of Shape 1 (grain.vertical) on the operator axis rather than the column-alias axis.)*

- `crates/smart-folder/src/parser/dates.rs` *(deep slice: smart-folder + search)*: The parser converts user date input to Unix timestamps at **start-of-day in local time** (line 127: `date.and_hms_opt(0, 0, 0)` then `Local::from_local_datetime`). Tantivy stores and queries dates via `TantivyDateTime::from_timestamp_secs()` (search/lib.rs:806, 815) using the same Unix-seconds representation. However, the range semantics differ: smart-folder's `before:` generates `m.date < ?` (strictly less than start-of-day), while Tantivy's `before:` uses `Bound::Included(Term)` (line 813), making the threshold inclusive. A message with `date = start_of_day_timestamp` would match Tantivy `before:DATE` but not smart-folder `m.date < start_of_day_timestamp`. Neither path applies a time-zone adjustment at the comparison boundary, so the discrepancy depends on the user's local time at search-submission moment.

  Tags: contracts=grain.vertical; enforcement=sealed-constructor; promise=`before:`/`after:` boundary inclusivity is identical across all query paths.

### Shape 12 - Partial-enrichment contract mismatch

- `crates/core/src/search_pipeline.rs` *(deep slice: core + seen + label-colors)*: The `enrich_from_sql()` function (line 398-415) is called only in the combined path (line 212), never in SQL-only (line 141) or Tantivy-only (line 162). SQL-only uses `db_thread_to_unified()` (line 141) which hardcodes `match_kind: MatchKind::Body` and `also_matched: Vec::new()`, and Tantivy-only propagates the Tantivy result's attribution (lines 392-393). The three paths use different enrichment strategies: SQL-only gets zero attribution, Tantivy-only preserves message-level attribution, combined gets both SQL metadata + Tantivy attribution. The contract is that `enrich_from_sql()` is optional metadata enrichment, not required state-correction, but callers of `search_sql_fallback()` (line 95-130) get SQL-only results without ever knowing they lack the SQL-metadata enrichment step available in the main search path.

  Tags: contracts=completion-state,canonical-entry; enforcement=sealed-constructor,capability-token; promise=a `UnifiedSearchResult` reaching the renderer is fully enriched, regardless of which internal path produced it.

## Parking Lot

Items that surfaced during audit but do not meet the promise rule. Kept here so future passes don't re-discover them as findings. Each cluster names *why* it's parked, so it stays parked.

### Shape 13 - Parallel store entry-point split

Three parallel content stores (`body_store`, `inline_image_store`, `attachment_pack`) each implement nearly-identical `put` / `get` / `delete` contracts. Body store has both async and synchronous batch-get entry points; inline image store has the same; attachment pack is async-only.

**Why parking lot:** "Similar APIs differ" is only a discrepancy if the architecture promises substitutability. Today the three stores are separate concrete types with no shared trait, no shared call site that polymorphs over them, no documented capability contract. Their similarity is convergent design, not promised equivalence. If a future shared-trait design lands ("ContentStore" as a real abstraction), the per-store entry-point split becomes a #2 (canonical-entry) finding under that trait - but only then.

The following entries were previously listed under Shape 2 and are moved here for the same reason - no shared substitutability promise:

- `crates/stores/src/inline_image_store.rs:187-209` *(deep slice: stores + crypto-key + common)* - `get_batch_sync` is a synchronous variant of `get` / `get_batch` (async). Both paths execute inside `spawn_blocking`, but the sync path is callable only from existing blocking contexts while async paths go through `with_conn`. Current convention: callers on blocking threads invoke `get_batch_sync` directly and pass the connection; other callers use async `get` / `get_batch` which re-acquire the lock. The dual API is convenient for nested callers (e.g. `db::with_conn` → db query that needs inline images) but creates two independent paths that must both be tested and maintained as the store evolves.

- `crates/stores/src/body_store.rs:293-346` *(deep slice: stores + crypto-key + common)* - `get_batch_sync` mirrors `get_batch` (async) with identical decompression logic. Both paths chunk large message IDs, lock the connection, decompress outside the lock. Convention: if the async and sync paths ever diverge in their decompression logic or chunking strategy, a caller that used the wrong entry point would silently produce different results. The function comment notes this is "for callers already on a blocking thread" but the type system does not prevent a non-blocking caller from accidentally invoking the sync variant via an unsafe block or tokio spawn_blocking.

- `crates/stores/src/attachment_pack.rs:1-100` *(deep slice: stores + crypto-key + common)* - Pack store has no explicit sync variant; all operations go through async/await. However, the recovery path at `open()` time (lines 197-212) calls `recover_and_open_current_pack` via `spawn_blocking` but without an explicit re-entrant guard. If a caller holding an open `PackStore` tried to call `open()` again from inside a blocking task, they would risk double-locking or double-recovery.

If a `ContentStore` trait ever lands and these stores become substitutable, all three entries above gain a shared promise and graduate back to inventory.

### Provider `create_label` color returns

Three entries previously listed under Shape 2 - `crates/jmap/src/ops.rs:608-609`, `crates/graph/src/ops/mod.rs:366-367`, `crates/imap/src/ops.rs:1016-1017` - all observe that `ProviderOps::create_label` returns `color_bg: None, color_fg: None` hardcoded, with the canonical colors written separately by sync ingest.

**Why parking lot:** there is no shared promise between the two paths. The action service's `create_label` return is deliberately stale-by-design - colors come from sync ingest, not from the create response. The architecture explicitly routes "what's the canonical color of this label" through the post-sync DB row, not through the create-op return. The two paths are not in disagreement; they are answering different questions ("what is the immediate creation result" vs "what is the post-sync canonical state").

If the action service ever needs the immediate color (e.g., a UI optimistic-update use case), the cure is to give `ProviderOps::create_label` a richer return type that promises the colors - and at that point these entries become real inventory entries under contract #3 (completion-state). Until then, they belong here.

## Out of scope for this document

- The Drafts pill semantics question (total-vs-unread contract). Tracked separately in `TODO.md`. That is a product decision about what the pill *should* count; this document is about ensuring the count means what the matching list says it means, whatever the product answer is.
- **Cross-client folder/label move reconciliation** - the specific data-loss case where another client moves a thread and Graph/JMAP partial-delta sync sees only the new membership, leaving the source-folder row stale. Tracked in `TODO.md` under "cross-client folder/label moves." The long-term fix is the per-message membership store pattern documented in `docs/architecture.md`. The general "merge-vs-replace is convention-by-helper-choice" problem this represents *is* inventory material - Shape 4 and roadmap #4 cover it. The TODO is only the cross-client edge case that needs the per-message membership store as its specific cure.
- Implementation order, design sketches for each contract failure, migration scope, fidelity ceilings, and the cross-crate capability-construction design decision - `docs/contracts-roadmap.md`.
