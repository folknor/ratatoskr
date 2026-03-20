# Contacts: Problem Statement

## Overview

Ratatoskr needs contact management as a first-class feature. Enterprise users maintain large address books across multiple accounts, rely on organization directories (GAL) to find colleagues, and use contact groups for recurring communication patterns.

The contact system serves three user profiles:

1. **Most users** — never touch contacts directly. Autocomplete just works; synced contacts populate silently from their providers.
2. **Light editors** — occasionally tweak a display name or add a note. They do this inline from the reading pane, never opening a dedicated contacts interface.
3. **Contact/group managers** — actively manage contact groups (distribution lists, project teams) and browse/edit contacts in bulk. They need a full management surface.

The backend infrastructure is complete — provider sync (Graph, Google People API, CardDAV), auto-collected seen addresses, contact groups with nested expansion, photo caching, and FTS5 search are all implemented. This document specifies the UI layer.

There are no address books. Contacts exist in a single flat pool. Account/source is tracked internally for sync purposes, but the user sees one unified contact list. Groups handle organization.

## Autocomplete

Autocomplete appears in two places: the compose recipient field (To/Cc/Bcc) and the calendar event attendee field. The behavior is identical in both.

### Data Sources

Autocomplete searches across three pools, all stored locally:

1. **Synced contacts** — contacts pulled from provider APIs (Exchange, Google, CardDAV). These have display names, emails, and sometimes photos and organization info.
2. **Seen addresses** — auto-collected from message headers during sync. Scored by direction (sent-to weighted higher than received-from) and recency.
3. **Organization directory (GAL)** — cached locally at startup and refreshed by polling. Never fetched mid-keystroke.

All three pools are blended into a single search. The user does not see or care which source a suggestion came from.

### GAL Caching

The organization directory is pre-fetched and stored locally so that autocomplete is always a local operation — no network requests during typing. The cache is:

- **Populated at startup** for each connected account that has a directory API (Graph `/users`, Google Directory API)
- **Refreshed by polling** — frequency should be as aggressive as the provider allows without causing rate limiting issues. Hourly as a baseline; shorter intervals if the API permits.

This means newly added employees or org changes take up to one poll interval to appear in autocomplete. This is an acceptable tradeoff for guaranteeing zero-latency autocomplete.

### Ranking

Results are ranked primarily by **recency** — how recently the user interacted with this address. Secondary signals (frequency of interaction, source priority, match quality) may refine the ordering, but recency dominates.

The exact ranking formula is not specified here. It should be tuned empirically once the UI is functional.

### Input Behavior

The user types in a text field. After a short debounce (immediate or near-immediate for local search), a dropdown appears below the field showing matching results.

Each suggestion row shows:

```
Alice Smith  alice.smith@corp.com
Bob Jones    bob@example.com
```

**Name** on the left, **email** on the right. Ranking order communicates relevance — no other visual indicators needed.

### Selection

- **Click** or **Enter** on a suggestion adds it as a token in the field (rectangular with slightly rounded corners)
- **Arrow keys** navigate the suggestion list
- **Tab** accepts the top suggestion
- **Space, comma, semicolon, Tab, or Enter** on raw text tokenizes it directly (basic email validation only)
- **Paste** tokenizes immediately — no autocomplete dropdown. Handles bare email addresses and RFC 5322 name+address formats: `Name <email>`, `"Name" <email>`, `email <email>`. The display name is extracted and shown on the token.
- **Right-click** a token opens a context menu: Cut, Copy, Paste, Delete. For group tokens, an additional "Expand group" option replaces the group token with individual tokens for each member. In compose (not calendar), the menu also includes "Move to To/Cc/Bcc" (showing only the fields the token is not already in).
- **Drag and drop** tokens between fields (To, Cc, Bcc) to move them
- Multiple tokens can be added — the field grows vertically as needed

**Dropdown lifecycle:** The autocomplete dropdown is only visible while there are matching results. If the user is typing an address that matches nothing (e.g., "alice@" with no known alice@), the dropdown disappears and stays gone. At that point, Tab/Enter/Space/comma/semicolon act purely as tokenizers — they validate that the text looks like a plausible email address, convert it to a token, and move the cursor to accept more input. The dropdown only reappears when the user starts a new token that produces matches.

### Contact Group Tokens

Contact groups appear in the suggestion list alongside individual contacts. They are visually distinct (e.g., a group icon or label indicating member count). Groups match primarily on the **group name** — a group should rank high when the user types its name (e.g., "engineering"), but rank very low when the query matches a member inside the group. If the user types "alice", they want Alice the person, not every group that contains an Alice.

