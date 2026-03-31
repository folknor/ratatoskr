# Implementation Plan

Prioritized implementation plan for Ratatoskr features.

## Implementation Status

### Tier 1 — Shell / Unblockers ✅ COMPLETE

| Task | Commits | Review Status |
|------|---------|---------------|
| Command Palette 6a (keyboard dispatch) | `133ee45`, fix `0b751df` | Reviewed: availability check added, failed second chord re-processed |
| Command Palette 6b (palette UI stage 1) | `81d3e08`, fix `169e952` | Reviewed: settings block, max-height enforced |
| Command Palette 6c (parameterized / stage 2) | `3868511`, fix `e761e34` | Reviewed: unified-view account fallback, label/folder semantics |
| Command Palette 6d (command-backed surfaces) | `4e27423` | Pending review |
| Sidebar Phase 1A (live data wiring) | `a8b5cd4`, fix `d609045` | Reviewed: All Accounts reload, 1000-thread limit restored |
| Sidebar Phase 1B (smart folder scoping) | `938827d` | Reviewed: clean |
| Sidebar Phase 1C (unread counts) | `efb10ed` | Reviewed: clean (silent error-to-zero noted as observability gap) |
| Sidebar Phase 1D (hierarchy) | `d573585`, fix `9f24e2b` | Reviewed: system-folder children fixed |
| Accounts Phase 0 (data model) | `938827d`, fixes `0b751df` `f332842` | Reviewed: sort order read path, provider inserts, Graph finalization |
| Accounts Phases 1-7 (UI) | `5803271` | Pending review |
| Status Bar | `d4e6f02`, fix `81a2ef9` | Reviewed: settings visibility, idle collapse, BTreeMap ordering |

**Tier 1 delivers:** Command palette with keyboard dispatch + stage 2 parameterized commands, command-backed toolbar buttons, live sidebar with folder hierarchy and real unread counts, first-launch onboarding wizard with color picker, account management in settings, status bar with priority-based content.

**Remaining Tier 1 work (lower priority, not blocking Tier 2):**
- Command Palette 6e (override persistence) — save/load user keybinding overrides
- Command Palette 6f (keybinding management UI) — settings panel for rebinding
- Sidebar Phase 1E (pinned searches section) — blocked on search app integration
- Sidebar Phase 2 (strip actions) — blocked on command palette being mature enough

### Rich Text Editor ✅ COMPLETE

| Task | Commits | Review Status |
|------|---------|---------------|
| Phase 1: Document model + plain text editing | `e07ab49` | Reviewed: 4 findings (pending style, undo styling, hit testing, PosMap), all fixed in `3db1c8b` |
| Phase 2: Inline formatting | (included in `e07ab49`) | Reviewed with Phase 1 |
| Phase 3: Block types + HTML round-trip | (included in `e07ab49`) | Reviewed with Phase 1 |
| Widget polish (cursor, selection, vertical movement) | `6f7b842` | Reviewed: selection last-line fix `9cd1269`, link-at-end-boundary fix `9cd1269` |
| Phase 4: Structured clipboard | `7581e69`, fixes `7f75e07` `8b92aea` `65091eb` | Reviewed: paste link preservation, multi-block list items, stale cache, redo links |
| Scrolling | `edaacd3`, fix `de1ae5a` | Reviewed: cursor visibility fix, auto-scroll per-line precision |
| Phase 5: Compose assembly + signatures + reply quoting | `6a8e0bc`, fix `8da7278` | Reviewed: blank signature handling, index clamping |
| Phase 5: Block::Image | `994c57e`, fixes `08baaf8` `2f10831` | Reviewed: image paste, img-in-heading, nested inline wrapper parsing |
| List flattening (Block::ListItem) + auto-exit rule | `651fc4e`, fix `55258cd` | Reviewed: indent_level in layout/draw/hit-testing |
| Drag auto-scroll | (included in `651fc4e`) | Reviewed with list flattening |

**Crate:** `crates/rte/` — 14,300+ lines, 652 tests, zero clippy warnings.

**What it delivers:** From-scratch WYSIWYG rich text editor for iced. Document model with 6 block types (Paragraph, Heading, ListItem, BlockQuote, HorizontalRule, Image), Arc structural sharing, 8 invertible edit operations with position mapping, Slate-inspired normalization, fleather-inspired heuristic rules engine, undo/redo with grouping, HTML round-trip via html5ever, structured clipboard with formatting+link preservation, compose document assembly (signatures, reply quoting, forward headers), and a full iced Widget with paragraph caching, exact cursor placement, per-line selection, scrolling, and drag auto-scroll.

