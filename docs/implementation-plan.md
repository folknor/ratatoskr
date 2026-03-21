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
| Contacts autocomplete + token input | `docs/contacts/autocomplete-implementation-spec.md` | Substantially complete (2026-03-21). Token input widget, RFC 5322 paste parser, autocomplete dropdown, arrow key nav, right-click context menu, group/GAL search, recency ranking, N+1 fix, account selector, delete confirmation. Missing: drag-and-drop between To/Cc/Bcc fields (not wired), GAL caching, keyboard interception for dropdown. |
| Search app integration (slices 5-6) | `docs/search/app-integration-spec.md` | Substantially complete (2026-03-21). Unified pipeline wired (Tantivy+SQL fallback), multi-value from/to, debounce, generational tracking, dead code cleanup, `delete_all_pinned_searches`, smart folder token migration. Missing: typeahead (Phase 3), "Search here" (Phase 4), smart folder graduation. |

### Tier 3 — Compose / Advanced Surfaces

| Task | Spec | Status |
|------|------|--------|
| Pop-out compose window | Not yet written | Enhanced (2026-03-21). Token input recipients, formatting toolbar (stubs), discard confirmation, cc_addresses, attribution line. Still: no rich text, no sending, no drafts, no attachments, no signatures. |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Phases 1-6 substantially complete (2026-03-21). RenderingMode toggle, overflow menu, session restore, Save As (.eml/.txt), cc_addresses, error banner, per-window generation tracking. Missing: file picker for Save As, HTML rendering, Archive/Delete wiring. |
| Signatures | `docs/signatures/implementation-spec.md` | Phases 1-3, 5 substantially complete (2026-03-21). DbSignature extended, html_to_plain_text, CRUD handler with transactional defaults (raw SQL, not core CRUD), async loading, delete confirmation, active_signature_id, finalize_compose. Missing: rich text editor in signature editor, Phase 4 account switching. |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Substantially complete (2026-03-21). CRUD, sidebar rendering, relative dates, delete_all, auto-expiry. Missing: staleness label, graduation to smart folder. |

### Tier 4 — Additive Management

| Task | Spec | Status |
|------|------|--------|
| Contacts management + import | Not yet written | Substantially complete (2026-03-21). Full contacts crate: CardDAV/Google/Graph sync, dedup, unified search, CRUD. `crates/contact-import/` crate with CSV/vCard import, encoding detection, column mapping. Import wizard UI in settings. Missing: LDAP, provider write-back on edit, XLSX import. |
| Emoji picker | Not yet written | Done ✅ (2026-03-21). 9-category searchable grid (Smileys, People, Nature, Food, Activities, Travel, Objects, Symbols, Flags) with skin tone selector (6 variants), recent emoji persistence, 340+ emoji. |
| Read receipts (outgoing) | No spec needed | Not started |

### Tier 5 — Calendar

