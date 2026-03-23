# Protocol & Server Feature Roadmap

Features needed to close the gap with Outlook for enterprise M365/Exchange users processing high email volumes. Organized by adoption impact.

---

## Cross-Provider Architecture

Ratatoskr supports four providers: **Exchange (Graph)**, **Gmail API**, **JMAP (Stalwart)**, **IMAP**. Most features on this roadmap are natively supported by only one or two providers. The architecture must handle this gracefully.

### The Local Fallback Principle

Every feature gets a **local-only implementation** as the baseline. Provider-native support is an optimization on top. The UI never distinguishes — a category is a category whether it syncs to Exchange or lives only in the local DB.

| Feature | Status | Exchange (Graph) | Gmail API | JMAP | IMAP | Local Fallback |
|---|---|---|---|---|---|---|
| [Labels](research-provider-label-colors.md) | ✅ Backend complete (missing: label picker UI) | Native (`categories`) | Labels (partial overlap) | `keywords` | IMAP keywords (limited) | Local-only labels+colors |
| Auto-responses | ⬚ Not started | `automaticRepliesSetting` (read+write, v1.0) | `VacationSettings` (read+write) | `VacationResponse` (read+write, RFC 8621) | ManageSieve (server-dependent) | N/A — requires server |
| Auto-collected contacts | ✅ Done | People API (ranked) | "Other Contacts" | Nothing | Nothing | `seen_addresses` table |
| [@Mentions](mentions.md) | ⚠️ Compose-only feature (missing: @-autocomplete UI). Phase 1 backend to be removed. | N/A | N/A | N/A | N/A | Insert @Name text + add to To/CC |
| [Reactions](reactions.md) | ❌ Dropped as user-facing feature. No unified model across providers. Backend stays for defensive sync. | Read-only (extended props) | MIME reactions (read+write) | Nothing | Nothing | N/A |
| [Scheduled send](scheduled-send.md) | ⚠️ Backend complete (missing: schedule picker UI, "Scheduled" virtual folder) | Native (deferred delivery) | Native | `EmailSubmission.sendAt` | Nothing | Local timer + send-on-wake |
| [Roaming signatures](signatures.md) | ⚠️ Backend complete (missing: compose signature placement UI). Exchange fetch permanently blocked — no public API. | N/A (no API, never will be) | Gmail API settings | JMAP Identity | Nothing | Local-only signatures |
| [Cloud attachments](cloud-attachments.md) | ⚠️ Partial (OneDrive done, Google Drive done) | OneDrive via Graph | Google Drive API | Nothing | Nothing | Local large-file warning only |
| [Tracking blocking](tracking-blocking.md) | ⚠️ Mostly done (remaining: read receipt prompt UI) | N/A (client-side) | N/A (client-side) | N/A (client-side) | N/A (client-side) | Fully local |
| [Shared mailboxes](shared-mailboxes.md) | ⚠️ Partial (Graph sync + sidebar done, JMAP in progress) | Native (delegate access) | Native (delegation) | Shared via ACL | IMAP ACL (RFC 4314) | N/A — requires server support |
| [Public folders](public-folders.md) | ⚠️ Partial (EWS client + sidebar pins done) | Native (legacy Exchange) | Nothing | Nothing | Nothing | N/A — Exchange-only concept |
| [BIMI](bimi.md) | ⚠️ Backend complete (missing: avatar display in message list) | N/A (DNS + headers) | N/A (DNS + headers) | N/A (DNS + headers) | N/A (DNS + headers) | Fully local |

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

- [Labels (Color Flags)](research-provider-label-colors.md) — ✅ Backend complete. Missing: label picker UI, apply/remove from UI. See `docs/labels-unification/problem-statement.md`.
- [Auto-Responses](../auto-responses/problem-statement.md) — ⬚ Not started. Full read/write API on Exchange, Gmail, JMAP. Settings UI + status indicator needed.
- [Tracking Pixel / Read Receipt Blocking](tracking-blocking.md) — ⚠️ Mostly done. Remote image strip + AMP blocking + link tracking indicators all implemented (2026-03-22). Remaining: UI for read receipt prompts.
- [Cloud Attachment Linking](cloud-attachments.md) — ⚠️ Partial (OneDrive + Google Drive upload/permissions done)
- [IMAP CONDSTORE/QRESYNC](imap-condstore-qresync.md) — ⚠️ Phase 2 (CONDSTORE + deletion detection done, VANISHED parsing blocked on async-imap #130)
- [Shared / Delegated Mailboxes](shared-mailboxes.md) — ⚠️ Partial (Graph sync + Autodiscover + sidebar integration done, JMAP Sharing in progress. Remaining: thread loading, compose identity, per-mailbox config)
- [Public Folders](public-folders.md) — ⚠️ Partial (EWS client + offline sync + sidebar pins done. Remaining: thread loading, folder browser, reply/post wiring)

## Tier 2 — Keeps users from going back

Features users notice are missing after a week of daily use.

- [@Mentions](mentions.md) — ⚠️ Compose-only feature: @-autocomplete in compose (insert text + add to To/CC). Phase 1 Exchange backend code to be removed — unnecessary complexity.
- [Roaming Signatures](signatures.md) — ⚠️ Backend complete (Gmail + JMAP sync). Missing: signature placement in compose UI. Exchange fetch permanently blocked (Microsoft confirmed no plans for API).
- [Scheduled Send](scheduled-send.md) — ⚠️ Backend complete (server delegation + overdue handling). Missing: schedule picker UI, "Scheduled" virtual folder
- ~~[Reactions](reactions.md) — Dropped. No unified cross-provider model. Backend defensive code remains.~~

## Tier 3 — Differentiators and polish

Features that go beyond Outlook parity into "this client is actually better."

- [BIMI](bimi.md) — ⚠️ Backend complete (DNS + SVG + cache). Missing: BIMI logo display in message list avatars
---

## Implementation notes

- **No on-disk migration needed**: All of the above syncs from the server. When cutting over to the iced frontend, start fresh and re-sync.
- **Contacts are the critical dependency**: @mentions, compose autocomplete, and group resolution all depend on having contacts synced locally first.
- **Graph API carries the load**: Most Tier 1/2 features are Exchange/Graph-native. The Graph provider will carry the heaviest implementation burden.
- **JMAP is naturally well-aligned**: JMAP's `threadId`, `keywords`, and `EmailSubmission` cover categories, threading, and scheduled send natively for Stalwart users.
- **Local DB is the unifier**: Every feature uses the local DB as the canonical client-side store. Some rows sync to a server, some are local-only. The UI layer queries the local DB and never cares about provenance. The sync layer handles bidirectional updates per-provider.
- **The `ProviderOps` trait grows**: Many of these features imply new methods on `ProviderOps` (or a companion trait). Each method has a default implementation that does nothing (local-only fallback), and providers override where they have native support.
