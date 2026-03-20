# Implementation Plan

Prioritized implementation plan for Ratatoskr features. The first implementation-spec batch (Tiers 1–3) is written and reviewed. Remaining items are tracked below. The editor is in progress and not listed here.

## Spec Status

| Spec | Doc | Status | Key Review Findings |
|------|-----|--------|-------------------|
| Command palette app integration | `docs/command-palette/app-integration-spec.md` | Written, reviewed | Explicit `NavigationTarget` model needed; resolver must be async with generation tracking; stage-2 is single-step V1 |
| Sidebar | `docs/sidebar/implementation-spec.md` | Written, reviewed | Phase 1A is transitional (selected_label is semantically muddy); Phase 1D hierarchy is cross-provider schema work; Gmail stays flat |
| Accounts | `docs/accounts/implementation-spec.md` | Written, reviewed | Phase 0 is backend work (not purely app); color picker in setup is intentional product decision; health derivation needs token/sync fields on Account type |
| Status bar | `docs/status-bar/implementation-spec.md` | Written, reviewed | Warnings as HashMap not Vec; separate cycle indices; connection failures informational-only in V1 |
| Contacts autocomplete | `docs/contacts/autocomplete-implementation-spec.md` | Written, reviewed | Compose-first, reusable by design; email is Option on search results; recency dominates ranking; paste needs dedup policy |
| Search app integration | `docs/search/app-integration-spec.md` | Written, reviewed | Four result types with distinct roles; pre_search_threads is V1 shortcut; smart folder CRUD uses real CommandId system |
| Signatures | `docs/signatures/implementation-spec.md` | Written, reviewed | Editor crate path is flexible; hr separator is deliberate; signature-region tracking is pragmatic V1; depends on account settings |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Written, reviewed | Writable DB connection is cross-cutting decision; query-update merges on conflict; App owns state, sidebar mirrors |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Written, reviewed | Phase 1 is shared multi-window infrastructure; body rendering is plain-text-first (HTML in Phase 3); PDF export deferred |

## Tier 1 — Shell / Unblockers

The minimum for a non-seeded, navigable app. These form a single cluster — all are equally foundational.

### Command Palette App Integration
Six sub-slices (6a–6f). 6a (keyboard dispatch + infrastructure) ships first — every keyboard shortcut works. 6b (palette UI stage 1) follows. 6c (parameterized commands) needs the real `AppInputResolver`. 6d (command-backed UI surfaces) can parallel 6b/6c. 6e/6f (persistence, keybinding management UI) are lower priority.

**Spec:** `docs/command-palette/app-integration-spec.md`
**Depends on:** Command palette slices 1-3 (done).
**Unblocks:** Sidebar Phase 2 (action stripping), search smart folder management, keyboard shortcuts everywhere.
**Cross-cutting (Tier 1 execution guidance):** Introduces `NavigationTarget` enum that sidebar, search, and pinned searches should all adopt. This is one of the few architectural normalizations that clearly wants to happen early — it replaces the semantically muddy `selected_label: Option<String>` and gives the app an explicit view-state model. Should be implemented as part of slice 6a, not deferred.

### Sidebar
Five sub-phases. 1A (live data wiring) is the critical path — connects existing `get_navigation_state()` to the sidebar. 1B (smart folder scoping fix) is a small backend change. 1C (unread counts) follows 1A. 1D (hierarchy) is the largest piece — requires DB migration, provider sync changes across Graph/JMAP/IMAP, and a composed tree renderer. 1E (pinned searches section) can be scaffolded early but full lifecycle waits on search integration. Phase 2 (strip actions) is blocked on command palette.

**Spec:** `docs/sidebar/implementation-spec.md`
**Depends on:** Phase 2 depends on command palette slice 6.
**Unblocks:** Basic navigation for the entire app.
**Note:** Phase 1A is transitional — uses `selected_label: Option<String>` which is semantically muddy. The app should eventually adopt `NavigationTarget` from the command palette spec.

