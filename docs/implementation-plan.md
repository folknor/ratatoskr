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

### Tier 2 — Core Email Loop (next up)

| Task | Spec | Status |
|------|------|--------|
| Contacts autocomplete + token input | `docs/contacts/autocomplete-implementation-spec.md` | Not started |
| Search app integration (slices 5-6) | `docs/search/app-integration-spec.md` | Not started |

### Tier 3 — Compose / Advanced Surfaces

| Task | Spec | Status |
|------|------|--------|
| Pop-out compose window | Not yet written | Blocked on contacts autocomplete (editor ✅) |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Not started |
| Signatures | `docs/signatures/implementation-spec.md` | Not started (editor dependency satisfied ✅) |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Not started (blocked on search integration) |

### Tier 4 — Additive Management

| Task | Spec | Status |
|------|------|--------|
| Contacts management + import | Not yet written | Not started |
| Emoji picker | Not yet written | Not started |
| Read receipts (outgoing) | No spec needed | Not started |

### Tier 5 — Calendar

| Task | Spec | Status |
|------|------|--------|
| Layer 1: Data model + mode switcher | No spec (product doc) | Done ✅ |
| Layer 2: Month view + mini-month | No spec (product doc) | Done ✅ |
| Layer 3: Day/Week/Work Week time grid | No spec (product doc) | Done ✅ |
| Layer 4: Event CRUD + popover/modal | Not started | |
| Layer 5: Provider sync (Google/Graph/CalDAV) | Not started | |

**Deferred calendar review items (tracked, not blocking):**
- `calendar_default_view` setting seeded in DB but never read — `CalendarState::new()` hardcodes Month. Should read from settings table at boot.
- New v63 schema fields (`title`, `timezone`, `recurrence_rule`, `organizer_name`, `rsvp_status`, `created_at` on events; `sort_order`, `is_default`, `provider_id` on calendars) not surfaced through `DbCalendarEvent`/`DbCalendar` types. Follow-on layers will need these in the canonical types.
- `SELECT *` in some calendar queries — should use explicit column lists to avoid breakage if columns are added/reordered.
- Missing FK constraints on `calendar_attendees`/`calendar_reminders` → `calendar_events`. Orphaned rows possible if events deleted without cleanup. `db_delete_calendar_event` doesn't cascade.
- `mix()` made pub speculatively in theme.rs — revert if not used externally.
- Unicode arrows (◀/▶) in mini-month nav — should use icon:: helpers for consistency.
- O(n²) overlap computation in `set_total_columns()` — compares every event pair. Fine for typical day counts (<20 events) but should have a comment or TODO for future optimization.
- Full event cloning per view rebuild in `events_for_date()` — manually clones every field (no Clone derive). Currently moot with empty event sets but will matter when real provider data arrives.
- `TimeGridConfig` rebuilt unnecessarily for Month view — the Month branch in `rebuild_view_data()` builds a throwaway day view config that's never rendered. Wasteful, if harmless.
- No scroll-to-now/working-hours — time grid renders 0–24 from midnight with no auto-scroll to current time or business hours.
- No recurrence icon on event blocks — spec requires 🔁 indicator for recurring events, but `TimeGridEvent` has no `is_recurring` field yet.
- Weekend columns not narrower in week view — spec notes this is common but says "often" not "must."

## Spec Status

*(Full audit 2026-03-21 — per-feature reports in `docs/<feature>/discrepancies.md`)*

| Spec | Doc | Audit Status |
|------|-----|-------------|
| Command palette app integration | `docs/command-palette/app-integration-spec.md` | Core infra solid (Slices 1-4). `NavigateToLabel` entirely dead. Palette not componentized. 5 commands return None. No chord indicator. |
| Sidebar | `docs/sidebar/implementation-spec.md` | Phases 1A-1E complete. Best cross-cutting compliance. Minor: date format, `CycleAccount` dead, `NavigationTarget` still deferred. |
| Accounts | `docs/accounts/implementation-spec.md` | Wizard UI mostly matches. Discovery faked, OAuth unwired, protocol selection placeholder, core CRUD bypassed, no editor/health/reauth/deletion. |
| Status bar | `docs/status-bar/implementation-spec.md` | Scaffold faithful. All 3 data pipelines unwired (sync/warnings/confirmations). Idle collapses to zero height (spec says fixed). Appears in pop-outs. |
| Contacts autocomplete | `docs/contacts/autocomplete-implementation-spec.md` | Token input widget exists. Autocomplete dropdown entirely missing. No compose wiring. Core CRUD bypassed. Import crate missing. |
| Search app integration | `docs/search/app-integration-spec.md` | Backend (Slices 1-4) fully implemented. App search is SQL LIKE stub — entire pipeline unreachable from UI. Smart folder migration not started. |
| Editor | `docs/editor/architecture.md` | Very faithful. 652 tests (doc says 428). Doc has stale claims. Minor dead code (`_last_click`, `prepare_move_up/down`). |
| Signatures | `docs/signatures/implementation-spec.md` | Basic CRUD + settings UI + compose assembly. Plain text editor (not rich text). Core CRUD bypassed. Phases 4-5 missing. |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Substantially implemented. Missing `delete_all_pinned_searches`. Date format diverges. No graduation to smart folder. |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Phase 1 complete. Phase 2 mostly done (missing fields). Phases 3-6 not started. Compose is UI shell only. |
| Main layout | `docs/main-layout/iced-implementation-spec.md` | Core structure matches. App-local DB shim bypasses core's `get_thread_detail()`. Phase 3 interaction deferred. No body rendering. |

