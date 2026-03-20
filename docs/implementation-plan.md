# Implementation Plan

Prioritized implementation plan for Ratatoskr features. The editor is in progress and not listed here.

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

### Tier 2 — Core Email Loop (next up)

| Task | Spec | Status |
|------|------|--------|
| Contacts autocomplete + token input | `docs/contacts/autocomplete-implementation-spec.md` | Not started |
| Search app integration (slices 5-6) | `docs/search/app-integration-spec.md` | Not started |

### Tier 3 — Compose / Advanced Surfaces

| Task | Spec | Status |
|------|------|--------|
| Pop-out compose window | Not yet written | Blocked on editor + contacts autocomplete |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Not started |
| Signatures | `docs/signatures/implementation-spec.md` | Not started (blocked on editor Phase 3) |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Not started (blocked on search integration) |

### Tier 4 — Additive Management

| Task | Spec | Status |
|------|------|--------|
| Contacts management + import | Not yet written | Not started |
| Emoji picker | Not yet written | Not started |
| Read receipts (outgoing) | No spec needed | Not started |

### Tier 5 — Major Independent Workstream

| Task | Spec | Status |
|------|------|--------|
| Calendar | Not yet written | Not started |

## Spec Status

| Spec | Doc | Key Review Findings |
|------|-----|-------------------|
| Command palette app integration | `docs/command-palette/app-integration-spec.md` | Explicit `NavigationTarget` model needed; resolver must be async with generation tracking; stage-2 is single-step V1 |
| Sidebar | `docs/sidebar/implementation-spec.md` | Phase 1A is transitional (selected_label is semantically muddy); Phase 1D hierarchy is cross-provider schema work; Gmail stays flat |
| Accounts | `docs/accounts/implementation-spec.md` | Phase 0 is backend work (not purely app); color picker in setup is intentional product decision; health derivation needs token/sync fields on Account type |
| Status bar | `docs/status-bar/implementation-spec.md` | Warnings as HashMap not Vec; separate cycle indices; connection failures informational-only in V1 |
| Contacts autocomplete | `docs/contacts/autocomplete-implementation-spec.md` | Compose-first, reusable by design; email is Option on search results; recency dominates ranking; paste needs dedup policy |
| Search app integration | `docs/search/app-integration-spec.md` | Four result types with distinct roles; pre_search_threads is V1 shortcut; smart folder CRUD uses real CommandId system |
| Signatures | `docs/signatures/implementation-spec.md` | Editor crate path is flexible; hr separator is deliberate; signature-region tracking is pragmatic V1; depends on account settings |
| Pinned searches | `docs/search/pinned-searches-implementation-spec.md` | Writable DB connection is cross-cutting decision; query-update merges on conflict; App owns state, sidebar mirrors |
| Pop-out message view | `docs/pop-out-windows/message-view-implementation-spec.md` | Phase 1 is shared multi-window infrastructure; body rendering is plain-text-first (HTML in Phase 3); PDF export deferred |

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

Tier 3:
  Pop-Out Message View Phase 1 (shared multi-window infra)
    ├── Pop-Out Compose (+ Editor + Contacts Autocomplete + Signatures)
    └── Calendar pop-out (Tier 5)
  Signatures (depends on: Editor Phase 3)
  Pinned Searches (depends on: Search App Integration)

Tier 4:
  Contacts Management + Import (depends on: Contacts Autocomplete)
  Emoji Picker (independent)

Tier 5:
  Calendar (depends on: Tier 1 shell being solid ✅)
```

## Cross-Cutting Concerns

- **Writable DB connection:** Multiple features need local-state writes (pinned searches, attachment collapse, session restore, keybinding overrides, account metadata). The first feature to land should establish the `local_conn` pattern. This is a cross-cutting architecture decision, not owned by any single spec.
- **NavigationTarget enum:** The command palette spec introduces this. Sidebar, search, and pinned searches should all adopt it to replace the semantically muddy `selected_label: Option<String>`. Promoted to Tier 1 execution guidance — should land in slice 6a, not deferred.
- **Generational load tracking:** Appears in nearly every spec and is now implemented across sidebar navigation, thread loading, palette option loading, and search (when wired). Should be treated as a foundational primitive with a shared helper, not reimplemented per-feature.
- **Editor** is in progress and not tracked here. Together with contacts autocomplete, it unblocks serious compose work. Signatures depend on editor Phase 3 (HTML round-trip).
- **Pop-out windows** are deliberately split into compose (heavy dependencies) and message-view (mostly independent, but Phase 1 is shared infrastructure).
- **Contacts** are deliberately split into autocomplete (core email loop blocker) and management (additive, Tier 4).
- **Result type convergence:** The search specs identify four overlapping thread-result types (`UnifiedSearchResult`, `Thread`, `DbThread`, `SearchResult`). These should converge into a unified thread-presentation type. The natural time to do this is during search app integration (Tier 2) — specifically when wiring `UnifiedSearchResult` → `Thread` conversion in Phase 1 and the smart folder `DbThread` adapter in Phase 2. Not a blocker, but one of the cleaner refactor seams now visible.
- **Label/folder semantics:** The resolver now checks provider type and rejects Add/Remove Label on folder-based providers (Exchange/IMAP/JMAP). Move to Folder is the correct operation for those providers. This distinction is enforced in `AppInputResolver` and `Db::is_folder_based_provider()`.