### Accounts
Seven phases (0–7). Phase 0 (data model) adds DB columns and CRUD — this is backend work, not purely app-side. Phase 1 (first-launch detection) and Phase 2–3 (wizard state machine + views) are the core onboarding flow. Phase 4 (app wiring) connects the wizard to the main app. Phase 5 (settings management) adds account cards with health indicators. Phase 6 (sidebar enhancements) and Phase 7 (error states) are polish.

**Spec:** `docs/accounts/implementation-spec.md`
**Depends on:** No prior UI specs; includes required backend/data-model work in Phase 0 (new DB columns, migration, CRUD functions).
**Unblocks:** User onboarding. Without this, the app requires a seeded database.
**Note:** The writable DB connection needed here (for account_color, account_name, sort_order) is the same cross-cutting decision needed by pinned searches, session restore, and keybinding overrides. Whichever feature lands first should establish the pattern.

### Status Bar
Single implementation phase with 5 ordered steps. Small, self-contained. Warnings use `HashMap<String, AccountWarning>` (at most one per account). Separate cycle indices for warnings and sync progress.

**Spec:** `docs/status-bar/implementation-spec.md`
**Depends on:** Nothing.
**Unblocks:** Sync/auth feedback (linked to accounts spec — auth failures need recovery surface).

## Tier 2 — Core Email Loop

### Contacts: Autocomplete + Token Input
Six phases. Phase 1 (token input widget) is the largest custom widget effort — full `advanced::Widget` implementation. Phase 2 (autocomplete dropdown) is a separate overlay managed by the parent view. Phase 3 (paste handling) needs dedup within paste, dedup against existing tokens, and invalid-address dropping. Phase 4–6 (context menu, drag-and-drop, group tokens) are additive.

**Spec:** `docs/contacts/autocomplete-implementation-spec.md`
**Depends on:** Nothing (backend complete).
**Unblocks:** Compose (together with the editor). Required before pop-out compose windows.
**Note:** Compose-first, reusable by design. Calendar attendee and group editor reuse the same widget with different parent orchestration.

### Search App Integration
Four phases. Phase 1 (search bar + execution + generational tracking) is the core — generational load tracking is a correctness requirement. Phase 2 (smart folder migration) uses real `CommandId`/`CommandArgs` for CRUD. Phase 3 (operator typeahead) is the richest UI work. Phase 4 ("Search here" + polish) ties into sidebar.

**Spec:** `docs/search/app-integration-spec.md`
**Depends on:** Search slices 1-4 (done).
**Unblocks:** Pinned searches (downstream). Makes the app feel "real" for daily use.
**Note:** `pre_search_threads` clone is a V1 shortcut — should eventually restore via `NavigationTarget` re-navigation (as the pinned searches spec already does).

## Tier 3 — Compose / Advanced Surfaces

### Pop-Out Windows: Compose
**Spec not yet written.** Depends on editor + contacts autocomplete + signatures.

**Depends on:** Editor (in progress), contacts autocomplete, signatures.
**Unblocks:** Full compose workflow.

### Pop-Out Windows: Message View
Six phases. Phase 1 (multi-window architecture) is shared infrastructure — `iced::daemon` migration, window registry, per-window routing, cascade close. This is platform work reused by compose and calendar pop-outs. Phase 2 (basic message view) delivers plain text body — HTML rendering is Phase 3. Phase 6 (Save As) delivers .eml and .txt only — PDF deferred.

**Spec:** `docs/pop-out-windows/message-view-implementation-spec.md`
**Depends on:** Nothing heavy (HTML rendering pipeline exists).
**Unblocks:** Multi-monitor message reference workflows. Phase 1 also unblocks compose and calendar pop-outs.

### Signatures
Five phases. Phase 1 (data model) extends existing schema and CRUD. Phase 2 (settings UI) reuses the rich text editor for signature editing — this is the first real test of the editor's HTML round-trip. Phase 3 (compose insertion) defines document structure with signature region tracking. Phase 4 (account switching) handles replacement with edit detection. Phase 5 (send path) adds the wrapper div and RFC 3676 separator.

**Spec:** `docs/signatures/implementation-spec.md`
**Depends on:** Editor Phase 3 (HTML round-trip).
**Unblocks:** Complete compose workflow.
**Note:** Per-account default signature dropdown in account settings depends on the accounts implementation being real enough.

