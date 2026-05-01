# Chats: Spec vs. Code Discrepancies

Audit date: 2026-05-01

Phase 1 backend, Phase 2 timeline rendering, and Phase 3 sidebar integration
are largely in place. Designation, mark-read, signature handling, compose,
and the polish phase are still missing or partial.

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

2. **Entering a chat does not mark it read.** `enter_chat_view`
   (`handlers/chat.rs`) loads the timeline and refreshes the contacts list
   only. `Message::ChatReadMarked` is dispatched in `update.rs` but resolves
   to `Task::none()`. The spec's `mark_chat_read_local` /
   `mark_chat_read_remote` helpers do not exist anywhere. Unread counts on
   the sidebar stay stuck after opening a chat. (Phase 2 gap.)

3. **No signature stripping or quote collapsing.** `phase-2-plan.md`
   specifies a reusable `crates/common/src/signature_strip.rs` module with
   `strip_signature` (layers 1-3) and `collapse_quotes`. Neither function
   nor module exists. `chat_bubble` renders the raw `body_text` from the
   body store verbatim, so signatures and quoted reply chains both leak
   into the bubble UI. (Phase 2 gap; Phase 5 not started.)

4. **"Show full message" affordance is dead code.** `ChatTimeline` owns an
   `expanded: HashSet<String>` and a `ChatTimelineMessage::ToggleExpand`
   variant; `update()` toggles the set; `chat_bubble` receives the
   per-message expanded flag as `_expanded` (unused). No element in the
   bubble emits `ToggleExpand`, so the toggle path is unreachable from the
   UI. Once stripping lands this needs to render a "show full message"
   affordance and switch the body source on expansion. (Phase 2 gap.)

5. **Inline chat compose missing.** No inline composer at the bottom of
   the timeline, no Enter-to-send, no per-thread reply targeting, no emoji
   shortcode translation, no signature-hide-in-own-view treatment. (Phase 4
   not started.)

## Medium

6. **`ChatTimeline.contact_email` is the local-DB lowercased form.** Both
   `enter_chat_view` and the contact list pass through `email.to_lowercase()`
   inside the core layer, but the App-side `active_chat` and the Sidebar
   `active_chat` mirror are populated from the *user-facing* string. If
   chat contacts ever display mixed-case email (e.g. from contacts table
   with original casing), the active-row highlight in the sidebar will
   compare-fail. Today this is benign because dev-seed lowercases.

7. **`active_chat` deviates from the Phase 2 plan.** `phase-2-plan.md`
   §"Layout Architecture" calls for `NavigationTarget::Chat` to be the
   single source of truth - "no `active_chat: Option<String>` field". The
   shipped App stores `active_chat` on both `App` and `Sidebar`, and the
   view layer / command-dispatch / update path branches on
   `active_chat.is_some()`. There is no `navigation_target` field; the
   `NavigationTarget::Chat` variant is only ever used as a transient
   dispatch argument. Functionally equivalent, structurally diverged.

8. **No "view as email" toggle.** Spec calls for a per-conversation
   affordance to switch back to the threaded reading-pane view without
   undesignating the contact. No route, no menu, no command. (Phase 6
   polish.)

9. **No production backfill for `thread_participants`.** Implementation
   phases plan a "post-migration fixup" that parses every existing
   `messages` row's address fields. The dev workflow doesn't need it
   (dev-seed wipes and reseeds), but the production-launch checklist
   relies on it. Not yet written. (Phase 1 deferred.)

10. **`get_chat_timeline` does not deduplicate cross-account messages.**
    The cursor is `(date, message_id)` per spec, but the timeline never
    deduplicates messages that arrive via cross-account aggregation
    (e.g. the same Message-ID seen on both Gmail and a forwarding alias).
    Phase 6 explicitly calls this out, but it's worth flagging now
    because dev-seed does not exercise multi-account duplicates.

## Implemented (Phase 1)

- Schema: `thread_participants`, `chat_contacts`, `threads.is_chat_thread`,
  partial index `idx_threads_chat`, partial index
  `idx_thread_participants_email`. All in single v100 migration.
