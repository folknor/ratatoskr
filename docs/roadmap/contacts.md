# Contacts & Groups

**Tier**: 1 — Blocks switching from Outlook
**Status**: ⚠️ **Partial** — Local contact DB with frequency ranking, avatars, notes. `seen_addresses` auto-collected during sync with direction-weighted ranking (Phase 1). FTS5 prefix search on contacts with email-aware tokenizer, two-tier ranking (explicit > observed), LIKE fallback for seen_addresses (Phase 2). Exchange personal contacts sync via Graph `/me/contacts` with per-folder delta sync, 410 fallback, reference-counted deletes, `display_name_overridden` flag for user edit protection (Phase 3). Compose autocomplete returns explicit contacts, server-synced contacts, and observed addresses. Gravatar integration, contact sidebar with stats/colleagues/shared files. **Missing**: Google People API sync, distribution list/group resolution, contact photos from server.

---

- **What**: Exchange-stored personal contacts, distribution lists, M365 Groups
- **Dependency**: Needed for @mentions, compose autocomplete, group expansion

## Cross-provider behavior

| Provider | Contacts API | Groups |
|---|---|---|
| Exchange (Graph) | `/me/contacts`, `/me/contactFolders` | Distribution lists, M365 Groups, security groups via `/groups` |
| Gmail API | Google People API (`people.connections.list`) | Google Groups (admin SDK, limited access) |
| JMAP | No standardized contacts (JSContact is separate RFC, Stalwart support varies) | None |
| IMAP | Nothing | Nothing |

## Pain points

- JMAP/IMAP accounts have no server-side contacts at all. 100% local. Users who only use Stalwart+IMAP need a fully functional local address book that doesn't feel like a second-class citizen.
- Group resolution is recursive: an M365 Group can contain other groups. Distribution lists can be nested. Need to resolve to final email addresses without infinite loops.
- Google Groups are admin-scoped: a normal user can't list group members via the API unless they're an admin or the group is public. May need to fall back to locally-observed recipients from past group emails.
- Contact photos: Exchange and Google both support contact photos. These should be cached locally and displayed in the message list/compose. For JMAP/IMAP accounts, no photos unless BIMI provides a logo.
- M365 Groups are overloaded: a Group is simultaneously a shared mailbox, a Teams team, a SharePoint site, and a Planner plan. For our purposes we only care about "list of email addresses", but the API surface is complex.
- Sync frequency: contacts change less often than email, but a stale contact list means autocomplete misses new hires. Need a sensible sync interval (hourly? daily?) and delta sync where supported (Graph has `/me/contacts/delta`).

## Implementation Phases

### Phase 1 — `seen_addresses` ingestion ✅

Parse `From`/`To`/`Cc`/`Reply-To` during sync via `mail-parser`. Populate `seen_addresses` with direction-weighted ranking and recency decay. Canonicalize emails, resolve display name conflicts. Immediate autocomplete value for every provider, forces core schema and ranking decisions early.

### Phase 2 — FTS5 contact search ✅

Add `contacts_fts` (FTS5) with email-aware tokenizer (`tokenchars='@._-'`), prefix indexes, and content-sync triggers. Two-tier ranking: explicit contacts above observed addresses. LIKE fallback for graceful degradation. Validates the explicit > observed ranking model before provider sync lands.

### Phase 3 — Exchange personal contacts sync ✅

Graph `/me/contacts` with per-folder delta sync via `graph_contact_delta_tokens`. Source tracking (`source` column: `'user'` vs `'graph'`), `display_name_overridden` flag to protect user edits from sync overwrites. Reference-counted deletes via `graph_contact_map` — shared emails only removed when no mappings remain. 410 Gone fallback to full sync with stale contact pruning. Syncs every 20th mail cycle. Top-level contact folders only (nested folders deferred).

### Phase 4 — Local groups + compose expansion

