# Rust Core Architecture: Moving Business Logic to the Metal

**Date**: March 2026
**Context**: Ratatoskr's service layer currently lives in TypeScript, with SQLite access through Tauri's SQL plugin. This works for moderate mailboxes but breaks down at scale. Clients with hundreds of emails per day want 60GB+ of email locally with instant search — something no desktop email client does well today.

This document lays out what "closer to the metal" means concretely: storage architecture, search engine, Rust-side business logic, and the migration path to get there.

---

## Table of Contents

- [The Problem at Scale](#the-problem-at-scale)
- [Current Coupling Audit](#current-coupling-audit)
- [Target Architecture](#target-architecture)
- [Layer 1: SQLite for Metadata](#layer-1-sqlite-for-metadata)
- [Layer 2: Compressed Body Store](#layer-2-compressed-body-store)
- [Layer 3: Tantivy for Full-Text Search](#layer-3-tantivy-for-full-text-search)
- [Layer 4: Rust Business Logic](#layer-4-rust-business-logic)
- [Rust Crate Ecosystem](#rust-crate-ecosystem)
- [IPC and State Management](#ipc-and-state-management)
- [Precedent: How Others Do It](#precedent-how-others-do-it)
- [How Existing Email Clients Handle Scale](#how-existing-email-clients-handle-scale)
- [Migration Path](#migration-path)
- [Performance Projections](#performance-projections)

---

## The Problem at Scale

Current architecture bottlenecks at 60GB+ / 10M+ messages:

| Component | Current | Problem at Scale |
|-----------|---------|-----------------|
| **Message bodies** | SQLite rows (50-500KB each) | SQLite constructs entire rows in memory on any column access. 500KB bodies bloat the DB file, kill write throughput, and make VACUUM require 120GB+ free space |
| **Full-text search** | SQLite FTS5 (trigram tokenizer) | FTS5 ranked queries hit 20+ seconds at 6M rows. Trigram tokenizer creates massive index (10-20GB+). No faceted search, no fuzzy matching |
| **Sync logic** | TypeScript | Every DB write crosses the JS→plugin→SQLite IPC bridge. Syncing 50 messages = ~50 round trips. Bound by V8's single thread for CPU-heavy MIME parsing |
| **Email actions** | TypeScript | Optimistic UI works, but every action hits IPC overhead |
| **DB queries** | Tauri SQL plugin | All 32 service files make IPC calls for every read/write. Latency compounds on complex views |

---

## Coupling Audit (Pre-Migration Snapshot)

> **Note**: This section documents the state before Phase 0. The facade layer (`src/core/`) has since been implemented, decoupling most of these direct service imports. Kept for historical context and to track what was migrated.

Before any Rust migration, we need to understand how deeply the frontend is coupled to the service/DB layer. **68 out of 94+ component/hook/store files import directly from services.** Business logic is scattered throughout the UI tier.

### Severity Map

#### CRITICAL — Direct DB Access or Heavy Business Logic in UI

| File | Problem |
|------|---------|
| `stores/smartFolderStore.ts` | Calls `getDb()` directly (line 136), executes raw SQL queries in store actions |
| `hooks/useEmailListData.ts` | 8+ direct DB queries, raw SQL execution via `db.select()`, smart folder query building, complex pagination mixed with service calls |
| `components/email/ActionBar.tsx` | 9+ service calls (archive, trash, star, spam, mute, pin, snooze, follow-up, quick steps) + 4+ DB operations + Gmail client access |
| `components/ui/ContextMenuPortal.tsx` | 10+ service calls + 4+ DB operations + Gmail sync trigger + quick step execution. Nearly identical action surface to ActionBar |
| `components/composer/Composer.tsx` | Calls `getDb()` directly for draft lookup, orchestrates auto-save lifecycle, contact upserts on send, settings reads for behavior |
| `components/accounts/AddAccount.tsx` | OAuth flow + direct `insertAccount()` DB call |

#### HIGH — Store CRUD or Significant Business Logic

| File | Problem |
|------|---------|
| `stores/labelStore.ts` | Full CRUD lifecycle in store: `getLabelsForAccount()`, `upsertLabel()`, `updateLabelSortOrder()`, `deleteLabel()`, plus `getGmailClient()` for remote sync |
| `components/email/ThreadView.tsx` | Multiple DB calls (`getMessagesForThread`, `getAllowlistedSenders`, `getSetting`), `markThreadRead` service call |
| `hooks/useKeyboardShortcuts.ts` | DB queries (`getMessagesForThread`, `deleteThread`), Gmail service calls (`deleteDraftsForThread`, `triggerSync`, `getGmailClient`) |
| `components/settings/SettingsAiTab.tsx` | Multiple DB writes (`setSetting`, `setSecureSetting`, `setBundleRule`) |
| `components/settings/SmartLabelEditor.tsx` | DB reads + `backfillSmartLabels()` business logic call |

#### MEDIUM — Settings Persistence or Service Calls

| File | Problem |
|------|---------|
| `stores/uiStore.ts` | 15+ `setSetting().catch(() => {})` calls — every preference change writes to DB inline |
| `stores/accountStore.ts` | `setSetting()` in `setActiveAccount()` |
| `stores/shortcutStore.ts` | `getSetting()`/`setSetting()` for keymap persistence |
| `components/composer/AddressInput.tsx` | `searchContacts()` DB query for autocomplete |
| `components/email/InlineReply.tsx` | `upsertContact()`, `getSetting()`, `getDefaultSignature()`, `sendEmail()`, `archiveThread()` |
| `components/email/ContactSidebar.tsx` | `getThreadById()`, `getThreadLabelIds()`, `isVipSender()`, gravatar fetch |
| `components/email/FollowUpDialog.tsx` | 3 DB calls for follow-up CRUD |
| `components/email/EmailRenderer.tsx` | `addToAllowlist()` — image allowlist DB write |
| `components/search/SearchBar.tsx` | `searchMessages()` DB search |
| `components/search/AskInbox.tsx` | `askMyInbox()` AI service |
| `components/email/SmartReplySuggestions.tsx` | AI service + cache cleanup |
| `components/email/ThreadSummary.tsx` | AI service + cache cleanup |
| `components/tasks/AiTaskExtractDialog.tsx` | AI service + `insertTask()` DB write |
| `components/tasks/TasksPage.tsx` | `handleRecurringTaskCompletion()` business logic |
| `components/layout/MultiSelectBar.tsx` | `deleteThread()` + `getGmailClient()` |
| `components/layout/EmailList.tsx` | `getMessagesForThread()` + `getGmailClient()` |
| `components/dnd/DndProvider.tsx` | `addThreadLabel()`, `removeThreadLabel()` |
| Various settings tabs | DB reads/writes for their specific domains |

### Systemic Patterns

**Pattern 1: Stores persist to DB inline.** `uiStore`, `accountStore`, `shortcutStore`, `labelStore`, `smartFolderStore` all call `setSetting()` or DB write functions directly inside store actions. Every preference change, every label edit, every shortcut customization crosses into the DB layer from the store.

**Pattern 2: Components call 32+ different DB service functions.** `getMessagesForThread()`, `getThreadsForAccount()`, `getAllowlistedSenders()`, `searchContacts()`, `getSetting()`, `getLabelsForAccount()`, `getCategoriesForThreads()`, `getActiveFollowUpThreadIds()`, `getBundleRules()`, etc. — all called directly from React components.

**Pattern 3: Business logic duplicated across action surfaces.** `ActionBar.tsx`, `ContextMenuPortal.tsx`, `useKeyboardShortcuts.ts`, and `MultiSelectBar.tsx` all independently orchestrate the same email actions (archive, trash, star, move, etc.) with slightly different calling patterns. The `emailActions.ts` consolidation helped, but the callers still access DB directly for thread lookups, draft deletion, and sync triggers.

**Pattern 4: Direct database connection access.** Two files call `getDb()` to get a raw connection: `smartFolderStore.ts` (line 136) and `Composer.tsx` (line 383). This is the most extreme coupling — they bypass even the service layer.

**Pattern 5: Sync/background triggers from UI.** Keyboard shortcuts and context menus call `triggerSync()` and `getGmailClient()` directly. Auto-save is started/stopped from a `useEffect` in `Composer.tsx`. These should be Rust-owned background processes, not UI-triggered.

### What This Means for Migration

The frontend cannot be swapped to Rust commands incrementally in its current state. If we change `getMessagesForThread()` from a TS DB call to a Tauri command, we have to update **every component and hook that imports it** — and they import it for different reasons with different surrounding logic.

The required intermediate step is a **facade layer**: a single API module that all UI code imports from, with swappable implementations (TS today, Rust tomorrow). This is Phase 0 of the migration — the prerequisite for everything else.

---

## Target Architecture

Three-layer storage with Rust owning the data:

```
┌─────────────────────────────────────────────────┐
│                  React UI (View Layer)            │
│  Zustand stores = thin caches                     │
│  Calls Rust commands, listens for Rust events     │
└──────────────┬──────────────────┬─────────────────┘
               │ Tauri Commands   │ Tauri Events/Channels
               │ (queries)        │ (notifications)
┌──────────────▼──────────────────▼─────────────────┐
│              Rust Core (Data Layer)                │
│                                                    │
│  ┌──────────┐  ┌──────────┐  ┌──────────────────┐ │
│  │ Sync     │  │ Actions  │  │ Search           │ │
│  │ Engine   │  │ Engine   │  │ Engine           │ │
│  └────┬─────┘  └────┬─────┘  └────┬─────────────┘ │
│       │              │              │               │
│  ┌────▼──────────────▼──────────────▼─────────────┐ │
│  │              Storage Layer                      │ │
│  │                                                 │ │
│  │  SQLite        Body Store       Tantivy Index   │ │
│  │  (metadata)    (compressed)     (full-text)     │ │
│  │  <2GB          15-20GB          3-5GB           │ │
│  └─────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────┘
```

**Total projected storage for 60GB of raw email: ~20-27GB** (vs 60GB+ in current all-in-SQLite approach).

---

## Layer 1: SQLite for Metadata

**Expected size**: <2GB for 10M messages (~200 bytes of metadata per message)

Keep SQLite for what it does best — structured, relational data:
- Accounts, threads, labels, contacts, settings, filters, tasks
- Message metadata (from, to, subject, date, flags, labels, thread_id)
- Thread-label joins, message ordering, date sorting
- Offline operation queue, sync state

**What changes**: Remove message bodies and FTS content from SQLite. With bodies gone, the metadata DB stays small. At <2GB, SQLite is blazing fast — in-memory mmap, instant queries, VACUUM takes seconds instead of hours.

**Rust ownership**: Move from Tauri SQL plugin to Rust-owned `rusqlite` or `sqlx`. Direct function calls instead of IPC for every query.

**Recommended pragmas** for the metadata DB:
```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = normal;
PRAGMA temp_store = memory;
PRAGMA mmap_size = 2147483648;  -- 2GB, mmap the entire DB
PRAGMA cache_size = -64000;     -- 64MB page cache
```

---

## Layer 2: Compressed Body Store

**Expected size**: 15-20GB for 60GB of raw email (3-4x compression)

Email HTML is highly compressible — repeated tags, boilerplate, templated structure.

### Compression Strategy

| Method | Ratio | Notes |
|--------|-------|-------|
| zstd level 3 (no dictionary) | 3:1 to 4:1 | Baseline. 60GB → 15-20GB |
| zstd with per-domain dictionaries | 5:1 to 6:1 | Train on first ~100 emails per sender domain. Newsletters/transactional emails share 60-80% structure |
| zstd with per-domain dictionaries + dedup | 6:1 to 8:1 | Content-addressed storage deduplicates identical messages across accounts |

Per-domain dictionary training is the key insight. Emails from the same sender (GitHub notifications, Amazon orders, newsletter services) share enormous structural similarity. Google measured nearly 50% reduction on search HTML using dictionary compression for returning users. The same principle applies to email.

### Storage Backend Options

| Backend | Read Performance | Write Performance | Complexity | Rust Crate |
|---------|-----------------|-------------------|------------|------------|
| **LMDB (heed)** | Fastest (zero-copy mmap) | Moderate | Moderate (map size preallocation) | `heed` |
| **redb** | Fast (within 2x of LMDB) | Fast | Simple (pure Rust) | `redb` |
| **Filesystem (Maildir-like)** | Fast (OS page cache) | Fast | Simplest (debuggable) | `std::fs` |
| **Separate SQLite DB** | Good | Good | Familiar | `rusqlite` |

**Recommendation**: Start with a **separate SQLite database** (two columns: `message_id TEXT PRIMARY KEY, body BLOB`) with zstd compression. This is the lowest-risk first step — familiar tooling, easy to implement, and bodies no longer bloat the metadata DB.

Migrate to LMDB (heed) or redb later if SQLite body store becomes a bottleneck. LMDB's zero-copy reads are ideal for the read-heavy email pattern (95%+ reads), but the simpler path gets value faster.

### Benchmark Reference

From SQLite's own [Internal vs External BLOBs](https://sqlite.org/intern-v-extern-blob.html) analysis:
- BLOBs < 100KB: reads faster from SQLite than filesystem
- BLOBs > 100KB: filesystem wins

Email bodies average 50-500KB, right at the crossover. With compression bringing most bodies under 100KB, SQLite remains competitive — another reason to start there.

---

## Layer 3: Tantivy for Full-Text Search

**Expected size**: 3-5GB for 10M messages

[Tantivy](https://github.com/quickwit-oss/tantivy) is a full-text search engine **library** written in Rust, inspired by Apache Lucene. It embeds directly into your application — no separate process, no IPC overhead.

- **Version**: 0.25.1 (December 2025)
- **Stars**: ~14K GitHub
- **Downloads**: ~9.7M on crates.io
- **Maintained by**: Quickwit team (the company behind the Quickwit distributed search engine)
- **License**: MIT

### Performance

| Metric | Tantivy | SQLite FTS5 (trigram) |
|--------|---------|----------------------|
| Query latency (1M docs) | Sub-10ms (warm), sub-100ms (cold) | Sub-second |
| Query latency (5M docs) | Sub-10ms (warm), sub-100ms (cold) | Seconds, ranked queries slow |
| Query latency (10M+ docs) | ~35-80ms (172M-doc benchmark) | 20+ seconds for ranked queries |
| Indexing throughput | 30K docs/sec, multi-threaded | Single-threaded |
| Index size (10M emails) | 3-5GB | 10-20GB+ (trigram tokenizer) |
| Concurrent read/write | Yes (segment isolation) | WAL mode (limited) |

Tantivy is ~2x faster than Lucene in benchmarks. It uses block-max WAND for efficient top-K retrieval — only scores the documents that could make it into the result set.

### Features Relevant to Email

| Feature | Status |
|---------|--------|
| BM25 ranking | Built-in, default |
| Field-based search (from:, to:, subject:, body:) | Yes — typed schema fields |
| Date range queries (before:, after:) | Yes — DATE fields with range queries |
| Boolean queries (AND, OR, NOT) | Yes |
| Phrase search ("exact phrase") | Yes |
| Fuzzy search | Yes (Levenshtein distance) |
| Regex and wildcard | Yes |
| Faceted search (by label, folder, date) | Yes — hierarchical facets |
| Snippet/highlight generation | Yes — built-in `SnippetGenerator` with HTML output |
| Unicode / non-English text | Yes — tokenizers for 17 Latin languages, CJK via third-party |
| Fast fields (columnar data) | Yes — optimal for sort-by-date, filter-by-flag |

### Implemented Email Search Schema

Defined in `src-tauri/src/search/mod.rs`:

```rust
// Indexed + stored text fields (for display and search)
builder.add_text_field("subject", text_indexed_stored());
builder.add_text_field("from_name", text_indexed_stored());
builder.add_text_field("from_address", STRING | STORED);
builder.add_text_field("snippet", text_indexed_stored());

// Indexed text fields (search only, not stored)
builder.add_text_field("to_addresses", text_indexed());
builder.add_text_field("body_text", text_indexed());

// Stored identifiers (for joining back to SQLite)
builder.add_text_field("message_id", STRING | STORED);
builder.add_text_field("thread_id", STRING | STORED);
builder.add_text_field("account_id", STRING | STORED);

// Date field for range queries and sorting
builder.add_date_field("date", DateOptions::default().set_indexed().set_fast().set_stored());

// Fast filter fields (u64: 0 or 1)
builder.add_u64_field("is_read", NumericOptions::default().set_indexed().set_fast().set_stored());
builder.add_u64_field("is_starred", NumericOptions::default().set_indexed().set_fast().set_stored());
builder.add_u64_field("has_attachment", NumericOptions::default().set_indexed().set_fast().set_stored());
```

**Note**: Labels are not indexed in tantivy (they live in SQLite's `thread_labels` table and change frequently). The `label:` search operator currently requires post-filtering or SQLite fallback. Future option: index labels as a multi-valued text field and re-index on label changes.

### Query Flow

```
User types "from:alice project deadline after:2024-01-01"
  → Rust parses query (reuse/port searchParser.ts logic)
  → Tantivy query: BooleanQuery [
      TermQuery(from, "alice"),
      TermQuery(body, "project"),
      TermQuery(body, "deadline"),
      RangeQuery(date, 2024-01-01..now)
    ]
  → Tantivy returns top-K (thread_id, score, snippet) in <50ms
  → SQLite fetches full thread metadata by IDs
  → Results streamed to frontend via Tauri channel
```

### Comparison with SQLite FTS5

FTS5 can stay for simple, small-scope queries (e.g., filtering within a single folder view). Tantivy takes over for:
- Global search across all messages
- Ranked/scored results
- Complex multi-field queries
- Search-as-you-type with instant feedback
- Faceted browse (show result counts per label)

### Production Users

Tantivy powers: Quickwit (petabyte-scale log search), ParadeDB (Postgres FTS extension), Stract (open-source web search engine), Bloop (AI code search), LanceDB, OpenObserve. [Buzee](https://github.com/gsidhu/buzee-tauri) is a Tauri + tantivy full-text search desktop app — directly analogous to our use case.

No known email clients use tantivy yet. We would be the first. The closest analog is **notmuch** using Xapian (C++ search library) — tantivy is the modern Rust equivalent with comparable or better performance.

---

## Layer 4: Rust Business Logic

### What Moves to Rust

| Service | Current (TS) | Rust Equivalent | Why Move |
|---------|-------------|-----------------|----------|
| DB access (32 service files) | Tauri SQL plugin IPC | `rusqlite`/`sqlx` direct calls | Eliminates IPC overhead on every query |
| Sync engine | `syncManager.ts`, `sync.ts`, `imapSync.ts` | Rust module calling async-imap + DB directly | Batch DB writes without IPC. Multi-threaded MIME parsing |
| Email actions | `emailActions.ts` | Rust module with optimistic DB + queue | Direct DB access, no serialization overhead |
| Search | `searchParser.ts`, `searchQueryBuilder.ts` | Rust module querying tantivy | Native tantivy integration, streaming results |
| MIME parsing | `mail-parser` (already Rust, called via IMAP commands) | Same, but results go directly to DB | Skip serialization→IPC→deserialization→IPC→DB roundtrip |
| Email composition | Built in TS, passed to `lettre` via SMTP command | `mail-builder` crate | Build RFC 2822 messages natively in Rust |
| HTML sanitization | DOMPurify in browser | `ammonia` crate (optionally, keep DOMPurify too for defense-in-depth) | Sanitize before crossing IPC boundary |
| Threading (JWZ) | `threadBuilder.ts` | Port to Rust | CPU-intensive algorithm benefits from native speed |
| Filter engine | `filterEngine.ts` | Rust module | Runs during sync, benefits from direct DB access |

### What Stays in TypeScript

- React components and Zustand stores (UI layer)
- Keyboard shortcuts and navigation
- Composer (TipTap editor integration)
- Settings UI
- Any logic that primarily manipulates UI state

---

## Rust Crate Ecosystem

| Capability | Crate | Status | Notes |
|------------|-------|--------|-------|
| MIME parsing | `mail-parser` (Stalwart) | ✅ In use | Zero-copy, RFC 5322 compliant |
| Email composition | `mail-builder` (Stalwart) | Production-ready | Companion to mail-parser. Multipart MIME, attachments, auto-encoding |
| SMTP | `lettre` | ✅ In use | DKIM, TLS, multiple auth mechanisms |
| IMAP | `async-imap` | ✅ In use | See [imap-ecosystem-assessment.md](./imap-ecosystem-assessment.md) |
| HTML sanitization | `ammonia` | Production-ready (v4.1.2+) | Whitelist-based, built on html5ever. Fix RUSTSEC-2025-0071 in 4.1.2 |
| Full-text search | `tantivy` 0.25 | ✅ In use | 13-field email schema, BM25 ranking, structured search |
| SQLite | `rusqlite` 0.32 | ✅ In use | 67 commands, `spawn_blocking` + `std::sync::Mutex` pattern |
| Date/time | `chrono` 0.4 | ✅ In use | Date parsing for search index rebuild |
| Compression | `zstd` | Production-ready | For Phase 2 (body store) |
| KV store (optional) | `heed` (LMDB) | Production-ready | Zero-copy reads, memory-mapped. For Phase 2 |
| KV store (optional) | `redb` | Stable (1.0+) | Pure Rust, ACID, simpler than LMDB. For Phase 2 |

---

## IPC and State Management

### Communication Patterns

Tauri v2 provides three IPC mechanisms:

| Mechanism | Direction | Best For |
|-----------|-----------|----------|
| **Commands** | Frontend → Rust | Discrete operations: "archive thread", "get thread list", "search" |
| **Events** | Bidirectional | Lightweight notifications: "sync complete", "new messages", "thread updated" |
| **Channels** | Rust → Frontend (ordered streaming) | Large result sets, sync progress, search results as they arrive |

### State Ownership Pattern

Proven by Spacedrive and Delta Chat:

1. **Rust holds canonical state** — database, search index, sync state, offline queue
2. **Frontend queries state via commands** — like REST endpoints: `get_threads(folder, page)`, `get_thread_detail(id)`, `search(query)`
3. **Rust pushes change notifications via events** — lightweight signals: `{ type: "threads_changed", folder: "INBOX" }`
4. **Frontend re-fetches on notification** — Zustand stores listen for events, call commands to get fresh data

This is React Query's invalidation model with Tauri events as the invalidation trigger.

### Optimistic UI

Two patterns, use both during migration:

**Pattern A: Frontend-driven (use during migration)**
```
User clicks "Archive"
  → Zustand removes thread from list (optimistic)
  → invoke("archive_thread", { id })
  → Rust: updates DB, queues remote op, returns Ok/Err
  → On error: Zustand reverts, shows error toast
```

**Pattern B: Rust-driven (end goal)**
```
User clicks "Archive"
  → invoke("archive_thread", { id })  // returns immediately
  → Rust: updates DB (instant), emits "threads_changed", queues remote op
  → Frontend: receives event, re-fetches thread list from Rust
  → UI updates
```

Pattern B is cleaner. With sub-millisecond local SQLite queries, the Rust→event→re-fetch cycle is fast enough to feel instant.

### Type Safety Across the Boundary

TypeScript types are hand-written in `src/core/rustDb.ts` to match Rust struct definitions in `src-tauri/src/db/types.rs`. Both sides use `camelCase` serialization (`#[serde(rename_all = "camelCase")]` in Rust).

We evaluated tauri-specta for auto-generating TS types from Rust but removed it — the bindings were never exported and the hand-written types are more maintainable given the 1:1 mapping between TS service interfaces and Rust commands. The existing TS type definitions in `@/services/db/*` serve as the canonical contract; rustDb.ts imports these types directly.

---

## Precedent: How Others Do It

### Spacedrive (closest analog)

Tauri app with React frontend and heavy Rust backend (`sd-core`).

- **Database**: SeaORM on SQLite, fully managed in Rust
- **IPC**: Uses rspc (typesafe RPC) — define Rust functions, auto-generate TypeScript client
- **State sync**: Rust core emits invalidation events; frontend re-queries via rspc
- **Key lesson**: They built rspc specifically because raw Tauri commands don't scale when you have hundreds of endpoints. Worth adopting.

### Delta Chat (email client in Rust)

Full email client with Rust core (`deltachat-core-rust`) and thin frontends (Desktop, Android, iOS).

- **Architecture**: Rust library implements everything — IMAP, SMTP, encryption, contacts, threading
- **IPC**: JSON-RPC over stdio. Desktop starts Rust core as subprocess, communicates via stdin/stdout
- **Types**: TypeScript client auto-generated from Rust code
- **Key lesson**: A real-world email client proving the Rust-core pattern works. Their migration from C-FFI to JSON-RPC shows the value of a clean, typed API boundary.

### Lapce (Rust editor)

Uses a **proxy process** pattern — UI communicates with a `lapce-proxy` that handles filesystem and LSP. The proxy can run locally or remotely.

- **Key lesson**: If you ever want sync/storage on a server (webmail mode), the proxy pattern enables it.

---

## How Existing Email Clients Handle Scale

| Client | Search Tech | Body Storage | Performance at Scale |
|--------|------------|-------------|---------------------|
| **Apple Mail** | Core Spotlight (system-level) | Individual `.emlx` files + SQLite `Envelope Index` for metadata | Generally good. Hybrid architecture is the gold standard |
| **Thunderbird** | Gloda (SQLite) | mbox files (concatenated per folder) | Notoriously slow. FTS indexing drops to 1 msg/min under load. Users regularly rebuild index |
| **notmuch** | Xapian (C++ inverted index) | Maildir (one file per message) | Handles millions of messages well. Gold standard for local email search |
| **mu/mu4e** | Xapian | Maildir | Similar to notmuch |
| **Outlook (Windows)** | Windows Search integration | PST/OST files | Depends on OS search infrastructure |
| **Superhuman** | Server-side | Server-side | Instant (network latency only). Not applicable to local-first |
| **Gmail web** | Bigtable + custom indexing | Server-side | Sub-second on billions. Google's custom infra |

The pattern that works for local-first: **SQLite for metadata + filesystem/blob store for bodies + dedicated search engine (Xapian/tantivy) for full-text**.

Thunderbird is the cautionary tale: SQLite for everything (metadata + FTS + Gloda) breaks down at scale. Apple Mail's hybrid approach is what we should emulate, with tantivy replacing Spotlight.

---

## Migration Path

This can be done **incrementally** — each phase is independently shippable, and the app remains functional throughout.

### Phase 0: Frontend Decoupling (Facade Layer) ✅ COMPLETE

**Goal**: Create a clean API boundary so the UI never touches services/DB directly.

**Status**: Done. `src/core/` facade layer created with:
- `queries.ts` — all read operations, re-exports from `rustDb.ts`
- `mutations.ts` — all write operations, re-exports from `rustDb.ts`
- `rustDb.ts` — invoke() wrappers for all Rust commands

Components and stores import from `@/core` instead of `@/services/db/*`. Some service-layer imports remain for non-DB operations (AI, email providers, sync triggers).

**Commit**: `97bce95` — refactor: introduce src/core/ facade layer

### Phase 1: Rust-Owned Database ✅ COMPLETE

**Goal**: Eliminate IPC overhead on every DB query.

**Status**: Done. **67 Rust commands** covering all DB operations:
- `src-tauri/src/db/mod.rs` — `DbState` with `Arc<std::sync::Mutex<Connection>>` + `tokio::task::spawn_blocking` via `with_conn()` helper
- `src-tauri/src/db/queries.rs` — 25 core commands (threads, messages, labels, settings, categories, contacts)
- `src-tauri/src/db/queries_extra.rs` — 42 additional commands (contacts CRUD, filters, smart folders, smart label rules, follow-ups, quick steps, image allowlist, notification VIPs, bundle rules, thread categories)
- `src-tauri/src/db/types.rs` — all Rust struct definitions with `serde(rename_all = "camelCase")`
- `src/core/rustDb.ts` — invoke() wrappers for all 67 commands

**Key decisions**:
- Used `std::sync::Mutex` + `spawn_blocking` instead of `tokio::sync::Mutex` so rusqlite's blocking I/O doesn't hold up tokio worker threads
- Removed tauri-specta (dead weight — bindings were never exported, TS types are hand-written)
- UUID generation stays client-side (`crypto.randomUUID()`) for insert operations
- Dynamic `UPDATE SET` clauses via `dynamic_update()` helper in Rust

**What remains in TS**: Smart folder dynamic SQL queries (`querySmartFolderThreads`, `querySmartFolderUnreadCount`) — these take user-defined search queries and build SQL dynamically. They still execute via the Tauri SQL plugin. Could move to tantivy or a Rust SQL builder later.

**Commits**: `572bc16`, `218df0c`, `507a104`, `f9ee081`, `f0e95a8`, `e59d084`

### Phase 2: Separate Body Storage ✅ COMPLETE (core wired, polish remaining)

**Goal**: Remove message bodies from the metadata DB.

**Status**: Done. Separate `bodies.db` with zstd compression:

**Rust backend** (`src-tauri/src/body_store/`):
- `mod.rs` — `BodyStoreState` with separate SQLite DB (`bodies.db`), zstd level 3 compression (~3-4x ratio)
  - Schema: `message_id TEXT PRIMARY KEY, body_html BLOB, body_text BLOB`
  - Performance pragmas: WAL, 2GB mmap, 32MB cache
  - `put()` / `put_batch()` — compress and store bodies
  - `get()` / `get_batch()` — decompress and return bodies
  - `delete()` — remove bodies by message ID
  - `stats()` — count, compressed HTML/text byte totals
- `commands.rs` — 7 Tauri commands: `body_store_put`, `body_store_put_batch`, `body_store_get`, `body_store_get_batch`, `body_store_delete`, `body_store_stats`, `body_store_migrate`

**Migration**: `body_store_migrate` command reads body_html/body_text from metadata DB in batches (1000/batch), compresses into body store, then NULLs the columns in metadata DB. Runs automatically on app startup (idempotent — no-op once complete).

**Integration**:
- `upsertMessage()` now stores bodies in body store (fire-and-forget) and writes NULL to metadata DB columns
- `db_get_messages_for_thread` Rust command hydrates bodies from body store for messages with NULL body columns
- `rebuild_search_index` hydrates body_text from body store when rebuilding tantivy index
- `backfillService.ts` and `getRecentSentMessages()` hydrate bodies from body store

**Remaining polish**:
- [ ] Per-domain zstd dictionary training (currently uses generic level 3 compression)
- [ ] Drop body_html/body_text columns from messages table once migration is proven stable
- [ ] Add body store VACUUM/compaction command
- [ ] Consider LMDB (heed) or redb if SQLite body store becomes a bottleneck at scale

**Risk**: Low-medium. Requires careful migration of existing data.

### Phase 3: Tantivy Search ✅ COMPLETE (core wired, polish remaining)

**Goal**: Replace FTS5 with instant full-text search.

**Status**: End-to-end wiring complete. tantivy 0.25 integrated:

**Rust backend** (`src-tauri/src/search/`):
- `mod.rs` — `SearchState` with schema (13 fields), index management, BM25-ranked search
  - Schema: subject, from_name, from_address, to_addresses, body_text, snippet (text), date (fast), is_read/is_starred/has_attachment (fast u64 filters), message_id/thread_id/account_id (stored identifiers)
  - `search_with_filters()` — structured search with BooleanQuery combining free text, field filters, date ranges, and flag filters, always scoped by account_id
  - `index_message()` / `index_messages_batch()` — single and batch indexing with auto-dedup by message_id
  - `rebuild_search_index()` — batch-reads all messages from SQLite in 10K batches, indexes into tantivy
  - Index stored at `{app_data_dir}/search_index/`, 50MB writer heap, `ReloadPolicy::OnCommitWithDelay`
- `commands.rs` — 5 Tauri commands: `search_messages`, `index_message`, `index_messages_batch`, `delete_search_document`, `rebuild_search_index`

**TS frontend**:
- `rustDb.ts` — `searchMessages()` parses operators in TS (reuses existing `searchParser.ts`) and sends structured `SearchParams` to Rust
- `queries.ts` — exports `searchMessages` from rustDb (replaces FTS5 path)
- SearchBar and AskInbox now route through tantivy

**Sync integration**:
- Fire-and-forget `indexMessage()` calls after every message upsert in Gmail sync (`sync.ts`), IMAP sync (`imapSync.ts`), and IMAP SMTP provider (`imapSmtpProvider.ts`)

**Remaining polish**:
- [ ] Call `rebuildSearchIndex()` on first run or expose in settings UI to bootstrap index from existing messages
- [ ] Migrate smart folder queries from FTS5 to tantivy (currently still use SQLite SQL builder path)
- [ ] Drop FTS5 triggers and virtual table once tantivy is proven stable
- [ ] Port query parsing fully to Rust (currently parsed in TS, sent as structured params)
- [ ] Add streaming results via Tauri channels for large result sets
- [ ] Label filtering (currently labels live in SQLite, not tantivy — caller must post-filter)

**Commits**: `eb2bc0e`, `6d8b58f`

### Phase 4: Rust Sync Engine ✅ COMPLETE

**Goal**: Sync without IPC overhead. Entire IMAP pipeline in Rust.

**Status**: Done. IMAP sync runs entirely in Rust with a single `invoke()` call from TS. Zero IPC during the pipeline.

**Rust backend** (`src-tauri/src/sync/`):
- `mod.rs` — `SyncState` with per-account locking (prevents concurrent syncs)
- `commands.rs` — 2 Tauri commands: `sync_imap_initial`, `sync_imap_delta`
- `imap_initial.rs` — Full initial sync: list folders → per-folder (search UIDs → fetch chunks → convert → DB/body store/tantivy) → JWZ threading → store threads → cleanup orphans → update sync state. Circuit breaker for connection errors.
- `imap_delta.rs` — Delta sync: batch delta check → fetch new UIDs → process deltas → threading → store. Handles UIDVALIDITY changes (full folder resync).
- `pipeline.rs` — Shared DB operations: `store_bodies()`, `index_messages()`, `store_threads()`, `cleanup_orphan_threads()`, `sync_folders_to_labels()`, folder/account sync state management
- `convert.rs` — `ImapMessage` → `ConvertedMessage` with `MessageMeta` + `ThreadableMessage`. Local ID format: `imap-{accountId}-{folder}-{uid}`
- `folder_mapper.rs` — IMAP folder → Gmail-style label mapping (special-use flags, name fallback, syncable folder filtering)
- `config.rs` — Account reading, `ImapConfig` building, sync period settings
- `types.rs` — `SyncProgressEvent`, `ImapSyncResult`, `MessageMeta`

**TS wiring** (`syncManager.ts`):
- Feature-flagged via `use_rust_sync` setting (default: true), TS fallback preserved
- Progress events via Tauri event system (`imap-sync-progress`)
- Post-sync hooks run in TS using returned message IDs: filters (`applyFiltersToNewMessageIds`), smart labels (`applySmartLabelsToNewMessageIds`), AI categorization
- OAuth2 token refresh happens in TS before invoking Rust (Rust reads fresh token from DB)

**Commits**: `251e968`

### Phase 5: Rust Email Actions ✅ COMPLETE

**Goal**: Direct DB access for all email modify operations.

**Status**: Done. 15 Rust commands for local DB updates + 1 centralized queue command:

**Rust backend** (`src-tauri/src/email_actions/`):
- `mod.rs` — shared helpers: `remove_label`, `insert_label`, `remove_inbox_label`
- `commands.rs` — 15 action commands (archive, trash, permanent_delete, spam, mark_read, star, snooze, unsnooze, pin, unpin, mute, unmute, add_label, remove_label, move_to_folder) + `db_enqueue_pending_operation`
- Multi-statement actions (trash, spam, star, snooze, unsnooze, mute) use `unchecked_transaction()` for atomicity
- Local-only actions (pin, unpin, unmute) skip transactions

**Design decisions**:
- **DB update and queueing are separate concerns**: Rust commands only do the DB update. Queueing is handled by TS after provider execution outcome is known. This avoids double-execution (Rust can't know online/offline status) and keeps queue params in camelCase matching TS `EmailAction` discriminants
- **Centralized queue command**: `db_enqueue_pending_operation` replaces the old TS direct-SQL `enqueuePendingOperation`. All queue writes go through one Rust command
- Optimistic UI updates (Zustand store) stay in TS
- Provider execution (Gmail/IMAP) stays in TS
- `snoozeManager.ts` uses `emailActionUnsnooze` instead of direct SQL

**Risk**: Low-medium. Well-defined operations, straightforward port.

### Phase 6: Remaining Services — IN PROGRESS

Move one at a time as needed:
- ✅ Filter engine → `src-tauri/src/filters/` — `filters_evaluate` command reads filter rules from DB, matches against messages in Rust, returns per-thread actions. TS caller applies actions via `emailActions`. Pure computation + DB read in one IPC call (was: DB read IPC + JSON parse in TS + match loop).
- ✅ JWZ threading algorithm → `src-tauri/src/threading/` — `threading_build_threads` and `threading_update_threads` commands. Pure computation, no DB. Full JWZ algorithm: container-based linking, phantom parents, subject-based merging, deterministic thread IDs (djb2 hash, verified compatible with TS). Critical for Phase 4 (Rust sync).
- ✅ Categorization rule engine → `src-tauri/src/categorization/` — `categorize_thread_by_rules` and `categorize_threads_by_rules` commands. Pattern matching on sender/subject for Primary/Updates/Promotions/Social/Newsletters classification. Pure computation, no DB.
- ✅ Snooze checker → `email_action_unsnooze_batch` command — single transaction for N thread unsnoozes (was N individual IPC calls). TS checker calls once.
- ✅ Follow-up checker → `db_check_follow_up_reminders` command — fetches pending reminders, checks replies, cancels/triggers in one transaction (~30 round-trips → 1). Returns triggered reminders for TS notification dispatch.
- Remaining categorization (`categorizationManager.ts` AI fallback) — AI API is the bottleneck, not worth porting
- Scheduled send checker — Gmail API send is the bottleneck, not worth porting
- Notification manager — UI orchestration, already optimal
- Badge count — trivial (28 lines, 1-3 IPC), not worth porting

Remaining items are bottlenecked by external APIs or are already trivial orchestration. Phase 6 is effectively complete for meaningful performance gains.

**Known gaps:**
- **Filter apply post-sync is still per-action IPC**: Post-sync filter/smart-label hooks load messages from DB in TS and apply via `emailActions`. A future `filters_apply` Rust command could do matching + DB updates atomically — zero IPC per action.
- **`updateThreads` can't detect cross-thread bridges**: A new message bridging two previously separate threads may not merge them. Same limitation as the TS implementation. Full re-threading on sync would fix this but is expensive.

---

## Performance Projections

### Where the Wins Are

| Operation | Current (TS + SQL Plugin) | Projected (Rust Core) | Speedup |
|-----------|--------------------------|----------------------|---------|
| **Initial sync (10K messages)** | ~10K IPC round trips for DB writes | Zero IPC, direct DB writes | 5-10x |
| **MIME parsing (batch of 1000)** | mail-parser in Rust, results serialized to JSON, sent over IPC, deserialized in TS, sent back over IPC to DB | mail-parser → DB directly | 5-20x on the full pipeline |
| **Full-text search (10M messages)** | FTS5 trigram: 20+ seconds | Tantivy: <100ms | 200x+ |
| **Thread list query** | IPC → SQL plugin → SQLite → serialize → IPC | Direct rusqlite call | 2-5x |
| **Archive action** | IPC → SQL plugin → DB update, IPC → SQL plugin → queue insert | Direct DB update + queue insert | 2-3x |
| **Memory (10K cached messages)** | V8 heap: ~8MB (800 bytes/object with GC overhead) | Rust structs: ~2MB (200 bytes/struct) | 4x reduction |
| **Startup** | Wait for React mount → run migrations → init clients → start sync | Rust setup() hook: migrations + init + sync start before webview loads | Splash screen becomes actually useful |

### Where It Won't Matter Much

- **Network I/O**: IMAP/SMTP/Graph calls are network-bound. Rust won't make the mail server faster.
- **Email rendering**: HTML rendering happens in the webview regardless.
- **Single small operations**: One "star this thread" action is already fast. The win is in aggregate and batch operations.

### The Real Win: Enabling What Was Impossible

The point isn't just "faster." It's enabling use cases that don't work at all today:

- **60GB+ mailbox**: Works because bodies are compressed (15-20GB) and metadata is small (<2GB)
- **Instant search across millions**: Works because tantivy handles 10M+ docs in <100ms
- **Sync 500 emails without UI jank**: Works because sync runs in Rust threads, zero IPC, pushes one event when done
- **Background indexing during use**: Works because tantivy supports concurrent read/write
- **Multiple heavy accounts**: Works because Rust's memory efficiency means 5 accounts don't 5x the RAM

---

## Summary

The architecture is: **SQLite for metadata, compressed blob store for bodies, tantivy for search, Rust owns everything, React is a view layer.**

This is the Apple Mail architecture (SQLite Envelope Index + .emlx files + Spotlight) reimagined with modern Rust tooling. The key differences: tantivy instead of Spotlight (we control the search engine, not the OS), zstd compression (3-4x space savings), and a clean Tauri IPC boundary.

### Current Status (March 2026)

| Phase | Status | Key Metric |
|-------|--------|------------|
| **Phase 0**: Facade layer | ✅ Complete | `src/core/` with queries.ts, mutations.ts, rustDb.ts |
| **Phase 1**: Rust-owned DB | ✅ Complete | 67 Rust commands, all DB reads/writes via `invoke()` |
| **Phase 2**: Body storage | ✅ Core complete | `bodies.db` with zstd, 7 commands, auto-migration |
| **Phase 3**: Tantivy search | ✅ Core complete | tantivy 0.25, 5 commands, sync integration wired |
| **Phase 4**: Rust sync | ✅ Complete | 2 commands, 9 Rust modules, feature-flagged |
| **Phase 5**: Rust actions | ✅ Complete | 15 action commands + 1 centralized queue command |
| **Phase 6**: Remaining services | ✅ Effectively complete | 7 commands: filters, threading, categorization, batch unsnooze, batch follow-up |

**Phases 0, 1, 2, 3, and 5 are done.** The biggest user-visible improvements (eliminating IPC overhead on every query, instant full-text search, compressed body storage, atomic email actions) are delivered. Remaining phases (Rust sync engine, remaining services) are performance and architecture wins that can be tackled incrementally.

**Immediate next steps**:
1. Integration test Phase 4 Rust sync with a real IMAP account (`pnpm tauri dev`)
2. Phase 6 (remaining services) — continue porting independently as needed
3. Consider porting Gmail API sync to Rust (HTTP-based, lower priority than IMAP)
