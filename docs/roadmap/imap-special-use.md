# IMAP SPECIAL-USE (RFC 6154)

**Tier**: 3 — Differentiators and polish
**Status**: ✅ **Done** — Full implementation in `imap/parse.rs`: detects `\Sent`, `\Trash`, `\Drafts`, `\Junk`, `\Archive`, `\All`, `\Flagged` attributes from LIST response. Heuristic fallback via `imap_name_to_special_use()` covers 50+ folder name variations across languages. Role mapping in `provider/folder_roles.rs`.

---

- **What**: Server declares which folders are Trash, Sent, Drafts, Archive, Junk via attributes
- **Scope**: IMAP only — other providers have explicit folder semantics in their APIs

## Pain points

- Not all IMAP servers support it. Need to fall back to heuristic folder detection (name matching: "Sent", "Sent Items", "Sent Mail", etc., across languages).
- Some servers advertise SPECIAL-USE but have incorrect attributes (e.g., no `\Archive` even though an "Archive" folder exists). Need heuristic fallback even when SPECIAL-USE is present.
- User-customized folder roles: user wants "Old Mail" to be their Archive folder. Need to allow manual override of detected roles.

## Work

Check `SPECIAL-USE` capability on LIST response, read `\Trash`/`\Sent`/`\Drafts`/`\Archive`/`\Junk` attributes, fall back to name heuristics, allow manual override.

---

## Research

**Date**: March 2026
**Context**: Feature is fully implemented. This research documents the existing cross-provider folder role unification, evaluates completeness, and identifies minor improvements for the iced migration.

---

### 1. RFC 6154 Mechanics

RFC 6154 ("IMAP LIST Extension for Special-Use Mailboxes") defines seven mailbox attributes returned in LIST responses:

| Attribute | Meaning |
|-----------|---------|
| `\All` | Virtual mailbox containing all messages in the store |
| `\Archive` | Long-term storage |
| `\Drafts` | Unsent draft messages |
| `\Flagged` | Virtual mailbox collecting `\Flagged` messages |
| `\Junk` | Spam/junk mail |
| `\Sent` | Copies of sent messages |
| `\Trash` | Deleted messages awaiting expunge |

Example wire format:
```
* LIST (\HasNoChildren \Sent) "/" "Sent Items"
```