- Gmail / JMAP / Graph sync paths call `upsert_thread_participants` and
  `maybe_update_chat_state` per message during inline persistence. IMAP
  takes a different route - each message is inserted with a placeholder
  thread, JWZ threading runs over the batch, then `pipeline::store_threads`
  → `reassign_messages_and_repair_threads` → `rebuild_thread_participants`
  + `maybe_update_chat_state` reconciles the participants set and the
  `is_chat_thread` flag for every threaded group.
- `designate_chat_contact_sync` / `undesignate_chat_contact_sync`:
  transactional, recompute `is_chat_thread` flags, refresh summary row.
- `get_chat_contacts_sync` returns sidebar summary (display name, avatar
  via `contact_photo_cache`, latest preview, latest timestamp, unread,
  sort order).
- `get_chat_timeline_sync`: tuple cursor (`date`, `id`), descending
  ordered, paginated. `core/src/chat.rs` reverses to chronological after
  load and joins inline images.
- `is_chat_thread = 0` filter applied across `scoped_queries.rs`,
  `queries.rs`, and `navigation.rs` (15 sites) - chat threads excluded
  from inbox / folder / smart-folder / unread-count queries.
- `Chat` and `ChatList` generation brands in `core/src/generation.rs`.

## Implemented (Phase 2)

- `NavigationTarget::Chat { email }` variant + dispatch through
  `handle_navigate_to` → `enter_chat_view`.
- `ChatTimeline` component: state, messages, `Component` trait, refreshable
  `image_handles` cache (stable `image::Handle` IDs to keep iced's GPU
  cache warm).
- Bubble layout: sent/received alignment, `ChatBubbleSent` /
  `ChatBubbleReceived` container styles, `CHAT_BUBBLE_*` layout constants
  (`MAX_WIDTH`, `RADIUS`, `PAD`, `SPACING`, `GROUP_SPACING`,
  `DATE_SEPARATOR_SPACING`).
- Date separators ("Today" / "Yesterday" / "Month dd"), subject-change
  indicators (with Re:/Fwd: prefix normalisation), inline-image bubbles
  with stable image handles.
- Body text rendered from `BodyStoreState::get_batch` (plain text only -
  no HTML rendering yet, no stripping).
- "Load older" button at top → cursor-based paginated load.
- Snap-to-end scroll on initial load via
  `iced::widget::operation::snap_to_end`.
- Generation counter (`Chat`) discards stale timeline loads.
- Layout view branches on `active_chat.is_some()` to render
  sidebar + chat-timeline + status bar (no thread list / reading pane).

## Implemented (Phase 3)

- `chats_section` rendered between pinned searches and universal folders
  in `sidebar.rs`. Hidden when `chat_contacts.is_empty()`. Collapsible via
  `ToggleChatsSection`. Scope-independent.
- Per-contact entry: avatar (initials via `avatar_circle`), display name
  (truncated, bold when unread), short relative time ("now"/"5m"/"2h"/"3d"),
  preview text (truncated), active-highlight via `PinnedSearch { active }`
  button style. Click → `SidebarEvent::ChatSelected(email)` → handler
  enters chat view.
- `fire_chat_contacts_load` runs on boot via `load_navigation_and_threads`,
  on chat entry, and on sync completion. `ChatList` generation brand
  prevents stale overwrites.

## Not Started

- **Phase 4 (compose):** no chat composer, no per-thread reply targeting,
  no Enter-to-send / Shift+Enter inversion, no emoji shortcodes, no
  signature suppression for own-view, no attachment drop.
- **Phase 5 (signature refinement):** no per-sender learned-suffix
  pipeline, no `chat_learned_signatures` table, no confidence scoring.
- **Phase 6 (polish):** no virtual scrolling, no drag-and-drop reorder,
  no chat context menu, no cross-account dedup in timeline, no thread-level
  view-as-email toggle, no emoji picker, no undesignation confirmation.
