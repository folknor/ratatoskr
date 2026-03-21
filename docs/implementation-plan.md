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

**Crate:** `crates/rich-text-editor/` — 14,300+ lines, 652 tests, zero clippy warnings.

**What it delivers:** From-scratch WYSIWYG rich text editor for iced. Document model with 6 block types (Paragraph, Heading, ListItem, BlockQuote, HorizontalRule, Image), Arc structural sharing, 8 invertible edit operations with position mapping, Slate-inspired normalization, fleather-inspired heuristic rules engine, undo/redo with grouping, HTML round-trip via html5ever, structured clipboard with formatting+link preservation, compose document assembly (signatures, reply quoting, forward headers), and a full iced Widget with paragraph caching, exact cursor placement, per-line selection, scrolling, and drag auto-scroll.

**Architecture doc:** `docs/editor/architecture.md`

**Deferred (not blocking):**
- IME preedit/commit integration (platform capability)
- External HTML paste (iced Clipboard trait only provides plain text)

**Unblocks:** Signatures (Tier 3), Pop-Out Compose (Tier 3).

### Tier 2 — Core Email Loop

| Task | Spec | Status |
|------|------|--------|
| Contacts autocomplete + token input | `docs/contacts/autocomplete-implementation-spec.md` | Partially implemented (2026-03-21). Token input widget implemented (`crates/app/src/ui/token_input.rs`): wrapping flow layout, keyboard state machine, arrow keys, right-click context menu, group icon. Compose window has To/Cc/Bcc fields with `TokenInputValue` (`pop_out/compose.rs:475`). Backend: `search_contacts_for_autocomplete` in `crates/app/src/db/contacts.rs`, core `search_contacts()` with FTS5. **Not wired:** `dispatch_autocomplete_search()` / `should_trigger_autocomplete()` exist (`handlers/contacts.rs:223,254`) but are never called — autocomplete dropdown never renders. RFC 5322 paste parser (`token_input_parse.rs:26`) exists with tests but `Paste` handler uses naive `split([',', ';', '\n'])` instead (`compose.rs:450`). Missing: autocomplete dropdown rendering, keyboard interception for dropdown, drag-and-drop between fields, GAL caching. |
| Search app integration (slices 5-6) | `docs/search/app-integration-spec.md` | Substantially complete (2026-03-21). Unified pipeline wired (`handlers/search.rs:366`): Tantivy+SQL fallback. Multi-value from/to, debounce, generational tracking (`search_generation` at `main.rs:339`), smart folder token migration. `delete_all_pinned_searches` DB function exists (`db/pinned_searches.rs:295`) but has no `Message` variant — unwired from UI. Missing: typeahead (Phase 3), "Search here" (Phase 4), smart folder graduation. |

### Tier 3 — Compose / Advanced Surfaces

| Task | Spec | Status |
|------|------|--------|
| Pop-out compose window | Not yet written | Partially implemented (2026-03-21). Token input recipients (`compose.rs:475`), formatting toolbar (stubs — `compose.rs:403-408`, no-op), discard confirmation, cc_addresses, attribution line. Uses `iced::widget::text_editor` not `RichTextEditor` (`compose.rs:740`). Still: no rich text, no sending (`ComposeMessage::Send` validates but does not send), no drafts, no attachments, no signatures, no auto-save. |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Partially implemented (2026-03-21). RenderingMode toggle, overflow menu, Save As to downloads dir (no file picker — `handlers/pop_out.rs:579-611`), cc_addresses, error banner, per-window generation tracking (`pop_out_generation` at `main.rs:261`). **Not wired:** session save (`handlers/pop_out.rs:473`) and restore (`handlers/pop_out.rs:508`) exist but are never called — boot does not restore pop-outs. HTML rendering modes fall back to plain text (`message_view.rs:500-506`). `body_html` populated via `BodyLoaded` but never rendered. Archive/Delete menu items exist but handlers are no-ops (`handlers/pop_out.rs:153-161`). Missing: file picker for Save As, HTML rendering in pop-out, Archive/Delete wiring, session persistence. |
| Signatures | `docs/signatures/implementation-spec.md` | Phases 1-3, 5 partially implemented (2026-03-21). Phase 1: `DbSignature` extended with 7 columns (`crates/db/src/db/types.rs:536`). Core CRUD at `crates/core/src/db/queries_extra/compose.rs:91-327`. `html_to_plain_text` at `compose.rs:716`. Phase 2: signature list + editor overlay in settings (`tabs.rs:871,1016`), but editor uses `undoable_text_input` not rich text editor (`tabs.rs:1067-1087`). Async loading wired (`handlers/signatures.rs:119`). Delete confirmation. Phase 3: `assemble_compose_document` at `crates/rich-text-editor/src/compose.rs:52` (library code, not called from compose window). Phase 5: `finalize_compose_html`/`finalize_compose_plain_text` exist in core (`compose.rs:801,842`). App handlers use raw SQL, not core CRUD (`handlers/signatures.rs`). Missing: rich text editor in signature editor, Phase 4 account switching, draft restoration with signature state. |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Substantially complete (2026-03-21). CRUD wired, sidebar rendering (`sidebar.rs:435` with `ButtonClass::PinnedSearch`), relative dates, auto-expiry runs once at startup (`handlers/search.rs:130-136`, not periodic — no `iced::time::every` subscription). `delete_all_pinned_searches` DB function exists but no Message variant dispatches to it. Missing: staleness label, graduation to smart folder, periodic expiry, "Clear all" UI action. |

