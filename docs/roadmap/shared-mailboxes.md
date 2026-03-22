# Shared / Delegated Mailboxes

**Tier**: 1 — Blocks switching from Outlook
**Status**: 🟡 **Partially implemented** — Exchange (Graph) shared mailbox read/write/sync, Autodiscover discovery, and IMAP shared namespace discovery are implemented in core. Sidebar integration done (2026-03-22): shared mailboxes auto-populate in scope dropdown from Autodiscover results. JMAP Sharing in progress. Gmail delegation blocked (API limitation). Remaining: thread loading on shared mailbox selection, compose identity auto-selection, per-mailbox sync config.

---

- **What**: Any mailbox the user has delegate access to — shared mailboxes, other users' mailboxes, resource mailboxes (rooms/equipment). In enterprise M365, these auto-appear in Outlook when a user is granted Full Access.
- **Scope**: **Adoption blocker**. Enterprise clients cannot switch until this works. Many M365 orgs have dozens of shared/delegated mailboxes per user. Users switch between personal and delegated mailboxes constantly throughout the day.

## What actually auto-appears in Outlook

When you add a corporate Exchange account, Outlook may auto-populate additional mailboxes in the sidebar. These can be any of:

1. **Shared Mailboxes** — no license, no direct login. Created by admins for team use (support@, invoicing@, sales@). Delegates are granted access and Exchange **auto-maps** them into Outlook.
2. **User Mailboxes with Full Access** — a fully licensed user mailbox (e.g., `invoicing@company.com` that's actually a regular user account) where the current user has been granted **Full Access**. Exchange auto-maps these identically to shared mailboxes. From the user's perspective in Outlook, they look the same.
3. **Resource Mailboxes** — room or equipment mailboxes. Less commonly auto-mapped, but possible if Full Access was granted for management purposes.

**Exchange auto-mapping does not distinguish between these types.** If a user has Full Access to any mailbox — shared, user, or resource — Exchange can auto-map it. The Graph API treats them uniformly: access via `/users/{mailbox-id}/messages` regardless of type.

This means Ratatoskr doesn't need to care what *kind* of mailbox it is. The implementation is mailbox-type-agnostic: discover what the user has access to, present them uniformly, respect permissions.

## Permission types (Exchange)

Three separate permission grants that may or may not overlap:

| Permission | What it allows | Typical use |
|---|---|---|
| **Full Access** | Read, write, delete messages in the mailbox. Triggers auto-mapping. | Shared mailboxes, exec assistant accessing boss's inbox |
| **Send As** | Send email with the mailbox's address as the From. Recipient cannot tell it wasn't the mailbox owner. | Shared mailboxes, service accounts |
| **Send on Behalf** | Send email on behalf of the mailbox. From shows "User on behalf of Mailbox". | Exec assistants, team delegation |

A user may have Full Access but not Send As (can read but not impersonate), or Send As but not Full Access (can send from but not read — rare but possible). The client must check each permission independently.

## Cross-provider behavior

| Provider | Mechanism | Discovery |
|---|---|---|
| Exchange (Graph) | Full Access / Send As / Send on Behalf grants. Auto-mapping. All mailbox types accessed uniformly via `/users/{id}/messages`. | **No single Graph endpoint lists all accessible mailboxes.** Auto-mapping info is in EWS (`GetMailboxAutoMapping`), not cleanly exposed in Graph. Options: (a) EWS fallback for discovery, (b) user manually adds delegated mailboxes by email address, (c) attempt to access known mailbox IDs and check for 403. |
| Gmail API | Account-level delegation — full inbox access to another user's account | `users.settings.delegates.list` for outbound; inbound delegation is account-level |
| JMAP | ACL-based sharing per mailbox | Server-dependent; Stalwart supports JMAP Sharing (RFC 9670) |
| IMAP | ACL extension (RFC 4314) — per-folder permissions | `GETACL`/`LISTRIGHTS` commands; server support varies widely |

## Pain points

- **Discovery is the hardest problem (Exchange)**: there is no clean Graph API to ask "what mailboxes does this user have access to?" Auto-mapping is an Exchange/Outlook concept not fully surfaced in Graph. Options: (a) call EWS autodiscover (additional protocol dependency), (b) let the user manually add delegated mailboxes by typing the email address (Outlook does this too — "Open Another Mailbox"), (c) try to hit `/users/{email}/mailFolders` for known mailboxes and see if it succeeds or 403s. Likely need option (b) as the baseline with (a) as an enhancement.
- **Identity switching on send**: when replying from a shared mailbox, the "From" address must be the shared mailbox, not the user's personal address. Users frequently forget to check this in other clients. Ratatoskr should auto-set the From based on which mailbox the message was read in — this is a place to be better than Outlook. Must also distinguish Send As vs Send on Behalf (different headers, different recipient experience).
- **Notification routing**: new mail in a shared mailbox — does every delegate get notified? Exchange has per-user notification settings for shared mailboxes. The client needs to respect these. Spamming 10 delegates with notifications for every incoming support@ email is unusable.
- **Shared state visibility**: when User A reads/flags/categorizes/moves a message in a shared mailbox, User B must see that state change. This is the core value of shared mailboxes — team triage. Categories on shared mailbox messages are shared (unlike personal mailboxes). Flags may or may not be shared depending on Exchange configuration.
- **Sent Items routing**: when sending from a shared mailbox, where does the Sent copy go? Exchange has a setting: copy to the sender's Sent Items, the shared mailbox's Sent Items, or both. Must respect this per-mailbox setting via Graph.
- **Multiple delegated mailboxes at scale**: enterprise users may have access to 10+ mailboxes. The sidebar needs collapsible sections, unread counts per delegated mailbox, ability to hide/reorder/pin. Some mailboxes are checked constantly (support@), others rarely (the old invoicing@ they still have access to).
- **Offline sync scope**: syncing every message from every delegated mailbox is excessive. Need configurable sync depth per mailbox (e.g., last 30 days for support@, full sync for the exec's inbox, no sync for rarely-used ones — fetch on demand).
- **Auth scope**: accessing another user's mailbox via Graph requires the right OAuth scopes (`Mail.Read.Shared`, `Mail.ReadWrite.Shared`, `Mail.Send.Shared`). These must be requested during auth. If the app registration doesn't have these scopes, delegated mailbox access silently fails.
- **IMAP ACL inconsistency**: RFC 4314 defines ACLs, but implementation varies wildly. Some servers support it fully, some partially, some not at all. Need capability detection and graceful degradation.
- **Gmail delegation quirks**: Gmail delegation is account-level (full inbox access), not per-folder. The delegated account appears as a separate "account" in the Gmail UI. Mapping this to the shared-mailbox mental model requires special handling — it's closer to "additional account" than "shared folder."

## Implementation Status

### Done

**Exchange (Graph) — shared mailbox access and sync:**
- `*.Shared` OAuth scopes (`Mail.ReadWrite.Shared`, `Mail.Send.Shared`, `Mail.Read.Shared`) requested during auth (`core/src/oauth.rs`, `core/src/discovery/registry.rs`).
- `GraphClient::for_shared_mailbox(mailbox_id)` creates a scoped client that shares the parent's HTTP client, token, and semaphore but has its own folder map, sync cycle, and category lock (`graph/src/client.rs`).
- `GraphClient::api_path_prefix()` returns `/me` for primary or `/users/{mailbox_id}` for shared. All operations in `graph/src/ops/` use `self.me()` (delegates to `api_path_prefix()`), and all sync URLs in `graph/src/sync/` use `client.api_path_prefix()` — so every API call (read, write, move, delete, folder list, delta sync) transparently works against shared mailboxes.
- `send_as_shared_mailbox()` and `send_on_behalf_of()` in `graph/src/ops/mod.rs` implement Send As (only `from` set) and Send on Behalf (`from` + `sender` set) via `POST /users/{shared}/messages` + `/send`.
- Autodiscover XML discovery (`graph/src/autodiscover.rs`): `discover_shared_mailboxes()` calls `outlook.office365.com/autodiscover/autodiscover.xml` with OAuth token, parses `AlternativeMailbox` elements via `quick-xml`. Returns SMTP address, display name, and type (Delegate, TeamMailbox, etc.).
- Per-shared-mailbox sync orchestration (`graph/src/shared_mailbox_sync.rs`): `sync_shared_mailbox()` creates a scoped client and runs initial (30-day lookback) or delta sync depending on existing delta tokens. `sync_all_shared_mailboxes()` iterates all enabled shared mailboxes; failures are independent.
- DB schema (`db/src/db/migrations.rs`): migration v51 adds `graph_shared_mailbox_delta_tokens` (per-mailbox, per-folder delta links) and `shared_mailbox_sync_state` (enable/disable sync, last sync time, error tracking).
- Sync state management (`sync/src/state.rs`): `enable_shared_mailbox_sync()`, `disable_shared_mailbox_sync()`, `get_enabled_shared_mailboxes()`, `update_shared_mailbox_sync_status()`, plus CRUD for shared mailbox delta tokens.

**IMAP — namespace and ACL discovery:**
- NAMESPACE command (RFC 2342): `discover_namespaces()` in `imap/src/connection.rs` sends the raw `NAMESPACE` command and parses the response into `NamespaceInfo` (personal, other_users, shared entries with prefix and delimiter). Full parser with tests for standard, NIL, and multi-entry responses.
- `classify_folder_namespace()` maps a folder path to `NamespaceType` (Personal, OtherUsers, Shared) by prefix matching.
- `list_shared_folders()` in `imap/src/client/mod.rs` lists folders under other-users and shared namespace prefixes via `LIST {prefix}*`, annotating each with its `NamespaceType`.
- MYRIGHTS (RFC 4314): `discover_myrights()` in `imap/src/connection.rs` queries the authenticated user's rights on a folder. Parses the `AclRight` variants into a compact rights string.
- DB schema (`db/src/db/migrations.rs`): migration v54 adds `namespace_type TEXT` column to `labels`, storing the IMAP namespace classification per folder.
- Types (`imap/src/types.rs`): `NamespaceType` enum, `NamespaceEntry`, `NamespaceInfo` structs. `ImapFolder` has an optional `namespace_type` field.

**Sidebar integration (2026-03-22):**
- `SharedMailbox` type and `Db::get_shared_mailboxes()` query in app crate (`app/src/db/types.rs`, `app/src/db/accounts.rs`).
- `Db::upsert_shared_mailbox()` for persisting Autodiscover results with auto-enable.
- Shared mailboxes rendered in sidebar scope dropdown with users icon (`app/src/ui/sidebar.rs`).
- `SidebarMessage::SelectSharedMailbox` / `SidebarEvent::SharedMailboxSelected` for selection.
- Shared mailboxes loaded at boot from `shared_mailbox_sync_state` table.
- When Autodiscover discovers shared mailboxes during Graph sync, they auto-populate in the sidebar.

### Remaining

- **Gmail delegation**: Account-level delegation is not implementable via public Gmail API (cannot discover inbound delegation; accessing delegated mailbox requires domain-wide delegation or internal session mechanisms). Documented as a known limitation. Send-As aliases work.
- **JMAP Sharing (RFC 9670)**: In progress (being implemented separately).
- **App-level shared mailbox selection handler**: `SharedMailboxSelected` event is emitted but the App doesn't yet load threads for the selected shared mailbox. Needs navigation state scoping.
- **Compose identity auto-selection**: When replying from shared mailbox context, auto-set From to shared mailbox address. `send_as_shared_mailbox()` and `send_on_behalf_of()` APIs exist.
- **Configurable sync depth per shared mailbox**: Currently hardcoded to 30 days initial lookback. No per-mailbox sync depth setting.
- **Notification routing**: Client-side per-delegate notification preferences not implemented.
- **Sent Items routing configuration**: `saveToSentItems` behavior not yet configurable per shared mailbox.

---

## Research

**Date**: March 2026
**Context**: Ground-up implementation research for the pure Rust iced app. Architecture: 19-crate Cargo workspace with `ratatoskr-core` as facade, provider crates (`gmail`, `graph`, `jmap`, `imap`), and the `app` crate for the iced UI.

---

### 1. Exchange Graph API for Shared Mailboxes

#### Accessing another user's mailbox

All mailbox types are accessed uniformly via `/users/{id-or-upn}`:

```
GET /users/{shared-mailbox-email}/messages
GET /users/{shared-mailbox-email}/mailFolders
POST /users/{shared-mailbox-email}/sendMail
```

Same API surface as personal mailbox, just a different user identifier. No special "shared mailbox" endpoints.

#### OAuth scopes

| Scope | Allows | Admin consent required? |
|---|---|---|
| `Mail.Read.Shared` | Read messages in shared/delegated folders | No |
| `Mail.ReadWrite.Shared` | Read, write, delete messages in shared/delegated folders | No |
| `Mail.Send.Shared` | Send mail from shared/delegated mailboxes | No |

**Key differences from personal scopes:**

- **Delegated-only.** No application permission equivalents.
- **Superset behavior.** `Mail.Read.Shared` grants access to both personal and shared mailboxes.
- **Silent failure.** If the app registration only has `Mail.Read`/`Mail.ReadWrite`/`Mail.Send`, accessing `/users/{shared-mailbox}/messages` returns 403 with no indication that `.Shared` scopes are needed.

Change notification subscriptions on shared mailboxes do **not** work with `.Shared` delegated scopes — require application `Mail.Read` instead.

#### The discovery problem

**There is no Graph API endpoint that returns "all mailboxes this user has access to."** Every approach has significant drawbacks:

**Approach 1: EWS Autodiscover XML.** Exchange auto-mapping records accessible mailboxes. The XML Autodiscover endpoint can be called with an OAuth token against `https://outlook.office365.com/autodiscover/autodiscover.xml`. Parse `AlternativeMailbox` elements. However: no Rust EWS crate exists, auto-mapping can be disabled per-grant (`-AutoMapping $false`), and security-group-granted access never auto-maps. Won't find everything.

**Approach 2: Probing `/users/{email}/mailFolders`.** 200=access, 403=no access, 404=doesn't exist. Requires already knowing which addresses to try. Viable as validation, useless for discovery.

**Approach 3: User manually adds by email.** Outlook's own "Open Another Mailbox" pattern. Always works regardless of Exchange configuration. **The reliable baseline.**

**Approach 4: Exchange Online PowerShell / admin APIs.** `Get-MailboxPermission` requires Exchange admin privileges. Not viable for a desktop client.

**Approach 5: Graph beta endpoints.** None exist as of March 2026.

**Approach 6: Autodiscover v2 (HTTP/JSON).** Returns connectivity info (EWS URL, ActiveSync URL), not auto-mapped mailboxes. Doesn't help.

**Recommended strategy**: Manual-add as baseline (Approach 3), with Autodiscover XML as an enhancement for Exchange accounts (Approach 1). The XML endpoint accepts OAuth tokens. Parse `AlternativeMailbox` elements — ~100-200 lines of focused XML parsing code, not a full EWS client.

#### Send As vs Send on Behalf

Both use `POST /users/{shared-mailbox-upn}/sendMail`:

**Send As**: Set `from` to shared mailbox. Message appears to come directly from it.

**Send on Behalf**: Set `from` to shared mailbox AND `sender` to delegate. Recipient sees "Delegate on behalf of SharedMailbox".

Exchange enforces permissions server-side. If user only has Send on Behalf and omits `sender`, Exchange rejects or auto-fills.

#### saveToSentItems behavior

`saveToSentItems: true` saves to the **sender's** (delegate's) Sent Items. Exchange admin settings (`MessageCopyForSentAsEnabled`, `MessageCopyForSendOnBehalfEnabled`) independently control whether a copy goes to the **shared mailbox's** Sent Items. The client cannot control the shared mailbox copy — it's an admin setting. Default `saveToSentItems: false` for shared mailbox sends.

#### Delta sync for shared mailboxes

Works identically: `GET /users/{shared-mailbox-id}/mailFolders/{folderId}/messages/delta`. Same token mechanism. Each shared mailbox maintains independent delta state.

#### Notification preferences

Exchange does not expose per-delegate notification preferences via Graph. Must be a client-side setting per delegate per shared mailbox.

---

### 2. Gmail Delegation

#### Send-As aliases (`users.settings.sendAs`)

Already supported. Outbound identity only — does not grant ability to read the aliased mailbox.

#### Account-level delegation (`users.settings.delegates`)

`users.settings.delegates.list` lists who the signed-in user has *delegated to* (outbound), **not** who has delegated to them (inbound). Cannot query "which accounts have delegated to me?" through the Gmail API.

Full delegation grants read/send/delete access to the entire account. No per-folder delegation.

**Accessing a delegated mailbox**: Use delegator's email as `userId` in API calls. However, this requires either domain-wide delegation (admin-level service account) or internal session mechanisms not exposed via public API.

**Practical strategy**: Support Send-As aliases (done). Document that full delegation requires Google Workspace admin configuration. Real limitation but affects fewer users than Exchange shared mailboxes.

---

### 3. JMAP Sharing (RFC 9670)

#### What it specifies

RFC 9670 (published November 2024, Standards Track) defines:

- **Principal**: Users, groups, resources. Methods: get/query/set/changes.
- **ShareNotification**: Permission change tracking. Read-only.
- Three properties on shareable objects (like Mailbox): `isSubscribed`, `myRights`, `shareWith`.

**Discovery is built into the protocol**: The JMAP Session object's `accounts` array includes all accounts the user has access to. Elegant — unlike Graph where discovery is absent.

#### Implementation status

Stalwart has documented ACL/sharing support, but specific RFC 9670 implementation status is unclear (published November 2024, so full implementation would be recent).

`jmap-client` v0.4 has a `principal` module but doesn't document RFC 9670 compliance. Sharing-specific types and methods would likely need to be added.

---

### 4. IMAP ACL (RFC 4314)

#### Commands

| Command | Purpose |
|---|---|
| `GETACL <mailbox>` | List all ACL entries (requires `a` right) |
| `SETACL <mailbox> <id> <rights>` | Set/modify ACL |
| `DELETEACL <mailbox> <id>` | Remove ACL entry |
| `LISTRIGHTS <mailbox> <id>` | Query grantable rights |
| `MYRIGHTS <mailbox>` | Get authenticated user's rights |

Rights: `l` (lookup), `r` (read), `s` (set seen), `w` (write flags), `i` (insert), `p` (post), `k` (create child), `x` (delete), `t` (set deleted), `e` (expunge), `a` (admin).

#### Accessing shared namespaces

RFC 2342 (Namespace) exposes shared mailboxes under separate prefixes: `"Other Users/"`, `"#user/"`, etc. Server-specific. Discovered via `NAMESPACE` command.

#### Server support

| Server | ACL | Notes |
|---|---|---|
| **Dovecot** | Full (v1.2+) | Requires `acl_shared_dict` config for discovery |
| **Cyrus IMAP** | Full | Best-in-class ACL |
| **Stalwart** | Supported | Maps to JMAP sharing internally |
| **Exchange IMAP** | **Not supported** | Shared access only via Graph |
| **Gmail IMAP** | **Not supported** | Compatibility layer only |

**Practical reality**: ACL only relevant for self-hosted Dovecot/Cyrus. The two largest providers don't support it.

#### async-imap ACL support

`async-imap` has **no built-in ACL commands**. Use `Session::run_command_and_check_ok()` for raw commands + custom response parsing. ACL responses have a simple format. `NAMESPACE` similarly needs raw command approach. `imap-codec` doesn't have ACL support either — any implementation will be custom.

---

### 5. Identity Management for Send

#### The data model

```rust
struct SendIdentity {
    id: String,
    account_id: String,
    email: String,
    display_name: String,
    mailbox_id: Option<String>,       // For shared mailboxes
    send_mode: SendMode,              // SendAs vs SendOnBehalf
    send_endpoint: String,            // e.g., "/users/support@contoso.com/sendMail"
    save_to_personal_sent: bool,
    signature: Option<String>,
}
```

A user may have 6+ possible From addresses across 3 accounts.

#### Auto-selecting the right From address

Priority rules:
1. **Replying from shared mailbox context**: Use shared mailbox identity
2. **Replying to a message sent to a specific alias**: Match To/Cc against known identities
3. **Composing from shared mailbox sidebar**: Default to shared mailbox identity
4. **Composing from personal context**: Account's primary identity
5. **Fallback**: Account's primary identity

This is where Ratatoskr can be significantly better than Outlook. The common failure mode in every client: user replies from personal address when they meant to reply from shared mailbox.

#### How other clients handle it

**Thunderbird**: Manual identity configuration. Matches To/Cc on reply. Works but requires setup.
**Apple Mail**: Auto-detects Exchange Send-As. Unreliable for shared mailbox identity selection.
**Outlook**: Most sophisticated — auto-maps shared identities, defaults based on folder context. But From switching UI is buried and users frequently send from wrong address.

---

### 6. Multi-Mailbox Sync Architecture

#### Sync depth per mailbox

| Setting | Purpose | Default |
|---|---|---|
| Sync enabled | Sync vs fetch-on-demand | true |
| Sync depth | Time range | 30 days |
| Sync folders | Which folders | Inbox + Sent |
| Push notifications | Real-time connection | false (poll on open) |

#### Separate sync contexts

Each shared mailbox is a separate "account" from the API:
- **Graph**: Independent delta tokens, independent throttling per mailbox. 10 shared mailboxes = 10x API quota.
- **JMAP**: Independent state tokens per account.
- **IMAP**: Independent UIDVALIDITY/UID space per namespace.

The sync engine should model each delegated mailbox as a separate sync context with its own state tokens, schedule, local DB partition, and error/retry state.

#### Bandwidth considerations

Prioritize the mailbox the user currently has open. Batch sync for background mailboxes. Respect `Retry-After` per-mailbox. Exponential backoff per-mailbox, not global.

---

### 7. Relevant Rust Crates

#### EWS client crates

**None exist.** For Autodiscover XML parsing, use `quick-xml` or `roxmltree` (both mature). ~100-200 lines of focused code.

#### graph-rs-sdk

Auto-generated Rust wrapper for Microsoft Graph. Extremely large. Not recommended — our custom Graph client is sufficient. Shared mailbox access is just changing `/me/` to `/users/{shared-mailbox}/`.

#### IMAP ACL

Neither `async-imap` nor `imap-codec` support ACL. Custom implementation via raw commands regardless of library choice.

---

### Summary: Implementation Priority

| Area | Difficulty | Impact | Priority | Status |
|---|---|---|---|---|
| Graph shared mailbox read/write (paths + scopes) | Low | Critical | P0 | **Done** |
| Send As / Send on Behalf via Graph | Low | High | P1 | **Done** |
| Autodiscover XML for auto-mapping | Medium | High for enterprise UX | P1 | **Done** |
| Per-shared-mailbox delta sync | Medium | Critical | P0 | **Done** |
| IMAP namespace/ACL discovery | Medium | Medium (Dovecot/Cyrus) | P2 | **Done** |
| Sidebar integration (scope dropdown) | Low | Critical (baseline) | P0 | **Done** (2026-03-22) — auto-populates from Autodiscover |
| Thread loading on shared mailbox selection | Medium | Critical for UX | P0 | Not started (App handler for SharedMailboxSelected) |
| Send identity auto-selection | Medium | Critical for UX | P0 | Not started |
| Per-mailbox sync depth config | Medium | High for scale | P1 | Not started |
| JMAP Sharing (RFC 9670) | Medium-High | Medium | P2 | In progress (separate implementation) |
| Gmail delegation | Blocked | Low | P3 | Blocked (API limitation) |