When selected, a contact group is added as a **single token** representing the group. It is not expanded into individual addresses in the compose/attendee field. The email is sent to all members of the group (expanded at send time).

The user cannot remove individual members from a group token in the compose field. To modify group membership, they must use the contact management interface. This keeps the compose field simple — a group is an atomic unit.

**Bcc suggestion (compose only):** When a contact group is added to the To or Cc field, a banner appears suggesting the user move it to Bcc (to avoid exposing all member addresses to each other). The banner is dismissible and not blocking — it's a nudge, not a gate.

**Group creation suggestion (compose only):** When a paste tokenizes into 10+ addresses, a dismissible banner suggests saving them as a contact group. Accepting opens the group creation flow pre-populated with those addresses.

## Inline Contact Editing

In the reading pane, sender and recipient pills (From, To, Cc) are not the same as compose tokens. Each pill has a small inline edit button. Clicking it opens a popover for quick contact editing — this is how profile (2) users interact with contacts without ever opening a management interface.

### Popover Contents

- **Display name** — editable text field. For synced contacts, this is a local override only (the `display_name_overridden` flag prevents future syncs from reverting the edit). The provider's display name is not changed.
- **Email** — primary email, editable
- **Email 2** — secondary email, optional
- **Phone** — optional
- **Groups** — add/remove group memberships. This field works identically to the compose autocomplete fields (type to search, token-based selection) but only matches contact groups. **Hidden entirely if no groups exist** — no empty field, no "create group" affordance here. Group creation happens in the contact management interface.
- **Notes** — small free-text field

Every field saves immediately on edit — no Save/Cancel buttons. Changes sync back to the provider where supported.

### Matching

The search matches against both display name and email address. Matching is prefix-based and accent-insensitive (leveraging the existing FTS5 index with `unicode61` tokenizer).

Partial matches work naturally: typing "ali" matches "Alice Smith" (name prefix) and "alice@..." (email prefix). Typing "corp" matches "alice@corp.com" (email domain).

### Deduplication

The same email address may exist across multiple sources (synced from Exchange, synced from Google, seen in message headers). Autocomplete deduplicates by email address — the user sees one suggestion per unique email, not one per source. When duplicates exist, the display name from the highest-priority source is used (synced contact > seen address > GAL).

## Contact Management

Contact management lives in Settings. It is not a top-level mode — most users never need it. Profile (3) users (contact/group managers) navigate to Settings to browse, edit, and organize contacts and groups. A "Manage Contacts" command is registered in the command palette for quick access.

### Layout

Two stacked lists, each compact — roughly 5 entries visible at a glance without scrolling. Contacts on top, groups below. Both sorted by recency. Each list has its own filter input at the top for narrowing results. Only synced and local contacts are shown — seen addresses and GAL entries do not appear in the management interface (they are autocomplete-only data sources).

```
┌─ Contacts ───────────────────────────────────────────────┐
│ [Filter contacts...                                    ] │
│                                                          │
│ ┌──────────────────────────────────────────────────────┐ │
│ │ Ralph Wiggum                    ralph@corp.com       │ │
│ │ Phone: +47 123 456              ralph2@other.com     │ │
│ │ Company: Springfield Inc                             │ │
│ │ Notes: Prefers email over calls                      │ │
│ │ Groups: [Engineering] [Project X]                    │ │
│ │ [🔵 Work Account] [🟢 Gmail]                        │ │
│ └──────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────┐ │
│ │ Lenny Leonard                   lenny@corp.com       │ │
│ │ Company: Springfield Inc                             │ │
│ │ Groups: [Engineering]                                │ │
│ │ [🔵 Work Account]                                   │ │
│ └──────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────┐ │
│ │ ...                                                  │ │
│ └──────────────────────────────────────────────────────┘ │
│                                                          │
│─ Groups ─────────────────────────────────────────────────│
│ [Filter groups...                                      ] │
│                                                          │
│ ┌──────────────────────────────────────────────────────┐ │
│ │ Engineering                          12 members      │ │
│ │ Created: 2026-01-15        Last updated: 2026-03-18  │ │
│ └──────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────┐ │
│ │ Project X                             5 members      │ │
│ │ Created: 2026-02-01        Last updated: 2026-03-10  │ │
│ └──────────────────────────────────────────────────────┘ │
└──────────────────────────────────────────────────────────┘
```

### Contact Cards

Each contact card shows all available information at a glance. Empty fields are hidden — a minimal contact (just name and email) is a compact two-line card.