**Architecture doc:** `docs/editor/architecture.md`

**Deferred (not blocking):**
- IME preedit/commit integration (platform capability)
- External HTML paste (iced Clipboard trait only provides plain text)

**Unblocks:** Signatures (Tier 3), Pop-Out Compose (Tier 3).

### Tier 2 — Core Email Loop

| Task | Spec | Status |
|------|------|--------|
| Contacts autocomplete + token input | `docs/contacts/autocomplete-implementation-spec.md` | Substantially complete (2026-03-22). **All 6 phases implemented.** Phase 1: token input widget with wrapping flow, keyboard state machine, group icon, drag detection. Phase 2: autocomplete wired end-to-end — `dispatch_autocomplete_search()` called from `pop_out.rs`, dropdown renders with keyboard nav (Up/Down/Enter/Tab/Escape via `autocomplete_open` flag), generation-tracked results. Phase 3: RFC 5322 paste parser wired (`token_input_parse.rs`), bulk paste banner for 10+ addresses. Phase 4: context menu with Delete/Expand group/Move to field, recursive group expansion with display name lookup. Phase 5: drag detection (4px threshold), `DragStarted` message, context menu "Move to" as primary move mechanism. Phase 6: Bcc nudge banner when group added to To/Cc, bulk paste banner. GAL cache table exists and is searched during autocomplete; GAL pre-fetch awaits sync orchestrator. Remaining: GAL directory API calls (need provider client access). |
| Search app integration (slices 5-6) | `docs/search/app-integration-spec.md` | Substantially complete (2026-03-21). Unified pipeline wired (`handlers/search.rs:366`): Tantivy+SQL fallback. Multi-value from/to, debounce, generational tracking (`search_generation` at `main.rs:339`), smart folder token migration. `delete_all_pinned_searches` DB function exists (`db/pinned_searches.rs:295`) but has no `Message` variant — unwired from UI. Missing: typeahead (Phase 3), "Search here" (Phase 4), smart folder graduation. |

### Tier 3 — Compose / Advanced Surfaces

| Task | Spec | Status |
|------|------|--------|
| Pop-out compose window | Not yet written | Partially implemented (2026-03-21). Token input recipients (`compose.rs:475`), formatting toolbar (stubs — `compose.rs:403-408`, no-op), discard confirmation, cc_addresses, attribution line. Uses `iced::widget::text_editor` not `RichTextEditor` (`compose.rs:740`). Still: no rich text, no sending (`ComposeMessage::Send` validates but does not send), no drafts, no attachments, no signatures, no auto-save. |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Partially implemented (2026-03-21). RenderingMode toggle, overflow menu, Save As to downloads dir (no file picker — `handlers/pop_out.rs:579-611`), cc_addresses, error banner, per-window generation tracking (`pop_out_generation` at `main.rs:261`). **Not wired:** session save (`handlers/pop_out.rs:473`) and restore (`handlers/pop_out.rs:508`) exist but are never called — boot does not restore pop-outs. HTML rendering modes fall back to plain text (`message_view.rs:500-506`). `body_html` populated via `BodyLoaded` but never rendered. Archive/Delete menu items exist but handlers are no-ops (`handlers/pop_out.rs:153-161`). Missing: file picker for Save As, HTML rendering in pop-out, Archive/Delete wiring, session persistence. |
| Signatures | `docs/signatures/implementation-spec.md` | Phases 1-3, 5 partially implemented (2026-03-21). Phase 1: `DbSignature` extended with 7 columns (`crates/db/src/db/types.rs:536`). Core CRUD at `crates/core/src/db/queries_extra/compose.rs:91-327`. `html_to_plain_text` at `compose.rs:716`. Phase 2: signature list + editor overlay in settings (`tabs.rs:871,1016`), but editor uses `undoable_text_input` not rich text editor (`tabs.rs:1067-1087`). Async loading wired (`handlers/signatures.rs:119`). Delete confirmation. Phase 3: `assemble_compose_document` at `crates/rte/src/compose.rs:52` (library code, not called from compose window). Phase 5: `finalize_compose_html`/`finalize_compose_plain_text` exist in core (`compose.rs:801,842`). App handlers use raw SQL, not core CRUD (`handlers/signatures.rs`). Missing: rich text editor in signature editor, Phase 4 account switching, draft restoration with signature state. |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Substantially complete (2026-03-21). CRUD wired, sidebar rendering (`sidebar.rs:435` with `ButtonClass::PinnedSearch`), relative dates, auto-expiry runs once at startup (`handlers/search.rs:130-136`, not periodic — no `iced::time::every` subscription). `delete_all_pinned_searches` DB function exists but no Message variant dispatches to it. Missing: staleness label, graduation to smart folder, periodic expiry, "Clear all" UI action. |

