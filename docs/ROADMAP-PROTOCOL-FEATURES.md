# Protocol & Server Feature Roadmap

Features needed to close the gap with Outlook for enterprise M365/Exchange users processing high email volumes. Organized by adoption impact.

---

## Cross-Provider Architecture

Ratatoskr supports four providers: **Exchange (Graph)**, **Gmail API**, **JMAP (Stalwart)**, **IMAP**. Most features on this roadmap are natively supported by only one or two providers. The architecture must handle this gracefully.

### The Local Fallback Principle

Every feature gets a **local-only implementation** as the baseline. Provider-native support is an optimization on top. The UI never distinguishes — a category is a category whether it syncs to Exchange or lives only in the local DB.

| Feature | Exchange (Graph) | Gmail API | JMAP | IMAP | Local Fallback |
|---|---|---|---|---|---|
| Categories | Native (`categories`) | Labels (partial overlap) | `keywords` | IMAP keywords (limited) | Local-only labels+colors |
| Contacts | Native (`/me/contacts`) | People API | Not standardized | Nothing | Local address book |
| Auto-collected contacts | People API (ranked) | "Other Contacts" | Nothing | Nothing | `seen_addresses` table |
| @Mentions | Native (`mentions`) | Nothing | Nothing | Nothing | Local-only, no server flag |
| Reactions | Native (`reactions`) | Nothing | Nothing | Nothing | Local-only |
| Scheduled send | Native (deferred delivery) | Native | `EmailSubmission.sendAt` | Nothing | Local timer + send-on-wake |
| Roaming signatures | Native (roaming settings) | Gmail API settings | Nothing | Nothing | Local-only signatures |
| Cloud attachments | OneDrive via Graph | Google Drive API | Nothing | Nothing | Local large-file warning only |
| Tracking blocking | N/A (client-side) | N/A (client-side) | N/A (client-side) | N/A (client-side) | Fully local |
| Shared mailboxes | Native (delegate access) | Native (delegation) | Shared via ACL | IMAP ACL (RFC 4314) | N/A — requires server support |
| Public folders | Native (legacy Exchange) | Nothing | Nothing | Nothing | N/A — Exchange-only concept |
| BIMI | N/A (DNS + headers) | N/A (DNS + headers) | N/A (DNS + headers) | N/A (DNS + headers) | Fully local |

### Multi-Account UX

Users may have 3 Exchange accounts and 1 JMAP account. Key design decisions:

- **Contacts**: Unified autocomplete across all accounts, per-account storage. Compose autocomplete searches everything; contact management UI shows provenance (which account owns each contact). New contacts prompt for which account to create on (or local-only). Dedup by email address for display, but never merge underlying records.
- **Categories**: Per-account category lists (Exchange users may have different category sets on different accounts). Category picker in UI shows union of all, disambiguates on conflict (e.g., "Red" means different things on two accounts — show account badge).
- **Scheduled send**: Account-agnostic in compose UI. Implementation delegates to server-native if available, falls back to local timer. User doesn't know or care.
- **General rule**: Read path is unified (search/autocomplete/display merges across accounts). Write path is account-aware (changes route to the correct provider or local store).

### Auto-Collected Contacts

Separate from explicit contacts. Three tiers of contact data:

1. **Explicit contacts** — user deliberately created or synced from server. Highest trust/priority in autocomplete.
2. **Server-suggested** — Exchange People API (ML-ranked by interaction patterns), Gmail "Other Contacts". Synced periodically. Medium priority.
3. **Locally observed** — addresses seen in From/To/Cc headers across all accounts. Built passively during sync and message display.

Local schema: `seen_addresses` table with `email`, `display_name`, `last_seen`, `interaction_count`, `direction` (sent > received for ranking — sending to someone is a stronger signal than receiving from them), `account_id`.

