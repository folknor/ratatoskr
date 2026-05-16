# Contracts Roadmap

Implementation arc for landing the compile-time enforcement named in `docs/glossary/discrepancies.md`. That doc is the *what is broken* doc - the contract failures, the evidence, the tags. This doc is the *what we do about it* doc: which contract to land first, why, what the design surface looks like, what success looks like, and what fidelity each contract reaches in Rust.

This is not a product roadmap. It does not name dates. It names sequencing, the type-design sketch for each contract, the migration scope, and the open design questions that need to be answered before a contract's type lands. The migrations themselves will get their own PR-shaped tracking when the design questions are answered.

## Reading list

Before touching any of this, read in order:

1. `docs/architecture.md` - the guiding principle ("make the right thing the only thing"), crate boundaries, the existing settled patterns. The composite-operations section is directly relevant to contract #4.
2. `docs/glossary/folders-labels.md` - the binding rules for thread aggregates, folder/label storage split, prefix conventions. Several contracts encode these rules into types. Note in particular the **per-field reducer rule** - `is_read` is MIN (all-read), `is_starred` / `is_replied` / `is_forwarded` are ANY, `last_message_at` is MAX. This is not uniform.
3. `docs/glossary/discrepancies.md` - the contract-failure taxonomy and tagged inventory. Every migration below references entries there.

## Fidelity

Not every enforcement technique reaches the same ceiling in Rust. Each contract migration below carries a `Fidelity:` annotation. Naming this honestly matters: it determines whether the contract is *structurally enforced* or whether the contract is *named and reviewer-disciplined*.

### High fidelity - compile-time enforcement is total within the technique's scope

