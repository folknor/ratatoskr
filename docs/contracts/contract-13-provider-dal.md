# Contract #13: Provider Database Boundary

## Problem

Four provider crates (`gmail`, `graph`, `imap`, `jmap`) and the `sync` crate write directly to the main application database with inline SQL. There is no unified data access layer. The same tables — `messages`, `labels`, `contacts`, `threads` — are written by four independent SQL implementations with different conflict resolution, different column subsets, and different transaction patterns.

This is not a theoretical hygiene concern. When four providers independently write to the same table with slightly different INSERT/ON CONFLICT clauses, any schema change or invariant adjustment must be discovered and updated in four places. There is no single place that owns "how a label gets written" or "how a message gets upserted." Provider SQL is silently diverging.

Contract #12 defines which crates may depend on `rusqlite`. This contract defines the write-path architecture that replaces the current inline SQL in provider and sync crates.

## Current State

### Three layers of database coupling

Provider database usage falls into three structurally different categories. Each requires a different extraction strategy.

### Layer 1: Shared thread persistence (partially centralized)

`sync::persistence` already serves as a de facto DAL for thread-level writes. All four providers delegate to it:

- `upsert_thread_aggregate` — thread record upsert from message aggregates
- `upsert_thread_participants` — sender/recipient tracking
- `replace_thread_labels` — label assignments
- `maybe_update_chat_state` — chat thread detection
- `delete_messages_and_cleanup_threads` — cascading deletion
- `compute_thread_aggregate` — read thread state from messages

Gmail, JMAP, and Graph all call the same functions. IMAP calls most of them, with some inline equivalents in `sync_pipeline.rs`.

These functions take `&Transaction` with raw SQL. They are in `sync`, not `db`. The abstraction boundary exists but is in the wrong crate.

### Layer 2: Per-provider message/attachment storage (duplicated)

Each provider has its own message upsert and attachment upsert writing to the shared `messages` and `attachments` tables:

| Provider | Function | Tables |
|---|---|---|
| Gmail | `sync/storage.rs:upsert_messages` | messages, attachments |
| Graph | `sync/persistence.rs:upsert_messages` | messages, attachments |
| JMAP | `sync/storage.rs:upsert_messages` | messages, attachments |
| IMAP | `sync_pipeline.rs:DbInsertData::insert` | threads, messages, attachments |

The SQL is nearly identical across providers. The column set is the same (shared `messages` schema); only the source data shapes differ (Gmail API message vs. Graph message vs. JMAP Email vs. IMAP envelope).

### Layer 3: Provider-specific domain storage (scattered, heterogeneous)

Each provider writes to domain-specific or provider-specific tables with unique schemas and different write patterns.

**Shared tables with provider-specific column usage:**

`labels` is written by all four providers with divergent SQL:
- Gmail: `INSERT OR REPLACE` with `type`, no rights columns
- Graph: `INSERT OR REPLACE` with `cat:` prefix convention for categories
- IMAP: `INSERT ON CONFLICT` with `imap_folder_path`, `imap_special_use`, `parent_label_id`
- JMAP: `INSERT ON CONFLICT` with 9 mailbox rights columns, `is_subscribed`, `parent_label_id`

`contacts` is written by Gmail (People API + otherContacts), Graph (Exchange contacts), and JMAP (ContactCard), each with provider-specific contact-map tables for sync bookkeeping.

`signatures` is written by Gmail and JMAP with bidirectional sync logic.

`calendars` / `calendar_events` / `calendar_attendees` / `calendar_reminders` are written by Gmail and JMAP calendar sync.

**Provider-local state tables:**

- Gmail: `google_contact_map`, `google_other_contact_map`
- Graph: `graph_contact_map`, `graph_subscriptions`
- IMAP: `folder_sync_state`
- JMAP: `jmap_push_state`
- Sync: `jmap_sync_state`, `graph_folder_delta_tokens`, `graph_contact_delta_tokens`, `graph_shared_mailbox_delta_tokens`, `shared_mailbox_sync_state`

**Public folder tables** (Graph + IMAP): `public_folders`, `public_folder_items`, `public_folder_sync_state`, `public_folder_pins`

**Reaction tables** (Gmail + Graph): `message_reactions` with provider-specific ingestion logic

## Contract

### 1. `db` owns all writes to shared tables

Providers must not write directly to tables shared across the application (`messages`, `threads`, `attachments`, `labels`, `contacts`, `signatures`, `calendars`, `calendar_events`, `thread_labels`, `thread_participants`, `contact_groups`, `contact_group_members`). Instead, `db` exposes typed write APIs that providers call after translating their protocol payloads into common write structs.

### 2. `sync::persistence` moves into `db`

The existing thread persistence functions (`upsert_thread_aggregate`, `replace_thread_labels`, `upsert_thread_participants`, `maybe_update_chat_state`, `delete_messages_and_cleanup_threads`, `compute_thread_aggregate`) already represent the correct abstraction. They should move from `sync` to `db`, preserving their `&Transaction` signatures. The `sync` crate and all provider crates import them from `db` instead.

### 3. Message and attachment writes unify

The four per-provider `upsert_messages` / `upsert_attachments` implementations collapse into a single `db` API. Providers translate their protocol message types into a `MessageInsertRow` (or equivalent) struct that `db` owns. `db` handles the INSERT/ON CONFLICT mechanics.

