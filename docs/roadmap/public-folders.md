# Public Folders

**Tier**: 1 — Blocks switching from Outlook
**Status**: 🟡 **In progress** — EWS SOAP client implemented in `crates/graph/src/ews/` (FindFolder, GetFolder, FindItem, GetItem, CreateItem), PR_REPLICA_LIST decoder done, EffectiveRights parsing done. Autodiscover routing implemented in `crates/graph/src/autodiscover.rs`. Offline sync for pinned folders implemented in `crates/graph/src/public_folder_sync.rs`. IMAP NAMESPACE public folders implemented in `crates/imap/src/public_folders.rs`. DB schema in place (`crates/db/`). Missing: UI integration.

---

- **What**: Hierarchical shared folder trees accessible to the entire organization (or subsets). Legacy Exchange concept, still heavily used in enterprises that have been on Exchange for 15+ years.
- **Scope**: Exchange-only. Microsoft has been trying to deprecate public folders since Exchange 2013. Enterprise customers refuse. They're still supported in Exchange Online/M365.

## Cross-provider behavior

| Provider | Support |
|---|---|
| Exchange (Graph) | Partial Graph support; full support via EWS. Microsoft keeps threatening to deprecate but never does. |
| Gmail API | No equivalent concept |
| JMAP | No equivalent concept |
| IMAP | Shared namespaces (RFC 2342) are conceptually similar but architecturally different |

## Pain points