### Tier 4 — Additive Management

| Task | Spec | Status |
|------|------|--------|
| Contacts management + import | Not yet written | Substantially complete (2026-03-22). Backend: CardDAV sync, Google/Graph contact sync, dedup, unified search, contact group CRUD. `crates/import/` with CSV/vCard import, encoding detection, column mapping. Settings UI: contact/group lists with styled group/account pills (Badge containers), slide-in editors, new/edit/delete with confirmation, import wizard. **Group creation from import now works** — creates `contact_groups` and links members. **Distinct save behavior** — local contacts auto-save, synced contacts show explicit Save button (enabled when dirty). **Provider write-back** dispatched after save (JMAP fully implemented in core, Google/Graph scaffolded, CardDAV pending). **Inline contact editing** — sender name in reading pane is clickable, opens settings editor. Missing: LDAP, XLSX import (deferred), CardDAV PUT for write-back. |
| Emoji picker | Not yet written | Done ✅ (2026-03-21). 10-category searchable grid (Recent, Smileys, People, Nature, Food, Activities, Travel, Objects, Symbols, Flags) with skin tone selector (6 variants via `SkinTone::ALL`), recent emoji persistence, 340+ emoji (`crates/app/src/ui/emoji_picker.rs`). |
| Read receipts (outgoing) | No spec needed | Not started |

### Tier 5 — Calendar

| Task | Spec | Status |
|------|------|--------|
| Layer 1: Data model + mode switcher | No spec (product doc) | Done ✅ |
| Layer 2: Month view + mini-month | No spec (product doc) | Done ✅ |
| Layer 3: Day/Week/Work Week time grid | No spec (product doc) | Done ✅ |
| Layer 4: Event CRUD + popover/modal | No spec (product doc) | Done ✅ |
| Layer 5: Provider sync (Google/Graph/CalDAV) | No spec (product doc) | Done ✅ — Google Calendar API, Microsoft Graph, and CalDAV all wired. CalDAV client in `crates/core/src/caldav/` and `crates/calendar/src/caldav/` (PROPFIND, REPORT, iCalendar parsing, ctag-based incremental sync). |
| Full spec compliance pass (2026-03-22) | `docs/calendar/problem-statement.md` | 37/50 gaps resolved ✅ — see `docs/calendar/discrepancies.md` |

**Resolved in spec compliance pass (2026-03-22):** v63 schema fields surfaced in types + FromRow, `SELECT *` eliminated, CASCADE delete on all event delete paths, `availability` + `visibility` columns (migration v65), `calendar_default_view` read at boot, recurrence expansion (DAILY/WEEKLY/MONTHLY/YEARLY with INTERVAL/COUNT/UNTIL), recurrence icon on event blocks, two-tier event detail (compact 300px popover → full two-panel modal with mini day view), organizer/attendees/reminders/RSVP status display, calendar selector/timezone/availability/visibility in editor, ISO week numbers in month view, narrower weekend columns, calendar list with color dots + visibility toggles, event dots on mini-month, clickable agenda items, view switcher in sidebar header, Ctrl+1 Switch to Mail / Ctrl+2 Toggle Calendar / distinct Switch to Calendar + Switch to Mail commands, pop-out calendar window, 📅 email-to-calendar button on expanded messages, unsaved changes prompt, ✕ close buttons. `UpsertCalendarEventParams` + `LocalCalendarEventParams` structs replace long arg lists. All 4 provider sync paths populate new fields.