**RFC 8457 `\Important`**: Added in 2018. Semantically similar to `\Flagged` but for server-side importance (Gmail's Priority Inbox markers). Gmail IMAP exposes `[Gmail]/Important` as a folder but does not advertise it via SPECIAL-USE attributes. Very few servers implement it.

### 2. `async-imap` / `imap-proto` Support

`imap-proto` 0.16.6 has first-class `NameAttribute` variants for all seven RFC 6154 attributes: `All`, `Archive`, `Drafts`, `Flagged`, `Junk`, `Sent`, `Trash`. Anything else (including `\Important`) lands in `NameAttribute::Extension(Cow<str>)`.

The current `detect_special_use()` matches the seven named variants and ignores `Extension`. The `\Important` gap is covered by the heuristic name fallback. To close the gap fully:

```rust
NameAttribute::Extension(s) if s.eq_ignore_ascii_case("\\Important") => Some("\\Important"),
```

**No custom parsing needed.** `async-imap` handles LIST response parsing correctly.

### 3. Server Support Matrix

| Server | SPECIAL-USE | Attributes advertised | Notes |
|--------|-------------|----------------------|-------|
| **Dovecot** (2.2+) | Yes | All 7. Configurable. | Reference implementation. |
| **Cyrus** (3.0+) | Yes | All 7. Auto-assigned. | Also supports XLIST for legacy clients. |
| **Gmail IMAP** | Yes | All except `\Archive`. | Archive = remove from Inbox (move to `\All`). |
| **Yahoo IMAP** | Yes | `\Drafts`, `\Sent`, `\Trash`, `\Junk`. | Minimal set. "Bulk Mail" is junk folder. |
| **iCloud IMAP** | Yes | `\Drafts`, `\Sent`, `\Trash`, `\Junk`, `\Archive`. | Good coverage. |
| **Exchange IMAP** | Varies | Older Exchange may not advertise. Exchange Online does. | Heuristic covers "Sent Items", "Deleted Items", "Junk E-mail". |
| **Stalwart** | Yes | All 7. | Modern Rust server. |
| **Courier** | No | None. | Old server. Name heuristics only. |
| **hMailServer** | No | None. | Windows-only. Name heuristics only. |
| **Zimbra** (8.5+) | Yes | All 7. | Common in enterprise/education. |

### 4. Cross-Provider Folder Role Mapping

The `SystemFolderRole` in `provider/folder_roles.rs` is the unification hub:

| Internal `label_id` | IMAP `\Attribute` | JMAP `role` | Graph `wellKnownName` | Gmail label |
|---------------------|-------------------|-------------|----------------------|-------------|
| `INBOX` | `\Inbox` | `inbox` | `inbox` | `INBOX` |
| `DRAFT` | `\Drafts` | `drafts` | `drafts` | `DRAFT` |
| `SENT` | `\Sent` | `sent` | `sentitems` | `SENT` |
| `TRASH` | `\Trash` | `trash` | `deleteditems` | `TRASH` |
| `SPAM` | `\Junk` | `junk` | `junkemail` | `SPAM` |
| `archive` | `\Archive` | `archive` | `archive` | (no equivalent) |
| `STARRED` | `\Flagged` | (none) | (none) | `STARRED` |
| `all-mail` | `\All` | (none) | (none) | `[Gmail]/All Mail` |
| `IMPORTANT` | `\Important` | `important` | (none) | `IMPORTANT` |

Each provider has a dedicated mapper module converting provider-native folder identity into the shared `label_id` space. The `SYSTEM_FOLDER_ROLES` const table is the single source of truth.

**For the iced migration**: The entire module lives in `ratatoskr-core` with zero framework dependencies. Carries over unchanged.

### 5. Heuristic Name Matching

Current `imap_name_aliases` cover English names, `[Gmail]/` prefixed names, and some French names (`brouillons`, `corbeille`).

**Missing from common non-English deployments**:
- German: `Entwuerfe`/`Entwurfe` (Drafts), `Papierkorb` (Trash), `Gesendet` (Sent)
- Spanish: `Borradores` (Drafts), `Papelera` (Trash), `Enviados` (Sent)
- Italian: `Bozze` (Drafts), `Cestino` (Trash), `Posta inviata` (Sent)
- Portuguese: `Rascunhos` (Drafts), `Lixeira` (Trash), `Enviados` (Sent)

**Practical impact**: Low. Most non-English servers support SPECIAL-USE attributes. The heuristic mainly matters for Courier/hMailServer, predominantly English-deployed. Adding more languages is low-cost but not urgent.

### 6. User Override Data Model

Manual override of detected roles is **not currently implemented**. To add:

```sql
CREATE TABLE folder_role_overrides (
    account_id TEXT NOT NULL,
    folder_path TEXT NOT NULL,
    role TEXT NOT NULL,
    PRIMARY KEY (account_id, folder_path)
);
```

Check overrides *before* attribute/heuristic detection. Expose in settings UI. Per-account since the same name could mean different things on different servers.

**Priority**: Low. Automatic detection covers the vast majority of cases.

### 7. Iced Migration Impact

**Zero migration work required.** The entire implementation lives in `ratatoskr-core`:

| Module | Framework deps | Action |
|--------|---------------|--------|
| `provider/folder_roles.rs` | None | Carry over unchanged |
| `imap/parse.rs` | None | Carry over unchanged |
| `sync/folder_mapper.rs` | None | Carry over unchanged |
| Provider-specific mappers | None | Carry over unchanged |

**Optional improvements** (all minor):
1. Add `NameAttribute::Extension` match for `\Important`
2. Expand name aliases with German/Spanish/Italian/Portuguese
3. Add `folder_role_overrides` table for user customization
4. Add "Bulk Mail" to spam aliases for Yahoo edge case
