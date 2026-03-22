# JMAP Sharing (RFC 9670)

**Tier**: 2 — Enhances JMAP provider parity
**Status**: 🟡 **Phases 1-2 done** — `jmap-client` fork has full RFC 9670 support (Principal CRUD, ShareNotification, `myRights`/`isSubscribed`/`shareWith` on Mailbox). Phase 1 (Session discovery + revocation) and Phase 2 (shared account sync orchestration with independent state tokens) are wired. Remaining phases (3-6) not started.

---

- **What**: JMAP's native mechanism for shared mailboxes, delegated access, and permission management. RFC 9670 (published November 2024) defines Principal objects, ShareNotification tracking, and per-mailbox ACLs — all integrated into the protocol rather than bolted on as a separate system.
- **Scope**: JMAP providers only (primarily Stalwart). The elegant counterpart to Exchange's Autodiscover-based shared mailbox discovery and IMAP's NAMESPACE/ACL extensions.

## Why JMAP Sharing is architecturally simpler than Graph/IMAP

Unlike Exchange (where discovery was the hardest problem — no clean Graph endpoint lists accessible mailboxes), **JMAP hands us shared accounts for free**. The Session object's `accounts` map already includes every account the user has access to, with `is_personal = false` for shared ones. No Autodiscover, no probing, no manual entry needed.

The sharing data model is also cleaner:

| Concept | Exchange (Graph) | IMAP | JMAP |
|---|---|---|---|
| Discovery | Autodiscover XML / manual add | NAMESPACE + MYRIGHTS | **Session `accounts` map** — automatic |
| Permission model | Full Access / Send As / Send on Behalf (3 separate grants) | RFC 4314 ACL rights string (`lrswipcxtea`) | `myRights` struct (9 typed booleans) + `shareWith` map |
| Permission changes | No notification mechanism | No notification mechanism | **ShareNotification** objects |
| Subscription | N/A | N/A | `isSubscribed` per mailbox |
| Identity | Separate lookup | N/A | Principal objects with email/type/accounts |

## What's available in jmap-client

### Session-level discovery

`Session::accounts()` → `HashMap<String, Account>` where each `Account` has:
- `name: String` — display name
- `is_personal: bool` — `false` for shared accounts
- `is_read_only: bool` — whether the current user has write access
- `account_capabilities: HashMap<String, Capabilities>` — what JMAP data types this account supports

The Session also advertises sharing support via capabilities:
- `urn:ietf:params:jmap:principals` → `PrincipalsCapabilities { current_user_principal_id, account_id_for_principal }`
- `urn:ietf:params:jmap:principals:owner` → per-account capability with `principal_id` of the account owner

### Principal objects

Full CRUD via `Principal/get`, `Principal/set`, `Principal/query`, `Principal/changes`:

```
Principal {
    id, type (Individual|Group|Resource|Location|Domain|List),
    name, description, email, timezone,
    accounts: HashMap<String, PrincipalAccount>,  // accessible JMAP accounts
    aliases: Vec<String>,
    members: Vec<String>,                         // group membership
    acl: HashMap<String, Vec<ACL>>,               // who can do what
    quota, picture, dkim, secret,
}
```

Query filters: `accountIds`, `email`, `name`, `text`, `type`, `members`, `domainName`, quota range.

Helper methods on `Client`: `individual_create()`, `principal_get()`, `principal_query()`, `principal_set_name()`, `principal_set_members()`, `principal_set_capabilities()`, `principal_destroy()`, etc.

### Mailbox sharing properties

Already present on every `Mailbox<Get>` object (fetched but currently discarded during sync):

| Property | Type | Accessor |
|---|---|---|
| `myRights` | `Option<MailboxRights>` | `mb.my_rights()` |
| `isSubscribed` | `bool` | `mb.is_subscribed()` |
| `shareWith` | `Option<HashMap<String, HashMap<ACL, bool>>>` | `mb.acl()` / `mb.take_acl()` |

`MailboxRights` fields: `may_read_items`, `may_add_items`, `may_remove_items`, `may_set_seen`, `may_set_keywords`, `may_create_child`, `may_rename`, `may_delete`, `may_submit`.

`ACL` enum: `ReadItems`, `AddItems`, `RemoveItems`, `SetSeen`, `SetKeywords`, `CreateChild`, `Rename`, `Delete`, `Submit`, `Administer`.

Set operations: `mailbox_set.is_subscribed(bool)`, `mailbox_set.acl_set(principal_id, acl_map)`, `mailbox_set.acl_update(principal_id, acl, bool)`.

Query filter: `Filter::is_subscribed(bool)` — filter mailboxes by subscription state.

### ShareNotification objects

Read-only notification records for permission changes:

