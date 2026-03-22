# Categories (Color Flags)

> **Superseded by [Labels Unification](../labels-unification/problem-statement.md).**
> Categories are now unified into the labels system as `label_kind = 'tag'` entries. Exchange categories, IMAP keywords, and JMAP keywords all sync to the `labels` table. See the labels unification spec for the current design. This roadmap doc is retained for historical context.

**Tier**: 1 — Blocks switching from Outlook
**Status**: ✅ **Backend complete** — Unified into labels system (Phases 1-5 of labels unification). — All provider backends implemented. Exchange Graph master category list sync (`crates/graph/src/category_sync.rs`), Gmail label-to-category sync with hex colors (`crates/gmail/src/sync/labels.rs`), JMAP keyword-to-category mapping (`crates/jmap/src/sync/mailbox.rs`, `crates/jmap/src/sync/storage.rs`), IMAP PERMANENTFLAGS detection (`crates/imap/src/client/`, `crates/imap/src/raw.rs`). Unified color model using Exchange's 25 presets as canonical palette with nearest-match mapping (`crates/label-colors/src/category_colors.rs`). `ProviderOps` trait has `apply_category`/`remove_category` mutation methods (`crates/provider-utils/src/ops.rs`), implemented in each provider crate (`crates/graph/src/ops/mod.rs`, `crates/gmail/src/ops.rs`, `crates/jmap/src/ops.rs`, `crates/imap/src/ops.rs`). `message_categories` join table populated during sync for all three API providers (Graph, Gmail, JMAP). `categories` table with full schema in `crates/db/src/db/migrations.rs` (display_name, color_preset, color_bg, color_fg, provider_id, sync_state). **Still missing**: category picker UI, user-initiated apply/remove from UI, IMAP keyword write-back for categories.

---

- **What**: Per-user string labels with associated colors, applied to messages
- **Scope**: Per-user on personal mailboxes; shared visibility on shared mailboxes and public folders

## Cross-provider behavior

| Provider | Native support | Behavior |
|---|---|---|
| Exchange (Graph) | Full — `categories` on messages, master list via `/me/outlook/masterCategories` | Sync master list + per-message categories bidirectionally |
| Gmail API | Labels function as both folders and categories. Color supported. | Map Gmail labels to categories where label is not a system/folder label. Imperfect — Gmail's model conflates the two concepts. |
| JMAP | `keywords` on emails — arbitrary string keys, boolean values. No color. | Use keywords as category names, store colors locally. |
| IMAP | `FLAGS`/keywords — server support varies wildly, many servers limit to system flags only | Local-only categories with IMAP flag sync as best-effort. |

## Pain points

- Gmail label/category/folder conflation: need heuristics to decide which labels are "categories" vs structural folders. System labels (`INBOX`, `SENT`, `TRASH`) are obvious, but user-created labels are ambiguous.
- IMAP keyword support is unreliable: some servers silently drop custom keywords, others have hard limits on keyword count. Must detect and fall back to local-only.
- Color mapping: Exchange has a fixed set of preset colors. Gmail has its own color palette. JMAP/IMAP have no color concept. Need a unified color model that round-trips cleanly to Exchange and degrades gracefully elsewhere.
- Shared mailbox categories: on Exchange, categories applied to messages in a shared mailbox are visible to all users with access. This is a feature users rely on for team triage ("I marked it Red, that means it's handled"). Must preserve this behavior for Graph accounts.
- Multi-account category conflicts: user has "Urgent" as red on Account A and blue on Account B. The category picker needs to handle this without confusion.

## Work

Sync master category list per account, display on messages, allow apply/remove, persist locally, round-trip to server where supported. Local-only fallback for IMAP.

---

## Research

**Date**: March 2026
**Context**: Ground-up implementation for iced (pure Rust) rewrite. This research covers all four provider backends and the decisions needed for a unified category model.

---

### 1. Exchange Graph API Surface for Categories

Exchange has the most complete category implementation of any provider. Categories are a first-class concept with a per-user master list and per-item assignment.

#### Master Category List

**Endpoint**: `GET /me/outlook/masterCategories` (or `/users/{id}/outlook/masterCategories` for shared/delegated)