### Tier 4 — Additive Management

| Task | Spec | Status |
|------|------|--------|
| Contacts management + import | Not yet written | Substantially complete (2026-03-21). Backend: CardDAV sync (`crates/core/src/carddav/`), Google/Graph contact sync, dedup, unified search (`core/src/db/queries.rs:680`), contact group CRUD (`core/src/db/queries_extra/contact_groups.rs`). `crates/contact-import/` crate with CSV/vCard import, encoding detection, column mapping (`contact-import/src/lib.rs`). Settings UI: contact/group lists (`tabs.rs:1146`), slide-in editor overlays, new/edit/delete with confirmation, import wizard overlay (`tabs.rs:2182`). App-level contacts CRUD uses raw SQL in `crates/app/src/db/contacts.rs`, not core CRUD functions. Missing: LDAP, provider write-back on edit, XLSX import. |
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

**Deferred calendar review items (tracked, not blocking):**
- `calendar_default_view` setting seeded in DB but never read — `CalendarState::new()` hardcodes Month. Should read from settings table at boot.
- New v63 schema fields (`title`, `timezone`, `recurrence_rule`, `organizer_name`, `rsvp_status`, `created_at` on events; `sort_order`, `is_default`, `provider_id` on calendars) not surfaced through `DbCalendarEvent`/`DbCalendar` types. Layer 4/5 may have partially addressed this — needs verification.
- `SELECT *` in some calendar queries — should use explicit column lists to avoid breakage if columns are added/reordered.
- Missing FK constraints on `calendar_attendees`/`calendar_reminders` → `calendar_events`. Orphaned rows possible if events deleted without cleanup. `db_delete_calendar_event` doesn't cascade.
- `mix()` made pub speculatively in theme.rs — revert if not used externally.
- Unicode arrows (◀/▶) in mini-month nav — should use icon:: helpers for consistency.
- O(n²) overlap computation in `set_total_columns()` — compares every event pair. Fine for typical day counts (<20 events) but should have a comment or TODO for future optimization.
- Full event cloning per view rebuild in `events_for_date()` — manually clones every field (no Clone derive). Now relevant with real provider data from sync.
- `TimeGridConfig` rebuilt unnecessarily for Month view — the Month branch in `rebuild_view_data()` builds a throwaway day view config that's never rendered. Wasteful, if harmless.
- No scroll-to-now/working-hours — time grid renders 0–24 from midnight with no auto-scroll to current time or business hours.
- No recurrence icon on event blocks — spec requires indicator for recurring events, but `TimeGridEvent` has no `is_recurring` field yet.
- Weekend columns not narrower in week view — spec notes this is common but says "often" not "must."
- CalDAV iCalendar parsing is hand-rolled (not `calcard` crate) — works for standard events but may miss edge cases in complex RRULE/VTIMEZONE data.

## Spec Status

*(Full audit 2026-03-21 — per-feature reports in `docs/<feature>/discrepancies.md`)*