**Pain points**:
- Display name conflicts: same email, different display names across messages. Use most-recent or most-frequent.
- Volume: at hundreds of emails/day, this table grows fast. Need periodic pruning or decay (addresses not seen in 12+ months drop in rank).
- Privacy: locally-observed contacts should never sync upstream. They're client-side only.
- Dedup against explicit contacts: if `john@example.com` exists in the real contacts store, the `seen_addresses` entry should defer to it (use the explicit contact's display name, photo, etc.).

---

## Tier 1 — Blocks switching from Outlook

These are features enterprise users actively rely on daily. Missing any of these is a reason not to switch.

### Categories (Color Flags)

- **What**: Per-user string labels with associated colors, applied to messages
- **Scope**: Per-user on personal mailboxes; shared visibility on shared mailboxes and public folders

**Cross-provider behavior**:
| Provider | Native support | Behavior |
|---|---|---|
| Exchange (Graph) | Full — `categories` on messages, master list via `/me/outlook/masterCategories` | Sync master list + per-message categories bidirectionally |
| Gmail API | Labels function as both folders and categories. Color supported. | Map Gmail labels to categories where label is not a system/folder label. Imperfect — Gmail's model conflates the two concepts. |
| JMAP | `keywords` on emails — arbitrary string keys, boolean values. No color. | Use keywords as category names, store colors locally. |
| IMAP | `FLAGS`/keywords — server support varies wildly, many servers limit to system flags only | Local-only categories with IMAP flag sync as best-effort. |

**Pain points**:
- Gmail label/category/folder conflation: need heuristics to decide which labels are "categories" vs structural folders. System labels (`INBOX`, `SENT`, `TRASH`) are obvious, but user-created labels are ambiguous.
- IMAP keyword support is unreliable: some servers silently drop custom keywords, others have hard limits on keyword count. Must detect and fall back to local-only.
- Color mapping: Exchange has a fixed set of preset colors. Gmail has its own color palette. JMAP/IMAP have no color concept. Need a unified color model that round-trips cleanly to Exchange and degrades gracefully elsewhere.
- Shared mailbox categories: on Exchange, categories applied to messages in a shared mailbox are visible to all users with access. This is a feature users rely on for team triage ("I marked it Red, that means it's handled"). Must preserve this behavior for Graph accounts.
- Multi-account category conflicts: user has "Urgent" as red on Account A and blue on Account B. The category picker needs to handle this without confusion.

**Work**: Sync master category list per account, display on messages, allow apply/remove, persist locally, round-trip to server where supported. Local-only fallback for IMAP.

### Contacts & Groups

- **What**: Exchange-stored personal contacts, distribution lists, M365 Groups
- **Dependency**: Needed for @mentions, compose autocomplete, group expansion

**Cross-provider behavior**:
| Provider | Contacts API | Groups |
|---|---|---|
| Exchange (Graph) | `/me/contacts`, `/me/contactFolders` | Distribution lists, M365 Groups, security groups via `/groups` |
| Gmail API | Google People API (`people.connections.list`) | Google Groups (admin SDK, limited access) |
| JMAP | No standardized contacts (JSContact is separate RFC, Stalwart support varies) | None |
| IMAP | Nothing | Nothing |

**Pain points**:
- JMAP/IMAP accounts have no server-side contacts at all. 100% local. Users who only use Stalwart+IMAP need a fully functional local address book that doesn't feel like a second-class citizen.
- Group resolution is recursive: an M365 Group can contain other groups. Distribution lists can be nested. Need to resolve to final email addresses without infinite loops.
- Google Groups are admin-scoped: a normal user can't list group members via the API unless they're an admin or the group is public. May need to fall back to locally-observed recipients from past group emails.
- Contact photos: Exchange and Google both support contact photos. These should be cached locally and displayed in the message list/compose. For JMAP/IMAP accounts, no photos unless BIMI provides a logo.
- M365 Groups are overloaded: a Group is simultaneously a shared mailbox, a Teams team, a SharePoint site, and a Planner plan. For our purposes we only care about "list of email addresses", but the API surface is complex.
- Sync frequency: contacts change less often than email, but a stale contact list means autocomplete misses new hires. Need a sensible sync interval (hourly? daily?) and delta sync where supported (Graph has `/me/contacts/delta`).

**Work**: Sync contacts to local DB per-account, unified autocomplete across all accounts, local address book for accounts without server-side contacts, group resolution for compose.

### Tracking Pixel / Read Receipt Blocking

- **What**: Block remote image loading by default (defeats tracking pixels), suppress MDN (Message Disposition Notification) headers
- **Scope**: Client-side only — identical implementation across all providers

**Pain points**:
- Blocking remote images breaks legitimate email layouts: newsletters, marketing emails, and even some corporate templates rely on remote images for logos, banners, formatting. Need a "load images for this message" toggle and a per-sender/per-domain allowlist.
- Read receipts (`Disposition-Notification-To` header): some corporate environments expect read receipts. Blocking them entirely may violate workplace expectations. Need a per-account or per-sender policy (auto-send, ask, never).
- Tracking pixels are invisible 1x1 images — but some "tracking" is done via uniquely-parameterized URLs on visible images. Blocking all remote images is the only reliable defense, but it's heavy-handed.
- AMP for Email: some senders use AMP emails that phone home. Treat AMP content as remote content and block by default.
- HTML email `<link>` tags and CSS `@import`: remote CSS is another tracking vector. Block external stylesheets, inline only.

**Work**: Default-block remote images in HTML render, strip/suppress `Disposition-Notification-To`, per-sender allowlist, "load images for this message" one-shot button.

### Cloud Attachment Linking (OneDrive / Google Drive)

- **What**: Attachments above a size threshold uploaded to cloud storage, shared as links instead of inline

**Cross-provider behavior**:
| Provider | Cloud storage | Auto-linking |
|---|---|---|
| Exchange (Graph) | OneDrive via `/me/drive` | Outlook auto-converts large attachments to OneDrive links |
| Gmail API | Google Drive | Gmail prompts for Drive link above 25MB |
| JMAP | None built-in | N/A |
| IMAP | None built-in | N/A |

**Pain points**:
- Incoming link detection: users receive emails with OneDrive/Google Drive/SharePoint links that should render as "attachments" in the UI, not as raw URLs in the body. Need URL pattern detection for major cloud providers and rendering them as downloadable attachment chips.
- Permission management: uploading to OneDrive and sharing a link requires setting permissions (org-wide? specific recipients? anyone with link?). Defaulting wrong is either a security issue (too open) or a usability issue (recipient can't access).
- Offline compose: user composes offline with a large attachment. Can't upload to OneDrive yet. Need to queue the upload and convert to link on send when connectivity returns.
- JMAP/IMAP accounts: no cloud storage integration. Options are: (a) just send the large file if the server allows it, (b) warn the user about size limits, (c) offer a local integration with a third-party storage provider (complex, probably out of scope initially).
- Mixed accounts in compose: user has an Exchange account and a Stalwart account. Compose defaults to Exchange sender — cloud linking works. They switch sender to Stalwart mid-compose — cloud linking no longer available. UI needs to handle this gracefully.

**Work**: OneDrive upload for Exchange accounts, Google Drive for Gmail accounts, incoming link detection across all providers, graceful degradation for JMAP/IMAP.

### IMAP CONDSTORE/QRESYNC (RFC 7162)

- **What**: Efficient delta sync for IMAP — server tracks mod-sequences, client fetches only changes since last sync
- **Scope**: Stalwart and most modern IMAP servers support this. Critical for users not on Graph/JMAP.

**Pain points**:
- Capability detection: not all IMAP servers support CONDSTORE/QRESYNC. Need to detect via `CAPABILITY` response and fall back to full UID comparison if absent. The fallback must still work at scale (50k+ messages in a mailbox).
- QRESYNC requires `ENABLE QRESYNC` — must be sent after authentication. Some servers advertise QRESYNC but have buggy implementations. Need defensive handling of malformed `VANISHED` responses.
- Mod-sequence storage: need to persist the highest mod-seq per mailbox per account in the local DB, and handle the case where the server's mod-seq resets (e.g., after a mailbox recreation).
- Interaction with message moves: IMAP doesn't have a native "move" operation pre-RFC 6851 (`MOVE` extension). Without `MOVE`, a copy+delete looks like a new message + an expunge, which complicates delta sync.
- Flagged-only changes: CONDSTORE can report that a message's flags changed without re-downloading the message. Need to handle flag-only updates efficiently (update local DB flags, don't re-fetch body).

**Work**: Detect CONDSTORE/QRESYNC capability, implement mod-seq tracking, use `CHANGEDSINCE` in FETCH, handle `VANISHED` for expunges, fall back to UID comparison when unsupported.

### Shared / Delegated Mailboxes

- **What**: Any mailbox the user has delegate access to — shared mailboxes, other users' mailboxes, resource mailboxes (rooms/equipment). In enterprise M365, these auto-appear in Outlook when a user is granted Full Access.
- **Scope**: **Adoption blocker**. Enterprise clients cannot switch until this works. Many M365 orgs have dozens of shared/delegated mailboxes per user. Users switch between personal and delegated mailboxes constantly throughout the day.

#### What actually auto-appears in Outlook

When you add a corporate Exchange account, Outlook may auto-populate additional mailboxes in the sidebar. These can be any of:

1. **Shared Mailboxes** — no license, no direct login. Created by admins for team use (support@, invoicing@, sales@). Delegates are granted access and Exchange **auto-maps** them into Outlook.
2. **User Mailboxes with Full Access** — a fully licensed user mailbox (e.g., `invoicing@company.com` that's actually a regular user account) where the current user has been granted **Full Access**. Exchange auto-maps these identically to shared mailboxes. From the user's perspective in Outlook, they look the same.
3. **Resource Mailboxes** — room or equipment mailboxes. Less commonly auto-mapped, but possible if Full Access was granted for management purposes.

**Exchange auto-mapping does not distinguish between these types.** If a user has Full Access to any mailbox — shared, user, or resource — Exchange can auto-map it. The Graph API treats them uniformly: access via `/users/{mailbox-id}/messages` regardless of type.

This means Ratatoskr doesn't need to care what *kind* of mailbox it is. The implementation is mailbox-type-agnostic: discover what the user has access to, present them uniformly, respect permissions.

#### Permission types (Exchange)

Three separate permission grants that may or may not overlap:

| Permission | What it allows | Typical use |
|---|---|---|
| **Full Access** | Read, write, delete messages in the mailbox. Triggers auto-mapping. | Shared mailboxes, exec assistant accessing boss's inbox |
| **Send As** | Send email with the mailbox's address as the From. Recipient cannot tell it wasn't the mailbox owner. | Shared mailboxes, service accounts |
| **Send on Behalf** | Send email on behalf of the mailbox. From shows "User on behalf of Mailbox". | Exec assistants, team delegation |

A user may have Full Access but not Send As (can read but not impersonate), or Send As but not Full Access (can send from but not read — rare but possible). The client must check each permission independently.

**Cross-provider behavior**:
| Provider | Mechanism | Discovery |
|---|---|---|
| Exchange (Graph) | Full Access / Send As / Send on Behalf grants. Auto-mapping. All mailbox types accessed uniformly via `/users/{id}/messages`. | **No single Graph endpoint lists all accessible mailboxes.** Auto-mapping info is in EWS (`GetMailboxAutoMapping`), not cleanly exposed in Graph. Options: (a) EWS fallback for discovery, (b) user manually adds delegated mailboxes by email address, (c) attempt to access known mailbox IDs and check for 403. |
| Gmail API | Account-level delegation — full inbox access to another user's account | `users.settings.delegates.list` for outbound; inbound delegation is account-level |
| JMAP | ACL-based sharing per mailbox | Server-dependent; Stalwart supports JMAP Sharing (RFC 9670) |
| IMAP | ACL extension (RFC 4314) — per-folder permissions | `GETACL`/`LISTRIGHTS` commands; server support varies widely |

**Pain points**:
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

**Work**: Delegated mailbox discovery (manual-add baseline, EWS auto-mapping as enhancement for Exchange), uniform presentation regardless of mailbox type, sync with configurable depth, auto-set From address with Send As / Send on Behalf distinction, respect per-permission capabilities in UI, shared state delta sync, Sent Items routing, request `*.Shared` OAuth scopes.

### Public Folders

- **What**: Hierarchical shared folder trees accessible to the entire organization (or subsets). Legacy Exchange concept, still heavily used in enterprises that have been on Exchange for 15+ years.
- **Scope**: Exchange-only. Microsoft has been trying to deprecate public folders since Exchange 2013. Enterprise customers refuse. They're still supported in Exchange Online/M365.

**Cross-provider behavior**:
| Provider | Support |
|---|---|
| Exchange (Graph) | Partial Graph support; full support via EWS. Microsoft keeps threatening to deprecate but never does. |
| Gmail API | No equivalent concept |
| JMAP | No equivalent concept |
| IMAP | Shared namespaces (RFC 2342) are conceptually similar but architecturally different |

**Pain points**:
- **Graph API gaps**: public folder access via Graph is limited compared to EWS. Some operations (creating items in public folders, managing permissions) may require falling back to EWS, which Microsoft is also trying to deprecate. Moving target.
- **Hierarchy depth**: public folder trees can be deeply nested — 10+ levels. Orgs use them as filing systems, knowledge bases, shared calendars, even discussion forums. The folder browser UI must handle deep hierarchies efficiently (lazy-load children, don't fetch the entire tree upfront).
- **Volume**: a public folder can contain tens of thousands of items. Same scale challenges as personal mailboxes, but multiplied by the number of public folders the user accesses.
- **Mixed content types**: public folders can contain emails, calendar items, contacts, tasks, notes, and custom forms. For an email client, focus on mail-enabled public folders (which receive email) and email item folders. Ignore calendar/contact/task public folders initially.
- **Permissions**: public folder permissions are a separate system from shared mailbox delegation. Roles include Owner, PublishingEditor, Editor, PublishingAuthor, Author, NonEditingAuthor, Reviewer, Contributor, None. The client must check and respect these per-folder.
- **Favorites**: Outlook lets users "favorite" specific public folders so they appear in the sidebar. Need a similar mechanism — the full public folder tree is too large to display by default.
- **Offline sync**: do not sync public folders by default. Only sync favorited/pinned public folders, and even then with configurable depth. A full public folder sync could be enormous.
- **Organizational inertia**: the reason these matter is that many enterprise customers have decades of institutional knowledge filed in public folders. "Where's the vendor agreement template?" "It's in Public Folders > Legal > Templates > Vendor." This is real workflow that can't be dismissed.

**Work**: Browse public folder hierarchy (lazy-loaded) for Exchange accounts, favorite/pin specific folders to sidebar, sync favorited folders only, respect per-folder permissions, handle mail-enabled public folders as mailboxes. Accept Graph API limitations and consider EWS fallback for operations Graph doesn't support.

---

## Tier 2 — Keeps users from going back

Features users notice are missing after a week of daily use.

### @Mentions

- **What**: `@User` in email body, recipient gets the message auto-flagged
- **Dependency**: Contacts & Groups sync (Tier 1)

**Cross-provider behavior**:
| Provider | Native support | Behavior |
|---|---|---|
| Exchange (Graph) | Full — `mentions` collection on message | Sync mention metadata, auto-flag mentioned user's copy |
| Gmail API | Nothing | Local-only: detect @-patterns in body, no server-side flagging |
| JMAP | Nothing | Local-only |
| IMAP | Nothing | Local-only |

**Pain points**:
- Display: Exchange stores mentions as structured metadata separate from the body HTML. The body contains the display text ("@John Smith") but the `mentions` collection has the resolved email/user ID. Need to correlate the two for highlighting.
- Compose: need @-autocomplete that triggers on `@` character in the compose editor, searches unified contacts, and inserts both the display text and the mention metadata (for Exchange accounts).
- Non-Exchange accounts: can still insert "@John Smith" text in the body (it's just text), but there's no server-side flagging. The recipient's client won't auto-flag it. Acceptable degradation — the visual cue in the body is still useful.
- Parsing incoming @mentions from non-Exchange senders: some people manually type "@Name" in emails. No metadata to parse — could attempt heuristic matching against contacts, but likely not worth the false positives.

**Work**: Display mentions on Exchange messages, @-autocomplete in compose using unified contacts, insert mention metadata for Exchange sends, text-only fallback for other providers.

### Roaming Signatures

- **What**: Signatures stored server-side, synced across clients

**Cross-provider behavior**:
| Provider | Native support | API |
|---|---|---|
| Exchange (Graph) | Roaming signatures (relatively new, ~2021) | Graph beta endpoints / EWS roaming settings |
| Gmail API | Signature in settings | `users.settings.sendAs` — per-alias signatures |
| JMAP | Nothing standardized | N/A |
| IMAP | Nothing | N/A |

**Pain points**:
- First-run experience: user adds their Exchange account, expects their signature to appear in compose automatically. If we don't fetch it, they have to manually recreate it — immediate negative impression.
- HTML signatures: signatures are rich HTML (logos, formatted text, links). Need to render them in compose and handle the boundary between user-typed content and the signature block.
- Multiple signatures: Exchange supports multiple signatures (new email vs reply). Gmail supports per-alias signatures. Need a signature picker or smart default (use reply signature for replies, new-email signature for new compose).
- JMAP/IMAP accounts: purely local signatures. Need a signature editor that stores locally. Same UI, just no server sync.
- Signature images: signatures often contain inline images (company logos, headshots). These are the 14KB PNGs that compound at volume. When fetching a roaming signature, need to extract inline images and deduplicate them in the attachment store.
- Corporate-managed signatures: some orgs push signatures via Exchange transport rules (appended server-side on send). Client-side signature would double up. Need to detect this — if the server appends a signature, don't insert one client-side. Hard to detect reliably.

**Work**: Fetch server-side signature on account setup for Exchange/Gmail, local signature editor for all accounts, handle HTML signatures in compose, smart default selection for reply vs new.

### Scheduled Send

- **What**: Compose now, deliver later at a specified time

**Cross-provider behavior**:
| Provider | Native support | Mechanism |
|---|---|---|
| Exchange (Graph) | Deferred delivery via extended properties | Server holds message until send time |
| Gmail API | Native scheduled send | Server-side |
| JMAP | `EmailSubmission` with `sendAt` | Server holds until send time |
| IMAP/SMTP | Nothing | Client must hold and send |

**Pain points**:
- IMAP fallback: the client must keep the message and send it at the scheduled time. If the client is closed/offline at send time, the message doesn't go. Need to communicate this clearly ("this will only send if Ratatoskr is running at the scheduled time") or implement a send-on-next-wake queue.
- Time zones: user schedules for "9 AM Monday" — whose Monday? Need explicit time zone handling in the schedule picker. Display in local time, store as UTC, convert to recipient's time zone for preview ("arrives ~9 AM EST for recipient").
- Cancellation: for server-side scheduled send, need to support cancel/reschedule. For Exchange this means deleting the deferred message from Drafts. For local fallback, just remove from the local queue.
- Scheduled view: need a "Scheduled" mailbox/view showing all pending scheduled messages across accounts, with ability to edit, reschedule, or cancel. This is a virtual folder, not a real server-side mailbox.
- Multi-account: user has Exchange (server-side scheduling) and IMAP (local scheduling). The UI should be identical. But the reliability characteristics differ — worth a subtle indicator?

**Work**: Schedule picker in compose, server-native send for Exchange/Gmail/JMAP, local timer+queue for IMAP, "Scheduled" virtual view, cancel/reschedule support.

### Reactions

- **What**: Emoji reactions on email messages (Exchange/new Outlook feature)

**Cross-provider behavior**:
| Provider | Native support |
|---|---|
| Exchange (Graph) | Full — `reactions` collection on message |
| Gmail API | Nothing |
| JMAP | Nothing |
| IMAP | Nothing |

**Pain points**:
- Phase 1 priority: even before displaying reactions, must not break when a message has reaction metadata. Defensive deserialization — ignore unknown fields rather than erroring.
- Display: reactions appear as a row of emoji chips below the message (like Slack/Teams). Each chip shows the emoji + count + who reacted. This is a new UI element with no existing equivalent in the client.
- Local-only reactions for non-Exchange: could implement local-only reactions that only the user sees. Questionable value — reactions are social, local-only defeats the purpose. Probably better to just not show the reaction UI on non-Exchange accounts.
- Sync: reactions can change after initial sync (someone reacts later). Need to handle updates to the reactions collection during delta sync.
- Compose: adding a reaction is a PATCH to the message on Graph. Need to handle the case where the user reacts to a message but is offline (queue and sync later? or require connectivity?).

**Work**: Phase 1 — defensive deserialization. Phase 2 — display reactions on Exchange messages. Phase 3 — allow reacting on Exchange accounts. Skip local fallback.

---

## Tier 3 — Differentiators and polish

Features that go beyond Outlook parity into "this client is actually better."

### BIMI (Brand Indicators for Message Identification)

- **What**: Verified sender brand logos displayed next to messages from authenticated domains
- **Scope**: Client-side only — DNS lookup + header check, works identically across all providers

**Pain points**:
- Performance: every unique sender domain requires a DNS TXT lookup + potential SVG fetch. At hundreds of emails/day with diverse senders, this is a lot of lookups. Need aggressive caching per domain (logos don't change often — cache for days/weeks).
- Validation: BIMI requires DMARC pass. Must check `Authentication-Results` header for DMARC status. If the header is missing or DMARC failed, don't display the logo (it's unverified).
- SVG rendering: BIMI logos are SVG Tiny PS (a restricted SVG profile). Need an SVG renderer that handles this subset. Full SVG renderers may work, but the spec is specific about what's allowed.
- VMC (Verified Mark Certificate): full BIMI validation requires checking a VMC certificate (X.509 with the logo embedded). This is the "verified" part. Without VMC checking, you can still display the logo but can't claim it's verified. VMC checking is complex (certificate chain validation, embedded logo comparison). Start without VMC, add later.
- Fallback: for domains without BIMI, fall back to colored initials or gravatar. BIMI should feel like an enhancement over the default avatar, not a required element.

**Work**: DNS BIMI record lookup, SVG logo fetch + cache per domain, check DMARC pass in `Authentication-Results`, display in sender avatar slot. Skip VMC validation initially.

### IMAP SPECIAL-USE (RFC 6154)

- **What**: Server declares which folders are Trash, Sent, Drafts, Archive, Junk via attributes
- **Scope**: IMAP only — other providers have explicit folder semantics in their APIs

**Pain points**:
- Not all IMAP servers support it. Need to fall back to heuristic folder detection (name matching: "Sent", "Sent Items", "Sent Mail", etc., across languages).
- Some servers advertise SPECIAL-USE but have incorrect attributes (e.g., no `\Archive` even though an "Archive" folder exists). Need heuristic fallback even when SPECIAL-USE is present.
- User-customized folder roles: user wants "Old Mail" to be their Archive folder. Need to allow manual override of detected roles.

**Work**: Check `SPECIAL-USE` capability on LIST response, read `\Trash`/`\Sent`/`\Drafts`/`\Archive`/`\Junk` attributes, fall back to name heuristics, allow manual override.

---

## Implementation notes

- **No on-disk migration needed**: All of the above syncs from the server. When cutting over to the iced frontend, start fresh and re-sync.
- **Contacts are the critical dependency**: @mentions, compose autocomplete, and group resolution all depend on having contacts synced locally first.
- **Graph API carries the load**: Most Tier 1/2 features are Exchange/Graph-native. The Graph provider will carry the heaviest implementation burden.
- **JMAP is naturally well-aligned**: JMAP's `threadId`, `keywords`, and `EmailSubmission` cover categories, threading, and scheduled send natively for Stalwart users.
- **Local DB is the unifier**: Every feature uses the local DB as the canonical client-side store. Some rows sync to a server, some are local-only. The UI layer queries the local DB and never cares about provenance. The sync layer handles bidirectional updates per-provider.
- **The `ProviderOps` trait grows**: Many of these features imply new methods on `ProviderOps` (or a companion trait). Each method has a default implementation that does nothing (local-only fallback), and providers override where they have native support.
