# Chats: Spec vs. Code Discrepancies

Audit date: 2026-03-30

Phase 1 (backend) is fully implemented. Phase 2 (UI) is partially implemented. Phases 3-4 not started.

---

## High

1. **No sidebar CHATS section.** Central to the feature. Core API `get_chat_contacts()` exists but is never called from the app. No sidebar rendering, no designation UI, no entry point for users to see or access chats.

2. **Chat timeline does not render message bodies.** `ChatMessage` carries only metadata (subject/date/sender). UI uses subject as placeholder bubble text with a TODO. The spec calls for stripped bodies, quote collapsing, and "show full message" - none implemented.

3. **Entering a chat does not mark it read.** `enter_chat_view()` loads the timeline only. `ChatReadMarked` handler is inert (`Task::none()`). No local or remote mark-read helpers.

4. **Inline chat compose missing.** Spec describes reply/send from the chat view. No compose widget or send flow exists in chat_timeline.rs or chat handler.

## Medium

5. **No auto-scroll-to-bottom.** Handler has a TODO instead of snapping to newest message on chat entry.

6. **No "view as email" toggle.** Spec describes switching a conversation back to normal thread view. No toggle or route-level affordance.

7. **ChatMessage lacks body/attachment fields.** Core type has no body text or attachment state, blocking richer chat rendering.

## Implemented

- DB schema: `thread_participants`, `chat_contacts`, `threads.is_chat_thread` (migration 78)
- Provider sync populates participation and designation state (all 4 providers)
- Core chat APIs: designate/undesignate, get contacts, get timeline, pagination
- Dedicated chat route + timeline view with bubble layout, date separators, sent/received alignment
- Chat-thread exclusion from normal mail views via `is_chat_thread = 0` filters
- Generation counter (`Chat` brand) wired correctly