Returns `outlookCategory` objects with two fields:
- `displayName` (String) — unique per user, **immutable after creation** (read-only)
- `color` (categoryColor enum) — one of 25 preset constants plus `None`

Each category also has an opaque `id` (GUID). New accounts get 6 default categories ("Red category", "Orange category", etc.).

**CRUD**:
- **Create**: `POST /me/outlook/masterCategories` with `{ "displayName": "...", "color": "preset9" }`. Returns 201 with the new category including its `id`. `displayName` must be unique — duplicates return a conflict error.
- **Update**: `PATCH /me/outlook/masterCategories/{id}` — **can only change `color`**. Cannot rename. This is a significant constraint: to "rename" a category, you must delete the old one, create a new one, and re-apply it to all affected messages.
- **Delete**: `DELETE /me/outlook/masterCategories/{id}`. Does not remove the category string from messages — messages retain the `displayName` in their `categories` array even after the master list entry is deleted. This creates orphaned category references.

**Permissions**: `MailboxSettings.Read` for list/get, `MailboxSettings.ReadWrite` for create/update/delete.

#### The 25 Preset Colors

Colors are abstract constants (`preset0` through `preset24`), not hex values. The actual rendered color depends on the Outlook client. The documented mapping for Outlook desktop:

| Constant | Color Name | Constant | Color Name |
|----------|------------|----------|------------|
| `None` | No color | `preset12` | Gray |
| `preset0` | Red | `preset13` | DarkGray |
| `preset1` | Orange | `preset14` | Black |
| `preset2` | Brown | `preset15` | DarkRed |
| `preset3` | Yellow | `preset16` | DarkOrange |
| `preset4` | Green | `preset17` | DarkBrown |
| `preset5` | Teal | `preset18` | DarkYellow |
| `preset6` | Olive | `preset19` | DarkGreen |
| `preset7` | Blue | `preset20` | DarkTeal |
| `preset8` | Purple | `preset21` | DarkOlive |
| `preset9` | Cranberry | `preset22` | DarkBlue |
| `preset10` | Steel | `preset23` | DarkPurple |
| `preset11` | DarkSteel | `preset24` | DarkCranberry |

**Key gotcha**: Microsoft documents these as logical names, not hex values. Outlook for Windows, Outlook for Mac, OWA, and the new Outlook all render these slightly differently. We will need to pick concrete hex values for each preset and accept that they will not be pixel-identical to what Outlook shows.

#### Per-Message Categories

The `categories` field on a message is a `String[]` — an array of `displayName` strings (not IDs). This is a critical design detail:

- **Applying**: `PATCH /me/messages/{id}` with `{ "categories": ["Red category", "Project expenses"] }`. This is a **full replacement** — you must include the complete desired list, not just additions.
- **Removing**: Same PATCH with the category omitted from the array.
- **Gotcha**: Categories on messages are matched by string name, not by master list ID. If a user renames a category in Outlook (which Outlook does by delete + recreate behind the scenes), existing messages retain the old name string.

#### Shared Mailboxes

Access a shared mailbox's categories via `/users/{shared-mailbox-id}/outlook/masterCategories`. Each user accessing a shared mailbox sees their **own** master category list for color mapping, but categories applied to messages in the shared mailbox are visible to all users. This means User A applies "Urgent" (red) and User B sees the string "Urgent" but may display it with a different color if their master list maps "Urgent" differently (or doesn't have it at all).

#### Delta Sync

Graph's message delta endpoint (`GET /me/mailFolders/{id}/messages/delta`) returns messages with changed properties, including `categories`. You can use `$select=categories` to minimize payload. There is **no dedicated delta endpoint for the master category list itself** — you must poll it in full.

#### Real-World Gotchas

1. **Read-after-write latency**: After PATCHing categories on a message, a subsequent GET may return stale data for a few hundred milliseconds. The Graph API has eventual consistency for some properties.
2. **Batch limits**: When applying a category to all messages in a thread, each message requires a separate GET (to read current categories) + PATCH. For large threads this is N*2 HTTP requests. Graph's JSON batching (`POST /$batch`) can help but has a 20-request-per-batch limit.
3. **The `inferenceClassification` field is not a category** despite appearing similar. It is a separate Focused/Other classification managed by Exchange ML and should not be conflated with user categories.

