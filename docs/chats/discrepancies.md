# Chats: Spec vs. Code Discrepancies

Audit date: 2026-05-01

Phase 1 backend, Phase 2 timeline rendering, and Phase 3 sidebar integration
are largely in place. Designation, compose, and the polish phase are still
missing.

Phase legend (rough): `1` data model + queries, `2` timeline view,
`3` sidebar, `4` compose, `5` signature-stripping refinement, `6` polish.

---

## High

1. **No designation UI.** `rtsk::chat::designate_chat_contact` /
   `undesignate_chat_contact` are wired through `db::queries_extra::chat`
   and exercised by `dev-seed`, but no path in the app calls them. There is
   no contact-editor toggle, no command-palette command, no context menu.
   Users cannot create or remove chat contacts at runtime - the sidebar
   CHATS section only ever shows what dev-seed wrote. (Phase 1 / Phase 3
   gap.)

5. **Inline chat compose missing.** No inline composer at the bottom of
   the timeline, no Enter-to-send, no per-thread reply targeting, no emoji
   shortcode translation, no signature-hide-in-own-view treatment. (Phase 4
   not started.)

## Not Started

- **Phase 4 (compose):** no chat composer, no per-thread reply targeting,
  no Enter-to-send / Shift+Enter inversion, no emoji shortcodes, no
  signature suppression for own-view, no attachment drop.
- **Phase 5 (signature refinement):** no per-sender learned-suffix
  pipeline, no `chat_learned_signatures` table, no confidence scoring.
- **Phase 6 (polish):** no virtual scrolling, no drag-and-drop reorder,
  no chat context menu, no cross-account dedup in timeline, no thread-level
  view-as-email toggle, no emoji picker, no undesignation confirmation.
