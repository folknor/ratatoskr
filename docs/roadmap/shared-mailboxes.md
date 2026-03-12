# Shared / Delegated Mailboxes

**Tier**: 1 — Blocks switching from Outlook
**Status**: ❌ **Not implemented** — Gmail Send-As aliases are functional (fetch, store, FromSelector in compose, smart alias selection for replies), but this is outbound identity only, not shared mailbox access. No `*.Shared` OAuth scopes requested for Graph. No delegation discovery, no shared mailbox reading, no IMAP ACL, no JMAP Sharing.

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

## Work

Delegated mailbox discovery (manual-add baseline, EWS auto-mapping as enhancement for Exchange), uniform presentation regardless of mailbox type, sync with configurable depth, auto-set From address with Send As / Send on Behalf distinction, respect per-permission capabilities in UI, shared state delta sync, Sent Items routing, request `*.Shared` OAuth scopes.