| Task | Spec | Status |
|------|------|--------|
| Layer 1: Data model + mode switcher | No spec (product doc) | Done ✅ |
| Layer 2: Month view + mini-month | No spec (product doc) | Done ✅ |
| Layer 3: Day/Week/Work Week time grid | No spec (product doc) | Done ✅ |
| Layer 4: Event CRUD + popover/modal | No spec (product doc) | Done ✅ |
| Layer 5: Provider sync (Google/Graph/CalDAV) | No spec (product doc) | Done ✅ — Google Calendar API, Microsoft Graph, and CalDAV all wired. CalDAV client in `crates/calendar/src/caldav/` (PROPFIND, REPORT, iCalendar parsing, ctag-based incremental sync). |

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
| Command palette app integration | `docs/command-palette/app-integration-spec.md` | Core infra solid (Slices 1-4). `NavigateToLabel` wired end-to-end. `provider_kind`/`current_view` fixed. Chord indicator added. Snooze presets implemented. Recency sort wired. Palette componentized (Component trait + PaletteEvent enum). 4 commands return None (`NavMsgNext/Prev`, `EmailSelectAll/FromHere`). |
| Sidebar | `docs/sidebar/implementation-spec.md` | Phases 1A-1E + Phase 2 complete. Best cross-cutting compliance. Spam/All Mail wired. O(n^2) fixed. Dead code cleaned. Relative dates. Chevron styling. `CycleAccount` recursive pattern fixed. Magic number `28` replaced with `PINNED_SEARCH_QUERY_MAX_CHARS`. Minor: `CycleAccount` parent arm dead, `NavigationTarget` still deferred. Mixed drafts unified (local+server). |
| Accounts | `docs/accounts/implementation-spec.md` | Wizard substantially complete. Real discovery wired, OAuth flow, protocol selection, credential validation, core CRUD for creation, account editor in settings, `AccountHealth` enum, account deletion, duplicate detection, drag-to-reorder (custom DragState). Missing: re-auth flow, sidebar Phase 6 (color dots). |
| Status bar | `docs/status-bar/implementation-spec.md` | Scaffold + pipelines wired. `IcedProgressReporter` + `SyncEvent` types implemented. Idle fixed height. Settings toggle wired. Generational tracking. Not in pop-outs. Remaining: connect sync orchestrator, wire confirmations to email actions. |
| Contacts autocomplete | `docs/contacts/autocomplete-implementation-spec.md` | Token input + autocomplete dropdown + compose wiring complete. Paste parser, arrow keys, right-click menu, group/GAL search, recency ranking, N+1 fix, account selector, delete confirmation. Contact CRUD delegates to core. `crates/contact-import/` crate with CSV/vCard import and import wizard UI. |
| Search app integration | `docs/search/app-integration-spec.md` | Backend + app integration (Slices 1-5) complete. Unified pipeline wired. Smart folder token migration done. Missing: typeahead (Phase 3), "Search here" (Phase 4), smart folder graduation. |
| Editor | `docs/editor/architecture.md` | Very faithful. Doc stale claims fixed. Double/triple click implemented. `SetBlockAttrs` added. Minor: `prepare_move_up/down` still unwired infrastructure. |
| Signatures | `docs/signatures/implementation-spec.md` | Phases 1-3, 5 substantially complete. DbSignature extended (7 cols), html_to_plain_text, transactional defaults (raw SQL, not core CRUD), async loading, delete confirmation, active_signature_id, finalize_compose. Missing: rich text editor, Phase 4 account switching. |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Substantially complete. `delete_all_pinned_searches` added. Relative dates. Auto-expiry. Missing: staleness label, graduation to smart folder. |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Phases 1-6 substantially complete. RenderingMode toggle, overflow menu, session restore, Save As, cc_addresses, error banner, per-window generation. Missing: file picker, HTML rendering, Archive/Delete wiring. Compose enhanced (discard confirm, attribution line). |
| Main layout | `docs/main-layout/iced-implementation-spec.md` | Core structure matches. Core's `get_thread_detail()` wired (body store, ownership, label colors, attachment persistence). HTML rendering pipeline (DOM-to-widget). Thread list keyboard nav. Search scope indicator. Per-message Reply/ReplyAll/Forward. Phase 3 substantially done: keyboard nav, multi-select (Ctrl+click toggle, Shift+click range, SelectAll, `selected_threads: HashSet`), auto-advance (`AutoAdvanceDirection` enum), `ModifiersChanged`. Remaining: inline reply composer, `FocusedRegion` dispatch. `PreSearchView` pattern done (`was_in_folder_view` replaces `pre_search_threads` clone). |

## Dependency Graph