### 4. Label writes unify

The four divergent label INSERT implementations collapse into a single `db` API that accepts a provider-neutral label write struct. Provider-specific columns (IMAP folder path, JMAP rights, etc.) are represented as optional fields on that struct. `db` owns the conflict resolution strategy.

### 5. Contact, signature, and calendar writes unify

Same principle as labels: `db` owns the write API for `contacts`, `signatures`, and calendar tables. Provider-specific sync bookkeeping (contact maps, sync tokens) is handled separately (see rule 6).

### 6. Provider-local state tables are explicitly scoped

Tables that exist solely for one provider's sync protocol (`folder_sync_state`, `jmap_push_state`, `graph_subscriptions`, `google_contact_map`, etc.) and sync coordination tables (`jmap_sync_state`, `graph_folder_delta_tokens`, `shared_mailbox_sync_state`, etc.) may remain as provider-owned storage under one of two arrangements:

- **Option A:** The provider keeps inline SQL for its own tables, and those tables are documented as provider-owned in the schema. The provider must not also write to shared tables.
- **Option B:** `db` provides narrow storage modules for provider state (e.g., `db::sync_tokens`, `db::provider_maps`), and providers delegate to those.

This contract does not mandate which option. Both are acceptable as long as the boundary between provider-local tables and shared tables is explicit and enforced.

### 7. Provider reads are lower priority

A provider reading `accounts` for its OAuth tokens, or reading `messages` for IMAP UIDs, is a weaker form of coupling than writing to shared tables. These reads may remain as inline SQL during migration, provided no new broad read patterns are added. Long-term, reads should also route through `db` APIs, but write-path unification takes priority.

## Migration Shape

### Phase A: Move `sync::persistence` into `db`

- Move `compute_thread_aggregate`, `upsert_thread_aggregate`, `upsert_thread_participants`, `replace_thread_labels`, `maybe_update_chat_state`, `delete_messages_and_cleanup_threads`, and `query_user_emails` from `sync/persistence.rs` into `db`.
- Keep `store_message_bodies`, `store_inline_images`, `index_search_documents` in `sync` (they target `stores` and the search index, not the main DB).
- Preserve the existing `&Transaction` signatures in this phase. Transaction-shape redesign is deferred; Phase A is an ownership move, not a transaction API redesign.
- Move the storage-behavior tests for these functions with the code into `db`.
- Update all callers (gmail, graph, jmap, imap, sync/pipeline) to import from `db`.

### Phase B: Unify message and attachment writes

- Define `MessageInsertRow` and `AttachmentInsertRow` structs in `db`.
- Implement a single `db::insert_messages` / `db::insert_attachments` function.
- Have each provider map its protocol type into the common struct.
- Keep thread aggregate recomputation and participant/label updates as explicit DAL calls rather than folding them into `insert_messages` in this phase.
- Remove the four per-provider upsert implementations.

### Phase C: Unify label writes

- Define a `LabelWriteRow` struct in `db` with optional provider-specific fields.
- Implement a single `db::upsert_labels` function that handles conflict resolution.
- If one fully unified label write API becomes too contorted, it is acceptable to split regular label/folder writes from category-style label writes behind two `db` APIs, as long as `db` still owns the shared-table SQL.
- Replace the four per-provider label persist functions.

### Phase D: Unify contact, signature, and calendar writes

- Same pattern as Phase C for each domain.
- Treat this as a bucket of later sub-phases, not one implementation step. Contacts, signatures, and calendar persistence have different write shapes and should be split if that keeps the migration reviewable.
- Contact sync bookkeeping tables (`google_contact_map`, `graph_contact_map`) either stay provider-owned or move to `db::provider_maps`.

### Phase E: Scope provider-local state

- Enumerate all provider-local tables.
- Document which tables are provider-owned vs. shared.
- Decide Option A or B per table group.
- Enforce the boundary.

## What This Eliminates

- Four independent SQL implementations for the same table silently diverging
- Schema changes that must be discovered and updated in four places
- Conflict resolution differences between providers writing to the same table
- Provider crates encoding application-level storage invariants that belong in `db`
- The risk that adding a column to `messages` or `labels` requires changes in gmail, graph, imap, and jmap simultaneously

## Relationship to Contract #12

Contract #12 defines which crates may depend on `rusqlite`. This contract defines the write-path architecture for the provider and sync crates specifically. When both contracts are satisfied:

- `db` and `stores` own all direct `rusqlite` access
- `sync` and provider crates translate protocol payloads into typed write structs
- `db` owns SQL shape, transaction boundaries, and conflict resolution for all shared tables
- Provider-local state tables have explicit ownership, whether in the provider or in `db`

## Open Questions

1. After Phase A preserves the current `&Transaction` signatures, should a later phase redesign the DAL around higher-level transaction ownership in `db`, or is explicit transaction passing the intended steady state?
2. Should IMAP's `sync_pipeline.rs` inline threading logic be unified with `sync::pipeline.rs`, or do they represent genuinely different threading strategies?
3. For label writes, is one struct with many optional fields better than a base struct with provider-specific extension traits?
4. Should the `settings` table (currently used as a key-value store for Google sync tokens) be replaced with a dedicated sync-token table?
5. How should `message_reactions` writes be handled? Gmail reactions are actual messages; Graph reactions are extended properties with separate polling. The write path may need provider-specific handling even behind a unified API.
