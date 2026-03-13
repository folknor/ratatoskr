# Protocol & Server Feature Roadmap

Features needed to close the gap with Outlook for enterprise M365/Exchange users processing high email volumes. Organized by adoption impact.

---

## Cross-Provider Architecture

Ratatoskr supports four providers: **Exchange (Graph)**, **Gmail API**, **JMAP (Stalwart)**, **IMAP**. Most features on this roadmap are natively supported by only one or two providers. The architecture must handle this gracefully.

### The Local Fallback Principle

Every feature gets a **local-only implementation** as the baseline. Provider-native support is an optimization on top. The UI never distinguishes — a category is a category whether it syncs to Exchange or lives only in the local DB.

| Feature | Status | Exchange (Graph) | Gmail API | JMAP | IMAP | Local Fallback |
|---|---|---|---|---|---|---|
| [Categories](categories.md) | ⚠️ Partial | Native (`categories`) | Labels (partial overlap) | `keywords` | IMAP keywords (limited) | Local-only labels+colors |
| [Contacts](contacts.md) | ⚠️ Partial | Native (`/me/contacts`) | People API | Not standardized | Nothing | Local address book |
| Auto-collected contacts | ⚠️ Partial | People API (ranked) | "Other Contacts" | Nothing | Nothing | `seen_addresses` table |
| [@Mentions](mentions.md) | ❌ | Native (`mentions`) | Nothing | Nothing | Nothing | Local-only, no server flag |
| [Reactions](reactions.md) | ❌ | Native (`reactions`) | Nothing | Nothing | Nothing | Local-only |
| [Scheduled send](scheduled-send.md) | ✅ Local | Native (deferred delivery) | Native | `EmailSubmission.sendAt` | Nothing | Local timer + send-on-wake |
| [Roaming signatures](signatures.md) | ⚠️ Partial | Native (roaming settings) | Gmail API settings | Nothing | Nothing | Local-only signatures |
| [Cloud attachments](cloud-attachments.md) | ❌ | OneDrive via Graph | Google Drive API | Nothing | Nothing | Local large-file warning only |
| [Tracking blocking](tracking-blocking.md) | ✅ Images | N/A (client-side) | N/A (client-side) | N/A (client-side) | N/A (client-side) | Fully local |
| [Shared mailboxes](shared-mailboxes.md) | ❌ | Native (delegate access) | Native (delegation) | Shared via ACL | IMAP ACL (RFC 4314) | N/A — requires server support |
| [Public folders](public-folders.md) | ❌ | Native (legacy Exchange) | Nothing | Nothing | Nothing | N/A — Exchange-only concept |
| [BIMI](bimi.md) | ❌ | N/A (DNS + headers) | N/A (DNS + headers) | N/A (DNS + headers) | N/A (DNS + headers) | Fully local |
| [IMAP SPECIAL-USE](imap-special-use.md) | ✅ Done | N/A | N/A | N/A | Native | N/A |

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

- [Categories (Color Flags)](categories.md) — ⚠️ Partial
- [Contacts & Groups](contacts.md) — ⚠️ Partial
- [Tracking Pixel / Read Receipt Blocking](tracking-blocking.md) — ⚠️ Mostly done (MDN detection + policy table added)
- [Cloud Attachment Linking](cloud-attachments.md) — ❌ Not implemented
- [IMAP CONDSTORE/QRESYNC](imap-condstore-qresync.md) — ⚠️ Phase 1 (modseq tracking)
- [Shared / Delegated Mailboxes](shared-mailboxes.md) — ❌ Not implemented
- [Public Folders](public-folders.md) — ❌ Not implemented

## Tier 2 — Keeps users from going back

Features users notice are missing after a week of daily use.

- [@Mentions](mentions.md) — ❌ Not implemented
- [Roaming Signatures](signatures.md) — ⚠️ Partial
- [Scheduled Send](scheduled-send.md) — ✅ Done (local)
- [Reactions](reactions.md) — ❌ Not implemented

## Tier 3 — Differentiators and polish

Features that go beyond Outlook parity into "this client is actually better."

- [BIMI](bimi.md) — ❌ Not implemented
- [IMAP SPECIAL-USE](imap-special-use.md) — ✅ Done

---

## Implementation notes

- **No on-disk migration needed**: All of the above syncs from the server. When cutting over to the iced frontend, start fresh and re-sync.
- **Contacts are the critical dependency**: @mentions, compose autocomplete, and group resolution all depend on having contacts synced locally first.
- **Graph API carries the load**: Most Tier 1/2 features are Exchange/Graph-native. The Graph provider will carry the heaviest implementation burden.
- **JMAP is naturally well-aligned**: JMAP's `threadId`, `keywords`, and `EmailSubmission` cover categories, threading, and scheduled send natively for Stalwart users.
- **Local DB is the unifier**: Every feature uses the local DB as the canonical client-side store. Some rows sync to a server, some are local-only. The UI layer queries the local DB and never cares about provenance. The sync layer handles bidirectional updates per-provider.
- **The `ProviderOps` trait grows**: Many of these features imply new methods on `ProviderOps` (or a companion trait). Each method has a default implementation that does nothing (local-only fallback), and providers override where they have native support.