```
Tier 1 — COMPLETE:
  Command Palette 6a-6d ✅ (6a-6d enhanced 2026-03-21: NavigateToLabel, chord indicator, snooze presets, recency sort, provider_kind/current_view fixed)
  Sidebar 1A-1D ✅ (enhanced 2026-03-21: Spam/AllMail, O(n²) fix, relative dates, dead code cleanup)
  Accounts 0-7 ✅ (enhanced 2026-03-21: real discovery, OAuth, protocol selection, core CRUD, account editor, health, deletion, duplicate detection)
  Status Bar ✅ (enhanced 2026-03-21: IcedProgressReporter, SyncEvent, idle height fix, settings toggle, generational tracking)

  Remaining (lower priority):
    Command Palette 6e-6f (persistence, keybinding UI)
    Sidebar Phase 2 — satisfied (no actions to strip)

Tier 2 — SUBSTANTIALLY COMPLETE (2026-03-21):
  Contacts Autocomplete ✅ (dropdown, paste parser, arrow keys, context menu, group search, recency ranking)
    └── Pop-Out Compose (Tier 3 — enhanced but still needs rich text, sending, drafts)
  Search App Integration ✅ (unified pipeline wired, token migration, dead code cleanup)
    └── Pinned Searches ✅ (delete_all, relative dates, auto-expiry)
    └── Sidebar 1E ✅ (pinned searches section complete)

Rich Text Editor — COMPLETE ✅ (enhanced 2026-03-21: double/triple click, SetBlockAttrs, doc fixes)
  Unblocks: Signatures, Pop-Out Compose

Tier 3 — SUBSTANTIALLY COMPLETE (2026-03-21):
  Pop-Out Message View Phases 1-6 ✅ (rendering modes, overflow menu, session restore, Save As, cc_addresses, error banner, generation tracking)
    ├── Pop-Out Compose (enhanced: discard confirm, token recipients, cc_addresses)
    └── Calendar pop-out (Tier 5)
  Signatures Phases 1-3,5 ✅ (DbSignature extended, html_to_plain_text, transactional defaults, async loading, delete confirm, finalize_compose)
  Pinned Searches ✅

  Remaining Tier 3 work:
    Signatures Phase 4 (account switching in compose)
    Rich text editor in signature editor
    Pop-out compose: sending, drafts, auto-save, attachments, rich text
    Pop-out message view: file picker for Save As, HTML rendering

Tier 4:
  Contacts Management + Import ✅ (substantially complete: CardDAV/Google/Graph sync, dedup, contact-import crate, import wizard)
  Emoji Picker ✅ (complete: 9 categories incl. Flags, skin tones, recent persistence, 340+ emoji)

Tier 5 — COMPLETE (2026-03-21):
  Calendar Layers 1-5 ✅ (depends on: Tier 1 shell being solid ✅)
    Layer 4: Event CRUD with detail/editor/delete overlays ✅
    Layer 5: Provider sync — Google Calendar API ✅, Microsoft Graph ✅, CalDAV ✅
    CalDAV client: PROPFIND/REPORT, iCalendar parsing, ctag-based incremental sync
    Account card drag-to-reorder in settings ✅ (custom DragState, not iced_drop)

Main Layout (cross-cutting, 2026-03-21):
  Core's get_thread_detail() wired ✅
  HTML email rendering pipeline (DOM-to-widget) ✅
  Thread list keyboard nav ✅
  Search scope indicator ✅
  Per-message Reply/ReplyAll/Forward ✅
  Multi-select + auto-advance ✅ (Ctrl+click, Shift+click range, SelectAll, AutoAdvanceDirection, ModifiersChanged)
  PreSearchView pattern ✅ (was_in_folder_view replaces pre_search_threads clone)
```

## Cross-Cutting Concerns

*(Verified by full 10-feature audit 2026-03-21. Detailed reports in `docs/<feature>/discrepancies.md`.)*