**Remaining calendar items (require custom iced widget work or provider API integration):**
- Drag interactions (move, resize, range-select) — need custom `advanced::Widget` with continuous position mapping. Spec acknowledges as "hardest to implement well in iced."
- Scroll-to-now — blocked by iced fork lacking `scroll_to()` API (UI.md:60).
- Multi-day spanning bars in month view — need fundamentally different layout approach (absolute positioning across cells).
- RSVP action buttons — need provider API round-trips (Google/Graph/CalDAV RSVP endpoints).
- Recurring event edit/delete prompts ("this / following / all") — need exception tracking + provider API.
- Meeting invite detection (F2-F4) — need iCalendar MIME part parsing in email pipeline.
- Attendee input with autocomplete — blocked on contacts autocomplete dropdown (Tier 2 gap).

**Minor polish (not blocking):**
- `mix()` pub in theme.rs — revert if unused externally.
- Unicode arrows (◀/▶) in mini-month nav — should use `icon::` helpers.
- O(n²) overlap in `set_total_columns()` — fine for typical counts.
- `TimeGridConfig` rebuilt unnecessarily for Month view — wasteful, harmless.
- CalDAV iCalendar parsing hand-rolled — works for standard events, may miss complex RRULE/VTIMEZONE edge cases.

## Spec Status

*(Full audit 2026-03-21, calendar update 2026-03-22 — per-feature reports in `docs/<feature>/discrepancies.md`)*