```
ShareNotification {
    id, created (UTCDate),
    changed_by: ChangedBy { name, email, principal_id },
    object_type: String,            // "Mailbox", "Calendar", etc.
    object_account_id: String,
    object_id: String,
    old_rights: Option<HashMap<String, bool>>,
    new_rights: Option<HashMap<String, bool>>,
    name: Option<String>,
}
```

Methods: `ShareNotification/get`, `/changes`, `/set` (destroy-only), `/query`, `/queryChanges`.
Filters: `after`, `before`, `objectType`, `objectAccountId`.

## Current state in crates/jmap/

The JMAP provider syncs mailboxes in `sync/mailbox.rs` via `fetch_all_mailboxes()` → `MailboxGet::new(&account_id)`. This fetches all properties from the server but the code only reads `id()`, `name()`, `role()`, `parent_id()`. The `myRights`, `isSubscribed`, and `shareWith` fields are deserialized then discarded.

The Session is accessed in `ops.rs` only for `session.username()` (profile) and `session.submission_capabilities()` (scheduled send). `session.accounts()` is never called — shared accounts are invisible.

All sync operations use `request.default_account_id()` — only the primary account is synced. There is no concept of syncing against a non-primary account.

## Implementation plan

### Phase 1: Shared account discovery from Session

**Goal**: JMAP shared mailboxes auto-populate in the sidebar, matching Graph/Autodiscover behavior.

**What to do**:

1. **Read `session.accounts()` during sync** (`crates/jmap/src/sync/mod.rs`). After the initial mailbox sync completes, iterate the Session's accounts map. For each account where `is_personal == false`, extract the account ID, display name, and `is_read_only` flag.

2. **Persist as shared mailbox entries**. The `shared_mailbox_sync_state` table and `SharedMailbox` app type already exist. Use the JMAP account ID as the `mailbox_id`. The `mailbox_id` column is a provider-opaque unique identifier — Graph uses an email address, JMAP uses an account ID string like `"a1234"`. Both are unique within a Ratatoskr account and that's all the column requires. Call `upsert_shared_mailbox()` to insert with `is_sync_enabled = 1` so the shared account auto-appears in the sidebar (coarse-grained account-level discovery). Fine-grained per-mailbox subscription control is added in Phase 4.

3. **Wire into existing sidebar flow**. No sidebar changes needed — `get_shared_mailboxes()` already reads from `shared_mailbox_sync_state` and the sidebar already renders them. The only new code is in the JMAP sync pipeline.

4. **Handle revoked access**. On every sync, compare the current Session's non-personal accounts against `shared_mailbox_sync_state` rows. If a previously-known shared account is no longer in the Session (admin revoked access server-side), set `is_sync_enabled = 0` and `sync_error = "Access revoked — account no longer in JMAP Session"`. Do not delete the row — the user may have local data they want to see. The sidebar should render revoked mailboxes with a visual indicator (greyed out, warning icon) and skip them during sync.

**Key detail**: The `JmapClient` wraps `jmap_client::Client` behind an `Arc<RwLock<Arc<Client>>>`. To read the session: `let session = self.client.inner().session();` — this returns a reference to the parsed Session. The `session.accounts()` iterator yields account IDs; `session.account(id)` returns `Option<&Account>`.

**Files touched**: `crates/jmap/src/sync/mod.rs` (add shared account discovery after initial sync), `crates/jmap/src/ops.rs` (add discovery to `sync_initial` and `sync_delta` flows).

**No new dependencies or DB migrations** — reuses existing `shared_mailbox_sync_state` table.

### Phase 2: Sync mailboxes from shared accounts

**Goal**: When a shared mailbox is selected in the sidebar, its folder structure and messages are available.

**What to do**:

1. **Parameterize the sync pipeline by JMAP account ID**. Currently `sync/mailbox.rs` uses `request.default_account_id()` everywhere. The jmap-client `MailboxGet::new(account_id)` and `EmailGet::new(account_id)` already accept an arbitrary account ID — the server will return data for any account the authenticated user has access to. `SyncCtx` is an existing struct in `crates/jmap/src/sync/mod.rs` (lines 42-50) holding `client`, `account_id`, `db`, `body_store`, `inline_images`, `search`, and `progress`. Its `account_id` field is the Ratatoskr account ID used for DB writes. Add a `jmap_account_id: Option<String>` field — when `Some`, sync functions pass it to `MailboxGet::new()` / `EmailGet::new()` instead of `request.default_account_id()`.

2. **Create `jmap_shared_mailbox_sync()`** in a new `crates/jmap/src/shared_mailbox_sync.rs`, following the `graph/src/shared_mailbox_sync.rs` pattern. For each enabled shared mailbox in `shared_mailbox_sync_state`:
   - Build a `SyncCtx` with the shared account's JMAP ID as the target account.
   - Run `sync_mailboxes()` scoped to that account → labels are persisted with the shared account ID.
   - Run email sync (initial or delta based on existing JMAP sync state for that account).
   - Each shared account has independent Mailbox and Email state tokens.