| Spec | Doc | Audit Status |
|------|-----|-------------|
| Command palette app integration | `docs/command-palette/app-integration-spec.md` | Core infra solid (Slices 1-4). `NavigateToLabel` wired end-to-end (`handlers/commands.rs:146`). `provider_kind`/`current_view` fixed. Chord indicator added. Snooze presets implemented. Recency sort wired. Palette implements `Component` trait (`ui/palette.rs:307`). No `PaletteEvent` enum — events dispatched via direct messages. 4 commands return None (`NavMsgNext/Prev` at `command_dispatch.rs:319`, `EmailSelectAll/FromHere` at `command_dispatch.rs:462-464`). `scroll_to_selected()` is a no-op (`ui/palette.rs:380`). |
| Sidebar | `docs/sidebar/implementation-spec.md` | Phases 1A-1E + Phase 2 complete. Best cross-cutting compliance. Spam/All Mail wired. O(n^2) fixed. Dead code cleaned. Relative dates. Chevron styling. Minor: `SidebarEvent::CycleAccount` never emitted — parent handler at `main.rs:898` is dead code (`sidebar.rs:122-134` handles internally). `NavigationTarget` still deferred. Mixed drafts: count path handles both (`get_draft_count_with_local`), list path returns server-synced only. |
| Accounts | `docs/accounts/implementation-spec.md` | Wizard substantially complete. Real discovery wired, OAuth flow, protocol selection, credential validation, core CRUD for creation (`create_account_sync` at `add_account.rs:936`), account editor in settings. `AccountHealth` enum exists but `compute_health()` always receives `None`/`true` for `token_expires_at`/`is_active` — always returns Healthy (`main.rs:1370`). Account deletion uses raw `DELETE FROM accounts` (`main.rs:1194`), not core CRUD. Drag-to-reorder implemented (`AccountDragState` at `settings/types.rs:747`, wired via `AccountGripPress`/`AccountDragMove`/`AccountDragEnd` at `settings/update.rs:410,136,139`). Missing: re-auth flow (Phase 7 is TODO stub at `settings/update.rs:76-78`), sidebar Phase 6 (color dots). |
| Status bar | `docs/status-bar/implementation-spec.md` | Scaffold implemented. `IcedProgressReporter` + `SyncEvent` types exist (`status_bar.rs:143-166`). `create_sync_progress_channel()` factory exists but is never called — no sync orchestrator connection (`main.rs:733-736` dispatch exists, no sender). Idle fixed height. Settings toggle wired. `Message::SyncProgress(SyncEvent)` variant exists but nothing sends it. Generational tracking methods (`begin_sync_generation`, `prune_stale_sync` at `status_bar.rs:237-264`) exist but are never called. Confirmation dispatch: `show_confirmation()` only called from placeholder reauth handler (`main.rs:1051`); no email action handlers call it because `Message::EmailAction` is a no-op (`main.rs:619`). Not in pop-outs. Remaining: connect sync orchestrator, wire confirmations to email actions, wire token expiry warnings. |
| Contacts autocomplete | `docs/contacts/autocomplete-implementation-spec.md` | Token input widget implemented and wired in compose. `AutocompleteState` struct exists with generation counter (`compose.rs:106`). Backend search functions exist in both core and app layers. **Autocomplete search never triggered**: `dispatch_autocomplete_search()` at `handlers/contacts.rs:223` is dead code. No autocomplete dropdown rendered in compose view. RFC 5322 paste parser at `token_input_parse.rs:26` is dead code (Paste handler uses naive split). App-level contacts CRUD uses raw SQL (`db/contacts.rs`), not core CRUD functions. `crates/contact-import/` crate exists with CSV/vCard import (`contact-import/src/lib.rs`). Import wizard UI in settings (`tabs.rs:2182`). |
| Search app integration | `docs/search/app-integration-spec.md` | Backend + app integration (Slices 1-5) complete. Unified pipeline wired (`handlers/search.rs:366`). Smart folder token migration done. `SearchBlur` is a no-op (`main.rs:640`). `SearchState` initialized per-search inside `execute_search()` (`handlers/search.rs:356`), not stored on `App`. Missing: typeahead (Phase 3), "Search here" (Phase 4), smart folder graduation. |
| Editor | `docs/editor/architecture.md` | Very faithful. Double/triple click implemented. `SetBlockAttrs` added. `prepare_move_up/down` (`widget/cursor.rs:413,463`) tested but not wired into widget event path — vertical movement uses column-offset fallback. `TextAlignment` defined (`document.rs:150`) but not stored on block variants — `Block::attrs()` hardcodes `Left` (`document.rs:332-338`). |
| Signatures | `docs/signatures/implementation-spec.md` | Phases 1-3, 5 partially implemented. DbSignature extended (7 cols at `crates/db/src/db/types.rs:536`). `html_to_plain_text` at `compose.rs:716`. Transactional defaults in core. App handlers use raw SQL, not core CRUD (`handlers/signatures.rs`). Signature editor uses `undoable_text_input` not rich text editor (`tabs.rs:1067-1087`). Async loading wired. Delete confirmation. `assemble_compose_document` exists as library code (`rich-text-editor/src/compose.rs:52`) but is not called from compose window. Missing: rich text editor in editor, Phase 4 account switching, draft restoration. |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Substantially complete. CRUD wired. Sidebar rendering with `ButtonClass::PinnedSearch` (`sidebar.rs:435`). Relative dates. Auto-expiry at startup only (not periodic — `handlers/search.rs:130-136`). `delete_all_pinned_searches` DB function exists (`db/pinned_searches.rs:295`) but no Message variant dispatches to it. Missing: staleness label, graduation to smart folder, periodic expiry, "Clear all" UI. |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Partially implemented. RenderingMode toggle, overflow menu, Save As to downloads dir (no file picker — `handlers/pop_out.rs:579-611`), cc_addresses, error banner, per-window generation (`pop_out_generation` at `main.rs:261`). Session save/restore functions exist but are dead code (`handlers/pop_out.rs:473,508`). HTML rendering modes fall back to plain text (`message_view.rs:500-506`). Archive/Delete handlers are no-ops (`handlers/pop_out.rs:153-161`). Compose enhanced (discard confirm, attribution line, token recipients). |
| Main layout | `docs/main-layout/iced-implementation-spec.md` | Core structure matches. **Core's `get_thread_detail()` NOT wired** — bridge at `db/threads.rs:109-134` exists but is never called. Thread selection at `main.rs:1526` uses `Db::get_thread_messages()` + `Db::get_thread_attachments()` (raw SQL). Consequences: `body_html`/`body_text` always `None` (`db/threads.rs:194-195`), `is_own_message` always `false` (`db/threads.rs:198`), no resolved label colors, no body store access. HTML rendering pipeline exists (`html_render.rs:70`) and is called from `widgets.rs:1088` but cannot fire because `body_html` is always `None`. Thread list keyboard nav wired (j/k via command palette). Search scope indicator. Per-message Reply/ReplyAll/Forward. Multi-select implemented: `selected_threads: HashSet<usize>` (`thread_list.rs:147`), Ctrl+click toggle (`thread_list.rs:314`), Shift+click range (`thread_list.rs:332`), `SelectAll` (`thread_list.rs:355`). Auto-advance implemented: `AutoAdvanceDirection` enum (`thread_list.rs:16`), `auto_advance()` method (`thread_list.rs:259`). `ModifiersChanged` subscription (`main.rs:254,469`). `PreSearchView` pattern done (`was_in_folder_view` at `main.rs:298`). Remaining: inline reply composer, `FocusedRegion` dispatch (field exists at `main.rs:285` but region-specific behavior not fully implemented). |

