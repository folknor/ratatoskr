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

Whether a write operation has full coverage of the entity (replace) or partial coverage (merge) is a *capability* of the provider path, not a helper choice. Gmail full-thread sync has full coverage and calls a provider-local replace wrapper; Graph/JMAP partial delta has partial coverage and calls provider-local merge wrappers. Optimistic same-client label intent lives in `pending_thread_label_intents`; the remaining risk is partial-delta sync reconstructing full membership when the delta cannot.

Composite operations and per-member operations are similarly capability-distinguished. A composite must not enqueue per-member retries (the composite's own preflight covers the retry); a non-composite member call must enqueue. The current shape is structural: composites call `_no_enqueue` entry points; the public enqueueing entry point is unreachable from inside a composite.

The cure is capability-encoded entry points. Full-thread replace wrappers live inside the Gmail/IMAP provider paths that have complete coverage; partial-delta merge wrappers live inside the Graph/JMAP provider paths that do not. Per-member dispatch goes through a `_no_enqueue` entry point that composites use; the public entry point that enqueues is structurally unreachable from inside a composite.

### 5. Validated Domain Type Missing

The type allows representations that should be impossible. `kw:keyword` / `cat:category` / `importance:high` are domain values modeled as `String` with prefix conventions - a `LabelId` of `"keyword"` (missing the `kw:` prefix) or `"importance:medium"` (not a valid importance) type-checks. `decrypt_or_raw(value)` accepts both encrypted and plaintext at the same call site, so a writer that forgot to encrypt looks identical to a reader that handles legacy. A color override stored as `(Some(bg), None)` for the foreground is half a value; the resolver falls back to hash even though a partial value was supplied.

The cure is parse-at-the-boundary, total types inward. `LabelKind` is an enum whose variants take validated payload types (`Keyword(KeywordName)`, `Category(CategoryName)`, `Importance(ImportanceLevel)`, `GmailUser(GmailLabelId)`, ...) - the payload types are themselves private-fielded and can only be built by their own validating parsers, so the enum is sealed by inclusion. `StoredSecret` is a parsed type - plaintext credential rows are rejected instead of flowing through a tolerant accessor. `LabelStyle { bg, fg }` is a complete pair; partial values do not exist.

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

External input - protocol payloads, on-disk values, user strings - is parsed into a total domain type at the boundary. Inward code never sees the raw form. `LabelKind::parse(raw: &str, provider: MailProviderKind) -> Result<LabelKind, ParseError>` is the only constructor from raw external values; `LabelKind` itself is an enum whose variants are sealed by their payload types (validated newtypes that have their own boundary parsers). `StoredSecret::parse(raw: &str)` accepts only the encrypted storage shape; readers see only the parsed type.

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

Two issues remain in active inventory. Each names the shared promise, the contract failure(s) being violated, and the cure shape. Resolution work for each is sequenced in `docs/contracts-roadmap.md` or in a dedicated design slice.

### Drafts Pill Semantics

Every universal-folder pill in the sidebar shows an `is_read = 0` count via `get_unread_counts_by_folder` (`crates/db/src/db/queries_extra/scoped_queries.rs`), except Drafts. Drafts is special-cased in `build_universal_folders` (`crates/core/src/db/queries_extra/navigation.rs`) to show *total* drafts via `get_draft_count_with_local` (synced drafts plus local drafts, no `is_read` filter). With dev-seed, the Personal/Drafts pill says (6) but only 2 of those are unread.

The Promise Rule violation: pill counts across the sidebar use the same visual shape (a number with no qualifier) and silently answer different membership questions. A user reading the Inbox pill and the Drafts pill cannot tell that one is "unread" and the other is "total."

The pill and the list answer two distinct questions, and the cure is to name them as such:

- **Universal unread-pill question.** "How many synced threads in this folder are unread?" Uniform across every universal folder, including Drafts. The pill is the `is_read = 0` count over `thread_folders` membership. Drafts, Sent, Trash, Spam, and Archive will rarely show a number under this question; that is the accepted cost of a single legible rule, and the alternative (per-folder predicates with per-folder pill styling) is not pursued.
- **Drafts-list question.** "What compositional artifacts are pending my attention?" Synced draft threads plus local-only drafts (the `local_drafts` table - pre-sync compositions with no message-id, no thread, and no `is_read` column). Local drafts are not in the read/unread state space at all; they belong to a different question, not to the pill's question with a carve-out.

The discrepancy was the collision: both questions render with the same visual shape (a number in the same pill widget), and the field is named `unread_count` everywhere, which suggests they answer the same question with the same predicate. They do not.

**Cure: two distinct count types, neither assignable to the other.** Branch removal in `build_universal_folders` is necessary but not sufficient - both counts are `i64` today, so a future reroute can regress silently (the existing inline comments already documented the unread-only direction while `navigation.rs:170` kept calling `get_draft_count_with_local`; documentation alone is not enforcement). The enforcement skin is wrapper types:

```rust
// crates/db/src/db/queries_extra/scoped_queries.rs
pub struct UniversalUnreadCount(i64);   // is_read = 0 subset of thread_folders membership
pub struct DraftTotalCount(i64);        // synced drafts + local drafts

pub fn get_unread_counts_by_folder(...) -> Result<Vec<(FolderId, UniversalUnreadCount)>, String>;
pub fn get_draft_count_with_local(...) -> Result<DraftTotalCount, String>;

// crates/app/src/ui/widgets/nav.rs
pub fn nav_button<'a, M: Clone + 'a>(
    ico: Option<iced::widget::Text<'a>>,
    label: &'a str,
    active: bool,
    size: NavSize,
    badge: Option<UniversalUnreadCount>,   // not i64
    on_press: M,
) -> Element<'a, M>;
```

`get_draft_count_with_local` continues to exist for callers that legitimately want the total (pane headers, compose-pane indicators), but `DraftTotalCount` is not assignable to `UniversalUnreadCount`, so it cannot reach `nav_button`. A future contributor who removes the `if *id == "DRAFT"` branch and later re-introduces a "fix" that routes total back into the pill produces a type error, not a silent regression. The wrapper types live in `db` alongside the count queries; this is a within-crate sealing concern, not cross-crate. The `Thread::from_local_draft` constructor (`crates/app/src/db/threads.rs`) continues to stamp `is_read: true` for rendering, which is correct under the new framing - local drafts are outside the read/unread space, and `true` is the rendering-neutral default.

Tags: contracts=canonical-entry; enforcement=sealed-constructor,capability-token; promise=the universal-pill widget is fed only by the unread-count question (`is_read = 0` over `thread_folders` membership), structurally; the Drafts-list and total-count questions are typed disjointly from the pill question and cannot be confused at compile time.

### Cross-Client Folder/Label Move Reconciliation

The failure case is the *same account* observed by *two clients of the same provider*. Outlook-on-the-web moves a thread Inbox → Archive on a Microsoft 365 account; Ratatoskr (Graph delta sync) picks up the change and ends up with the thread in *both* folders, because the delta reports what the changed message is in now but not what it is no longer in. Cross-provider moves are out of scope - Ratatoskr never moves anything between providers.

`thread_folders` and `thread_labels` are written by two structurally different kinds of writer, against the same target rows. The Promise Rule violation is that both writer kinds claim to answer "what folders/labels does this thread currently live in," but only one of them can answer correctly.

**Full-coverage writers.** Gmail (`crates/provider-sync/src/gmail/sync/storage.rs:176-201`) and IMAP (`crates/provider-sync/src/imap/thread_store.rs:87-98, 121-142`) both call `replace_full_thread_folders` / `replace_full_thread_labels`, which `delete_thread_*_rows` then `insert_thread_*_rows` from the union of all messages' label IDs. The destructive replace is sound because the message set the helper sees is the complete current truth: Gmail's `threads.get` returns every message; IMAP's `thread_store` rebuilds aggregate state from all cached per-folder per-message rows (each IMAP message lives in exactly one folder, so `messages.imap_folder` is already per-message ground truth).

**Partial-coverage writers.** Graph delta (`crates/provider-sync/src/graph/sync/persistence.rs:190-233`) and JMAP delta (`crates/provider-sync/src/jmap/sync/storage.rs:187-214`) only see changed messages. They cannot destructively replace - doing so would erase rows contributed by the N-1 sibling messages they did not fetch this page. They therefore call `merge_partial_delta_*` helpers that are `INSERT OR IGNORE` only (`crates/db/src/db/queries_extra/thread_persistence.rs:632, 663`), never `delete_thread_*`. The merge can add a new folder/label row but cannot remove a stale one.

**Per-table state of the broken paths.** Graph leaks stale rows in *both* `thread_folders` (line 210) and `thread_labels` (line 211). JMAP leaks stale rows only in `thread_folders` (line 196): JMAP keyword-shaped labels flow through `recompute_thread_keyword_labels` (`crates/provider-sync/src/keyword_membership.rs:63`), which derives `thread_labels` from the `message_keywords` per-message table and *is* the cure pattern, just applied today only to keywords. Graph categories and Graph user-style labels (Importance, etc.) currently flow into `thread_labels` via the same broken merge as folders.

**Same-client moves are fine** because the action service mutates `thread_folders` locally before dispatching the provider call (`crates/db/src/db/queries_extra/email_actions.rs`: `remove_folder` does `DELETE FROM thread_folders WHERE folder_id = ?`, `insert_folder` does `INSERT OR IGNORE`). The source row is gone in the same transaction; the subsequent provider echo idempotently re-inserts the destination row. Label-side same-client moves go through `pending_thread_label_intents` (`docs/optimistic-label-intent.md`) and have their own settling story.

**Severity, post-labels-unification.** A stale `thread_labels` row on a member-bearing label now renders the whole group pill via the `thread_labels JOIN label_group_members` path (`docs/labels-unification/redesign.md` § "Message pill rendering"). Before unification, a stale row was a per-account label the message UI did not foreground; now it shapes like a deliberate "apply group" action, and users who never used a group see it attached to threads they did not touch.

**Why no type-level cure prevents the underlying bug.** Capability tokens around `merge_partial_delta_*` cannot manufacture information the partial delta does not carry. The fix has to land in the data model: partial-delta writers need a per-message scope where destructive replace *is* safe (because we know the full current state of *that one message*), and the thread aggregate becomes a recomputed view over the per-message union. The keyword-membership slice already implements this pattern in miniature.

This is also a #3 completion-state failure, not only a #4 mutation-capability failure. A `thread_folders` row set produced by additive merge from a partial delta is observationally equal to one produced by full-coverage replace: both are sets of `(account_id, thread_id, folder_id)` rows. Downstream renderers cannot distinguish a complete aggregate from a partial one carrying stale carryover. The cure makes the completion step (per-message recompute) the only path to a value that downstream code accepts; partial aggregates do not type-check where complete aggregates are required.

#### Cure alternatives, ranked by enforcement strength

The goal stated for this work is architectural protection of future selves: a contributor should not be able to write the wrong shape without the compiler stopping them. That is what ranks the options.

**A. Per-message membership tables + provider-sync-owned high-level helpers, aligned with roadmap option 4.** The architectural cure, with the enforcement skin that the option-4 layering already buys.

Schema additions in `db` (analogous to the existing `message_keywords`, `crates/db/src/db/schema/02_mail.sql:223`):

- `message_folders (account_id, message_id, folder_id, PRIMARY KEY (account_id, message_id, folder_id))` for Graph and JMAP folder membership. IMAP needs no new table because `messages.imap_folder` already carries the per-message ground truth (each IMAP message instance lives in exactly one folder).
- `message_labels (account_id, message_id, label_id, PRIMARY KEY (account_id, message_id, label_id))` for all non-keyword label-shaped membership: Graph categories (`cat:*`), Graph importance (`importance:*`), and any future user-label shapes. One table, kind discriminated by the `label_id` prefix as parsed by `LabelKind`. The doc previously sketched separate `message_categories` and `message_labels` tables; categories do not need a separate table because their kind is already encoded in the prefix of a validated `label_id`. `message_keywords` stays separate because it carries the raw provider keyword text for round-trip preservation; future unification with `message_labels` is a follow-on decision, not blocking.

Raw row primitives in `db` (option 4: boring, batch-shaped, no delta-awareness):

```rust
// crates/db/src/db/queries_extra/message_membership.rs (new)
pub fn replace_message_folder_rows(tx: &Transaction, account_id: &str, message_id: &str,
                                   folders: &[&FolderKind]) -> Result<(), String>;
pub fn replace_message_label_rows(tx: &Transaction, account_id: &str, message_id: &str,
                                  labels: &[&LabelKind]) -> Result<(), String>;
pub fn delete_message_membership_rows(tx: &Transaction, account_id: &str, message_id: &str)
    -> Result<(), String>;

// crates/db/src/db/queries_extra/thread_persistence.rs (existing, with additions)
pub fn recompute_thread_folders_from_messages(tx: &Transaction, account_id: &str, thread_id: &str)
    -> Result<(), String>;
pub fn recompute_thread_labels_from_messages(tx: &Transaction, account_id: &str, thread_id: &str)
    -> Result<(), String>;
```

`replace_message_*_rows` is the scoped operation: `DELETE WHERE account_id = ? AND message_id = ?` then `INSERT OR IGNORE` the current set, safe because the partial delta knows the full current state of *that one message*. `recompute_thread_*_from_messages` is destructive at thread scope but safe because the source it reads from (`message_folders` ∪ `messages.imap_folder` / `message_labels` ∪ `message_keywords`) is the complete per-message ground truth.

High-level helpers in `provider-sync` (option 4: provider-semantic orchestration where the provider knowledge lives). One transaction-scoped helper per coverage shape - each helper owns its full operation, so there is no "replace per-message rows" step separable from "recompute aggregate":

```rust
// crates/provider-sync/src/thread_membership.rs

/// Full-coverage replace. Used by Gmail full-thread sync and IMAP full-thread
/// rebuild. The caller is asserting it has every message of the thread.
pub fn replace_thread_membership_from_full_coverage(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    folders: &HashSet<&FolderKind>,
    labels: &HashSet<&LabelKind>,
) -> Result<(), String>;

/// Per-message replace + aggregate recompute, atomic in one helper. Used by
/// Graph delta and JMAP delta. The caller knows the full current state of
/// `message_id` only; the helper owns the recompute that derives thread truth
/// from the per-message union, so the recompute cannot be forgotten.
pub fn replace_message_membership_and_recompute(
    tx: &Transaction,
    account_id: &str,
    thread_id: &str,
    message_id: &str,
    folders: &HashSet<&FolderKind>,
    labels: &HashSet<&LabelKind>,
) -> Result<(), String>;
```

Neither helper has a constructor / writer split. There is no `ThreadMembershipUpdate` payload object to construct then pass somewhere. Each helper does the full transaction-scoped work and returns. A future contributor cannot land per-message rows and then forget the recompute, because the only entry point is the helper that runs both.

A new provider integration calls one of these two helpers. The raw `db` primitives remain `pub` per option 4 - cross-crate enforcement is partial, not absolute - but the typed inputs (`FolderKind`, `LabelKind` after #5c lands) and the two named entry points make the right path obvious and the wrong shape (calling `delete_thread_folder_rows` then `insert_thread_folder_rows` ad-hoc from a delta site) visibly off-pattern in code review. This is the fidelity ceiling the roadmap accepts for #4.

**Deletion and rethreading hooks.** The per-message tables must stay coherent through message deletes, message reassignment (JWZ rethreading reassigns messages to a different thread), and tombstones - the same hook points the existing `message_keywords` recompute already wires. `db::queries_extra::delete_messages_and_cleanup_threads` and the JWZ rethread paths must call `delete_message_membership_rows` for the affected message-ids and then `recompute_thread_*_from_messages` for every affected `thread_id` (both the old thread the message left and the new thread it joined, if applicable). Same shape as the existing keyword path; the cure does not introduce new hook points, only new rows that share them.

**Cost.** Schema migration; write-path changes in Graph (`set_thread_labels` becomes one call to `replace_message_membership_and_recompute` per changed message), JMAP (same shape; the existing `recompute_thread_keyword_labels` call survives and runs alongside the new `recompute_thread_labels_from_messages`, or the two recomputes unify in a follow-on); Gmail and IMAP switch their existing replace helpers to call `replace_thread_membership_from_full_coverage`. Deletion and rethreading paths gain the per-message-membership cleanup call.

**Action-service writes are a separate follow-on slice.** The action service does not have full per-thread coverage at action time - a "move to Archive" mutation is a single-folder delta, not a complete folder-set rewrite - so it fits neither helper. The plausible answer mirrors `pending_thread_label_intents` (`docs/optimistic-label-intent.md`): a `pending_thread_folder_intents` table that the action service writes, with the read path overlaying intent on provider truth, and `threads.folder_membership_generation` bumped on confirmed provider truth so satisfied intents can be cleared. This is plausible enough to name but not designed enough to implement. Open questions for the design slice:

- Generation counter shape (one per thread, or per (thread, folder)?). The label-intent slice uses one `label_membership_generation` per thread.
- Add/Remove merge algebra when the user issues two moves before the first echo lands (e.g. Inbox → Archive, then Archive → Trash).
- Clear-on-provider-truth: which provider event clears a pending intent, given that Graph/JMAP partial deltas may arrive for unrelated messages of the same thread.
- Overlay-aware read shape: every list/count query that joins `thread_folders` needs to know to consult the intent table. The label-intent slice landed this via `user_visible_label_exists_fragment`; the folder version needs the analogous fragment and every reader audited.
- Whether `email_actions::insert_folder` / `remove_folder` remove direct `thread_folders` writes entirely or keep them as a faster-than-overlay first pass.

**B. Capability-token gate alone, no per-message tables.** A `FullThreadCoverage` zero-size witness lives in `provider-sync::thread_membership`, produced only by helpers called from Gmail/IMAP full-thread paths. A single high-level entry point in `provider-sync` (`replace_thread_membership_from_full_coverage`) requires the witness; delta paths cannot synthesize one.

Enforces: a future contributor cannot add a destructive thread-scope replace to a partial-delta path - the entry point that does the destructive replace requires a witness the delta path cannot construct.

Does not fix: Graph/JMAP delta still leaks stale rows. The witness only blocks *making it worse*. It is an enforcement-only option, suitable as a stopgap that locks the current state and prevents regression, not as a complete fix.

**C. Periodic full-thread reconciler.** A background job picks threads with stale `last_full_resync_at`, fetches full provider truth (Graph's `messages` for a conversation, JMAP's `Email/get` over thread membership), and applies the equivalent of `replace_full_thread_*`.

Enforces: nothing. The wrong write paths remain reachable and the next contributor can write more of them. Reduces user-visible drift latency to "reconciler interval" but does not close it.

Suitable as a *transitional* measure under (A): turn it on while the per-message tables are being rolled out, then turn it off once the per-message paths cover all four providers. Not suitable as the end state.

**D. (A) + the option-4 layering - recommended.** The data-model fix from A makes the partial-delta paths correct. The option-4 layering (provider-semantic high-level helpers in `provider-sync`, raw row primitives in `db`) is the same enforcement skin (B) would have delivered standalone: the only sanctioned destructive thread-scope replace is `replace_thread_membership_from_full_coverage`, named for its precondition; the only partial-coverage entry is `replace_message_membership_and_recompute`, which owns both halves of the per-message → aggregate write atomically. The reconciler from C optionally runs during migration. This is the only option that delivers both correctness and architectural enforcement at the fidelity ceiling option 4 accepts.

#### What enforcement specifically prevents

Wrong-by-construction outcomes that the recommended cure eliminates or makes visibly off-pattern:

- A per-message write path forgets to call the recompute - structurally impossible, since the only sanctioned entry point (`replace_message_membership_and_recompute`) does both halves atomically. There is no separable "write rows" step.
- A delta path is "optimized" by switching to destructive thread-scope replace - the only thread-scope-replace entry point (`replace_thread_membership_from_full_coverage`) is named for the precondition. A delta-site caller invoking it from a partial page is visibly wrong in code review and contradicts the type name; under #5c (typed `FolderKind` / `LabelKind`) the inputs would also have to be assembled from non-existent full-thread truth.
- A future Gmail refactor moves Gmail to delta-first and silently keeps using `replace_thread_membership_from_full_coverage` - same precondition check; the call site that hands the helper an incomplete folder set is the same line that documents itself as wrong.
- A new provider integration composes `delete_thread_folder_rows` + `insert_thread_folder_rows` ad-hoc from a delta path - technically reachable per option 4 (the raw helpers stay `pub` in `db`), but visibly off-pattern: every other provider's delta path calls one of the two `provider-sync::thread_membership` helpers. A reviewer who sees ad-hoc raw-row composition in a delta site knows to push back.
- Per-message rows go stale through message deletion or rethreading - the deletion/rethreading hooks call `delete_message_membership_rows` and the per-thread recompute, mirroring the existing `message_keywords` path. The hook points are shared, so forgetting one for the new tables means also forgetting it for the existing keyword table, which the suite already catches.

Cross-crate enforcement under option 4 is partial, not absolute. The roadmap accepts this trade as the high-fidelity option for #4 because the raw `db` primitives are small, batch-shaped, and have no delta semantics - the wrong shape is identifiable by what a delta-site caller is *composing*, not by what they're *allowed to call*. The two `provider-sync::thread_membership` entry points carry the semantic information that raw row helpers do not.

Tags: contracts=mutation-capability,completion-state; enforcement=sealed-constructor,capability-token; promise=thread folder/label membership reflects current provider truth across all sync paths; partial-coverage write paths can only land per-message rows + recompute aggregate, structurally, and the destructive thread-scope replace is named for the full-coverage precondition.

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
