# Public Folders

**Tier**: 1 — Blocks switching from Outlook
**Status**: ❌ **Not implemented**

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