- **Core CRUD bypass (substantially resolved):** Accounts now use `create_account_sync()` from core for creation. Signatures extracted to `handlers/signatures.rs` with transactional default-clearing semantics — but still raw SQL, not core CRUD function calls. Thread detail now wired through core's `get_thread_detail()`. Calendar events now have core CRUD via `crates/core/src/db/queries_extra/calendars.rs` (upsert with etag/ical_data/uid). Calendar sync uses core's `DbState`. Contacts CRUD delegates to core (`save_contact_sync`, `delete_contact_sync`, `load_contacts_for_settings_sync`). Pinned searches and message body loading for pop-outs still bypass core. Sidebar remains at zero core bypass.
- **Writable DB connection:** Multiple features need local-state writes (pinned searches, attachment collapse, session restore, keybinding overrides, account metadata). The first feature to land should establish the `local_conn` pattern. This is a cross-cutting architecture decision, not owned by any single spec.
- **NavigationTarget enum:** Still deferred. `selected_label: Option<String>` remains the flat marker for universal folders, smart folders, and account labels. The command palette spec introduced this but it was never implemented.
- **Generational load tracking:** Well-implemented throughout — three counters in App (`nav_generation`, `thread_generation`, `search_generation`) plus `option_load_generation` on `PaletteState`, `pop_out_generation` for pop-out windows, `sync_generations` map on status bar, `search_generation` on `AutocompleteState` for contacts. Previous gaps resolved: status bar has per-account generational tracking, signatures load async, pop-out windows use per-window generation. Remaining: calendar event loading.
- **Component trait:** Seven components extracted (Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard, Palette). Compose, calendar, and pop-out windows are not componentized.
- **Token-to-Catalog theming:** Very clean — zero inline style closures across all UI files. Previous palette exceptions resolved (now uses `TextClass` variants).
- **Subscription orchestration:** Infrastructure solid. Active subscriptions: keyboard listener, chord timeout, search debounce, status bar cycling, settings animation. `IcedProgressReporter` + `SyncEvent` + `create_sync_progress_channel()` implemented — sync orchestrator connection remaining. Compose auto-save timer still missing.
- **Dead code accumulation:** Substantially reduced (2026-03-21). ~20 of the original 20+ items resolved. Remaining: core signature CRUD functions, `CycleAccount` parent handler, `PendingChord.started`, `prepare_move_up/down`, obsolete `Db::get_thread_messages/attachments`. See TODO.md for full inventory.
- **Editor** is complete (all 5 phases, 652 tests). Signatures and compose are now unblocked. Together with contacts autocomplete, the editor enables the full compose workflow.
- **Pop-out windows** are deliberately split into compose (heavy dependencies) and message-view (mostly independent, but Phase 1 is shared infrastructure). Phase 1 is complete. Message view Phases 2-6 substantially complete (2026-03-21). Compose enhanced with discard confirmation, token input recipients, formatting toolbar stubs, cc_addresses — still no sending, drafts, auto-save, attachments, or rich text.
- **Contacts** are deliberately split into autocomplete (core email loop blocker) and management (additive, Tier 4). Token input widget + autocomplete dropdown + compose wiring substantially complete (2026-03-21). Paste parsing, arrow key nav, right-click menu, group search, recency ranking all implemented. Missing: GAL caching, drag-and-drop between fields, import crate.
- **Result type convergence:** The search specs identify four overlapping thread-result types (`UnifiedSearchResult`, `Thread`, `DbThread`, `SearchResult`). These should converge into a unified thread-presentation type. The natural time to do this is during search app integration (Tier 2) — specifically when wiring `UnifiedSearchResult` → `Thread` conversion in Phase 1 and the smart folder `DbThread` adapter in Phase 2. Not a blocker, but one of the cleaner refactor seams now visible.
- **Label/folder semantics:** The resolver now checks provider type and rejects Add/Remove Label on folder-based providers (Exchange/IMAP/JMAP). Move to Folder is the correct operation for those providers. This distinction is enforced in `AppInputResolver` and `Db::is_folder_based_provider()`.
- **Search execution wired (2026-03-21):** The unified pipeline is now reachable from the UI. App calls `search_pipeline::search()` with Tantivy when index is available, falls back to smart folder parser + SQL builder for structured queries, and uses LIKE only as last-resort for pure free-text without index.