3. **Add JMAP sync state per shared account**. Extend the existing `jmap_sync_state` table with a `shared_account_id TEXT` column defaulting to `NULL` for the primary account. The primary key becomes `(account_id, shared_account_id, state_type)`. This is simpler than a new table and consistent with JMAP's account-centric model — each shared account gets its own Mailbox and Email state tokens under the same Ratatoskr account. Migration: `ALTER TABLE jmap_sync_state ADD COLUMN shared_account_id TEXT; DROP INDEX IF EXISTS ...; CREATE UNIQUE INDEX ... ON jmap_sync_state(account_id, COALESCE(shared_account_id, ''), state_type);`

4. **Wire into the sync orchestration**. After the primary account's delta sync, call `sync_all_shared_mailboxes()` to iterate enabled shared mailboxes. Failures are independent — one shared mailbox timing out doesn't block others.

**Key architectural question**: Should shared account data go into the same `messages`/`threads` tables as primary data, or a separate partition? The existing Graph shared mailbox sync puts messages in the same tables with the shared mailbox's email as a scope discriminator. JMAP should follow the same pattern for consistency.

**Rate/quota concerns**: Syncing N shared accounts multiplies JMAP method calls by N. JMAP servers enforce `maxConcurrentRequests` and `maxCallsInRequest` at the session level (across all accounts), not per-account. Mitigations: (1) sync shared accounts sequentially, not in parallel — same approach as Graph's `sync_all_shared_mailboxes()` which iterates serially; (2) prioritize the currently-selected mailbox — if the user is viewing a shared mailbox, sync it first, defer others; (3) respect `Retry-After` per the JMAP spec (RFC 8620 §3.6.1) with per-account backoff; (4) use shorter initial sync lookback for shared accounts (already 30 days in the Graph implementation — same default for JMAP).

### Phase 3: Persist and surface `myRights` on mailboxes

**Goal**: The UI knows what actions the user can perform on each mailbox (hide delete button if `may_delete == false`, disable compose if `may_submit == false`, etc.).

**What to do**:

1. **Read `myRights` during mailbox sync** (`sync/mailbox.rs`). In the second pass loop, call `mb.my_rights()` and serialize the `MailboxRights` struct.

2. **DB schema**: Add 9 individual `INTEGER` columns to the `labels` table: `right_read INTEGER`, `right_add INTEGER`, `right_remove INTEGER`, `right_set_seen INTEGER`, `right_set_keywords INTEGER`, `right_create_child INTEGER`, `right_rename INTEGER`, `right_delete INTEGER`, `right_submit INTEGER`. All default `NULL` (meaning "unknown / not applicable" for non-JMAP providers). This avoids JSON serialization overhead in `get_navigation_state()` which is called on every sidebar render — 9 booleans read as integers are cheaper than deserializing a JSON blob on the hot path.

3. **Expose through core queries**. `get_navigation_state()` and `get_thread_detail()` should surface rights so the UI can gate actions. The `LabelInfo` type in core would gain an optional `my_rights` field.

4. **UI gating**. In the app crate, check `my_rights` before showing action buttons. For read-only shared mailboxes: hide move/delete/flag controls. For mailboxes without `may_submit`: hide reply/forward.

**Note**: `myRights` is also available on the primary account's mailboxes (where all rights are typically `true`). Persisting it uniformly means the UI code doesn't need to special-case "is this a shared mailbox?" — it just checks the rights.

### Phase 4: Subscription management

**Goal**: Users can subscribe/unsubscribe from shared mailboxes, controlling which ones appear in the sidebar and sync.

**What to do**:

1. **Read `isSubscribed` during mailbox sync**. Already available via `mb.is_subscribed()`. Persist alongside `myRights`.

2. **Set subscription via `Mailbox/set`**. The jmap-client `MailboxSet` builder has `.is_subscribed(bool)`. Wire a `subscribe_mailbox()` / `unsubscribe_mailbox()` function in `crates/jmap/src/ops.rs`.

3. **Filter by subscription in mailbox queries**. `Filter::is_subscribed(true)` can be used to only fetch subscribed mailboxes, reducing the set during sync.

4. **UI toggle**. A subscribe/unsubscribe action on shared mailbox entries in the sidebar or a shared mailbox management view.

**Relationship to Phase 1**: Phase 1 and Phase 4 operate at different granularity levels. Phase 1 is **account-level** discovery — "this JMAP session has access to 3 shared accounts" — and auto-enables them in the sidebar. Phase 4 is **mailbox-level** subscription — within a shared account that has 20 folders, the user subscribes to the 3 they actually care about. This mirrors Outlook's behavior: shared mailboxes auto-appear in the sidebar (account-level), but users choose which folders to expand/sync (mailbox-level). Before Phase 4 is implemented, all mailboxes within a discovered shared account are synced. After Phase 4, only `isSubscribed` mailboxes sync, and the UI gains a toggle for per-mailbox subscription via `Mailbox/set`.