- **Graph API gaps**: public folder access via Graph is limited compared to EWS. Some operations (creating items in public folders, managing permissions) may require falling back to EWS, which Microsoft is also trying to deprecate. Moving target.
- **Hierarchy depth**: public folder trees can be deeply nested — 10+ levels. Orgs use them as filing systems, knowledge bases, shared calendars, even discussion forums. The folder browser UI must handle deep hierarchies efficiently (lazy-load children, don't fetch the entire tree upfront).
- **Volume**: a public folder can contain tens of thousands of items. Same scale challenges as personal mailboxes, but multiplied by the number of public folders the user accesses.
- **Mixed content types**: public folders can contain emails, calendar items, contacts, tasks, notes, and custom forms. For an email client, focus on mail-enabled public folders (which receive email) and email item folders. Ignore calendar/contact/task public folders initially.
- **Permissions**: public folder permissions are a separate system from shared mailbox delegation. Roles include Owner, PublishingEditor, Editor, PublishingAuthor, Author, NonEditingAuthor, Reviewer, Contributor, None. The client must check and respect these per-folder.
- **Favorites**: Outlook lets users "favorite" specific public folders so they appear in the sidebar. Need a similar mechanism — the full public folder tree is too large to display by default.
- **Offline sync**: do not sync public folders by default. Only sync favorited/pinned public folders, and even then with configurable depth. A full public folder sync could be enormous.
- **Organizational inertia**: the reason these matter is that many enterprise customers have decades of institutional knowledge filed in public folders. "Where's the vendor agreement template?" "It's in Public Folders > Legal > Templates > Vendor." This is real workflow that can't be dismissed.

## Work

Browse public folder hierarchy (lazy-loaded) for Exchange accounts, favorite/pin specific folders to sidebar, sync favorited folders only, respect per-folder permissions, handle mail-enabled public folders as mailboxes. Accept Graph API limitations and consider EWS fallback for operations Graph doesn't support.

---

## Research

**Date**: March 2026
**Context**: Ground-up implementation research for the iced (pure Rust) email client. No assumptions about existing schemas or backend architecture.

---

### 1. Microsoft Graph API: No Public Folder Support

**Graph has no public folder endpoints.** Not in v1.0, not in beta. The `/me/mailFolders` and `/users/{id}/mailFolders` endpoints return only the user's personal mailbox folders. There is no `/publicFolders` resource, no way to browse the public folder hierarchy, and no way to read items from public folders via Graph.

This is not an oversight or a gap being worked on. Microsoft has explicitly stated that public folders are not part of Graph's scope. The Q&A on Microsoft Learn confirms: "Public folders aren't supported in the Graph API and given their legacy status, there are no plans for support in the near future."

The new Exchange Online Admin API (public preview November 2025) covers 6 admin-oriented endpoints (OrganizationConfig, AcceptedDomain, Mailbox, MailboxFolderPermission, DistributionGroupMember, DynamicDistributionGroupMember). None address public folder access. It targets Exchange admin PowerShell replacement, not end-user mailbox operations.

**Bottom line**: Public folder access requires EWS. There is no Graph alternative and none is coming.

---

### 2. EWS Retirement and the Public Folder Deadlock

#### Timeline

| Date | Event |
|---|---|
| October 2023 | Microsoft announces EWS retirement for Exchange Online |
| February 2025 | App Impersonation blocked in Exchange Online |
| October 2025 | Public folder migrations from Exchange 2010 and older blocked |
| **October 1, 2026** | **EWS requests to Exchange Online start being blocked** |

#### The deadlock

Microsoft has announced that after October 2026:

1. EWS will be blocked for Exchange Online.
2. **No APIs will be provided for programmatically creating, reading, updating, or deleting public folders.** Programmatic access will be restricted to "supported Outlook clients only" and bulk import/export.
3. Graph does not and will not support public folders.

This creates an impossible situation for third-party clients: the only API that supports public folders (EWS) is being retired, and no replacement is offered. Microsoft's recommended migration path is to move public folder content to M365 Groups, but enterprise customers have resisted this for over a decade.

#### What this means for Ratatoskr

Three scenarios, in order of likelihood:

1. **Microsoft extends the EWS deadline again** (high probability). They've delayed repeatedly. The October 2026 date may slip, especially given the lack of Graph parity for public folders. Enterprise pushback has been intense.
2. **Microsoft adds public folder support to Graph or the Admin API** before October 2026 (low probability but possible under pressure).
3. **EWS is actually blocked October 2026** (possible but would break every third-party client's public folder access simultaneously, triggering massive enterprise backlash).

**Strategy**: Build EWS-based public folder support now. It works today and will work for at least the next 6+ months. If EWS is actually blocked, public folders become inaccessible to all non-Outlook clients simultaneously — this becomes Microsoft's problem, not ours. If they extend the deadline or add Graph support, we adapt.

#### On-premises Exchange

EWS retirement only affects Exchange Online. On-premises Exchange Server (2016, 2019, and the upcoming Exchange Server Subscription Edition) will continue to support EWS indefinitely. Public folders via EWS will remain viable for on-premises deployments regardless of the Online timeline.

---

### 3. EWS Public Folder Operations

EWS provides comprehensive public folder access. The relevant operations:

#### Folder operations

| Operation | Purpose | Public folder support |
|---|---|---|
| `FindFolder` | Browse folder hierarchy (shallow or deep traversal) | Yes |
| `GetFolder` | Get folder properties, permissions, EffectiveRights | Yes |
| `CreateFolder` | Create new public folder | Yes |
| `UpdateFolder` | Modify folder properties/permissions | Yes |
| `DeleteFolder` | Delete public folder | Yes |
| `MoveFolder` | Move between public folders (not public-to-private) | Partial (Exchange 2013+) |

#### Item operations

| Operation | Purpose | Public folder support |
|---|---|---|
| `FindItem` | Search/list items in a folder (supports QueryString in 2013+) | Yes |
| `GetItem` | Read a specific item | Yes |
| `CreateItem` | Post new item (email, post item) | Yes |
| `UpdateItem` | Modify item properties | Yes |
| `DeleteItem` | Delete item | Yes |
| `MoveItem` | Move item to public folder | Yes |
| `CopyItem` | Copy item | Yes |

#### Not supported for public folders

- `SyncFolderHierarchy` — cannot use delta sync for the folder tree. Must use recursive `FindFolder`.
- `SyncFolderItems` — cannot use delta sync for folder contents. Must use `FindItem` with date filters or change tracking.
- `CopyFolder` — not supported in Exchange 2013+. Workaround: `CreateFolder` + `CopyItems`.
- `EmptyFolder` — not supported in Exchange 2013+. Workaround: `FindItem` + `DeleteItem`.

The lack of `SyncFolderItems` is the most significant limitation. It means there's no efficient change-detection mechanism for public folder contents — the client must poll with `FindItem` and compare against local state.

---

### 4. EWS Request Routing (Critical Complexity)

Public folder requests require special HTTP headers that must be obtained through Autodiscover. This is the most complex part of the implementation.

#### Architecture background

Exchange stores public folders in specialized mailboxes:

- **Primary hierarchy mailbox**: Contains the only writable copy of the folder hierarchy. One per organization.
- **Secondary hierarchy mailboxes**: Read-only copies of the hierarchy. Contain content for specific folders.
- **Content mailboxes**: Store the actual items. A folder's content may live on a different server than the hierarchy.

#### Routing hierarchy requests

Hierarchy operations (FindFolder, CreateFolder, UpdateFolder, DeleteFolder, MoveFolder) require two headers:

| Header | Value source |
|---|---|
| `X-AnchorMailbox` | `PublicFolderInformation` from Autodiscover GetUserSettings |
| `X-PublicFolderMailbox` | `InternalRpcClientServer` from Autodiscover GetUserSettings |

The Autodiscover call uses SOAP:

```xml
<a:RequestedSettings>
  <a:Setting>PublicFolderInformation</a:Setting>
  <a:Setting>InternalRpcClientServer</a:Setting>
</a:RequestedSettings>
```

Returns values like:
- `X-AnchorMailbox: SharedPublicFolder@contoso.com`
- `X-PublicFolderMailbox: 1ec2a236-ed93-4f88-b9c6-33e63fa4aa44@contoso.com`

#### Routing content requests

Content operations (FindItem, GetItem, CreateItem, etc.) require a different routing process:

1. Fetch hierarchy with `FindFolder`, requesting the `PR_REPLICA_LIST` extended property (tag `0x6698`, type Binary).
2. Decode the base64 `PR_REPLICA_LIST` value to get the content mailbox GUID.
3. Construct an SMTP address: `{GUID}@{domain}`.
4. Call Autodiscover with that address to get the `AutoDiscoverSMTPAddress`.
5. Set both `X-AnchorMailbox` and `X-PublicFolderMailbox` to the `AutoDiscoverSMTPAddress`.

Different folders may have different content mailboxes. The client must track which content mailbox serves each folder.

#### Practical impact

This means public folder access requires:
1. Autodiscover SOAP calls (for hierarchy routing).
2. Autodiscover POX calls (for content routing).
3. Tracking per-folder content mailbox mappings.
4. Setting per-request HTTP headers based on operation type.

This is significantly more complex than personal mailbox access, which uses a single EWS endpoint.

---

### 5. EWS in Rust: Implementation Path

#### Thunderbird's ews-rs crate

Thunderbird (Mozilla) has built a Rust EWS crate: [thunderbird/ews-rs](https://github.com/thunderbird/ews-rs). Key facts:

- **License**: MPL 2.0 (compatible with our use).
- **Architecture**: Built on `xml_struct` (custom derive macros) + `quick_xml`. Not serde-based — they found serde's data model doesn't map well to XML namespaces, which EWS requires.
- **Maturity**: ~73 commits as of March 2026. Active development. Thunderbird 145 (November 2025) shipped native Exchange support using this crate.
- **Scope**: Focuses on mail operations (the operations Thunderbird needs). Does not appear to cover public folder-specific operations.
- **Issues**: Bug reports go to Mozilla's Bugzilla, not GitHub Issues.

#### Options for Ratatoskr

**Option A: Use ews-rs directly.** Fork or depend on it. Add public folder operations (FindFolder with public folder root, PR_REPLICA_LIST property, etc.). Benefit: leverages Thunderbird's XML serialization infrastructure. Risk: MPL 2.0 requires file-level copyleft, and the crate's API is designed around Thunderbird's architecture.

**Option B: Build a minimal EWS client.** Using `quick-xml` + `reqwest`:

- **SOAP envelope**: ~30 lines of boilerplate XML template.
- **Operations needed**: FindFolder, GetFolder, FindItem, GetItem, CreateItem (for replies/posts). ~5-6 operations total.
- **Autodiscover**: GetUserSettings SOAP call + POX Autodiscover. ~2 additional request types.
- **XML types**: Public folder operations use a subset of EWS types. FolderId, ItemId, FindFolderResponse, FindItemResponse, Message type, PostItem type, PermissionSet, EffectiveRights. Maybe 20-30 types total.
- **Estimated scope**: 1500-2500 lines for a focused public-folder-only EWS client.

**Option C: Hybrid.** Use `quick-xml` for serialization (like Thunderbird does) but build our own operation types. Avoids the `xml_struct` dependency while still getting correct namespace handling.

**Recommendation**: Option B. A focused EWS client is more maintainable than depending on Thunderbird's crate (which will evolve for Thunderbird's needs, not ours). The shared-mailboxes research already identified `quick-xml` or `roxmltree` for Autodiscover XML parsing (~100-200 lines). Public folders extend this to a larger but still bounded EWS surface.

**Update (March 2026)**: Option B was implemented in `crates/graph/src/ews/` (~1600 lines across 4 modules: `mod.rs`, `client.rs`, `parsers.rs`, `xml_helpers.rs`). Uses `quick-xml` + `reqwest` with OAuth bearer auth. Covers FindFolder, GetFolder, FindItem (with paging), GetItem, CreateItem, plus `decode_replica_list()` for PR_REPLICA_LIST binary parsing and `EwsEffectiveRights` extraction. Additionally, Autodiscover routing lives in `crates/graph/src/autodiscover.rs` (~600 lines), offline sync for pinned folders in `crates/graph/src/public_folder_sync.rs` (~900 lines), and IMAP NAMESPACE public folders in `crates/imap/src/public_folders.rs` (~800 lines). DB tables (`public_folders`, `public_folder_items`, `public_folder_pins`, `public_folder_sync_state`, `public_folder_content_routing`) are defined in `crates/db/src/db/migrations.rs`.

#### Authentication

EWS in Exchange Online requires OAuth 2.0. Basic Auth is fully deprecated. Our existing OAuth flow for Graph can be reused — same Azure AD app registration, same tokens. The EWS endpoint (`https://outlook.office365.com/EWS/Exchange.asmx`) accepts the same OAuth bearer tokens. The required scope is `https://outlook.office365.com/EWS.AccessAsUser.All` (delegated) — this must be added to the app registration.

---

### 6. Public Folder Permissions

#### Permission levels

EWS exposes permissions via `GetFolder` with `folder:PermissionSet` in the requested properties. Each folder has a DACL (Discretionary Access Control List) with entries per user:

| Level | Create items | Create subfolders | Edit items | Delete items | Read items |
|---|---|---|---|---|---|
| Owner | Yes | Yes | All | All | FullDetails |
| PublishingEditor | Yes | Yes | All | All | FullDetails |
| Editor | Yes | No | All | All | FullDetails |
| PublishingAuthor | Yes | Yes | Owned | Owned | FullDetails |
| Author | Yes | No | Owned | Owned | FullDetails |
| NonEditingAuthor | Yes | No | None | Owned | FullDetails |
| Reviewer | No | No | None | None | FullDetails |
| Contributor | No | No | None | None | None |
| None | No | No | None | None | None |

#### EffectiveRights

For the current user's actual permissions, request the `EffectiveRights` additional property in `GetFolder`/`FindFolder` responses:

```xml
<t:EffectiveRights>
  <t:CreateAssociated>true</t:CreateAssociated>
  <t:CreateContents>true</t:CreateContents>
  <t:CreateHierarchy>true</t:CreateHierarchy>
  <t:Delete>true</t:Delete>
  <t:Modify>true</t:Modify>
  <t:Read>true</t:Read>
</t:EffectiveRights>
```

**EffectiveRights is the practical approach.** Rather than parsing the full PermissionSet and resolving which ACE applies, request EffectiveRights to get a boolean yes/no for each operation the current user can perform. Use this to enable/disable UI actions (reply, forward, delete, move, create subfolder).

---

### 7. Mail-Enabled Public Folders

Mail-enabled public folders have SMTP addresses and can receive email from distribution groups or direct sends. They are the most common public folder type in active enterprise use.

#### Discovery

No Graph or EWS endpoint directly lists all mail-enabled public folders. Discovery path:

1. Browse the public folder hierarchy via `FindFolder` from root.
2. Each folder's `FolderClass` indicates content type: `IPF.Note` = mail.
3. Mail-enabled folders have an associated email address, accessible via extended properties (PR_SMTP_ADDRESS or similar).
4. Alternatively, admins can provide a list of known public folder email addresses for manual-add.

#### Reading and replying

Items in mail-enabled public folders are standard email messages. `FindItem` and `GetItem` return `Message` types with From, To, Subject, Body, etc. Replies work via `CreateItem` with a `ReferenceItemId` pointing to the original.

For post items (non-email discussion items native to public folders), the type is `PostItem` with a `PostedTime` and `From` but no `To`. Replying creates a new `PostItem` in the same folder.

---

### 8. IMAP Shared Namespaces (RFC 2342)

While not Exchange public folders, IMAP shared namespaces provide a conceptually similar experience for self-hosted IMAP servers.

#### How it works

The `NAMESPACE` command returns three namespace types:

```
* NAMESPACE (("" "/")) (("Other Users/" "/")) (("Public/" "/"))
```

1. **Personal**: User's own mailboxes (prefix `""`, separator `/`).
2. **Other Users**: Shared mailboxes from specific users (prefix `"Other Users/"`)
3. **Shared/Public**: Organization-wide shared mailboxes (prefix `"Public/"`)

Clients discover namespaces via NAMESPACE, then LIST folders under each prefix.

#### Server implementations

| Server | Public namespace | ACL support | Notes |
|---|---|---|---|
| **Dovecot** | `type = public`, configurable prefix | Full (via ACL plugin) | Most common self-hosted server |
| **Cyrus IMAP** | Built-in, configurable via `altnamespace` | Full (best-in-class) | Heavy in academic/government |
| **Stalwart** | Supported | Via JMAP sharing internally | Newer entrant |
| **Exchange IMAP** | **Not supported** | **Not supported** | Exchange IMAP is a compatibility layer |
| **Gmail IMAP** | **Not supported** | **Not supported** | Gmail IMAP is a compatibility layer |

#### Implementation in async-imap

`async-imap` has no built-in NAMESPACE support. Same as IMAP ACL — use `Session::run_command_and_check_ok()` with raw command strings and custom response parsing. The NAMESPACE response format is simple (S-expression-like). ~50-80 lines of parsing code.

Once namespaces are discovered, folders under public/shared prefixes can be listed and accessed with standard IMAP LIST/SELECT/FETCH commands. The existing IMAP provider code should work with minimal changes — the difference is the folder path prefix, not the operations.

#### Practical value

Low-medium. Only relevant for Dovecot/Cyrus users. Exchange and Gmail IMAP don't expose shared namespaces. But for self-hosted enterprise IMAP, this provides a public-folder-like experience without EWS. Worth implementing as a lower-priority enhancement.

---

### 9. Data Model

#### Public folder hierarchy (local storage)

```rust
struct PublicFolder {
    id: String,                         // EWS FolderId
    account_id: String,                 // Which Exchange account
    parent_id: Option<String>,          // Parent folder FolderId (None = root)
    display_name: String,
    folder_class: String,               // "IPF.Note", "IPF.Contact", etc.
    total_count: u32,                   // Server-reported item count
    unread_count: u32,
    content_mailbox_guid: Option<String>, // PR_REPLICA_LIST — which server has content
    effective_rights: EffectiveRights,   // Cached from last fetch
    children_loaded: bool,              // Have we fetched children yet?
    last_hierarchy_sync: Option<i64>,   // Timestamp
}

struct EffectiveRights {
    can_create_contents: bool,
    can_create_hierarchy: bool,
    can_delete: bool,
    can_modify: bool,
    can_read: bool,
}

struct PublicFolderPin {
    id: String,
    account_id: String,
    folder_id: String,                  // References PublicFolder.id
    display_name: String,               // Cached for sidebar display
    sync_enabled: bool,                 // Whether to sync contents offline
    sync_depth_days: Option<u32>,       // How far back to sync (None = all)
    position: u32,                      // Sidebar ordering
}
```

#### Separation from personal mailbox

Public folders must be stored in separate tables from personal mailbox folders. They have different ID spaces, different sync mechanisms, different permission models, and different routing requirements. The personal mailbox uses Graph delta sync; public folders use EWS FindItem polling. Mixing them in the same folder table would create confusion.

Recommended tables:
- `public_folders` — hierarchy cache
- `public_folder_items` — synced items from pinned folders
- `public_folder_pins` — user's favorited folders + sync settings
- `public_folder_sync_state` — per-folder sync cursors (last-seen item timestamp, since there's no delta token)

#### IMAP shared namespace folders

IMAP shared/public folders can reuse the existing folder table with a `namespace_type` column (`personal`, `other_users`, `public`). They use standard IMAP sync (UIDVALIDITY/UID), so no separate sync mechanism is needed.

---

### 10. Offline Sync Strategy

#### Principle: Never sync by default

Public folder trees can contain thousands of folders with millions of items across an organization. Only sync what the user explicitly pins.

#### Sync for pinned folders

1. **Initial sync**: `FindItem` with `DateTimeReceived >= (now - sync_depth_days)`, sorted descending. Page through results. Store items in `public_folder_items`.
2. **Incremental sync**: `FindItem` with `DateTimeReceived >= last_sync_timestamp`. Compare returned items against local cache. Handle creates, updates (match by ItemId + ChangeKey), and deletes (items present locally but absent from server within the sync window).
3. **Poll interval**: Longer than personal mailbox. Default 5-10 minutes for pinned folders. No push notifications — EWS streaming notifications work for personal mailboxes but are unreliable for public folders across content mailboxes.

#### Hierarchy sync

1. **On-demand**: When the user opens the public folder browser, fetch root children via `FindFolder` (shallow traversal, `publicfoldersroot`).
2. **Lazy expansion**: When the user expands a folder, fetch its children. Cache in `public_folders` table.
3. **Background refresh**: For pinned folders only, periodically re-fetch the folder's direct properties (unread count, total count) via `GetFolder`.
4. **No full tree sync**: Never fetch the entire hierarchy. Organizations can have 10,000+ public folders.

#### Change detection without SyncFolderItems

Since `SyncFolderItems` is not supported for public folders, the fallback is:

- Track the highest `DateTimeReceived` or `LastModifiedTime` seen per folder.
- On each sync, `FindItem` for items newer than that timestamp.
- For deletions: periodically do a full `FindItem` (ID-only) for the sync window and diff against local cache. Expensive but necessary. Run infrequently (hourly for active folders).

---

### 11. What Thunderbird Does

Thunderbird 145 (November 2025) shipped native Exchange support via EWS built in Rust. Current status regarding public folders and shared resources:

- **Public folders**: Not supported. Thunderbird's EWS implementation covers personal mail operations only.
- **Shared folders/mailboxes**: Listed as "not yet supported" in their known limitations. Planned for a future release.
- **IMAP public folders**: Thunderbird has a long-standing bug (Bug 522848) about Exchange IMAP public folders not being visible/subscribable. Not resolved.

Thunderbird's `ews-rs` crate (MPL 2.0) provides the EWS type system and XML serialization infrastructure. It focuses on the operations Thunderbird needs for personal mail. Public folder operations (FindFolder on `publicfoldersroot`, `PR_REPLICA_LIST` handling, content mailbox routing) would need to be added.

**Relevance**: Thunderbird is in the same position as Ratatoskr — EWS-dependent for public folders, with no Graph alternative. Their approach is to build EWS support incrementally, starting with personal mail. Public folders are on their roadmap but not yet implemented.

---

### 12. Microsoft's Deprecation Trajectory

#### History of "deprecation"

- **Exchange 2007**: Public folders deprecated. Not removed.
- **Exchange 2013**: Public folders re-architected onto mailbox infrastructure. Still supported.
- **Exchange 2016/2019**: Public folders still supported.
- **Exchange Online**: Public folders still supported, with 1000-folder and 250GB limits per public folder mailbox.
- **2023**: EWS retirement announced. Public folder API access to end.
- **2025-2026**: Migration deadlines being enforced for legacy on-premises versions.

#### Microsoft's recommended migration path

Microsoft pushes M365 Groups as the replacement for public folders. Migration tooling exists (`New-PublicFolderMigrationRequest`). But M365 Groups:

- Are flat (no hierarchy) — public folders are deeply hierarchical.
- Have different permission models.
- Don't support the "shared filing cabinet" metaphor that enterprises rely on.
- Require per-group management overhead that admins resist at scale.

Enterprise adoption of the migration has been glacial. Many organizations have simply refused.

#### Realistic assessment

Public folders will exist in Exchange Online for years to come. Microsoft cannot remove them without losing major enterprise customers. The API access question (EWS retirement) is more uncertain, but Microsoft has a track record of extending deadlines when enterprise pushback is strong enough. The October 2026 EWS deadline is the most significant risk, but even if enforced, it may include carve-outs or extensions for specific operations.

---

### Summary: Implementation Priority

| Area | Difficulty | Impact | Priority | Status |
|---|---|---|---|---|
| Minimal EWS client (quick-xml + reqwest) | Medium | Critical (enables everything) | P0 | ✅ Done — `crates/graph/src/ews/` (~1600 lines, 4 modules). FindFolder, GetFolder, FindItem, GetItem, CreateItem with full XML request/response parsing. |
| EWS OAuth token acquisition (reuse existing flow) | Low | Critical | P0 | ✅ Done — reuses existing Graph OAuth bearer tokens. |
| PR_REPLICA_LIST decoding | Medium | Medium (correctness) | P1 | ✅ Done — `decode_replica_list()` extracts content mailbox GUIDs from binary extended property. |
| EffectiveRights permission checking | Low | High | P1 | ✅ Done — `EwsEffectiveRights` struct parsed from FindFolder/GetFolder responses. |
| Autodiscover for public folder routing | Medium | Critical | P0 | ✅ Done — `crates/graph/src/autodiscover.rs` (~600 lines). `discover_public_folder_routing()` for hierarchy headers, `discover_content_mailbox()` for content routing, `construct_replica_smtp()` for GUID-to-SMTP. |
| FindFolder hierarchy browsing (lazy-load) | Medium | Critical | P0 | 🟡 API ready — FindFolder operation implemented, `browse_public_folders()` in `crates/graph/src/public_folder_sync.rs`. DB caching via `public_folders` table. No lazy-load UI yet. |
| FindItem for folder contents | Medium | Critical | P0 | ✅ Done — FindItem with paging implemented. `sync_pinned_public_folder()` and `sync_all_pinned_folders()` in `crates/graph/src/public_folder_sync.rs` handle sync loop with local storage in `public_folder_items` table. |
| Pin/favorite folders to sidebar | Low | Critical for UX | P0 | 🟡 Backend done — `pin_public_folder()` / `unpin_public_folder()` in `crates/graph/src/public_folder_sync.rs`, `public_folder_pins` DB table. No sidebar UI yet. |
| Offline sync for pinned folders | Medium-High | High | P1 | ✅ Done — `crates/graph/src/public_folder_sync.rs` (~900 lines). Timestamp-based polling, deletion scan throttled to 1hr intervals, content routing cache in `public_folder_content_routing` table. |
| Mail-enabled folder reply/forward | Medium | High | P1 | 🟡 API ready — CreateItem operation implemented. |
| CreateItem (post to public folder) | Low | Medium | P2 | 🟡 API ready — CreateItem operation implemented. |
| IMAP NAMESPACE discovery | Low | Low-Medium (self-hosted only) | P2 | ✅ Done — `crates/imap/src/public_folders.rs` (~800 lines). `discover_imap_public_folders()` uses NAMESPACE + LIST, `check_folder_rights()` uses MYRIGHTS (RFC 4314). |
| IMAP shared namespace folder access | Low | Low-Medium | P2 | ✅ Done — `sync_imap_public_folder()` in `crates/imap/src/public_folders.rs`. Bridges to provider-agnostic `public_folders` DB table. |

The backend critical path is complete: EWS client, Autodiscover routing, FindFolder browsing, FindItem sync, pin/unpin, offline sync, and IMAP NAMESPACE support are all implemented across `crates/graph/` and `crates/imap/`. The remaining work is UI integration in `crates/app/`: public folder browser panel, sidebar pins, and folder content views.