Local contact groups in SQLite (name + address list). DFS expansion with cycle detection via visited set. Compose UI integration. Useful immediately for JMAP/IMAP users with no server-side groups.

### Phase 5 — Google People API sync

`people.connections.list` with sync tokens (7-day expiry). `otherContacts` as lower-priority autocomplete candidates. Respect quota limits on full syncs.

### Phase 6 — Exchange group resolution

`/groups/{id}/transitiveMembers` for M365 Groups and distribution lists. Server handles recursion and cycle detection. Track partial resolution (hidden members). Skip dynamic DLs and personal DLs for now.

### Phase 7 — Contact photos

Schema and cache hooks can be designed earlier, but actual photo fetching lands here to avoid request volume and complexity before core sync/search is proven. Exchange: one request per photo, cache by `changeKey`. Google: public URLs with `?sz=` resizing. Local disk cache with eviction.

### Phase 8 — CardDAV follow-up

`libdav` + `calcard` for Fastmail, Stalwart, and other CardDAV servers. Etag-based sync. Covers JMAP accounts once server support matures.

---

## Research

**Date**: March 2026
**Context**: Ground-up implementation for the iced (pure Rust) rewrite. No assumptions about existing Tauri/React schemas or backend. All provider interactions are raw HTTP via `reqwest` (Graph, Google) or `jmap-client` (JMAP).

---

### 1. Exchange (Microsoft Graph) Contacts

#### Personal contacts: `/me/contacts`

`GET /me/contacts` returns a paginated collection of contact resources. Default page size is 10, max 999 via `$top`. Pagination uses `@odata.nextLink` with server-managed `$skip` tokens.

**Field set** is generous: `displayName`, `givenName`, `surname`, `emailAddresses[]` (array of `{name, address}`), `businessPhones[]`, `homePhones[]`, `mobilePhone`, `companyName`, `department`, `jobTitle`, `officeLocation`, `businessAddress`, `homeAddress`, `otherAddress`, `birthday`, `personalNotes`, `categories[]`, `imAddresses[]`, plus metadata (`id`, `createdDateTime`, `lastModifiedDateTime`, `changeKey`, `parentFolderId`). Use `$select` to limit fields.

Contact folders (`/me/contactFolders`) support nesting. Most users have one folder ("Contacts"), but enterprise users may have many.

**Gotcha**: The `emailAddresses` array can contain multiple entries with no explicit "primary" marker. Display name in `emailAddresses[].name` often just repeats the address itself.

#### Delta sync: `/me/contactFolders/{id}/contacts/delta`

Delta sync is per-folder, not global. The mechanism uses two token types:

- `@odata.nextLink` with `$skiptoken` — more pages in the current round
- `@odata.deltaLink` with `$deltatoken` — round complete, save this for next sync

Deletions appear as objects with `@removed` annotation. Supports `$select` to limit returned properties.

**Token expiration**: Not fixed. In practice seems to be weeks to months for contacts, but there is no SLA. If a token expires, you get `410 Gone` and must do a full sync.

**Architecture implication**: Must store a `$deltatoken` per (account, contact_folder) pair.

#### Contact photos

`GET /me/contacts/{id}/photo/$value` returns JPEG. One HTTP request per photo — cannot batch-fetch. For 500 contacts, that's 500 requests for avatars. Need aggressive caching: fetch once, re-fetch only when `changeKey` changes (available via delta sync).

Contact photos are distinct from user profile photos (`/users/{id}/photo`). A personal contact's photo is user-uploaded; an org directory member's photo comes from Entra ID.

#### M365 Groups and distribution lists: `/groups`

| Type | `groupTypes` | `mailEnabled` | `securityEnabled` | Notes |
|------|-------------|---------------|-------------------|-------|
| M365 Group | `["Unified"]` | true | false | Has a shared mailbox, Teams integration |
| Distribution list | `[]` | true | false | Mail-only |
| Mail-enabled security group | `[]` | true | true | Rare |
| Security group | `[]` | false | true | No email, skip |

