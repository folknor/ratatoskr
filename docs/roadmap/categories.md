# Categories (Color Flags)

**Tier**: 1 — Blocks switching from Outlook
**Status**: ⚠️ **Partial** — Gmail `CATEGORY_*` labels are mapped, Graph `categories` field is parsed to `cat:` prefixed labels, and a rule engine + AI pipeline handles inbox triage (Primary/Updates/Promotions/Social/Newsletters). But this is automated inbox categorization, not the user-customizable color-flag categories described below. No master category list sync, no unified color mapping, no user-applied categories on individual messages.

---

- **What**: Per-user string labels with associated colors, applied to messages
- **Scope**: Per-user on personal mailboxes; shared visibility on shared mailboxes and public folders

## Cross-provider behavior

| Provider | Native support | Behavior |
|---|---|---|
| Exchange (Graph) | Full — `categories` on messages, master list via `/me/outlook/masterCategories` | Sync master list + per-message categories bidirectionally |
| Gmail API | Labels function as both folders and categories. Color supported. | Map Gmail labels to categories where label is not a system/folder label. Imperfect — Gmail's model conflates the two concepts. |
| JMAP | `keywords` on emails — arbitrary string keys, boolean values. No color. | Use keywords as category names, store colors locally. |
| IMAP | `FLAGS`/keywords — server support varies wildly, many servers limit to system flags only | Local-only categories with IMAP flag sync as best-effort. |

## Pain points

- Gmail label/category/folder conflation: need heuristics to decide which labels are "categories" vs structural folders. System labels (`INBOX`, `SENT`, `TRASH`) are obvious, but user-created labels are ambiguous.
- IMAP keyword support is unreliable: some servers silently drop custom keywords, others have hard limits on keyword count. Must detect and fall back to local-only.
- Color mapping: Exchange has a fixed set of preset colors. Gmail has its own color palette. JMAP/IMAP have no color concept. Need a unified color model that round-trips cleanly to Exchange and degrades gracefully elsewhere.
- Shared mailbox categories: on Exchange, categories applied to messages in a shared mailbox are visible to all users with access. This is a feature users rely on for team triage ("I marked it Red, that means it's handled"). Must preserve this behavior for Graph accounts.
- Multi-account category conflicts: user has "Urgent" as red on Account A and blue on Account B. The category picker needs to handle this without confusion.

## Work

Sync master category list per account, display on messages, allow apply/remove, persist locally, round-trip to server where supported. Local-only fallback for IMAP.