## Dependency Graph

```
Tier 1 — COMPLETE:
  Command Palette 6a-6d ✅ (NavigateToLabel, chord indicator, snooze presets, recency sort, provider_kind/current_view fixed)
  Sidebar 1A-1D ✅ (Spam/AllMail, O(n²) fix, relative dates, dead code cleanup)
  Accounts 0-7 ✅ (real discovery, OAuth, protocol selection, core CRUD for creation, account editor, health enum, deletion, duplicate detection, drag-to-reorder)
  Status Bar scaffold ✅ (IcedProgressReporter, SyncEvent types, idle height fix, settings toggle — sync orchestrator NOT connected)

  Remaining (lower priority):
    Command Palette 6e-6f (persistence, keybinding UI)
    Sidebar Phase 2 — satisfied (no actions to strip)

Tier 2 — PARTIALLY COMPLETE (2026-03-21):
  Contacts Autocomplete: Token input widget ✅, backend search ✅, autocomplete wiring ✗ (dropdown never renders, paste parser not called)
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
    Wire autocomplete dropdown + paste parser in compose
    Signatures Phase 4 (account switching in compose)
    Rich text editor in signature editor
    Pop-out compose: sending, drafts, auto-save, attachments, rich text
    Pop-out message view: session persistence, file picker for Save As, HTML rendering, Archive/Delete wiring

Tier 4:
  Contacts Management + Import ✅ (substantially complete: CardDAV/Google/Graph sync, dedup, contact-import crate, import wizard)
  Emoji Picker ✅ (complete: 10 categories incl. Recent + Flags, skin tones, recent persistence, 340+ emoji)

Tier 5 — COMPLETE (2026-03-21):
  Calendar Layers 1-5 ✅ (depends on: Tier 1 shell being solid ✅)
    Layer 4: Event CRUD with detail/editor/delete overlays ✅
    Layer 5: Provider sync — Google Calendar API ✅, Microsoft Graph ✅, CalDAV ✅
    CalDAV client: PROPFIND/REPORT, iCalendar parsing, ctag-based incremental sync
    Account card drag-to-reorder in settings ✅ (custom AccountDragState at settings/types.rs:747)

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
- **Dead code accumulation:** Partially reduced. Key remaining items: `load_thread_detail` bridge (`db/threads.rs:109-134`), `init_body_store` (`db/threads.rs:328-333`), core signature CRUD functions (unused by app), `SidebarEvent::CycleAccount` (`sidebar.rs:43`, parent handler at `main.rs:898`), `PendingChord.started` (`main.rs:143`), `prepare_move_up/down` (`widget/cursor.rs:413,463`), `dispatch_autocomplete_search` / `should_trigger_autocomplete` (`handlers/contacts.rs:223,254`), `RecipientField` (`token_input.rs:50`), `AUTOCOMPLETE_MAX_HEIGHT` / `AUTOCOMPLETE_ROW_HEIGHT` (`layout.rs:433,435`), `save_session_state` / `restore_pop_out_windows` / `SessionState::load` (`handlers/pop_out.rs:473,508`, `pop_out/session.rs:40`), `body_html` on `MessageViewState` (populated but never rendered).
- **Editor** is complete (all 5 phases, 652 tests). Signatures and compose are now unblocked. Together with contacts autocomplete wiring (not yet done), the editor enables the full compose workflow.
- **Pop-out windows** are deliberately split into compose (heavy dependencies) and message-view (mostly independent, but Phase 1 is shared infrastructure). Phase 1 is complete. Message view has UI scaffold but session persistence is dead code and HTML rendering falls back to plain text. Compose enhanced with discard confirmation, token input recipients, formatting toolbar stubs, cc_addresses — still no sending, drafts, auto-save, attachments, or rich text.
- **Contacts** are deliberately split into autocomplete (core email loop blocker) and management (additive, Tier 4). Token input widget implemented. Autocomplete search/dropdown NOT wired — `dispatch_autocomplete_search()` and `should_trigger_autocomplete()` are defined but never called (`handlers/contacts.rs:223,254`). RFC 5322 paste parser exists but is not called from `Paste` handler (`compose.rs:450`). Management UI in settings substantially complete. `crates/contact-import/` crate exists with CSV/vCard import and import wizard UI in settings (`tabs.rs:2182`).
- **Result type convergence:** The search specs identify four overlapping thread-result types (`UnifiedSearchResult`, `Thread`, `DbThread`, `SearchResult`). These should converge into a unified thread-presentation type. The natural time to do this is during search app integration (Tier 2) — specifically when wiring `UnifiedSearchResult` → `Thread` conversion in Phase 1 and the smart folder `DbThread` adapter in Phase 2. Not a blocker, but one of the cleaner refactor seams now visible.
- **Label/folder semantics:** The resolver now checks provider type and rejects Add/Remove Label on folder-based providers (Exchange/IMAP/JMAP). Move to Folder is the correct operation for those providers. This distinction is enforced in `AppInputResolver` and `Db::is_folder_based_provider()`.
- **Search execution wired (2026-03-21):** The unified pipeline is now reachable from the UI. App calls `search_pipeline::search()` with Tantivy when index is available (`handlers/search.rs:366`), falls back to smart folder parser + SQL builder for structured queries, and uses LIKE only as last-resort for pure free-text without index. `SearchBlur` is a no-op (`main.rs:640`). `SearchState` is re-initialized per search (`handlers/search.rs:356`), not stored on `App`.