**Member resolution**: `/groups/{id}/transitiveMembers` returns all members with nested groups flattened — **the server does cycle detection and recursion for you**. Can filter: `/groups/{id}/transitiveMembers/microsoft.graph.user`.

**Gotcha**: Dynamic distribution groups (membership defined by LDAP filter) are **not supported by Microsoft Graph at all**. Only resolvable via Exchange Online PowerShell.

**Gotcha**: Personal distribution lists (contact groups created by a user in Outlook) are stored as `IPM.DistList` items. **Microsoft Graph v1.0 has no first-class API for these.** The workaround requires parsing binary `PidLidDistributionListStream` via extended properties. Consider skipping personal DLs and letting users recreate them as local contact groups.

#### People API: `/me/people`

Returns contacts ranked by relevance (communication frequency, organizational proximity). Merges personal contacts with directory users. **Microsoft has put this endpoint in maintenance mode and recommends `/search` instead.** The Search People API (`POST /search/query` with `entityTypes: ["person"]`) supports KQL syntax and returns ranked results. Better for autocomplete.

#### Organization contacts / GAL

`GET /contacts` (not `/me/contacts`) returns organizational contacts from Entra ID. For the full GAL (org contacts + internal users), query `/users` as well. There is no single "GAL" endpoint — the Search People API queries across all sources.

**Permission note**: `/contacts` (org contacts) requires `OrgContact.Read.All`, which is an admin-consent permission. May not be available in all tenants.

---

### 2. Google People API

#### `people.connections.list`

`GET https://people.googleapis.com/v1/people/me/connections` with a required `personFields` mask. Pagination: `pageSize` 1-1000 (default 100), `pageToken` for continuation.

**Sync tokens**: Set `requestSyncToken=true` on initial full sync. Final page returns `nextSyncToken`. Subsequent calls with `syncToken` return only changes. Deleted contacts appear with `PersonMetadata.deleted = true`. **Tokens expire after exactly 7 days** — must full-sync again.

**Gotcha**: When using `syncToken`, all other parameters must match the original request. Changing `personFields` or `sortOrder` invalidates the token.

**Gotcha**: Google imposes a fixed, non-increasable quota on the first page of full syncs — cannot full-sync frequently.

**Gotcha**: Writes may have a propagation delay of several minutes for sync requests.

#### Other Contacts (auto-collected)

`GET https://people.googleapis.com/v1/otherContacts`. Contacts Google auto-creates from interactions. **Severely limited fields**: only `names`, `emailAddresses`, `phoneNumbers`, `photos`, `metadata`. Read-only. Requires separate scope: `contacts.other.readonly`. Supports sync tokens with 7-day expiry.

Useful as autocomplete candidates below explicit contacts in priority.

#### Contact photos

Photos come as a `url` field pointing to `lh3.googleusercontent.com` with `?sz={pixels}` for resizing. URLs are persistent and publicly accessible (no auth needed), but change when photo is updated.

#### Google Groups

**The Groups API requires admin SDK access.** A normal (non-admin) Google Workspace user cannot list group members. The only workaround is locally observing who appears in Cc/To when emails are sent to that group address — the `seen_addresses` approach. This is a fundamental limitation.

#### Rate limits

Default ~90 requests/minute/user for read operations. Generally generous enough for a desktop client syncing one user's contacts.

---

### 3. JMAP Contacts (JSContact)

#### Standards status

- **RFC 9553** (JSContact): Published 2024. JSON data model for contact cards.
- **RFC 9554** (extensions): Published 2024.
- **RFC 9555** (vCard conversion): Published 2024. Bidirectional vCard-to-JSContact mapping.
- **RFC 9610** (JMAP for Contacts): Published 2025. JMAP protocol binding — `AddressBook`, `Card`, `CardGroup` types.

Standards are final and complete. The question is implementation.

#### Stalwart's support