| Spec | Doc | Audit Status |
|------|-----|-------------|
| Calendar | `docs/calendar/problem-statement.md` | 37/50 spec gaps resolved (2026-03-22). Data model fully wired (v63 fields, availability/visibility, CASCADE delete). Two-tier event detail (popover→modal), attendees/RSVP/reminders/recurrence display, calendar list+dots+clickable agenda, view switcher in header, Ctrl+1/2, pop-out window, 📅 email-to-calendar, recurrence expansion, ISO week numbers, narrower weekends. Remaining: drag interactions (custom widget), scroll-to-now (blocked by iced API), multi-day spanning bars, RSVP action buttons (provider API), meeting invite detection (iCalendar parsing). Full details in `docs/calendar/discrepancies.md`. |
| Command palette app integration | `docs/cmdk/app-integration-spec.md` | Slices 6a-6e complete (2026-03-22). 68 commands registered. Palette is Component with PaletteEvent (ExecuteCommand/ExecuteParameterized/Dismissed/Error). Two-stage flow (command search + parameterized option pick). Full keyboard dispatch with chord sequencing and pending indicator. AppInputResolver with folder/label/cross-account options. Keybinding override persistence via `keybindings.json`. `EmailSelectAll` now wired. `is_muted`/`is_pinned` populated from Thread fields. Remaining: `NavMsgNext/Prev` return None (blocked on ReadingPane), `EmailSelectFromHere` returns None, `scroll_to_selected()` no-op (iced API), UsageTracker not persisted, Slice 6f (keybinding management UI) deferred, undo tokens (Slice 5) not started. |
| Sidebar | `docs/sidebar/implementation-spec.md` | Phases 1A-1E + Phase 2 complete. Best cross-cutting compliance. Spam/All Mail wired. O(n^2) fixed. Dead code cleaned. Relative dates. Chevron styling. Minor: `SidebarEvent::CycleAccount` never emitted — parent handler at `main.rs:898` is dead code (`sidebar.rs:122-134` handles internally). `NavigationTarget` still deferred. Mixed drafts: count path handles both (`get_draft_count_with_local`), list path returns server-synced only. |
| Accounts | `docs/accounts/implementation-spec.md` | Wizard substantially complete. Real discovery wired, OAuth flow, protocol selection, credential validation, core CRUD for creation (`create_account_sync` at `add_account.rs:936`), account editor in settings. `AccountHealth` enum exists but `compute_health()` always receives `None`/`true` for `token_expires_at`/`is_active` — always returns Healthy (`main.rs:1370`). Account deletion uses raw `DELETE FROM accounts` (`main.rs:1194`), not core CRUD. Drag-to-reorder implemented (`AccountDragState` at `settings/types.rs:747`, wired via `AccountGripPress`/`AccountDragMove`/`AccountDragEnd` at `settings/update.rs:410,136,139`). Missing: re-auth flow (Phase 7 is TODO stub at `settings/update.rs:76-78`), sidebar Phase 6 (color dots). |
| Status bar | `docs/status-bar/implementation-spec.md` | Scaffold implemented. `IcedProgressReporter` + `SyncEvent` types exist (`status_bar.rs:143-166`). `create_sync_progress_channel()` factory exists but is never called — no sync orchestrator connection (`main.rs:733-736` dispatch exists, no sender). Idle fixed height. Settings toggle wired. `Message::SyncProgress(SyncEvent)` variant exists but nothing sends it. Generational tracking methods (`begin_sync_generation`, `prune_stale_sync` at `status_bar.rs:237-264`) exist but are never called. Confirmation dispatch: `show_confirmation()` only called from placeholder reauth handler (`main.rs:1051`); no email action handlers call it because `Message::EmailAction` is a no-op (`main.rs:619`). Not in pop-outs. Remaining: connect sync orchestrator, wire confirmations to email actions, wire token expiry warnings. |
| Contacts autocomplete | `docs/contacts/autocomplete-implementation-spec.md` | All 6 phases implemented (2026-03-22). Autocomplete wired end-to-end: `dispatch_autocomplete_search()` called from `pop_out.rs`, dropdown renders with keyboard nav (Up/Down/Enter/Tab/Escape via `autocomplete_open`). RFC 5322 paste parser wired. Context menu: Delete/Expand group/Move to field. Bcc nudge banner for groups. Bulk paste banner (10+ addresses). Drag detection (4px threshold). GAL cache table and search wired; directory pre-fetch awaits provider clients. Group creation from import. Styled group/account pills. Distinct local-vs-synced save. Inline contact editing from reading pane (opens settings editor). Provider write-back dispatch (JMAP complete, Google/Graph scaffolded). Remaining: GAL directory API calls, CardDAV PUT, XLSX import (deferred). |
| Search app integration | `docs/search/app-integration-spec.md` | Backend + app integration (Slices 1-5) complete. Unified pipeline wired (`handlers/search.rs:366`). Smart folder token migration done. `SearchBlur` is a no-op (`main.rs:640`). `SearchState` initialized per-search inside `execute_search()` (`handlers/search.rs:356`), not stored on `App`. Missing: typeahead (Phase 3), "Search here" (Phase 4), smart folder graduation. |
| Editor | `docs/editor/architecture.md` | Very faithful. Double/triple click implemented. `SetBlockAttrs` added. `prepare_move_up/down` (`widget/cursor.rs:413,463`) tested but not wired into widget event path — vertical movement uses column-offset fallback. `TextAlignment` defined (`document.rs:150`) but not stored on block variants — `Block::attrs()` hardcodes `Left` (`document.rs:332-338`). |
| Signatures | `docs/signatures/implementation-spec.md` | Phases 1-3, 5 partially implemented. DbSignature extended (7 cols at `crates/db/src/db/types.rs:536`). `html_to_plain_text` at `compose.rs:716`. Transactional defaults in core. App handlers use raw SQL, not core CRUD (`handlers/signatures.rs`). Signature editor uses `undoable_text_input` not rich text editor (`tabs.rs:1067-1087`). Async loading wired. Delete confirmation. `assemble_compose_document` exists as library code (`rte/src/compose.rs:52`) but is not called from compose window. Missing: rich text editor in editor, Phase 4 account switching, draft restoration. |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Substantially complete. CRUD wired. Sidebar rendering with `ButtonClass::PinnedSearch` (`sidebar.rs:435`). Relative dates. Auto-expiry at startup only (not periodic — `handlers/search.rs:130-136`). `delete_all_pinned_searches` DB function exists (`db/pinned_searches.rs:295`) but no Message variant dispatches to it. Missing: staleness label, graduation to smart folder, periodic expiry, "Clear all" UI. |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Partially implemented. RenderingMode toggle, overflow menu, Save As to downloads dir (no file picker — `handlers/pop_out.rs:579-611`), cc_addresses, error banner, per-window generation (`pop_out_generation` at `main.rs:261`). Session save/restore functions exist but are dead code (`handlers/pop_out.rs:473,508`). HTML rendering modes fall back to plain text (`message_view.rs:500-506`). Archive/Delete handlers are no-ops (`handlers/pop_out.rs:153-161`). Compose enhanced (discard confirm, attribution line, token recipients). |
| Main layout | `docs/main-layout/iced-implementation-spec.md` | Core structure matches. **Core's `get_thread_detail()` NOT wired** — bridge at `db/threads.rs:109-134` exists but is never called. Thread selection at `main.rs:1526` uses `Db::get_thread_messages()` + `Db::get_thread_attachments()` (raw SQL). Consequences: `body_html`/`body_text` always `None` (`db/threads.rs:194-195`), `is_own_message` always `false` (`db/threads.rs:198`), no resolved label colors, no body store access. HTML rendering pipeline exists (`html_render.rs:70`) and is called from `widgets.rs:1088` but cannot fire because `body_html` is always `None`. Thread list keyboard nav wired (j/k via command palette). Search scope indicator. Per-message Reply/ReplyAll/Forward. Multi-select implemented: `selected_threads: HashSet<usize>` (`thread_list.rs:147`), Ctrl+click toggle (`thread_list.rs:314`), Shift+click range (`thread_list.rs:332`), `SelectAll` (`thread_list.rs:355`). Auto-advance implemented: `AutoAdvanceDirection` enum (`thread_list.rs:16`), `auto_advance()` method (`thread_list.rs:259`). `ModifiersChanged` subscription (`main.rs:254,469`). `PreSearchView` pattern done (`was_in_folder_view` at `main.rs:298`). Remaining: inline reply composer, `FocusedRegion` dispatch (field exists at `main.rs:285` but region-specific behavior not fully implemented). |

