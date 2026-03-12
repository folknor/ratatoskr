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
