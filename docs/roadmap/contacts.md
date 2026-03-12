# Contacts & Groups

**Tier**: 1 — Blocks switching from Outlook
**Status**: ⚠️ **Partial** — Local contact DB exists (`contacts` table with frequency ranking, avatars, notes). Contacts auto-collected on send. Compose autocomplete works against local contacts. Gravatar integration, contact sidebar with stats/colleagues/shared files. **Missing**: server-side sync (Exchange `/me/contacts`, Google People API), `seen_addresses` from received mail headers, distribution list/group resolution, contact photos from server.

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

## Work

Sync contacts to local DB per-account, unified autocomplete across all accounts, local address book for accounts without server-side contacts, group resolution for compose.