---

### 2. Gmail Label-as-Category Mapping

Gmail has no concept of "categories" separate from labels. Everything — system folders, user folders, user categories, and the tab categories — is a label. This is the core mapping challenge.

#### Distinguishing Category-Labels from Folder-Labels

There is **no API field** that marks a label as "used as a category" vs "used as a folder." The only signals available:

1. **`type` field**: `"system"` vs `"user"`. System labels include folders (`INBOX`, `SENT`, `TRASH`, `DRAFT`, `SPAM`, `STARRED`, `UNREAD`, `IMPORTANT`) and the tab categories (`CATEGORY_PERSONAL`, `CATEGORY_SOCIAL`, `CATEGORY_PROMOTIONS`, `CATEGORY_UPDATES`, `CATEGORY_FORUMS`). All user-created labels have type `"user"`.

2. **`labelListVisibility`**: `"labelShow"`, `"labelShowIfUnread"`, or `"labelHide"`. Labels visible in the label list are more likely to be category-like; hidden labels are often automation artifacts.

3. **`messageListVisibility`**: `"show"` or `"hide"`. Controls whether the label appears as a chip on messages in the message list. Labels with `"show"` here are more category-like.

4. **Nesting**: Gmail uses `/` in label names to indicate hierarchy (e.g., `"Work/Projects/Active"`). Nested labels are structurally folder-like. A label with no `/` and `messageListVisibility: "show"` is more likely a category.

**Practical heuristic**: Treat a user label as a "category" if it has `messageListVisibility: "show"` and is not nested (no `/` in name). Treat nested labels as folders. This is imperfect — some users create flat labels as folders and nested labels as categories — but it's the best available signal. Thunderbird does not attempt this distinction at all and shows all labels uniformly.

#### The `CATEGORY_*` System Labels

Gmail has five system labels: `CATEGORY_PERSONAL`, `CATEGORY_SOCIAL`, `CATEGORY_PROMOTIONS`, `CATEGORY_UPDATES`, `CATEGORY_FORUMS`. These correspond to Gmail's inbox tabs.

**Are they useful?** Mostly misleading for our purposes. They represent Gmail's automated inbox triage (equivalent to Exchange's Focused/Other), not user-applied categories. They should **not** appear in a user-facing category picker.

#### Gmail Label Colors

The `color` object on a label has two fields:
- `textColor` — hex string like `"#000000"`
- `backgroundColor` — hex string like `"#fb4c2f"`

Both must be set together from a **restricted palette of ~92 predefined hex values**. You cannot use arbitrary hex colors — the API rejects values not in the palette. Unlike Exchange's 25 named presets, Gmail offers ~92 color values that must be used as (textColor, backgroundColor) pairs.

#### Label CRUD

- `GET /gmail/v1/users/me/labels` — list all labels
- `POST /gmail/v1/users/me/labels` — create with name, visibility, and optional color
- `PATCH /gmail/v1/users/me/labels/{id}` — update name, visibility, color (unlike Exchange, **renaming is supported**)
- `DELETE /gmail/v1/users/me/labels/{id}` — removes the label and **also removes it from all messages**

**Applying labels to messages**: `POST /gmail/v1/users/me/messages/{id}/modify` with `addLabelIds`/`removeLabelIds`. This is additive/subtractive, unlike Exchange's full-replacement PATCH.

**Limit**: Maximum 10,000 labels per mailbox.

---

### 3. JMAP Keywords

JMAP's keyword system (RFC 8621) is the thinnest abstraction of the four providers.

#### How Keywords Work

Keywords on an Email object are a `String[Boolean]` map — keys are keyword strings, values are always `true`. A keyword is "set" by being present in the map and "unset" by being absent. There is no "master keyword list" concept.

IANA-registered keywords with semantic meaning: `$seen`, `$flagged`, `$draft`, `$answered`, `$forwarded`, `$phishing`, `$junk`, `$notjunk`.

