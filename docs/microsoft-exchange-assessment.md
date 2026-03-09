# Microsoft Exchange Email Support: Ecosystem Assessment

**Date**: March 2026
**Context**: Evaluating paths to add Microsoft Exchange/Outlook support to Ratatoskr. Currently we support Gmail API and generic IMAP/SMTP. Outlook.com/Exchange Online users can connect via IMAP+OAuth2, but this is limited compared to native API integration.

---

## Table of Contents

- [The Landscape](#the-landscape)
- [EWS: Exchange Web Services](#ews-exchange-web-services)
  - [Protocol Overview](#ews-protocol-overview)
  - [Deprecation Timeline](#ews-deprecation-timeline)
  - [thunderbird/ews-rs](#thunderbirdews-rs)
  - [Other EWS Implementations](#other-ews-implementations)
- [Microsoft Graph API](#microsoft-graph-api)
  - [Protocol Overview](#graph-api-overview)
  - [Auth and Access](#auth-and-access)
  - [Delta Sync](#delta-sync)
  - [Rate Limits](#rate-limits)
  - [EWS vs Graph Comparison](#ews-vs-graph-comparison)
- [Rust Libraries and Tools](#rust-libraries-and-tools)
  - [graph-rs-sdk (primary candidate)](#graph-rs-sdk)
  - [Other Rust Graph Crates](#other-rust-graph-crates)
  - [Code Generation Approaches](#code-generation-approaches)
  - [EWS Libraries in Rust](#ews-libraries-in-rust)
- [Reference Implementations in Other Languages](#reference-implementations-in-other-languages)
- [Open Source Clients with Exchange Support](#open-source-clients-with-exchange-support)
- [Integration Strategy for Ratatoskr](#integration-strategy-for-ratatoskr)

---

## The Landscape

Microsoft has two APIs for Exchange mailbox access:

1. **EWS (Exchange Web Services)** — SOAP/XML over HTTPS. Being **deprecated for Exchange Online** (Oct 2026 block, Apr 2027 permanent removal). Remains supported for on-premises Exchange Server indefinitely.

2. **Microsoft Graph API** — REST/JSON over HTTPS. The **recommended replacement**. Works with both Exchange Online (Microsoft 365) and personal Outlook.com/Hotmail/Live.com accounts. Does NOT work with on-premises Exchange.

Additionally, Exchange Online still supports **IMAP/SMTP with OAuth2** (XOAUTH2 SASL), which our app could use today with minimal changes. Basic auth for IMAP/SMTP on Exchange Online was already killed.

The "Graph" in Microsoft Graph is **not GraphQL** — it is a standard REST API using OData v4.0 conventions with JSON responses. The name refers to the unified graph of Microsoft 365 resources.

---

## EWS: Exchange Web Services

### EWS Protocol Overview

EWS is a SOAP/XML-over-HTTPS API. Clients POST SOAP XML envelopes to `/EWS/Exchange.asmx` and receive XML responses. Defined by three schema files: `Services.wsdl`, `Messages.xsd`, `Types.xsd`.

Core operations:

| Category | Operations |
|----------|-----------|
| Items | CreateItem, GetItem, FindItem, UpdateItem, DeleteItem, CopyItem, MoveItem, SendItem, MarkAllItemsAsRead, MarkAsJunk |
| Folders | CreateFolder, GetFolder, FindFolder, UpdateFolder, DeleteFolder, CopyFolder, MoveFolder, EmptyFolder, SyncFolderHierarchy, SyncFolderItems |
| Attachments | GetAttachment, CreateAttachment, DeleteAttachment |
| Contacts | ResolveNames, ExpandDL, GetRoomLists, GetRooms |
| Calendar | GetUserAvailability, GetUserOofSettings, CreateItem/GetItem for CalendarItems |
| Notifications | Subscribe, Unsubscribe, GetEvents, GetStreamingEvents |
| Sync | SyncFolderHierarchy, SyncFolderItems (incremental via SyncState tokens) |

Authentication: NTLM, Kerberos, Basic Auth (legacy), OAuth 2.0 (modern auth for Exchange Online).

### EWS Deprecation Timeline

This applies **only to Exchange Online / Microsoft 365**. On-premises Exchange Server retains full EWS support indefinitely.

| Date | Event |
|------|-------|
| March 1, 2026 | F1/F3/Kiosk license holders blocked (HTTP 403) |
| **October 1, 2026** | **EWS disabled by default for all tenants** (admins can opt-in via AllowList until Apr 2027) |
| **April 1, 2027** | **Permanent removal**, no re-enablement possible |

The deprecation was accelerated after the Midnight Blizzard security incident (January 2024) that exploited EWS.

### thunderbird/ews-rs

- **URL**: [github.com/thunderbird/ews-rs](https://github.com/thunderbird/ews-rs)
- **Stars**: 32
- **License**: MPL-2.0
- **Last commit**: February 13, 2026 (active)
- **Contributors**: Eleanor Dicharry, Justin Tracey, Brendan Abolivier, Andrew DeVries, Ben Campbell — all Thunderbird/MZLA employees
- **Published to crates.io**: No — consumed as a git dependency by Thunderbird's internal `ews_xpcom` crate
- **Issues**: Disabled on GitHub; bugs go to Mozilla Bugzilla under "Networking:Exchange"

**What it is**: A Rust crate defining EWS data structures and XML serialization/deserialization. It is the **type/protocol layer only** — not a full client. Depends on a companion crate, [`xml_struct`](https://github.com/thunderbird/xml-struct-rs), which provides derive macros for mapping Rust structs to/from XML via `quick_xml`. The team built custom XML serialization because `serde` cannot handle XML namespaces, which EWS requires.

**Supported EWS operations** (from `ews/src/types/`):
- CreateItem, GetItem, FindItem, UpdateItem, DeleteItem
- CreateFolder, GetFolder, FindFolder, UpdateFolder, DeleteFolder
- CopyItem, MoveItem, CopyFolder, MoveFolder
- SyncFolderHierarchy, SyncFolderItems
- MarkAllItemsAsRead, MarkAsJunk
- EmptyFolder
- SOAP envelope handling

**Notable absences**: SendItem, GetAttachment, CreateAttachment, ResolveNames, Subscribe/GetEvents/GetStreamingEvents (notifications), calendar-specific operations. Thunderbird 145 supports email only — no calendar or contacts via EWS yet.

**Is Thunderbird using it in production?** Yes. Thunderbird 145 (stable, November 2025) ships native EWS email support built on this crate. The architecture:

1. `ews-rs` (public) — protocol types, XML ser/de
2. `xml_struct` (public) — derive macros for XML mapping
3. `ews_xpcom` (Thunderbird-internal) — Rust-to-C++ bridge via XPCOM
4. `xpcom_async` (Thunderbird-internal) — async/await adapter for XPCOM callbacks
5. `moz_http` (Thunderbird-internal) — idiomatic Rust HTTP client wrapping Gecko's network stack

**What works in Thunderbird 145**: Full folder listing/sync, message sync (incremental via SyncFolderItems), message reading/sending/replying/forwarding, folder management (create/rename/delete/move), attachment handling, search (local).

**What doesn't work yet**: Calendar, address book/contacts, shared folder access.

**Assessment for Ratatoskr**: `ews-rs` provides types and XML ser/de but **no HTTP client, no authentication, no autodiscover, no session management**. Those live in Thunderbird's internal crates and are tightly coupled to Gecko's network stack (XPCOM). You would need to build all of that yourself on top of ews-rs types using `reqwest` or Tauri's HTTP plugin. Given that EWS is being deprecated for Exchange Online (the majority use case), the ROI is questionable.

### Other EWS Implementations

| Library | Language | Stars | Status | Assessment |
|---------|----------|-------|--------|-----------|
| [exchangelib](https://github.com/ecederstrand/exchangelib) | Python | 1,256 | Active (v5.6.0, Oct 2025) | **Gold standard.** Full EWS coverage: mail, calendar, contacts, tasks, autodiscover, OAuth2, NTLM. 387K weekly PyPI downloads. Best reference for understanding EWS operations. |
| [ews-javascript-api](https://github.com/gautamsi/ews-javascript-api) | JS | — | Semi-maintained | Port of Microsoft's C# managed API. Comprehensive but maintenance inactive. |
| [ews-cpp](https://github.com/otris/ews-cpp) | C++ | 75 | Active (Feb 2026) | Header-only C++11. Core operations covered. Depends on libcurl. |
| [php-ews](https://github.com/jamesiarmes/php-ews) | PHP | 575 | Jan 2024 | Full EWS client with NTLM auth. Good SOAP structure reference. |
| [ews-java-api](https://github.com/OfficeDev/ews-java-api) | Java | 872 | **Archived** | Microsoft's own reference impl. Dead. Community fork at [eischeit/ews-java-api](https://github.com/eischeit/ews-java-api). |
| [ews-managed-api](https://github.com/OfficeDev/ews-managed-api) | C# | 577 | **Archived** | Microsoft's original reference. Dead. |
| [Dust-Mail/ews-client](https://github.com/Dust-Mail/ews-client) | Rust | 8 | Oct 2023 | **Autodiscover only.** No mail operations. Minimal utility. |

---

## Microsoft Graph API

### Graph API Overview

Microsoft Graph is a unified REST API for all Microsoft 365 services. The Mail API subset covers:

| Category | Operations |
|----------|-----------|
| Messages | List, get, create, update, delete, send, reply, reply all, forward, move, copy |
| Mail Folders | List, get, create, update, delete (including child folders) |
| Attachments | List, get, create (file, item, reference; up to 150MB via upload sessions) |
| Search | `$filter`, `$search` (KQL), `$orderby`, `$select`, `$expand` OData query params |
| Categories | List, create, update, delete color-coded categories on messages |
| Rules | List, get, create, update, delete inbox rules |
| Subscriptions | Webhook subscriptions for new/changed/deleted messages (push notifications) |
| Delta Sync | `message.delta()` endpoint for incremental change tracking per folder |
| Focused Inbox | Access Focused/Other inbox overrides |

### Auth and Access

- **OAuth 2.0** via Microsoft Identity Platform (formerly Azure AD, now Microsoft Entra ID)
- Requires **Azure AD app registration** in Azure Portal (free to create)
- **Authorization Code flow with PKCE** — ideal for desktop/native apps, same pattern as our Gmail OAuth
- App must be registered as **multi-tenant + personal accounts** ("Accounts in any organizational directory and personal Microsoft accounts") using the `/common` token endpoint
- For personal-only accounts (Outlook.com/Hotmail), use `"consumers"` as tenant ID
- Key scopes: `Mail.Read`, `Mail.ReadWrite`, `Mail.Send`, `MailboxSettings.ReadWrite`
- Redirect URI: `http://localhost:{port}` — matches our existing OAuth server pattern (port 17248-17251)

**Who can use it**:
- Personal Outlook.com / Hotmail / Live.com accounts (free, consumer) — confirmed in Microsoft docs
- Microsoft 365 / Office 365 enterprise/education accounts
- The Graph API itself has **no separate cost** — mail APIs are not metered
- The user just needs a mailbox (Outlook.com provides one for free)

### Delta Sync

- `GET /me/mailFolders/{id}/messages/delta` — returns changed messages since last sync
- Returns `@odata.nextLink` (more pages) or `@odata.deltaLink` (use next time for incremental changes)
- Can filter by changeType: created, updated, deleted
- Per-folder operation (must track each folder individually, unlike Gmail's global History API)
- Can combine with **webhook subscriptions** for push+pull: webhook notifies of change, then delta query fetches actual changes
- **No expiration concern like Gmail's History API** (which expires after ~30 days) — delta tokens remain valid as long as the mailbox exists, though Microsoft recommends periodic full syncs

### Rate Limits

- 10,000 API requests per 10 minutes per app per mailbox
- 4 concurrent requests per app per mailbox
- 150MB upload within 5 minutes per app per mailbox
- Global: 130,000 requests per 10 seconds per app across all tenants
- Throttled requests get HTTP 429 with `Retry-After` header

### EWS vs Graph Comparison

| Aspect | EWS | Microsoft Graph |
|--------|-----|-----------------|
| Protocol | SOAP/XML | REST/JSON |
| Microsoft recommendation | **Deprecated** | **Recommended** |
| Exchange Online | Dying (Oct 2026 → Apr 2027) | Fully supported |
| Personal accounts (Outlook.com) | No | **Yes** |
| On-premises Exchange | **Supported indefinitely** | Not supported |
| Auth | Basic (killed), NTLM, Kerberos, OAuth2 | OAuth2 only |
| Delta/sync | SyncFolderItems + streaming notifications | Delta query + webhooks |
| Ease of implementation | Complex (SOAP/XML, verbose, namespaces) | Much easier (REST/JSON, OData) |
| SDK support | Legacy, not maintained | Active SDKs in 8 languages |

**Known Graph gaps** (things EWS can do that Graph cannot yet):
- Archive mailbox access (in progress)
- Public folder CRUD (will be restricted to Outlook clients only)
- Folder-associated information / user configuration (in progress)
- Some recurring event delta edge cases

None of these gaps are relevant for a consumer/prosumer email client.

---

## Rust Libraries and Tools

### graph-rs-sdk

**The primary candidate for adding Microsoft Graph support to Ratatoskr.**

- **Crate**: [graph-rs-sdk](https://crates.io/crates/graph-rs-sdk) v3.0.1
- **GitHub**: [sreeise/graph-rs-sdk](https://github.com/sreeise/graph-rs-sdk)
- **Stars**: ~145
- **License**: MIT
- **Downloads**: ~73,500 total, ~2,400/month
- **Last significant activity**: August 2025 (not hyper-active, but not dead)
- **Maintainer**: Sean Reeise (single maintainer — same risk pattern as many Rust ecosystem crates)

**What it provides**:
- Full Microsoft Graph API client, auto-generated from Microsoft's official [OpenAPI specs](https://github.com/microsoftgraph/msgraph-metadata)
- **Mail operations**: List/get/create/update/delete messages, send mail, mail folders, delta queries, attachments, paging
- **OAuth2/MSAL**: Built-in auth support — authorization code, client credentials, device code, PKCE. Has `graph-oauth` companion crate. Interactive WebView auth via `wry`/`tao`
- **Features**: Async + blocking, streaming/channel/iterator paging, upload sessions, OData query parameters, automatic token refresh, in-memory token cache
- Covers far more than mail (OneDrive, Teams, Calendar, Users, Groups) — we'd use a subset

**Assessment**: Maps well to our existing `EmailProvider` abstraction. We would create a `GraphProvider` (either TypeScript calling REST endpoints via Tauri HTTP plugin, or a Rust module using this SDK). The OAuth flow is nearly identical to what we already do for Gmail. Delta sync maps to our existing sync architecture.

**Concern**: Single maintainer, last commit ~8 months ago. If it becomes stale, the code-generation approach (below) is the fallback.

### Other Rust Graph Crates

| Crate | Assessment |
|-------|-----------|
| [msgraph-rs](https://github.com/whitefox82/msgraph-rs) | 3 stars, GPL-3.0 license (incompatible), much less complete than graph-rs-sdk. Skip. |
| [async-mailer-outlook](https://crates.io/crates/async-mailer-outlook) | Send-only via Graph API. Too narrow. |
| `azure_identity` (official Azure SDK) | Useful for auth flows but does not include Graph client. Azure SDK covers Azure services, not M365 APIs. |
| [outlook-pst-rs](https://github.com/microsoft/outlook-pst-rs) (Microsoft) | PST file parser. Actively maintained (commits today). Only relevant for offline import, not live mailbox access. |

### Code Generation Approaches

Since Microsoft Graph has an [official OpenAPI spec](https://github.com/microsoftgraph/msgraph-metadata) (continuously updated), we could generate a focused Rust client for just the mail endpoints.

| Tool | Assessment |
|------|-----------|
| [oas3-gen](https://github.com/eklipse2k8/oas3-gen) | **Most promising.** OpenAPI 3.1 type + client generator for Rust with explicit `--odata-support` flag for Microsoft Graph. MIT license, 15 stars, active (Feb 2026). If graph-rs-sdk proves stale, this is the best fallback. |
| [progenitor](https://github.com/oxidecomputer/progenitor) (Oxide Computer) | Mature OpenAPI 3.0.x generator (870 stars, MPL-2.0, active). Primarily targets Dropshot-style APIs — Graph's OData extensions may cause issues. Worth trying but unconfirmed. |
| [Kiota](https://github.com/microsoft/kiota) (Microsoft) | Microsoft's official API client generator. 3,671 stars, active. **Does NOT support Rust.** Issue #4436 tracks community interest — Microsoft says they'd "provide guidance" but won't build it. Dead end for Rust. |

**The raw HTTP approach**: Given that our app already makes raw HTTP calls via Tauri's HTTP plugin for the Gmail API (`GmailClient`), we could call Graph REST endpoints directly without any SDK. The API is straightforward REST/JSON — no SOAP/XML complexity. This would mean zero additional Rust dependencies and full control, at the cost of writing the request/response types ourselves.

### EWS Libraries in Rust

There is effectively **nothing usable**:

| Library | Assessment |
|---------|-----------|
| [thunderbird/ews-rs](https://github.com/thunderbird/ews-rs) | Types and XML ser/de only. No HTTP client, no auth, no autodiscover. Not on crates.io. Internal crates (ews_xpcom, moz_http) are coupled to Gecko. |
| [Dust-Mail/ews-client](https://github.com/Dust-Mail/ews-client) | Autodiscover protocol only. No mail operations. 8 stars, last commit Oct 2023. |

Building a full EWS client in Rust would require: XML namespace-aware ser/de (the hard part — why Thunderbird built `xml_struct`), SOAP envelope handling, autodiscover protocol (POX + SCP + redirect chains), NTLM authentication (for on-prem), and all the EWS operations. This is months of work for a protocol being deprecated.

---

## Reference Implementations in Other Languages

For understanding the protocols and informing our implementation:

| Library | Language | Best For |
|---------|----------|----------|
| [exchangelib](https://github.com/ecederstrand/exchangelib) (Python) | Python | Gold standard EWS reference. Every operation, every quirk documented. BSD-2-Clause. |
| Microsoft Graph SDKs ([C#](https://github.com/microsoftgraph/msgraph-sdk-dotnet), [JS](https://github.com/microsoftgraph/msgraph-sdk-javascript), [Go](https://github.com/microsoftgraph/msgraph-sdk-go), [Python](https://github.com/microsoftgraph/msgraph-sdk-python)) | Various | Official Graph SDK patterns — auth flows, paging, error handling, delta sync. All generated via Kiota. |
| [EmailEngine](https://github.com/postalsys/emailengine) (Node.js) | JS | Unifies IMAP, SMTP, Gmail API, **and Graph API** behind a single REST API. Good reference for multi-provider abstraction (similar to our `EmailProvider`). Source-available (commercial license). |

---

## Open Source Clients with Exchange Support

| Project | Language | Exchange Method | Assessment |
|---------|----------|----------------|-----------|
| **Thunderbird 145** | C++/JS/Rust | EWS (via ews-rs) | Production. Graph API planned for Q1 2026 to meet Oct 2026 deadline. |
| **GNOME Evolution** | C | EWS (evolution-ews) | Production. Discussing EWS deprecation impact; no confirmed Graph migration. |
| [Dust-Mail/core](https://github.com/Dust-Mail/core) | Rust/React/Tauri | Autodiscover only | Abandoned (3 stars, Mar 2024). Same tech stack as us (Tauri+React). |
| [rustmailer](https://github.com/rustmailer/rustmailer) | Rust | Claims Graph API | Proprietary (license key required). Can't use code, but confirms the approach is viable. |
| [MailClient](https://github.com/Gabi11124/MailClient) | — | Graph API + IMAP | Open source desktop client supporting Graph. Reference for Graph mail integration. |
| [prospect-mail](https://github.com/julian-alarcon/prospect-mail) | Electron | OWA wrapper | Just wraps Outlook Web App in Electron. No actual protocol implementation. |

**Notably**: There is a [Tauri discussion #5534](https://github.com/tauri-apps/tauri/discussions/5534) about signing in users and calling Microsoft Graph from a Tauri desktop app — worth reading for auth flow patterns.

---

## Integration Strategy for Ratatoskr

### Recommended approach: Microsoft Graph API

**Why Graph, not EWS**:
- EWS is dying for Exchange Online (the majority use case) — Oct 2026 block, Apr 2027 permanent removal
- Graph works with personal Outlook.com/Hotmail accounts (EWS doesn't)
- REST/JSON is dramatically simpler than SOAP/XML with namespaces
- Delta sync, webhook subscriptions, and richer features (categories, rules, focused inbox)
- Active SDK ecosystem vs. archived/deprecated EWS SDKs

**Implementation options** (in order of pragmatism):

1. **TypeScript Graph provider calling REST directly** — Lowest friction. Create a `GraphProvider` class implementing `EmailProvider`, making HTTP calls via Tauri's HTTP plugin (same pattern as `GmailClient`). Write the request/response types ourselves. Zero new Rust dependencies. Full control.

2. **Rust module using `graph-rs-sdk`** — More robust, type-safe. Add `graph-rs-sdk` to Cargo.toml, create Tauri commands for Graph operations (like our IMAP commands). The SDK handles OAuth, paging, delta queries. Risk: single maintainer, last active Aug 2025.

3. **Rust module with generated client** — Use `oas3-gen` with `--odata-support` to generate a focused Rust client from Microsoft's OpenAPI spec, covering only the mail endpoints we need. More work upfront, but no dependency on a third-party SDK maintainer.

**Auth flow** (nearly identical to our existing Gmail OAuth):
1. Register Azure AD app as multi-tenant + personal accounts
2. User provides their own app registration (like Gmail Client ID in Settings), or we ship a default
3. Authorization Code + PKCE flow → localhost redirect (port 17248-17251, same server)
4. Token endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/token`
5. Scopes: `Mail.ReadWrite Mail.Send MailboxSettings.ReadWrite offline_access`
6. Refresh token flow identical to Gmail's

**Sync architecture**:
- Delta queries per folder → maps to our existing per-account sync loop (60s interval)
- Delta tokens don't expire like Gmail History API (~30 days) — more reliable
- Optional: webhook subscriptions for real-time push (requires a public endpoint or polling fallback)

**What about on-premises Exchange?**
- On-prem Exchange supports IMAP/SMTP — users can already connect via our IMAP provider
- On-prem-only EWS access is niche and not worth the SOAP/XML complexity
- If demand emerges, consider it later — `ews-rs` types + `reqwest` could work, but it's a significant build

### Quick win: IMAP + OAuth2 for Outlook.com

Before building full Graph support, we could add **XOAUTH2 authentication for IMAP** to support Outlook.com/Exchange Online users today:
- Exchange Online supports IMAP with OAuth2 (XOAUTH2 SASL mechanism)
- Our IMAP provider already has an XOAUTH2 authenticator in `connection.rs`
- We'd need: Azure AD app registration, OAuth flow for Microsoft (same as Graph), pass the access token to IMAP AUTHENTICATE
- This gives Outlook users basic email access immediately, with Graph as a future upgrade for richer features

### Timeline suggestion

1. **Now**: Add Microsoft OAuth2 support + IMAP XOAUTH2 for Outlook.com (quick win, uses existing IMAP infrastructure)
2. **Next**: Build `GraphProvider` implementing `EmailProvider` (REST calls via Tauri HTTP, delta sync, send/reply/forward)
3. **Later**: Evaluate whether on-premises Exchange EWS support has enough demand to justify the SOAP/XML complexity