### Pinned Searches
Four phases. Phase 1 (schema + CRUD + sidebar) includes the writable DB connection (cross-cutting decision). Phase 2 (lifecycle + search bar) is the state machine. Phase 3 (graduation to smart folder) ties into the command palette. Phase 4 (auto-expiry) is 14-day cleanup per the updated product doc.

**Spec:** `docs/search/pinned-searches-implementation-spec.md`
**Depends on:** Search app integration.
**Unblocks:** Task-oriented triage workflows.

## Tier 4 — Additive Management

### Contacts: Management + Import
**Spec not yet written.** Settings management UI, group editor, import wizard.

**Depends on:** Contacts autocomplete spec (shared types/patterns).
**Unblocks:** Bulk contact management for enterprise users.

### Emoji Picker
**Spec not yet written.** Shared widget: `docs/emoji-picker/problem-statement.md`.

**Depends on:** Nothing (standalone widget).
**Unblocks:** Polish across multiple text input surfaces.

### Read Receipts (Outgoing)
No spec needed. Direct implementation: add `Disposition-Notification-To` header in provider send functions.

## Tier 5 — Major Independent Workstream

### Calendar
**Spec not yet written.** The largest remaining effort — new schema, provider APIs, custom iced widgets, four calendar views, event CRUD, RSVP, recurrence, pop-out window.

**Depends on:** Core app shell (Tier 1) should be solid first.
**Unblocks:** Enterprise calendar workflows. Does not block the email product.

## Dependency Graph

```
Tier 1 (parallel cluster):
  Command Palette Slice 6 (6a → 6b → 6c; 6d parallel; 6e → 6f)
    └── Sidebar Phase 2
  Sidebar Phase 1A-1E (1A is critical path)
  Accounts Phases 0-7 (Phase 0 is backend)
  Status Bar (single phase, 5 steps)

Tier 2:
  Contacts Autocomplete (6 phases)
    └── Pop-Out Compose (Tier 3, spec not yet written)
  Search App Integration (4 phases)
    └── Pinned Searches (Tier 3)

Tier 3:
  Pop-Out Message View Phase 1 (shared multi-window infra)
    ├── Pop-Out Compose (+ Editor + Contacts Autocomplete + Signatures)
    └── Calendar pop-out (Tier 5)
  Pop-Out Message View Phases 2-6 (feature-specific)
  Signatures (depends on: Editor Phase 3)
  Pinned Searches (depends on: Search App Integration)

Tier 4:
  Contacts Management + Import (depends on: Contacts Autocomplete)
  Emoji Picker (independent)

Tier 5:
  Calendar (depends on: Tier 1 shell being solid)
```

## Cross-Cutting Concerns

- **Writable DB connection:** Multiple features need local-state writes (pinned searches, attachment collapse, session restore, keybinding overrides, account metadata). The first feature to land should establish the `local_conn` pattern. This is a cross-cutting architecture decision, not owned by any single spec.
- **NavigationTarget enum:** Promoted to Tier 1 execution guidance (see command palette entry above). Should land in slice 6a, not deferred.
- **Generational load tracking:** Appears in nearly every spec (search, sidebar, command palette, pinned searches, status bar, contacts, accounts). Should be treated as a foundational primitive with a shared helper, not reimplemented per-feature.
- **Editor** is in progress and not tracked here. Together with contacts autocomplete, it unblocks serious compose work. Signatures depend on editor Phase 3 (HTML round-trip).
- **Pop-out windows** are deliberately split into compose (heavy dependencies) and message-view (mostly independent, but Phase 1 is shared infrastructure).
- **Contacts** are deliberately split into autocomplete (core email loop blocker) and management (additive, Tier 4).
- **Result type convergence:** The search specs identify four overlapping thread-result types (`UnifiedSearchResult`, `Thread`, `DbThread`, `SearchResult`). These should converge into a unified thread-presentation type. The natural time to do this is during search app integration (Tier 2) — specifically when wiring `UnifiedSearchResult` → `Thread` conversion in Phase 1 and the smart folder `DbThread` adapter in Phase 2. Not a blocker, but one of the cleaner refactor seams now visible.