- **Boundary parse** (#5): the parser is the only constructor from raw external values. The total domain type - `LabelKind`, `MailProviderKind`, `StoredSecret` - accepts no raw construction. Payload-carrying enum variants are constructors *by inclusion*, so the validation has to live one layer down: variant payloads are themselves private-fielded validated newtypes (`KeywordName`, `CategoryName`, `GraphGuid`, `ImapPath`, etc.) with their own boundary parsers. Total within the parsing crate.
- **Sealed constructor within-crate** (#1 grain.vertical, #3, #5b, #2): `pub` constructors with **private struct fields** plus accessor methods, taking typed inputs that prove preconditions. The constructor can be called from any crate that depends on the owning crate, but the struct value cannot be forged by an external `Struct { ... }` literal because the fields are private. Total fidelity is preserved as long as the typed input chain leads back to a private-fielded constructor in the same crate. (Public fields would defeat this - `pub` fields let external crates construct the struct directly and bypass the constructor entirely.)
- **Exhaustive dispatch** (#1 grain.scope, #5c match arms on enums): a `match` on an enum with no catch-all is compile-time enforced. Adding a variant is a compile error in every match.

### Cross-crate capability - the menu, and the standing answer

For capability tokens that cross a crate boundary, four options exist. Rust has no friend-crate mechanism, so the "constructor must be sealed against external misuse" property cannot be reached without ownership restructuring.

1. **Public constructor with private fields.** The struct's fields are private; only the constructor can produce values. The constructor itself is public - any crate that depends on the owning crate can call it. Fidelity: medium-low. The shape of the value is sealed, but the *act* of constructing one is by convention.

2. **Facade in a downstream crate.** The construction logic lives in a designated facade that takes typed evidence and produces the capability-token value via option 1's constructor. Fidelity: still medium-low. The facade raises the sanctioned-path visibility but does not seal construction.

3. **Sealed trait + reviewer discipline.** The owning crate exports a sealed trait whose impls live only in designated downstream crates. Fidelity: medium. Defends against accidental violation; doesn't defend against deliberate workaround.

4. **Restructure ownership.** The capability-token type *and* the high-level write helper that consumes it both live in a downstream crate. Fidelity: high. The type and its sole sanctioned constructor live in the same crate; standard within-crate sealing applies.

Options 2 and 3 raise the floor but do not reach the same compile-time guarantee as options 1 or 4. The honest binary is option 1 (medium) or option 4 (high).

**Standing answer for this project:** option 4 for capability contracts that are load-bearing for correctness - specifically contract #4 (mutation capability, where merge-vs-replace is data-loss-shaped). Contracts that initially looked cross-crate but on inspection are within-crate concerns (notably #2 canonical-entry - both the Drafts and search unifications are internal to their owning crates) achieve high fidelity through ordinary within-crate sealing and are not subject to this menu.

### Option 4 is a controlled split, not a weakening

The architecture-doc rule "shared-table SQL belongs to `db`" remains intact. What option 4 does is *clarify* the layering:

- **`db` keeps schema, migrations, and the raw row-level SQL primitives.** Operations like `db::raw::delete_thread_label_rows(tx, key)` and `db::raw::insert_thread_label_rows(tx, key, labels)` - boring, batch-shaped, no delta-awareness - stay in `db`. The SQL strings still live in `db`. The schema migration story is unchanged.
- **`provider-sync` owns the delta-semantic orchestration.** Whether a particular thread write is a full-snapshot replace or a partial-delta merge is *provider knowledge* - only the provider's sync code knows which delta semantics it's operating under. The capability-gated helpers `replace_thread_labels(input: ReplaceInput<…>, …)` and `merge_thread_labels(input: MergeInput<…>, …)` live where that knowledge lives, calling `db::raw` underneath.
- **Validation that mixes raw IDs with semantic decisions moves with the orchestration.** `filtered_membership_ids` - which today drops message-state label IDs and reserved IMAP system keywords before writing - is provider-semantic, not raw. It moves to `provider-sync`. Under #5c (typed `LabelKind`), most of what that filter does becomes structurally unrepresentable anyway; the filter's role during transition is defensive cleanup of legacy string IDs.

The rule isn't weakened. The clarification is: *provider-delta-aware orchestration of shared-table writes lives where the provider knowledge is*, and that is one layer up from `db`.

### Decided open questions

The following questions were open in the previous version of this doc; they are now resolved:

- **Cross-crate capability option for #4:** option 4. Verified cheap - the four helper bodies in `db::queries_extra::thread_persistence` are 15-25 lines each, comprised of DELETE-then-INSERT-OR-IGNORE loops; `db::raw` is a clean row-operation layer with no leaky internals.
- **Cross-crate capability option for #2:** not applicable - neither sub-case is cross-crate. Drafts orchestration is internal to `db`; search unification is internal to `core/search_pipeline`.
- **Staged-migration shape for option 4:** single-landing per atomic move, per the migration ground rules. The `sync::pipeline::store_threads` persistence half moves up to `provider-sync` because both of its current callers (`provider-sync/imap/imap_initial.rs`, `provider-sync/imap/imap_delta.rs`) live there. Threading (JWZ algorithm + `MessageMeta` types) stays in `sync`.

### Remaining open questions

- **Inclusive vs exclusive `DateBound`?** Resolved: `DateBound` emits exclusive bounds for both SQL and Tantivy.
- **JMAP non-keyword labels - possible or not?** If keyword-only by construction, Shape 10 is fully resolved by an IMAP-style recompute pattern applied to JMAP. If non-keyword JMAP labels can flow, this is a data-loss bug to fix during the #4 migration. Verify before designing.
- **Legacy plaintext credentials - still load-bearing?** If yes, `StoredSecret::parse` stays tolerant of both formats forever. If no, the parser becomes strict and legacy support moves to a one-shot re-encrypt migration.
- **`#5c` on-disk format:** boundary adapter at the DB read/write boundary (recommended) vs DB restructure (cleaner, larger).
- **`#5c` IPC wire format:** serde `#[serde(tag, content)]` on `FolderKind` / `LabelKind` (cleaner) vs `String` on the wire with parse-at-IPC-boundary (smaller migration).

## Sequencing

Sequence chosen by *leverage per migration* and *fidelity tier*. Land high-fidelity migrations first.

0. **Glossary fix** (`folders-labels.md` per-field reducer naming). Doc-only, blocks every contract that encodes the rule. Already landed.
1. **#5a Credentials** - smallest boundary-parse, debugs migration mechanics.
2. **#5-pre MailProviderKind** - prereq for #5c, second boundary-parse migration, still small. Lands in `types`.
3. **#1 grain.vertical** - high-fidelity sealed-constructor in `db`, highest leverage in the inventory.
4. **#3 Completion State** - same technique, contained in `search` (partial type) and `core/search_pipeline` (enriched type + result-set enum).
5. **#5b LabelStyle** - rides with #3's pattern (partial-to-complete transition). High fidelity, contained in `label-colors` and `app`.
6. **#4 Mutation Capability** - option 4 from §Fidelity. Raw row primitives stay in `db::raw`; capability-gated helpers and the `store_threads` persistence half move to `provider-sync`.
7. **#2 Canonical Answer** - high-fidelity within owning crates (`db` for Drafts, `core/search_pipeline` for search).
8. **#1 grain.scope** - refactor, no new types, easy after the vocabulary is settled.
9. **#5c FolderKind + LabelKind** - the broad migration. `MailLocator` as parse-product only; operation APIs stay narrow.

## Per-Contract Design

### 0. Glossary fix (prereq)

**Status:** landed.

`docs/glossary/folders-labels.md` previously stated that all four per-message booleans aggregate via `MAX()`. This was wrong: `is_read` is the only MIN (all-read); `is_starred`, `is_replied`, `is_forwarded` are ANY; `last_message_at` is `MAX(date)`. The doc now names each reducer per-field. Any sealed constructor that encodes the aggregate must use the corrected reducers, not the uniform MAX rule that the doc bug implied.

### 1. #5a Credentials - boundary parse (high fidelity)

**Status:** code path landed for current credential readers. `decrypt_or_raw` and `decrypt_if_needed` are removed; Gmail, Graph, JMAP, and IMAP consume `StoredSecret`. The external-construction check is pinned by the `StoredSecret` rustdoc `compile_fail` example.

**Inventory:** `crates/gmail/src/client.rs:122`, `crates/graph/src/client.rs:122`, `crates/common/src/crypto.rs:123-132`, `crates/common/src/crypto.rs:137-147`.

**Design sketch.** `StoredSecret::parse(raw: String) -> StoredSecret` handles both encrypted format (`base64:base64`) and legacy plaintext at the parse boundary. Parsing is an infallible classification step; decryption is the fallible operation. Returns a single typed value that downstream code consumes. Readers see only the parsed type.

```rust
pub struct StoredSecret(/* private: bytes + format discriminator */);

impl StoredSecret {
    pub fn parse(raw: String) -> StoredSecret {
        // tolerant: accepts encrypted (base64:base64) OR legacy plaintext;
        // returns typed value with format discriminator stored internally.
    }

    pub fn decrypt(&self, key: &[u8; 32]) -> Result<String, DecryptError> {
        // typed reader; per-format dispatch happens internally; callers cannot fall through to raw.
    }

    pub fn decrypt_optional(raw: Option<String>, key: &[u8; 32])
        -> Result<Option<String>, DecryptError>
    {
        // Optionality stays orthogonal to the storage format.
    }
}
```

The name is **`StoredSecret`**, not `EncryptedToken` - the latter would be misleading while the legacy plaintext path is still load-bearing. Once the plaintext path is eliminated (via a one-shot re-encrypt migration), the type could be renamed.

The `Option<String>` case (JMAP/IMAP credentials) becomes `Option<StoredSecret>`. The two-variant decryption API (`decrypt_or_raw` vs `decrypt_if_needed`) collapses to one.

**Fidelity:** high. Boundary parse within `common`. The parser is the only public constructor; readers consume the typed value. Cross-crate consumers (`gmail`, `graph`, `jmap`, `imap`) receive the parsed type via `common::crypto`. Private fields prevent external construction.

**Migration scope.** `crates/common/src/crypto.rs`; the four provider client modules that call `decrypt_or_raw` / `decrypt_if_needed`. ~10 call sites total.

**Open question.** Is the legacy plaintext path still load-bearing in 2026, or can it be migrated to "rejection + one-time re-encrypt" cleanly? Worth checking before deciding whether the parser accepts both formats forever or only during a migration window. If the answer is "rejection," the parse function becomes strict and the legacy support moves to a one-shot migration script - and the type can be renamed to `EncryptedSecret` to reflect the narrowed invariant.

**Success criteria.** `decrypt_or_raw` and `decrypt_if_needed` are gone. The four provider clients consume `StoredSecret` directly. A compile-fail test attempts to pass a raw `String` to a function expecting `StoredSecret` and fails.

### 2. #5-pre MailProviderKind - boundary parse (high fidelity)

**Status:** in progress. `types::MailProviderKind` exists with boundary parsing and serde-as-canonical-string. The central service provider dispatch parses normal account `provider` rows into the enum before matching, while the harness-only providers still use an explicit raw lookup before that boundary. The generic account-provider lookup returns `MailProviderKind`, and cloud-upload support is now keyed by `MailProviderKind`. The full workspace migration is still open.

**Inventory:** No direct entries (this is prereq infrastructure for #5c). Implicitly addressed by every Shape 6 entry where a provider-identity string flows alongside a label string.

**Design sketch.** A typed `MailProviderKind` enum lives in `types`:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MailProviderKind {
    Gmail,
    Graph,
    Jmap,
    Imap,
}

impl MailProviderKind {
    pub fn parse(raw: &str) -> Result<MailProviderKind, ParseError> { ... }
    pub fn as_str(&self) -> &'static str { ... }
}
```

Every existing `provider: &str` or `provider_name: String` parameter **in the mail-provider sense** becomes `provider: MailProviderKind`. Account rows in the DB serialize the kind via the `as_str` form. Wire types (IPC, log fields) round-trip through serde.

**The name is `MailProviderKind`, not `ProviderKind`.** "Provider" is overloaded across the codebase: OAuth identity providers (Google/Microsoft as IDP), the autodiscovery registry, calendar providers, cloud-attachment providers, and mail providers all exist. The bare name `ProviderKind` would invite confusion at every site where a different provider axis is meant. Adjacent provider axes get their own kind enums if a typed analog is warranted - they are not part of this migration's scope.

**Fidelity:** high. The parser is the only constructor; consumers receive the enum.

**Migration scope.** `crates/types/src/lib.rs` (new type); call sites in `core`, `service`, `provider-sync`, every mail-provider crate, the action service, dev-seed. Touches many files but each touch is mechanical.

**Open question.** Should the type live in `types` or `common`? `types` is the lighter-weight crate (per AGENTS.md, minimal deps, serde-only). `common` already depends on `types`. The argument for `types` is that `service-api` can depend on `types` without pulling in `common`. Recommend `types`.

**Success criteria.** No function in the workspace accepts `provider: &str` for the *mail-provider-identity sense*. Adjacent provider axes (OAuth, discovery, calendar, cloud attachments) are explicitly out of scope. A compile-fail test attempts to pass `"gmail"` as a string where `MailProviderKind` is expected and fails.

### 3. #1 grain.vertical - sealed constructor (high fidelity within crate)

**Status:** DateBound and thread-aggregate slices landed. `types::DateBound` is a sealed date-boundary parse product with exclusive SQL and Tantivy range emitters. Smart-folder parsed queries carry `DateBound`, SQL date clauses use `DateBound::to_sql_clause`, and Tantivy range queries use `DateBound::to_range_bound`. `db::queries_extra::ThreadAggregate` now has private fields, accessor methods, and a non-empty `compute_from_messages(first, rest)` in-memory constructor used by `sync::pipeline` and `dev-seed`. `NonReactionMessage` has private fields and a constructor boundary. Thread decoration and read/starred recompute paths now exclude reactions. Predicate-grain slices remain open.

**Inventory:** Shape 1 entries (chat.rs, thread_detail.rs, smart-folder), Shape 3 entries (thread_persistence.rs, sync/pipeline.rs, dev-seed), Shape 9 entries (search-pipeline grouping/metadata), Shape 11 (date boundary), parts of Shape 12.

**Design sketch.** One sealed `ThreadAggregate` struct with private fields, accessor methods, and per-field reducers enforced inside its constructors:

```rust
// crates/db/src/db/queries_extra/thread_persistence.rs

pub struct ThreadAggregate {
    // All fields are private. External crates cannot forge a literal.
    is_read: bool,
    is_starred: bool,
    last_date: i64,
    message_count: i64,
    has_attachments: bool,
    subject: Option<String>,
    snippet: String,
}

impl ThreadAggregate {
    /// SQL-owning constructor. Applies `is_reaction = 0` inline and uses
    /// the canonical per-field reducer (MIN for is_read, ANY for the others).
    pub fn compute_thread_aggregate(tx: &Transaction, account_id: &str, thread_id: &str)
        -> Result<ThreadAggregate, String> { ... }

    /// In-memory constructor for sync/pipeline and dev-seed.
    /// Takes a typed input newtype that proves the is_reaction = 0 filter.
    pub fn compute_from_messages(first: &NonReactionMessage, rest: &[NonReactionMessage])
        -> ThreadAggregate { ... }

    // Accessors. Fields stay private.
    pub fn is_read(&self) -> bool { self.is_read }
    pub fn is_starred(&self) -> bool { self.is_starred }
    pub fn last_date(&self) -> i64 { self.last_date }
    // ...etc
}

/// Typed proof that the `is_reaction = 0` filter has been applied.
pub struct NonReactionMessage { /* private fields */ }
```

Both constructors are `pub` so `sync::pipeline` and `dev-seed` can call them from outside `db`. Sealing comes from **private struct fields plus the typed input newtype**, not from `pub(crate)` visibility.

Per-field aggregate types (`ThreadReadAggregate`, etc.) are **not** introduced. The contract is *single place where the reducer rules live*, not *one type per rule*.

**No intermediate filtered-rowset type.** The two constructors each own their filter: `compute_in_tx` owns the SQL with `WHERE is_reaction = 0` inline; `compute_from_messages` takes a non-empty `NonReactionMessage` input set (the newtype is the proof).

For the query-builder side (the smart-folder motivating example), grain-branded predicates emit SQL against the right alias. A `ThreadPredicate` emits SQL against the `threads` table alias; a `MessagePredicate` emits SQL against the matched-messages subquery. The clause-list builders accept one or the other, not both.

For dates (Shape 11), `DateBound` lives in `types` with private fields and emitter methods. The Tantivy term type doesn't leak into `types`; the emitter is generic over the consumer's term type:

```rust
// crates/types/src/date_bound.rs

pub struct DateBound {
    timestamp: i64,
    direction: BoundDirection,
}

enum BoundDirection { Before, After }

impl DateBound {
    pub fn before(timestamp: i64) -> DateBound { ... }
    pub fn after(timestamp: i64) -> DateBound { ... }

    /// SQL emitter - generates a clause with inclusivity decided once.
    pub fn to_sql_clause(&self, column: &str, param_idx: usize) -> (String, i64) { ... }

    /// Generic range emitter - caller passes a closure that builds the consumer's
    /// term type (Tantivy's `Term`, or any other range key). The inclusivity
    /// choice is applied here; Tantivy doesn't need to leak into `types`.
    pub fn to_range_bound<T>(&self, make_term: impl FnOnce(i64) -> T)
        -> std::ops::Bound<T>
    {
        match self.direction { /* Included/Excluded per chosen semantics */ }
    }
}
```

**Fidelity:** high within `db`. The `ThreadAggregate` constructors are `pub`; the struct fields are private; the typed inputs are sealed. The grain-branded predicate types in `smart-folder` are sealed within `smart-folder`. `DateBound` in `types` is sealed against direct construction.

**Migration scope.**

- `crates/db/src/db/queries_extra/thread_persistence.rs` - `ThreadAggregate`, `NonReactionMessage`, `compute_in_tx`, `compute_from_messages`.
- `crates/db/src/db/queries_extra/thread_detail.rs` - `query_thread_state_decorations` consumes the typed aggregate.
- `crates/db/src/db/queries_extra/chat.rs` - chat unread query and recompute share the typed predicate.
- `crates/smart-folder/src/sql_builder.rs` - `ThreadPredicate` / `MessagePredicate`; date predicates consume `DateBound::to_sql_clause`.
- `crates/sync/src/pipeline.rs` - in-memory aggregate switches to `compute_from_messages`.
- `crates/dev-seed/src/threads.rs` - thread aggregate derives from the seeded message vec via `compute_from_messages`.
- `crates/types/src/date_bound.rs` - new module.
- `crates/search/src/lib.rs` - Tantivy `before:` / `after:` boundary uses `DateBound::to_range_bound` with a Tantivy-term closure.

**Success criteria.** All Shape 1 and Shape 3 (`grain.vertical`) inventory entries either get a `// resolved by contract #1 grain.vertical` annotation and disappear, or get reclassified as evidence for a different contract. A compile-fail test attempts to construct a `ThreadAggregate` via struct literal from outside `db` and fails. Another attempts to construct a `NonReactionMessage` outside `db` and fails. A third attempts to construct a `DateBound` via struct literal and fails.

### 4. #3 Completion State - sealed constructor (high fidelity within crate)

**Status:** search enrichment and app-thread constructor slices landed. Tantivy-only search now fetches thread metadata from SQL and applies `enrich_from_sql` before returning, dropping stale index hits that no longer have a thread row. This removes the `is_read: false` / `is_starred: false` placeholder leak for full-index free-text search. App `Thread` conversion defaults now live on associated constructors for DB threads, local drafts, public folder items, and search results. The broader partial/enriched type split remains open.

**Inventory:** Shape 2 entries (Thread converters, `MatchKind::Body` hardcoded, `is_read: false` hardcoded in Tantivy), Shape 12 (partial enrichment).

**Design sketch.** Two-type completion pairs with a single transition function. Owning crates named explicitly:

```rust
// crates/search/src/lib.rs - PartialSearchHit is owned by `search`
// because that's the crate that builds it from Tantivy results.

pub struct PartialSearchHit {
    // private fields; constructible only via `collect_results` and friends within `search`.
    score: f32,
    message_id: MessageId,
    match_kind: MatchKind,
    also_matched: Vec<MatchKind>,
    // ...partial metadata from Tantivy's stored fields
}

// crates/core/src/search_pipeline.rs - EnrichedSearchHit and SearchResults
// live in `core` because that's where the enrichment transition runs
// (`core` depends on `search`, so it can consume PartialSearchHit).

pub struct EnrichedSearchHit {
    // private fields. Constructible only via `from_partial`.
    score: f32,
    thread_id: ThreadId,
    is_read: bool,
    is_starred: bool,
    subject: Option<String>,
    // ...complete metadata
    match_kind: MatchKind,
    also_matched: Vec<MatchKind>,
}

impl EnrichedSearchHit {
    /// The only constructor. Combines a partial Tantivy hit with SQL thread data.
    pub fn from_partial(partial: search::PartialSearchHit, sql: &ThreadSqlRow)
        -> EnrichedSearchHit { ... }
}

pub enum SearchResults {
    FullIndex(Vec<EnrichedSearchHit>),
    Degraded(Vec<EnrichedSearchHit>),
}
```

`PartialSearchHit` lives where it's first constructed (`search`); `EnrichedSearchHit` lives where the enrichment runs (`core/search_pipeline`); `core` depends on `search`. The renderer in `app` imports `EnrichedSearchHit` only.

**Quality lives at the result-set level, not per-hit.** `SearchResults::{FullIndex, Degraded}` forces the renderer to `match` once at the view boundary and draw the degraded-mode banner at the appropriate level.

Same shape for the `Thread` constructors: four near-identical converters (`db_thread_to_app_thread`, `local_draft_to_app_thread`, `unified_result_to_thread`, the public-folder inline converter) collapse to `PartialThread` → `DecoratedThread` with a single `decorate(...)` transition.

`LabelStyle` is its own #5b migration - see that section for the crate-boundary split.

**Fidelity:** high. Private fields on both partial and enriched types; the transition function is the sole constructor for the enriched type.

**Migration scope.**

- `crates/search/src/lib.rs` - `PartialSearchHit` defined here; `collect_results` returns it. `MatchKind::Body` is no longer a default.
- `crates/core/src/search_pipeline.rs` - `EnrichedSearchHit` and `SearchResults` defined here; `from_partial` is the sole transition.
- `crates/app/src/helpers.rs`, `crates/app/src/db/pinned_searches.rs`, `crates/app/src/handlers/search.rs` - four Thread converters → associated constructors on `app::db::types::Thread`.

**Success criteria.** Renderer signatures accept only enriched types. The search view exhaustively matches `SearchResults::{FullIndex, Degraded}`. A compile-fail test attempts to pass `PartialSearchHit` to the result-row renderer and fails. The four Thread converters collapse to one constructor.

### 5. #5b LabelStyle - sealed constructor + crate-boundary split (high fidelity)

**Status:** landed for the documented surfaces. `label-colors::LabelStyleHex` is a complete `(bg, fg)` pair, `resolve_label_color` accepts only complete pairs, label write APIs reject partial DB pairs, and the labels schema has matching complete-or-missing CHECK constraints. The app UI now has `LabelPaint` with private fields; reading-pane label pills, thread-list label markers, sidebar label rows, and Settings label rows construct it from `LabelStyleHex` and pass `LabelPaint` to label-shaped widgets.

**Inventory:** Shape 5's `resolve_label_color` partial-pair entry, Shape 2's label-color resolver entry.

Sequenced adjacent to #3 because LabelStyle is a completion-state migration (partial-or-missing color → complete `(bg, fg)` pair). Rides on the same sealed-constructor pattern; contained within `label-colors` and `app` with no cross-crate construction concern.

**Design sketch.** Two types, one per crate-boundary, both with private fields:

```rust
// crates/label-colors/src/lib.rs - low-level, no iced dependency
pub struct LabelStyleHex {
    bg: HexColor,
    fg: HexColor,
}

impl LabelStyleHex {
    pub fn resolve(row: &LabelRow, palette: &PaletteFallback) -> LabelStyleHex { ... }
    pub fn bg(&self) -> HexColor { self.bg }
    pub fn fg(&self) -> HexColor { self.fg }
}

// crates/app/src/... - UI layer
pub struct LabelPaint {
    bg: iced::Color,
    fg: iced::Color,
}

impl LabelPaint {
    pub fn from_hex(hex: LabelStyleHex) -> LabelPaint { ... }
    pub fn bg(&self) -> iced::Color { self.bg }
    pub fn fg(&self) -> iced::Color { self.fg }
}
```

Partial values (`Some(bg), None`) cannot be constructed at either level. The resolver returns a complete pair or falls back to the palette.

**Fidelity:** high within each crate.

**Migration scope.** `crates/label-colors/src/lib.rs`, widget call sites (`reading_pane`, `thread_list`, sidebar).

**Success criteria.** Every widget that draws a label-shaped surface accepts `LabelPaint` and nothing else. A compile-fail test attempts to pass raw hex strings to the renderer and fails.

### 6. #4 Mutation Capability - capability token (high fidelity, option 4)

**Status:** composite no-enqueue slice landed. Label-group member dispatch now calls explicit `add_label_with_provider_no_enqueue` / `remove_label_with_provider_no_enqueue` helpers, so composite retries no longer depend on mutating `ActionContext::suppress_pending_enqueue` in `dispatch_member_ops`. The pending-op retry worker still uses `suppress_pending_enqueue` for normal retry-loop suppression; the broader merge-vs-replace capability migration remains open.

**Inventory:** Shape 4 entries (merge vs replace helpers, JMAP keyword path), Shape 7 (composite suppress flag), Shape 10 (partial-delta keyword loss as a #4 instance).

**Design sketch.** Option 4 from §Fidelity. The layering:

- **`db` keeps raw row primitives.** A small `db::raw` module exposes batch-shaped operations with no delta-awareness:
  ```rust
  // crates/db/src/db/raw/thread_membership.rs (new)
  pub fn delete_thread_label_rows(tx: &Transaction, key: ThreadKey) -> Result<(), String> { ... }
  pub fn insert_thread_label_rows(tx: &Transaction, key: ThreadKey, labels: &[&LabelKind]) -> Result<(), String> { ... }
  pub fn delete_thread_folder_rows(tx: &Transaction, key: ThreadKey) -> Result<(), String> { ... }
  pub fn insert_thread_folder_rows(tx: &Transaction, key: ThreadKey, folders: &[&FolderKind]) -> Result<(), String> { ... }
  ```
  These are intentionally boring. They know table names and column names; they do not know "is this a full snapshot or a delta page."

- **`provider-sync` owns the typed inputs and the high-level helpers.**
  ```rust
  // crates/provider-sync/src/thread_writes.rs (new)

  pub struct ReplaceInput<T> { /* private fields; full-thread coverage */ }
  pub struct MergeInput<T>   { /* private fields; partial-delta */ }

  impl<T> ReplaceInput<T> {
      /// Constructor takes typed evidence of full-thread coverage. Evidence types
      /// live in the same crate; only Gmail full-thread sync and the moved
      /// store_threads path can produce them.
      pub fn from_full_thread(evidence: FullThreadFetch, items: Vec<T>) -> ReplaceInput<T> { ... }
  }

  impl<T> MergeInput<T> {
      pub fn from_partial_delta(evidence: PartialDeltaPage, items: Vec<T>) -> MergeInput<T> { ... }
  }

  pub fn replace_thread_labels(tx: &Transaction, key: ThreadKey, input: ReplaceInput<LabelKind>)
      -> Result<(), String>
  {
      let labels = filtered_membership_ids(input.items());  // defensive cleanup, see below
      db::raw::delete_thread_label_rows(tx, key)?;
      db::raw::insert_thread_label_rows(tx, key, &labels)?;
      Ok(())
  }

  pub fn merge_thread_labels(tx: &Transaction, key: ThreadKey, input: MergeInput<LabelKind>)
      -> Result<(), String>
  {
      let labels = filtered_membership_ids(input.items());
      db::raw::insert_thread_label_rows(tx, key, &labels)?;
      Ok(())
  }
  ```
  `FullThreadFetch` and `PartialDeltaPage` are typed evidence; only legitimate caller sites within `provider-sync` can produce them.

- **`filtered_membership_ids` moves with the orchestration.** Today it drops message-state label IDs and reserved IMAP system keywords before writing - provider-semantic decisions, not row-level concerns. It lives in `provider-sync`. Under #5c (typed `LabelKind`), most of what it filters becomes structurally unrepresentable; during transition it stays as defensive cleanup over legacy `String` IDs.

- **`sync::pipeline::store_threads` persistence half moves up.** The function is currently called only from `provider-sync/imap/imap_initial.rs` and `provider-sync/imap/imap_delta.rs`. The JWZ-threading half (computing `ThreadGroup` values, the `MessageMeta` types) stays in `sync`; the persistence half (the per-thread aggregate + replace_thread_folders/labels calls) moves to `provider-sync`. After the move, the two IMAP callers compose: `let groups = sync::compute_thread_groups(...); provider_sync::store_thread_groups(groups, ...);`.

**Composite per-member dispatch (independent of cross-crate structure).** The per-member dispatch goes through `_no_enqueue` entry points typed as such. The public entry point that enqueues is `add_label(...)`; the composite-callable entry point is `add_label_no_enqueue(...) -> ActionOutcome`. The composite holds an `ActionContext` that does not include an `EnqueueCapability` token; the public `add_label` requires the token. `suppress_pending_enqueue: bool` disappears.

**Fidelity:** high. The typed inputs and the helpers live in the same crate as their legitimate constructors; standard within-crate sealing applies.

**Migration scope.**

- New: `crates/db/src/db/raw/thread_membership.rs` (boring row ops).
- New: `crates/provider-sync/src/thread_writes.rs` (`ReplaceInput`, `MergeInput`, `replace_thread_labels`, `merge_thread_labels`, evidence types, `filtered_membership_ids`).
- New: `crates/provider-sync/src/store_threads.rs` (persistence half lifted from `sync::pipeline`).
- `crates/db/src/db/queries_extra/thread_persistence.rs` - `replace_thread_*` / `merge_thread_*` deleted (or downgraded to private helpers if any internal caller still needs them).
- `crates/sync/src/pipeline.rs` - `store_threads` keeps JWZ-threading logic but stops calling `replace_thread_*`; persistence half is gone.
- `crates/provider-sync/src/imap/imap_initial.rs`, `imap_delta.rs` - compose the split: `sync::compute_thread_groups` + `provider_sync::store_thread_groups`.
- `crates/provider-sync/src/gmail/sync/storage.rs`, `graph/sync/persistence.rs`, `jmap/sync/storage.rs` - re-import from `provider_sync::thread_writes`. JMAP grows the missing `merge_thread_labels` call (Shape 10 / fixed if non-keyword JMAP labels exist, see open question).
- `crates/service/src/actions/label.rs` - split `add_label` into `add_label_no_enqueue` + `add_label` requiring `EnqueueCapability`.
- `crates/service/src/actions/label_group.rs` - composite calls only `_no_enqueue` entry points. `ActionContext.suppress_pending_enqueue` is deleted.

Per the migration ground rules, the move is a single-landing atomic PR. No source-level relocation shims. If the scope is too large for one landing, re-scope by helper-and-its-callers, with the corresponding type moving alongside the first helper.

**Remaining open question.** Does JMAP carry non-keyword raw labels? If keyword-only, Shape 10 is resolved by the IMAP-style recompute pattern applied to JMAP. If non-keyword JMAP labels can flow, this is a data-loss bug to fix during this migration.

**Success criteria.** Shape 4 inventory entries either resolve (`// resolved by contract #4`) or move to a "verified consistent under #4" note. Shape 7's composite preflight bug is structurally impossible: a compile-fail test attempts to call the enqueueing variant from inside a composite and fails. A compile-fail test attempts to construct a `ReplaceInput` from `provider-sync/graph` (which has only partial-delta evidence) and fails.

### 7. #2 Canonical Answer - sealed within owning crates (high fidelity)

**Inventory:** Shape 8 entries (drafts list/count, search vs fallback), partial Shape 12.

**Status:** Drafts list and search-fallback slices landed. `get_drafts_view` is the only public Drafts-list query; it returns a sealed `DraftsView` whose synced/local parts are available only through `into_parts`. `get_draft_threads_synced` and `get_local_draft_summaries` are crate-private. Search callers now go through `search()`, which returns `SearchResults::FullIndex` or `SearchResults::Degraded`; `search_sql_fallback` is private to `core/search_pipeline`.

**On inspection, neither sub-case is cross-crate.** Drafts orchestration is internal to `db` (synced query + local query both live there, the merge is a `db` function). Search unification is internal to `core/search_pipeline` (`search`, `search_sql_only`, `search_combined`, `search_sql_fallback` all live in the same module). Standard within-crate sealing applies - `pub(crate)` on the non-canonical entries, `pub` on the unified entry.

**Design sketch.**

- **Drafts (within `db`):**
  ```rust
  // crates/db/src/db/queries_extra/scoped_queries.rs

  // public: the only externally-callable Drafts list entry.
  pub fn get_drafts_view(...) -> Result<DraftsView, String> { ... }

  // public: the only externally-callable Drafts count entry.
  pub fn get_draft_count_with_local(...) -> Result<i64, String> { ... }

  // pub(crate): synced-only path. Visible to db's internal merge; not callable
  // from app, core, or anywhere else.
  pub(crate) fn get_draft_threads_synced(...) -> Result<Vec<Thread>, String> { ... }

  pub(crate) fn count_local_drafts(...) -> Result<i64, String> { ... }
  ```
  `DraftsView` is a private-fielded type - external callers consume it through accessors and cannot forge it.

- **Search (within `core/search_pipeline`):**
  ```rust
  // crates/core/src/search_pipeline.rs

  pub fn search(query: SearchQuery, opts: SearchOpts) -> Result<SearchResults, String> {
      // internal dispatch to search_sql_only / search_tantivy_only / search_combined / search_sql_fallback.
      // Returns SearchResults::FullIndex or SearchResults::Degraded.
  }

  // private: internal dispatch arms. Not callable from app.
  fn search_sql_fallback(...) -> Result<Vec<UnifiedSearchResult>, String> { ... }
  fn search_combined(...) -> Result<Vec<UnifiedSearchResult>, String> { ... }
  // ...etc
  ```

**Fidelity:** high within each owning crate. `pub(crate)` is sufficient because the failure mode is internal: a `db` consumer in `app` cannot reach `get_draft_threads_synced`; a `core::search_pipeline` consumer in `app` cannot reach `search_sql_fallback`.

**Migration scope.**

- `crates/db/src/db/queries_extra/scoped_queries.rs` - `get_draft_threads` becomes `pub(crate)` and renamed `get_draft_threads_synced`; `count_local_drafts` becomes `pub(crate)`; `get_drafts_view` and `get_draft_count_with_local` are the public entries.
- `crates/app/src/helpers.rs` - `load_threads_for_current_view` calls `get_drafts_view` directly; the previous app-layer merge in `helpers.rs:167-175` disappears behind the DB-owned canonical query.
- `crates/core/src/search_pipeline.rs` - `search_sql_fallback` and the internal-dispatch functions are private. `search` is the only public entry and returns `SearchResults`.

**Success criteria.** Shape 8 inventory entries resolve. A compile-fail test attempts to call `get_draft_threads_synced` from the sidebar render path and fails. A compile-fail test attempts to call `search_sql_fallback` from `app` and fails.

### 8. #1 grain.scope - exhaustive dispatch (high fidelity)

**Status:** landed for the documented failure shape. `ViewScope::to_account_scope()` is deleted; navigation and thread loading now dispatch on `ViewScope` exhaustively before constructing an `AccountScope` for personal-account query paths.

**Inventory:** `core/src/scope.rs:31-36`, parts of Shape 8.

**Design sketch.** `ViewScope::to_account_scope() -> Option<AccountScope>` is the failure shape. Replace with exhaustive dispatch:

```rust
pub fn threads_in_scope(scope: &ViewScope, ...) -> Vec<Thread> {
    match scope {
        ViewScope::AllAccounts => threads_all_accounts(...),
        ViewScope::Account(id) => threads_for_account(id, ...),
        ViewScope::SharedMailbox(id) => threads_for_shared_mailbox(id, ...),
        ViewScope::PublicFolder(id) => threads_for_public_folder(id, ...),
    }
}
```

Today the dispatch is spread across `crates/app/src/helpers.rs::load_threads_scoped`, `thread_query_label_for_selection`, and downstream call sites that each re-pattern-match. The migration consolidates the dispatch behind a single function whose signature requires the full `ViewScope` enum. A new `ViewScope` variant becomes a compile error in one place, not many.

**Fidelity:** high. Exhaustive `match` is compile-time enforced.

**Migration scope.** Mostly `crates/app/src/helpers.rs` and `crates/core/src/scope.rs`. Smaller than the other migrations; it's a refactor with no new types.

**Success criteria.** `to_account_scope` is deleted. Every consumer of `ViewScope` either takes the full enum and dispatches exhaustively, or takes a narrower type (e.g., `AccountScope`) that is structurally unreachable from the broader scope variants.

### 9. #5c FolderKind + LabelKind - boundary parse, separate types

**Status:** label/provider-dispatch slice landed. `types::LabelKind`, `FolderKind`, `SystemFolderId`, `MailLocator`, and private-field payload newtypes own provider-specific storage encodings. Provider label dispatch now accepts `LabelKind`; Gmail, Graph, JMAP, and IMAP match typed variants instead of string prefixes. Sync/dev-seed label synthesis and smart-folder system-folder shorthands construct through the typed boundary. Raw action/DB/wire IDs remain string-shaped at the outer boundary, and folder operation APIs remain transitional.

**Inventory:** Shape 6 entries (every `kw:` / `cat:` / `importance:` prefix call site), the system-folder-shorthand entry, parts of Shape 5 (validated domain).

**Design sketch.** Two separate enums in `types`, plus a `MailLocator` parse-product enum used only at parse boundaries. **Operation APIs accept narrow types.** Provider-native system labels are normalized to canonical Ratatoskr IDs on ingest, per `docs/glossary/folders-labels.md` - they never appear as provider-specific variants in `FolderKind`.

**Payload types are themselves private-fielded validated newtypes.** Because public enum variants are constructors by inclusion, the seal lives one layer down on the payload type.

```rust
// crates/types/src/folder_label.rs

// Validated payload types - private fields, parser-only construction.

pub struct KeywordName(String);
impl KeywordName {
    pub fn parse(raw: &str) -> Result<KeywordName, ParseError> {
        // validate: non-empty, RFC 5788 keyword charset, not a system-reserved $-prefix.
    }
    pub fn as_str(&self) -> &str { &self.0 }
}

pub struct CategoryName(String);
pub struct GraphGuid(String);
pub struct ImapPath(String);
pub struct GmailLabelId(String);
pub struct JmapId(String);
// each with its own private field + parse function.

// Folder and label kind enums - variants carry validated payloads.

pub enum FolderKind {
    /// Canonical Ratatoskr system folder. Gmail INBOX, Graph inbox, IMAP \Inbox,
    /// JMAP role:inbox - all normalize here on ingest via SYSTEM_FOLDER_ROLES.
    System(SystemFolderId),

    /// Provider-specific user folders. No GmailUser variant - Gmail user-created
    /// labels are LabelKind, not FolderKind, per glossary.
    GraphUser(GraphGuid),    // graph-{guid}
    JmapUser(JmapId),        // jmap-{id}
    ImapUser(ImapPath),      // folder-{path}
}

pub enum SystemFolderId {
    Inbox, Sent, Draft, Trash, Spam, Archive,
}

pub enum LabelKind {
    GmailUser(GmailLabelId),
    GraphCategory(CategoryName),
    GraphImportance(ImportanceLevel),  // High | Low; is_undeletable invariant is structural
    JmapKeyword(KeywordName),
    ImapKeyword(KeywordName),
}

pub enum ImportanceLevel { High, Low }

impl ImportanceLevel {
    pub fn opposite(self) -> ImportanceLevel {
        match self { Self::High => Self::Low, Self::Low => Self::High }
    }
}

pub enum MailLocator {
    Folder(FolderKind),
    Label(LabelKind),
}

pub enum Namespace {
    FromFolders,    // raw came from folders-table id; expect FolderKind
    FromLabels,     // raw came from labels-table id; expect LabelKind
    FromUserQuery,  // raw came from search syntax; disambiguate by prefix shape
}

impl FolderKind {
    pub fn parse(raw: &str, provider: MailProviderKind) -> Result<FolderKind, ParseError> { ... }
}

impl LabelKind {
    pub fn parse(raw: &str, provider: MailProviderKind) -> Result<LabelKind, ParseError> { ... }
}

impl MailLocator {
    pub fn parse(raw: &str, provider: MailProviderKind, namespace: Namespace)
        -> Result<MailLocator, ParseError> { ... }
}
```

The `Namespace` parameter is load-bearing - a raw string alone is genuinely ambiguous. The namespace tells the parser which side of the folder/label divide to expect.

**Operation APIs stay narrow:**

```rust
// crates/common/src/ops.rs (ProviderOps trait)
fn move_to_folder(&self, folder: &FolderKind, ...) -> ...;
fn add_label(&self, label: &LabelKind, ...) -> ...;
fn remove_label(&self, label: &LabelKind, ...) -> ...;
```

A folder cannot accidentally be passed where a label is expected. `MailLocator` exists only for parse-time discovery; no operation API accepts it.

`opposite_importance_label` becomes `ImportanceLevel::opposite`, returning `ImportanceLevel` - never a string.

**Fidelity:** high. The parsers are the only constructors from raw values; payload types are themselves boundary-parsed; consumers receive the typed enum and exhaustively match.

**Migration scope.** Largest of the migrations:

- `crates/types/src/folder_label.rs` - new types (enums + validated payload newtypes).
- Every `*_label_id` / `*_folder_id` field that today is `String` becomes the narrow typed variant.
- `MailActionIntent`, `MailOperation`, `WireMailOperation` use `FolderKind` and `LabelKind`.
- Every provider's `add_label` / `remove_label` / `create_label` / `move_to_folder` accepts the narrow type and exhaustively matches the variant.
- `crates/service/src/actions/label.rs::ensure_prefixed_tag_label` is replaced by `LabelKind` construction at the boundary.
- `crates/dev-seed/src/accounts.rs::seeded_user_label_id` returns `LabelKind`, not `String`.
- `crates/smart-folder/src/sql_builder.rs::IN_FOLDER_SHORTHANDS` is replaced by a `SystemFolderId` enum and exhaustive parse.
- **`filtered_membership_ids` in `provider-sync` (introduced in #4) shrinks.** Once `LabelKind` is the inward representation, message-state IDs and reserved IMAP keywords cannot be constructed as `LabelKind` values at all; the defensive filter only needs to handle the remaining transitional string-typed edges. After #5c lands, the filter likely disappears entirely.

**Remaining open design questions.**

- **On-disk format:** boundary adapter (recommended) vs DB restructure (cleaner, larger).
- **IPC wire format:** serde `#[serde(tag, content)]` round-trip vs string-on-the-wire with parse-at-IPC-boundary.

**Success criteria.** Every `strip_prefix("kw:")` / `strip_prefix("cat:")` / `strip_prefix("importance:")` call site is gone. Every `format!("kw:{}", ...)` is gone. `LabelKind::parse`, `FolderKind::parse`, and the payload-type `parse` functions are the only places that know about the prefix and shape encodings. Gmail INBOX appears nowhere as a provider-specific `FolderKind` variant; it parses to `FolderKind::System(SystemFolderId::Inbox)`.

## Migration Ground Rules

These apply to every contract migration.

- **Each migration is a sequence of compile-checked landings, not one mega-PR.** Introduce the type, migrate one consumer, repeat. Brokkr's compile-fail tests are the safety net.
- **No deprecated source shims.** Old call sites become compile errors during migration, not runtime warnings. No `#[deprecated]` wrappers that leave both APIs callable from new code; either the old function is gone or it returns a different type that doesn't satisfy the new signature. **This rule applies to cross-crate relocations too** - when a type or helper moves between crates (as in #4), the move is a single atomic landing; both crates do not expose the same source-level API simultaneously.
- **Boundary adapters are allowed where the boundary is real.** A DB migration that reads both old and new format during a transition window is a boundary adapter, not a source shim. An IPC version-bump compatibility shim that translates an old wire format into the new typed value is a boundary adapter. The rule is: adapters live at the edge (disk, wire, external input) where the format is genuinely outside our control; they do not live at the source level where we control both producer and consumer.
- **No `// TODO: migrate later` markers in code.** If a source-level call site can't migrate now, the type design isn't right yet - back to design, not technical debt. (Does not apply to boundary adapters, which are explicit migration boundaries.)
- **Each migration updates `discrepancies.md`** to retag or resolve the entries it addresses. The inventory shrinks as enforcement lands.

## Non-goals

- Not a product roadmap. This document does not address feature work, UX changes, or user-visible behavior.
- Not a bug-fix list. The discrepancies inventory has individual bugs that could be fixed in isolation; this document deliberately treats them as evidence of structural failures and routes fixes through type design rather than one-off patches.
- Not a refactoring policy. Migrations land in service of the named contract; broader cleanup ("while we're here, also rename this") rides separately.

## Cross-references

- `docs/glossary/discrepancies.md` - the tagged inventory each contract resolves.
- `docs/architecture.md` - the guiding principles and existing settled patterns. The composite-operations section is the de-facto pre-existing spec for contract #4. The "shared-table SQL belongs to `db`" rule is *clarified* by #4's row-primitives-vs-orchestration split, not weakened - raw SQL stays in `db::raw`.
- `docs/glossary/folders-labels.md` - the binding rules contracts #1, #3, #4, and #5c encode into types. Note the per-field reducer rule for thread aggregates.