## Dependency Graph

```
Tier 1 — COMPLETE:
  Command Palette 6a-6e + Slice 5 ✅ (NavigateToLabel, chord indicator, snooze presets, recency sort, keybinding persistence, undo tokens, usage persistence, NavMsgNext/Prev wired, SelectFromHere wired)
  Sidebar 1A-1D ✅ (Spam/AllMail, O(n²) fix, relative dates, dead code cleanup)
  Accounts 0-7 ✅ (real discovery, OAuth, protocol selection, core CRUD for creation, account editor, health enum, deletion, duplicate detection, drag-to-reorder)
  Status Bar scaffold ✅ (IcedProgressReporter, SyncEvent types, idle height fix, settings toggle — sync orchestrator NOT connected)

  Remaining (lower priority):
    Command Palette 6f (keybinding management UI — deferred past V1)
    Sidebar Phase 2 — satisfied (no actions to strip)

Tier 2 — SUBSTANTIALLY COMPLETE (2026-03-22):
  Contacts Autocomplete ✅ (all 6 phases: widget, dropdown wired, paste parser, context menu, drag, banners)
    ├── GAL cache table + autocomplete search ✅ (directory pre-fetch awaits provider clients)
    ├── Group creation from import ✅
    ├── Provider write-back dispatch ✅ (JMAP complete, Google/Graph scaffolded)
    ├── Inline contact editing from reading pane ✅ (opens settings editor)
    ├── Styled group/account pills ✅, distinct local-vs-synced save ✅
    └── Pop-Out Compose (Tier 3 — enhanced but still needs rich text, sending, drafts)
  Search App Integration ✅ (unified pipeline wired, token migration, dead code cleanup)
    └── Pinned Searches ✅ (CRUD, sidebar rendering, relative dates, startup expiry)
    └── Sidebar 1E ✅ (pinned searches section complete)

Rich Text Editor — COMPLETE ✅ (double/triple click, SetBlockAttrs, doc fixes)
  Unblocks: Signatures, Pop-Out Compose

Tier 3 — PARTIALLY COMPLETE (2026-03-21):
  Pop-Out Message View: UI scaffold ✅, session save/restore ✗ (dead code), HTML rendering ✗ (falls back to plain text)
    ├── Pop-Out Compose (enhanced: discard confirm, token recipients, cc_addresses — no send/draft/attachment)
    └── Calendar pop-out (Tier 5)
  Signatures Phases 1-3,5: data model ✅, settings UI ✅ (plain text editor only), compose assembly library ✅ (not called from compose)
  Pinned Searches ✅ (minus periodic expiry and "Clear all" UI)

  Remaining Tier 3 work:
    Signatures Phase 4 (account switching in compose)
    Rich text editor in signature editor
    Pop-out compose: sending, drafts, auto-save, attachments, rich text
    Pop-out message view: session persistence, file picker for Save As, HTML rendering, Archive/Delete wiring

Tier 4:
  Contacts Management + Import ✅ (2026-03-22: group import, styled pills, distinct save, provider write-back, inline editing, GAL cache)
  Emoji Picker ✅ (complete: 10 categories incl. Recent + Flags, skin tones, recent persistence, 340+ emoji)

Tier 5 — 37/50 SPEC GAPS RESOLVED (2026-03-22):
  Calendar Layers 1-5 ✅ (depends on: Tier 1 shell being solid ✅)
    Layer 4: Event CRUD with detail/editor/delete overlays ✅
    Layer 5: Provider sync — Google Calendar API ✅, Microsoft Graph ✅, CalDAV ✅
    CalDAV client: PROPFIND/REPORT, iCalendar parsing, ctag-based incremental sync
    Account card drag-to-reorder in settings ✅ (custom AccountDragState at settings/types.rs:747)
  Spec compliance pass (2026-03-22):
    Data model: v63 fields surfaced, SELECT* eliminated, CASCADE delete, availability+visibility (v65), default view ✅
    Recurrence: RRULE expansion (DAILY/WEEKLY/MONTHLY/YEARLY), recurrence icon on event blocks ✅
    Event detail: two-tier popover→modal, organizer/attendees/reminders/RSVP, calendar selector/timezone/availability/visibility ✅
    Views: ISO week numbers, narrower weekends ✅
    Sidebar: calendar list+dots+clickable agenda, view switcher in header ✅
    Navigation: Ctrl+1 Mail, Ctrl+2 Toggle, Switch to Calendar/Mail commands ✅
    Pop-out: calendar window (↗ button, command, PopOutWindow::Calendar) ✅
    Email: 📅 button on expanded messages (create event from subject/snippet) ✅
    Remaining ✗: drag interactions, scroll-to-now, multi-day bars, RSVP actions, invite detection

Main Layout (cross-cutting, 2026-03-21):
  Core's get_thread_detail() NOT wired ✗ (bridge exists at db/threads.rs:109 but never called; raw SQL path at main.rs:1526)
  HTML rendering pipeline exists but cannot fire ✗ (body_html always None due to above)
  Thread list keyboard nav ✅
  Search scope indicator ✅
  Per-message Reply/ReplyAll/Forward ✅
  Multi-select ✅ (HashSet<usize> at thread_list.rs:147, Ctrl+click, Shift+click range, SelectAll)
  Auto-advance ✅ (AutoAdvanceDirection at thread_list.rs:16)
  ModifiersChanged ✅ (main.rs:254,469)
  PreSearchView pattern ✅ (was_in_folder_view at main.rs:298)
```