## Dependency Graph

```
Tier 1 — COMPLETE:
  Command Palette 6a-6d ✅
  Sidebar 1A-1D ✅
  Accounts 0-7 ✅
  Status Bar ✅

  Remaining (lower priority):
    Command Palette 6e-6f (persistence, keybinding UI)
    Sidebar 1E (pinned searches section, needs search integration)
    Sidebar Phase 2 (strip actions, needs palette maturity)

Tier 2 (next):
  Contacts Autocomplete (independent)
    └── Pop-Out Compose (Tier 3, spec not yet written)
  Search App Integration
    └── Pinned Searches (Tier 3)
    └── Sidebar 1E (pinned searches section)

Rich Text Editor — COMPLETE ✅
  Unblocks: Signatures, Pop-Out Compose

Tier 3:
  Pop-Out Message View Phase 1 (shared multi-window infra)
    ├── Pop-Out Compose (+ Editor ✅ + Contacts Autocomplete + Signatures)
    └── Calendar pop-out (Tier 5)
  Signatures (depends on: Editor ✅ — ready to start)
  Pinned Searches (depends on: Search App Integration)

Tier 4:
  Contacts Management + Import (depends on: Contacts Autocomplete)
  Emoji Picker (independent)

Tier 5:
  Calendar Layers 1-3 ✅ (depends on: Tier 1 shell being solid ✅)
```

## Cross-Cutting Concerns

*(Verified by full 10-feature audit 2026-03-21. Detailed reports in `docs/<feature>/discrepancies.md`.)*

- **Core CRUD bypass (partially resolved):** Calendar CRUD, contacts/groups CRUD, and pop-out body/attachment loading now delegate to core functions. Calendar uses `create/update/delete/get_calendar_event_sync` and `load_calendar_events_for_view_sync`. Contacts use `save_contact_sync`, `delete_contact_sync`, `load_contacts_for_settings_sync` and group equivalents. Pop-out body loads use `BodyStoreState::get()`, attachment loads use `get_attachments_for_message()`. Schema for pinned_searches and contact extended columns moved to migration 64. **Remaining bypasses:** accounts, signatures, main-window thread messages/attachments, pinned search CRUD, palette label queries, and `load_raw_source`. Accounts and signatures remain the worst offenders.
- **Writable DB connection:** Multiple features need local-state writes (pinned searches, attachment collapse, session restore, keybinding overrides, account metadata). The first feature to land should establish the `local_conn` pattern. This is a cross-cutting architecture decision, not owned by any single spec.
- **NavigationTarget enum:** Still deferred. `selected_label: Option<String>` remains the flat marker for universal folders, smart folders, and account labels. The command palette spec introduced this but it was never implemented.
- **Generational load tracking:** Well-implemented where applied — three counters in App (`nav_generation`, `thread_generation`, `search_generation`) plus `option_load_generation` on `PaletteState`. Properly guards 8+ async load paths. Remaining gaps: status bar (no staleness detection for sync progress), signatures (synchronous loading), pop-out windows (no per-window generation).
- **Component trait:** Six components extracted (Sidebar, ThreadList, ReadingPane, Settings, StatusBar, AddAccountWizard). Palette, compose, calendar, and pop-out windows are not componentized.
- **Token-to-Catalog theming:** Very clean — zero inline style closures across all UI files. Two minor exceptions in palette (should be `TextClass` variants).
- **Subscription orchestration:** Infrastructure solid. Active subscriptions: keyboard listener, chord timeout, search debounce, status bar cycling, settings animation. Gap: sync pipeline entirely unwired (no `IcedProgressReporter`), compose auto-save timer missing.
- **Dead code accumulation:** Significant. 20+ identified items across features — see TODO.md for full inventory.
- **Editor** is complete (all 5 phases, 652 tests). Signatures and compose are now unblocked. Together with contacts autocomplete, the editor enables the full compose workflow.
- **Pop-out windows** are deliberately split into compose (heavy dependencies) and message-view (mostly independent, but Phase 1 is shared infrastructure). Phase 1 is complete. Compose is a UI shell only — no sending, drafts, auto-save, attachments, or rich text.
- **Contacts** are deliberately split into autocomplete (core email loop blocker) and management (additive, Tier 4). Token input widget exists but autocomplete dropdown is entirely missing — not wired to compose.
- **Result type convergence:** The search specs identify four overlapping thread-result types (`UnifiedSearchResult`, `Thread`, `DbThread`, `SearchResult`). These should converge into a unified thread-presentation type. The natural time to do this is during search app integration (Tier 2) — specifically when wiring `UnifiedSearchResult` → `Thread` conversion in Phase 1 and the smart folder `DbThread` adapter in Phase 2. Not a blocker, but one of the cleaner refactor seams now visible.
- **Label/folder semantics:** The resolver now checks provider type and rejects Add/Remove Label on folder-based providers (Exchange/IMAP/JMAP). Move to Folder is the correct operation for those providers. This distinction is enforced in `AppInputResolver` and `Db::is_folder_based_provider()`.
- **Search execution is a stub:** The entire backend pipeline (parser, SQL builder, Tantivy, unified pipeline) is unreachable from the UI. App uses `WHERE subject LIKE` instead. `SearchState` never initialized.