Custom keywords can be any string of 1-255 ASCII characters (range %x21-%x7e), excluding `( ) { ] % * " \`. Keywords starting with `$` are reserved for IANA-registered or vendor-specific use.

#### Setting Keywords via Email/set

The `jmap-client` crate provides `.keyword("name", true/false)` on the Email update builder. Adding a category: `set_req.update(eid).keyword("my-category", true)`. Removing: `.keyword("my-category", false)`.

#### Limits

RFC 8621 defines a `tooManyKeywords` error but does not mandate a specific numeric limit — it is server-dependent. Stalwart has no documented keyword count limit.

#### Color Storage Problem

JMAP has **no color concept for keywords**. Options:

1. **Local-only color storage**: Store `keyword → color` mapping in the local SQLite database per account. Simple, but colors do not sync across devices. This is the pragmatic choice for an initial implementation.
2. **JMAP annotations** (if supported): Store a JSON blob of `{ keyword: color }` mappings in a well-known annotation key. Non-standard, server-dependent.
3. **Convention-based encoding**: Encode color in the keyword name itself. Ugly, fragile, not recommended.

**Recommendation**: Local-only color storage with the option to add JMAP annotation sync later.

---

### 4. IMAP Keywords/Flags

#### RFC 3501 Keyword Basics

System flags (`\Seen`, `\Answered`, etc.) are universally supported. Keywords (arbitrary strings without `\` prefix) vary by server.

The `PERMANENTFLAGS` response code in `SELECT` tells the client what flags can be stored:
1. **Contains `\*`**: Server allows arbitrary custom keywords — best case
2. **Lists specific keywords**: Server supports those keywords only
3. **Missing entirely**: Client should assume all flags can be changed (per RFC)

#### Real-World Server Support

| Server | Custom Keywords | Limits | Notes |
|--------|----------------|--------|-------|
| **Dovecot** (Maildir) | Yes, with `\*` | **26 per mailbox** — keywords a-z stored in Maildir filenames | Most common self-hosted server |
| **Dovecot** (dbox/sdbox/mdbox) | Yes | No hard limit | Better for keyword-heavy use |
| **Exchange/Outlook.com** (IMAP) | Limited | Categories **not exposed** as IMAP keywords — only via Graph API | |
| **Gmail** (IMAP) | Via `X-GM-LABELS` | Proprietary GIMAP extension, not standard keywords | |
| **Yahoo** | No | Only system flags | |
| **iCloud** | Limited | Supports `\*` but behavior inconsistent | |
| **Fastmail** (IMAP) | Yes | Generous limits | But should use JMAP instead |
| **GMX/Web.de** | No | System flags only | |

#### Detection Strategy

Parse `PERMANENTFLAGS` from SELECT response. If `\*` is present, custom keywords are supported. Otherwise fall back to local-only categories.

`async-imap` exposes PERMANENTFLAGS in the SELECT response. Implementation means parsing it during SELECT, using `UID STORE +FLAGS (keyword_name)` / `-FLAGS` for servers that support `\*`, and falling back to local-only for servers without support.

---

### 5. Color Model Unification

#### Exchange Preset Hex Values

Microsoft does not publish official hex values. These are approximations from Outlook Web App CSS:

| Preset | Name | Approximate Hex | Preset | Name | Approximate Hex |
|--------|------|----------------|--------|------|----------------|
| preset0 | Red | #e7514a | preset13 | DarkGray | #6c6c6c |
| preset1 | Orange | #f09e38 | preset14 | Black | #3a3a3a |
| preset2 | Brown | #ab7b4f | preset15 | DarkRed | #a63019 |
| preset3 | Yellow | #c4b626 | preset16 | DarkOrange | #bd6e1b |
| preset4 | Green | #43a556 | preset17 | DarkBrown | #7e5a31 |
| preset5 | Teal | #2ea1a3 | preset18 | DarkYellow | #938516 |
| preset6 | Olive | #777c3d | preset19 | DarkGreen | #2a7b3f |
| preset7 | Blue | #3e72b8 | preset20 | DarkTeal | #1d7476 |
| preset8 | Purple | #9d58b0 | preset21 | DarkOlive | #535b23 |
| preset9 | Cranberry | #c33655 | preset22 | DarkBlue | #274e8d |
| preset10 | Steel | #658799 | preset23 | DarkPurple | #6e3e80 |
| preset11 | DarkSteel | #4b6876 | preset24 | DarkCranberry | #8f2539 |
| preset12 | Gray | #9b9b9b | | | |

#### Unified Color Model Options

**Option A: Exchange presets as the canonical palette.** Map every category color to one of the 25 presets. Perfect Exchange round-trip. Gmail users lose precise color choices when viewed in Ratatoskr.

- Pros: Perfect Exchange fidelity (the provider that matters most for enterprise users). 25 colors is plenty — Outlook itself only offers these.
- Cons: Gmail's ~92 colors get quantized down.

**Option B: Arbitrary hex colors internally, with nearest-match mapping.** Store colors as `(bg_hex, fg_hex)`. Map to Exchange presets on write (nearest perceptual match using CIE Delta E). Map to Gmail palette on write.

- Pros: More flexible. Gmail colors round-trip better.
- Cons: Exchange colors drift on write. Requires a color distance function.

**Option C: Exchange presets + "custom" overflow.** Exchange presets as the primary palette. Allow "custom" colors stored locally, mapped to nearest-preset for Exchange.

- Pros: Good UX for Exchange users, flexibility for others.
- Cons: Most complex.

**Recommendation**: Option A for initial implementation. Enterprise users (primary audience) are on Exchange. Revisit Option C if user feedback demands it.

#### How Thunderbird Handles This

Thunderbird's tag system: tags have user-configurable colors stored **locally** in `prefs.js`. Colors do **not** sync — they are per-installation only. No integration with Exchange categories or Gmail label colors. If you use Thunderbird on two machines, you get the same keywords but potentially different colors. We should do better by syncing colors to Exchange/Gmail where the API supports it.

---

### 6. Data Model Options

#### Schema for a Provider-Agnostic Category System

```sql
categories
  id              TEXT PRIMARY KEY    -- UUID, local
  account_id      TEXT NOT NULL       -- FK to accounts
  display_name    TEXT NOT NULL       -- "Urgent", "Project X"
  color_preset    TEXT                -- Exchange preset ID ("preset0"..."preset24", or NULL)
  color_bg        TEXT                -- Hex background color for display
  color_fg        TEXT                -- Hex text/foreground color for display
  provider_id     TEXT                -- Provider-side ID (Graph GUID, Gmail label ID, JMAP keyword, IMAP keyword)
  sync_state      TEXT                -- "synced", "pending_create", "pending_delete", "local_only"
  sort_order      INTEGER DEFAULT 0
  UNIQUE(account_id, display_name)