Stalwart announced full JMAP for Contacts support (RFC 9610) in October 2025. Also supports CardDAV. **Currently the only JMAP server that supports JMAP for Contacts.** Fastmail uses CardDAV for contacts.

#### `jmap-client` crate support

**`jmap-client` v0.4 does not support contacts.** No `AddressBook` or `Card` types. Adding support would require implementing RFC 9610 types. Since `jmap-client` and `calcard` are both Stalwart Labs, there's a reasonable chance they add this eventually.

**Options**:

1. **Wait for `jmap-client` RFC 9610 support.** Unknown timeline.
2. **Use CardDAV** against Stalwart and Fastmail. Both support it. Pragmatic path.
3. **Implement raw JMAP for Contacts** on top of `jmap-client`'s HTTP transport. Medium effort.
4. **Skip server-side contacts for JMAP accounts.** Rely on `seen_addresses`. MVP approach covering 90% of value.

**Recommendation**: Option 4 for MVP, option 2 (CardDAV) as follow-up.

---

### 4. CardDAV as a Fallback

#### When CardDAV matters

CardDAV (RFC 6352) is the universal contact sync protocol. Providers: Fastmail, Stalwart, iCloud, Nextcloud, Google (undocumented), Yahoo.

#### Rust CardDAV crates

| Crate | Version | Downloads | Status | Notes |
|-------|---------|-----------|--------|-------|
| [`libdav`](https://crates.io/crates/libdav) | 0.10.3 | 19K total, 3K recent | Active (Mar 2026) | CalDAV + CardDAV. Used by Pimalaya. Async (`reqwest`). |
| [`carddav`](https://crates.io/crates/carddav) | 0.1.1 | 3.5K total, 19 recent | Dead (2018) | Unusable. |

**`libdav` is the only viable CardDAV client crate.** Handles WebDAV PROPFIND/REPORT, vCard download, sync via ctag/etag comparison, addressbook discovery. Sync is etag-based: list all resources, diff etags, download changed vCards. Fine for contact-sized datasets.

#### vCard parsing crates

| Crate | Version | Downloads | Status | Notes |
|-------|---------|-----------|--------|-------|
| [`calcard`](https://crates.io/crates/calcard) | 0.3.2 | 25K total, 9.5K recent | Active (Dec 2025) | Stalwart Labs. vCard + JSContact, bidirectional conversion. Apache-2.0/MIT. |
| [`vcard_parser`](https://crates.io/crates/vcard_parser) | 0.2.2 | 9.5K total | Low activity | RFC 6350 only. |
| [`vcard`](https://crates.io/crates/vcard) | 0.4.13 | 50K total | Low activity | Basic vCard 3.0/4.0. |
| [`ical`](https://crates.io/crates/ical) | 0.11.0 | 927K total | Moderate | iCalendar + vCard. High downloads but dated. |

**`calcard` is the clear winner.** Only crate handling both vCard and JSContact with bidirectional conversion. Lenient parsing. Actively maintained by Stalwart Labs.

---

### 5. Auto-collected Contacts / `seen_addresses`

Most important contact source for IMAP and JMAP accounts lacking server-side contacts.

#### Header parsing

Relevant headers: `From`, `To`, `Cc`, `Reply-To`, `Sender`. **`mail-parser` (already a dependency)** handles all of these, returning structured `Addr` with display name and email. No additional crate needed.

#### Ranking algorithm

```
score = Σ (direction_weight × recency_decay(date))
```

- **Direction weight**: `sent_to` = 3.0, `sent_cc` = 1.5, `received_from` = 1.0, `received_cc` = 0.5
- **Recency decay**: `1.0 / (1.0 + days_since / 90.0)` — halves after 90 days

#### Dedup strategies

Canonicalize on lowercase email. For display name, priority: explicit contact name > most recent from sent message > most frequent across all messages > email address itself. Store `display_name_source` for provenance.

#### Privacy

Tag contacts by source (`exchange_sync`, `google_sync`, `jmap_sync`, `carddav_sync`, `local_observed`, `user_created`). Only write-back contacts with a server source.

---

### 6. Group / Distribution List Resolution

#### Exchange: Server does the work

`GET /groups/{id}/transitiveMembers/microsoft.graph.user` handles recursive expansion, cycle detection, deduplication. **No client-side recursive resolution needed for Exchange.**

Exceptions: dynamic distribution groups (not in Graph API), personal distribution lists (no clean Graph endpoint).

#### Google: Cannot resolve

Non-admin users cannot access Google Groups membership. Workaround: harvest membership from observed Cc/To patterns.

#### JMAP/IMAP: Local-only groups

Users create local groups. Simple: name + list of addresses. Store in SQLite.

#### Client-side resolution (local groups)

DFS with visited set for cycle detection. Deduplicate final addresses by canonical email. Cache resolved lists with generation counter.

#### Partial resolution

When nested Exchange groups contain hidden members, `transitiveMembers` omits them silently. Track total count vs resolved count and warn: "Resolved 47 of 52 members (5 hidden)".

---

### 7. Autocomplete

#### SQLite FTS5 vs tantivy

| Aspect | SQLite FTS5 | tantivy |
|--------|-------------|---------|
| Integration | Built into SQLite (already used) | Separate index/storage |
| Prefix search | Native: `MATCH 'jo*'` | Native via PhrasePrefix |
| Performance | ~10-30ms for 10K contacts | Sub-millisecond |
| Binary size | 0 — already linked | +2-4MB |
| Incremental updates | INSERT/DELETE on FTS table | Must commit segments |

**Recommendation: SQLite FTS5.** Contact datasets are small (even enterprise GALs <100K). FTS5 handles prefix search natively, integrates with existing SQLite stack, zero additional dependencies.

```sql
CREATE VIRTUAL TABLE contacts_fts USING fts5(
    email, display_name, company,
    content='contacts', content_rowid='id',
    tokenize='unicode61 remove_diacritics 2'
);
```

#### Ranking

Combine FTS5 BM25 relevance with frequency score and source priority:
```
final_rank = (source_weight × 1000) + (frequency_score × 10) + bm25_rank
```
Where `source_weight`: explicit=3, server_suggested=2, locally_observed=1.

---

### 8. Relevant Rust Crates Summary

#### Already in use

| Crate | Purpose |
|-------|---------|
| `mail-parser` 0.11 | RFC 5322 message + address parsing. Use for `seen_addresses` extraction. |
| `jmap-client` 0.4 | JMAP mail. Does **not** support contacts. |
| `reqwest` | HTTP client for Graph API and Google People API. |

#### Recommended additions

| Crate | Version | Downloads | Purpose | License |
|-------|---------|-----------|---------|---------|
| [`calcard`](https://crates.io/crates/calcard) | 0.3.2 | 25K / 9.5K recent | vCard + JSContact parsing/generation/conversion | Apache-2.0/MIT |
| [`libdav`](https://crates.io/crates/libdav) | 0.10.3 | 19K / 3K recent | CardDAV client. Only if CardDAV fallback implemented. | Apache-2.0/MIT |
| [`image`](https://crates.io/crates/image) | 0.25.10 | 105M / 17.7M recent | Photo decoding/resizing for avatar caching | Apache-2.0/MIT |

#### Evaluated and rejected

| Crate | Reason |
|-------|--------|
| `vcard_parser` 0.2.2 | `calcard` is strictly better |
| `vcard` 0.4.13 | `calcard` supersedes it |
| `ical` 0.11.0 | Primarily iCalendar. `calcard` more complete for vCard. |
| `email-address-parser` 3.0.0-rc5 | `mail-parser` already handles addresses. Redundant. |
| `tantivy` 0.25.0 | Overkill for contact autocomplete. SQLite FTS5 sufficient. |