### Phase 5: ShareNotification polling

**Goal**: When another user grants or revokes sharing permissions, the client detects this without requiring a full re-sync.

**What to do**:

1. **Poll `ShareNotification/changes`** during delta sync. Track the ShareNotification state token alongside Mailbox/Email state tokens.

2. **On new notifications**: If `object_type == "Mailbox"`, re-check the Session for account changes. A new share grant may add a new account to the Session; a revocation may remove one.

3. **Surface in UI**. Show a toast or notification when sharing permissions change: "Alice shared Inbox with you" or "Access to support@ was revoked."

4. **Acknowledge notifications**. ShareNotification/set supports destroy-only — destroy notifications after they've been shown to the user.

**DB schema**: A `jmap_share_notification_state` row in the existing `jmap_sync_state` table (state_type = "ShareNotification").

### Phase 6: Principal-based identity resolution

**Goal**: When replying from a shared mailbox, auto-select the correct From identity based on the shared account's principal.

**What to do**:

1. **Fetch the current user's Principal** via `Principal/get` using the `current_user_principal_id` from `PrincipalsCapabilities`. Cache the principal's `accounts` map.

2. **Map shared account → email address**. The Principal's `accounts` map lists all accessible accounts with metadata. The shared account's owner can be resolved via `principals:owner` capability on that account → `principal_id` → `Principal/get` → `email`.

3. **Feed into Send identity auto-selection**. When the user composes or replies from a shared mailbox context, set From to the shared mailbox's email address. This is the JMAP equivalent of Graph's `send_as_shared_mailbox()`.

4. **Check `may_submit`**. Only offer Send identity if the mailbox's `myRights.may_submit == true`.

## Server compatibility notes

- **Stalwart**: Full RFC 9670 support documented. The primary target for this implementation.
- **Fastmail**: Fastmail's sharing model predates RFC 9670 and uses a proprietary mechanism. Phase 1 (Session `accounts` map) is likely to work — the accounts map is core JMAP (RFC 8620), and Fastmail does expose shared accounts in it. Phase 6 (Principal-based identity resolution) is unlikely to work — Fastmail doesn't implement `urn:ietf:params:jmap:principals`. Phases 3-4 (`myRights`, `isSubscribed`) are RFC 9670 properties — Fastmail may support them since their proprietary sharing model predates the RFC and influenced it, but this is not guaranteed. Phase 5 (ShareNotification) is unlikely. Needs testing to confirm.
- **Cyrus IMAP (JMAP mode)**: RFC 9670 support status unknown. The JMAP implementation is less mature than Stalwart's.

All phases should gracefully degrade when the server doesn't advertise the required capability. The degradation boundary is:
- **Phase 1 works without RFC 9670** — Session `accounts` map is core JMAP (RFC 8620). Any conformant server exposes shared accounts here.
- **Phase 2 works without RFC 9670** — `Mailbox/get` and `Email/get` accept arbitrary account IDs per RFC 8620 §5.
- **Phase 3 (`myRights`) requires RFC 9670** — `myRights` and `isSubscribed` are defined in RFC 9670 §2.1, not RFC 8621. RFC 8621 defines the Mailbox object but without sharing properties. A server that only implements RFC 8621 will return `null` for these fields. Persist `NULL` and treat as "all rights granted" in the UI.
- **Phase 4 (`isSubscribed` / `Mailbox/set`) requires RFC 9670** — same RFC attribution as Phase 3. The `is_subscribed` setter and `Filter::is_subscribed` query filter are RFC 9670 features.
- **Phases 5-6 require RFC 9670** — ShareNotification and Principal objects are defined exclusively in RFC 9670. Check for `urn:ietf:params:jmap:principals` before attempting these.

## Implementation priority

| Phase | Difficulty | Impact | Dependencies |
|---|---|---|---|
| 1. Session account discovery | Low | High — shared mailboxes auto-appear | None |
| 2. Shared account sync | Medium | Critical — makes selection functional | Phase 1 + thread loading handler from shared-mailboxes.md |
| 3. `myRights` persistence | Low | Medium — enables UI permission gating | None (can be done independently) |
| 4. Subscription management | Low-Medium | Medium — user control over sidebar | Phase 1 |
| 5. ShareNotification polling | Medium | Low-Medium — nice-to-have for awareness | Phase 1 |
| 6. Principal identity resolution | Medium-High | High — correct From on send | Phase 2 + compose identity work from shared-mailboxes.md |

Phase 1 is the natural starting point — ~50 lines of code in the sync pipeline, no new tables, immediately visible in the sidebar.