message_categories
  message_id      TEXT NOT NULL
  account_id      TEXT NOT NULL
  category_id     TEXT NOT NULL       -- FK to categories.id
  PRIMARY KEY (account_id, message_id, category_id)
```

Separate from folder/label tables. Categories and folders are logically distinct even though Gmail conflates them.

#### Master List Per Account vs Global

**Per account** is the only viable option. Exchange categories are per-user-per-mailbox. Gmail labels are per-account. JMAP keywords are per-account. A "global" list adds many-to-many complexity without benefit.

#### Handling Cross-Account Conflicts

Same name, different colors across accounts: show both with their respective colors, qualified by account name in the picker. Same name, same color: optionally merge in display but track per-account internally.

---

### 7. Relevant Rust Crates

#### Color Manipulation

**[`palette`](https://crates.io/crates/palette)** v0.7.6 — 408K monthly downloads. Full color space library with perceptual distance calculations. **Useful only if** we go with Option B/C (nearest-match mapping). For Option A (Exchange presets as canonical), a 25-entry const array is all we need.

#### Provider Client Crates

No new crates needed:
- **Graph**: Existing `GraphClient` supports `GET /me/outlook/masterCategories` and `PATCH /me/messages/{id}` with `categories`. Just needs a `sync_master_categories` function.
- **Gmail**: Existing `GmailClient` has `list_labels`, `create_label`, `update_label`, `delete_label` with color support.
- **JMAP**: `jmap-client` 0.4 has `.keyword("name", bool)` on Email update builders.
- **IMAP**: `async-imap` supports `UID STORE +FLAGS`/`-FLAGS`. Need to add PERMANENTFLAGS detection.