- **Display name** — left-aligned, prominent
- **Email** — right-aligned on the same line as the name
- **Email 2** — right-aligned below the first email (if present)
- **Phone** — below the name (if present)
- **Company** — below the phone (if present)
- **Notes** — below the company (if present)
- **Groups** — colored pills showing group memberships (if any)
- **Account pills** — colored pills at the bottom showing which account(s) the contact was synced from

### Group Cards

Each group card shows:

- **Group name** — left-aligned, prominent
- **Member count** — right-aligned on the same line
- **Created date** — left-aligned below
- **Last updated date** — right-aligned below

### Editing Contacts

Clicking a contact card slides in an editor that covers the entire settings UI (same slide-in pattern used elsewhere in settings). The editor contains the same fields as the inline contact editing popover (display name, email, email 2, phone, groups, notes) plus any additional fields (company). All fields save immediately on edit.

For synced contacts, edits (except display name) are pushed back to the provider via its API. Display name is always a local-only override. If the provider rejects a write (e.g., read-only GAL/directory entries), the error is shown to the user.

A back button (← Back to Contacts) at the top returns to the contact list.

### Editing Groups

Clicking a group card slides in a group editor that covers the entire settings UI. The group editor has two sections stacked vertically:

**Top section — Add Members:** An identical contact list (with filter input) showing only contacts **not** already in the group. Clicking a contact card adds them to the group. Drag and drop from this list to the member grid below also works. A hint below the filter input reads something like "You can paste a large list of email addresses here" — always visible, guiding users who manage groups via Word documents.

**Bottom section — Group Details + Members:** The group name (editable), followed by a grid of current members. Each member is a square tile in a wrapping grid layout (like a file manager). Tiles show only the email address, which auto-breaks to fit the square. The grid fills horizontally, wrapping to new rows as needed.

```
┌─ ← Back to Groups ──────────────────────────────────────┐
│                                                          │
│ Group Name: [Engineering                               ] │
│                                                          │
│─ Add Members ────────────────────────────────────────────│
│ [Filter contacts...                                    ] │
│                                                          │
│ ┌──────────────────────────────────────────────────────┐ │
│ │ Carl Carlson                     carl@corp.com       │ │
│ │ [🔵 Work Account]                                   │ │
│ └──────────────────────────────────────────────────────┘ │
│ ┌──────────────────────────────────────────────────────┐ │
│ │ Moe Szyslak                      moe@corp.com       │ │
│ │ [🔵 Work Account]                                   │ │
│ └──────────────────────────────────────────────────────┘ │
│                                                          │
│─ Members (12) ───────────────────────────────────────────│
│                                                          │
│ ┌────────────┐ ┌────────────┐ ┌────────────┐            │
│ │  ralph@    │ │  lenny@    │ │  homer@    │            │
│ │  corp.com  │ │  corp.com  │ │  corp.com  │            │
│ └────────────┘ └────────────┘ └────────────┘            │
│ ┌────────────┐ ┌────────────┐ ┌────────────┐            │
│ │  barney@   │ │  frank@    │ │  alice@    │            │
│ │  corp.com  │ │  corp.com  │ │  corp.com  │            │
│ └────────────┘ └────────────┘ └────────────┘            │
│ ┌────────────┐ ┌────────────┐ ┌────────────┐            │
│ │  bob@      │ │  charlie@  │ │  diana@    │            │
│ │  other.com │ │  corp.com  │ │  corp.com  │            │
│ └────────────┘ └────────────┘ └────────────┘            │
└──────────────────────────────────────────────────────────┘
```

Clicking a member tile removes them from the group. All changes (adding, removing, renaming) save immediately.

### Creating Contacts and Groups

Each list has a "New Contact" / "New Group" button attached to the bottom of the list. Creating opens the same slide-in editor with empty fields.

The first field in the contact creation editor is an **account selector** — a dropdown listing all connected accounts that have a writable contacts backend (Exchange, Google, CardDAV-capable JMAP/IMAP), plus a **"Local"** option for contacts that don't sync to any provider. The selected account determines where the contact is synced to. Local contacts are stored only in the local database.

### Deleting Contacts and Groups

Delete is available in the slide-in editor for both contacts and groups. Deleting a group does not delete its members — it only removes the grouping. Deletion prompts for confirmation. For synced contacts and groups, the delete is pushed to the provider. If the provider rejects it, the error is shown.

### Importing Contacts

An import button at the top of the contact management UI opens the contact import wizard. Supports CSV, XLSX, and vCard files with automatic encoding/delimiter/column detection and user-correctable column mapping. See `docs/contacts/import-spec.md` for the full specification.