## Cross-Cutting Concerns

*(Verified by full 10-feature audit 2026-03-21. Detailed reports in `docs/<feature>/discrepancies.md`.)*

- **Core CRUD bypass (partially resolved):** Account creation uses `create_account_sync()` from core (`add_account.rs:936`). Account update (`main.rs:1207-1257`) and delete (`main.rs:1194`) use raw SQL. Signatures handlers (`handlers/signatures.rs`) use raw SQL with transactional default-clearing semantics, not core CRUD functions. Thread detail NOT wired through core — app uses raw SQL via `Db::get_thread_messages()` + `Db::get_thread_attachments()` (`main.rs:1526`). Calendar events have core CRUD via `crates/core/src/db/queries_extra/calendars.rs`. Calendar sync uses core's `DbState`. Contacts CRUD in app uses raw SQL (`crates/app/src/db/contacts.rs`), not core functions. Pinned searches use app-level raw SQL. Message body loading for pop-outs uses raw SQL (`db/threads.rs:248-268`), not body store. Sidebar: zero core bypass.
- **Writable DB connection:** Multiple features need local-state writes (pinned searches, attachment collapse, session restore, keybinding overrides, account metadata). The first feature to land should establish the `local_conn` pattern. This is a cross-cutting architecture decision, not owned by any single spec.
- **NavigationTarget enum:** Still deferred. `selected_label: Option<String>` remains the flat marker for universal folders, smart folders, and account labels. The command palette spec introduced this but it was never implemented.
- **Generational load tracking:** Implemented for core paths — three counters in App (`nav_generation` at `main.rs:329`, `thread_generation`, `search_generation` at `main.rs:339`) plus `option_load_generation` on `PaletteState`, `pop_out_generation` for pop-out windows (`main.rs:261`), `search_generation` on `AutocompleteState`. Status bar has generational tracking methods (`begin_sync_generation`, `prune_stale_sync` at `status_bar.rs:237-264`) but they are never called — dead code until sync orchestrator is connected. Remaining: calendar event loading.
- **Component trait:** Seven components extracted: Sidebar (`sidebar.rs:103`), ThreadList (`thread_list.rs:295`), ReadingPane (`reading_pane.rs:163`), Settings (`settings/update.rs:15`), StatusBar (`status_bar.rs:462`), AddAccountWizard (`add_account.rs:350`), Palette (`palette.rs:307`). Compose, calendar, and pop-out windows are not componentized.
- **Token-to-Catalog theming:** Very clean — zero inline style closures across all UI files. Previous palette exceptions resolved (now uses `TextClass` variants).
- **Subscription orchestration:** Infrastructure solid. Active subscriptions: keyboard listener, chord timeout, search debounce, status bar cycling, settings animation. `IcedProgressReporter` + `SyncEvent` + `create_sync_progress_channel()` exist (`status_bar.rs:143-166`) but sync orchestrator never calls them. Compose auto-save timer still missing.
- **Dead code accumulation:** Partially reduced. Key remaining items: `load_thread_detail` bridge (`db/threads.rs:109-134`), `init_body_store` (`db/threads.rs:328-333`), core signature CRUD functions (unused by app), `SidebarEvent::CycleAccount` (`sidebar.rs:43`, parent handler at `main.rs:898`), `prepare_move_up/down` (`widget/cursor.rs:413,463`), `save_session_state` / `restore_pop_out_windows` / `SessionState::load` (`handlers/pop_out.rs:473,508`, `pop_out/session.rs:40`), `body_html` on `MessageViewState` (populated but never rendered). **Resolved (2026-03-22):** `dispatch_autocomplete_search` and `should_trigger_autocomplete` are now wired. `RecipientField`, `AUTOCOMPLETE_MAX_HEIGHT`, `AUTOCOMPLETE_ROW_HEIGHT` are now used. `PendingChord.started` removed.
- **Editor** is complete (all 5 phases, 652 tests). Signatures and compose are now unblocked. With contacts autocomplete now wired (2026-03-22), the editor enables the full compose workflow.
- **Pop-out windows** are deliberately split into compose (heavy dependencies) and message-view (mostly independent, but Phase 1 is shared infrastructure). Phase 1 is complete. Message view has UI scaffold but session persistence is dead code and HTML rendering falls back to plain text. Compose enhanced with discard confirmation, token input recipients, formatting toolbar stubs, cc_addresses — still no sending, drafts, auto-save, attachments, or rich text.
- **Contacts (2026-03-22 — substantially complete):** Autocomplete fully wired end-to-end (all 6 spec phases). Token input widget with drag detection, context menu (Delete/Expand group/Move to field), Bcc nudge and bulk paste banners. RFC 5322 paste parser wired. GAL cache table + autocomplete search integration (directory fetch awaits provider clients). Group creation from import. Styled pills on contact cards. Distinct local-vs-synced save behavior. Inline contact editing from reading pane. Provider write-back dispatch (JMAP complete, Google/Graph scaffolded). Remaining: GAL directory API calls, CardDAV PUT, XLSX import (deferred).
- **Result type convergence:** The search specs identify four overlapping thread-result types (`UnifiedSearchResult`, `Thread`, `DbThread`, `SearchResult`). These should converge into a unified thread-presentation type. The natural time to do this is during search app integration (Tier 2) — specifically when wiring `UnifiedSearchResult` → `Thread` conversion in Phase 1 and the smart folder `DbThread` adapter in Phase 2. Not a blocker, but one of the cleaner refactor seams now visible.
- **Label/folder semantics:** The resolver now checks provider type and rejects Add/Remove Label on folder-based providers (Exchange/IMAP/JMAP). Move to Folder is the correct operation for those providers. This distinction is enforced in `AppInputResolver` and `Db::is_folder_based_provider()`.
- **Search execution wired (2026-03-21):** The unified pipeline is now reachable from the UI. App calls `search_pipeline::search()` with Tantivy when index is available (`handlers/search.rs:366`), falls back to smart folder parser + SQL builder for structured queries, and uses LIKE only as last-resort for pure free-text without index. `SearchBlur` is a no-op (`main.rs:640`). `SearchState` is re-initialized per search (`handlers/search.rs:356`), not stored on `App`.
