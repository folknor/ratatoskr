# Implementation Spec Plan

Prioritized plan for writing implementation specifications. Each entry represents an implementation spec to be written, not the implementation work itself. The editor is already in progress and not listed here.

## Tier 1 — Shell / Unblockers

The minimum for a non-seeded, navigable app. These form a single cluster — all are equally foundational.

### Command Palette Slice 6 Elaboration
Elaborate the existing roadmap's Slice 6 into a full implementation spec covering the three app integration paths: palette overlay UI, keyboard dispatch via iced subscriptions, and command-backed menus/toolbars/context surfaces. Must also cover `CommandArgs` enum and the typed execution contract (the most critical missing piece — see roadmap note), `CommandContext` assembly from app model state, real `CommandInputResolver` implementation querying `DbState`, and pending-chord UI indicator.

**Depends on:** Command palette slices 1-3 (done).
**Unblocks:** Sidebar Phase 2 (action stripping), search smart folder management, keyboard shortcuts everywhere.

### Sidebar
Proper implementation spec for Phase 1 (live data wiring from `get_navigation_state()`, hierarchy support for Exchange/IMAP/JMAP folders, scope selector) and Phase 2 (strip actions, blocked on command palette slice 6). Phase 1 is partially implemented — the spec should distinguish what exists from what remains.

**Depends on:** Phase 2 depends on command palette slice 6.
**Unblocks:** Basic navigation for the entire app.

### Accounts
Implementation spec for the UI layer: first-launch modal, multi-step add-account wizard (email → discovery → auth → color picker → success), settings account list with editor slide-in, account health indicators, deletion edge cases. Backend exists — this is purely app-side work.

**Depends on:** Nothing (backend complete).
**Unblocks:** User onboarding. Without this, the app requires a seeded database.

### Status Bar
Small, independent spec. Pairs naturally with accounts/sidebar work since auth failures and sync progress need a surface. Priority preemption state machine, cycling logic for multiple accounts, clickable warnings triggering recovery flows.

**Depends on:** Nothing.
**Unblocks:** Sync/auth feedback (linked to accounts spec — auth failures need recovery surface).

## Tier 2 — Core Email Loop

### Contacts: Autocomplete + Token Input
Focused implementation spec covering only the compose-critical path: recipient token input widget (custom `advanced::Widget`), autocomplete dropdown with unified local search (synced contacts + seen addresses + cached GAL), paste tokenization, token drag-and-drop between To/Cc/Bcc, group tokens, contact resolution. Explicitly excludes the management UI and import wizard.

**Depends on:** Nothing (backend complete).
**Unblocks:** Compose (together with the editor). Required before pop-out compose windows.

### Search App Integration (Slices 5-6 Elaboration)
Elaborate the existing implementation spec's slices 5-6: wire the unified search function into the app's search bar, generational load tracking (correctness requirement, not polish), smart folder migration to the unified pipeline, and `SearchResult` → thread list rendering. The backend pipeline is complete — this is app integration and smart folder unification.

**Depends on:** Search slices 1-4 (done).
**Unblocks:** Pinned searches (downstream). Makes the app feel "real" for daily use.

## Tier 3 — Compose / Advanced Surfaces

### Pop-Out Windows: Compose
Implementation spec for the compose pop-out window only. Multi-window lifecycle in iced, compose window layout (recipient fields, formatting toolbar, editor integration, signature, quoted content, attachments with drag-and-drop zones), draft auto-save, discard confirmation, session restore for compose windows. This is the heavier half and depends on editor + contacts autocomplete.

**Depends on:** Editor (in progress), contacts autocomplete.
**Unblocks:** Full compose workflow.

### Pop-Out Windows: Message View
Separate, simpler implementation spec. Single-message viewer with rendering mode toggle (plain/simple HTML/original HTML/source), header layout, reply/forward actions opening compose windows, Save As (.eml, .txt — PDF deferred). Lighter dependency profile than compose.

**Depends on:** Nothing heavy (HTML rendering pipeline already exists).
**Unblocks:** Multi-monitor message reference workflows.

### Signatures
Design spec for signature management in Settings: creating/editing/deleting signatures, per-account default signature assignment, rich text editing (reuses the editor's document model and widget), and signature insertion behavior in compose (new message, reply, forward — where it's placed, how it interacts with quoted content, what happens when switching From account).

**Depends on:** Editor (in progress).
**Unblocks:** Complete compose workflow (signatures are expected in every outgoing email for enterprise users).

### Pinned Searches
Implementation spec for the sidebar section, pinned search lifecycle (auto-creation, edit-in-place, refresh, dismissal, graduation to smart folder), SQLite persistence, and search bar interaction.

**Depends on:** Search app integration.
**Unblocks:** Task-oriented triage workflows.

## Tier 4 — Additive Management

### Contacts: Management + Import
Implementation spec for the settings management UI (contact cards, group editor, slide-in editing, explicit Save for synced contacts) and the import wizard (CSV/XLSX/vCard parsing, column mapping preview, merge toggle). Separate crate for import (`crates/contact-import/`).

**Depends on:** Contacts autocomplete spec (shared types/patterns).
**Unblocks:** Bulk contact management for enterprise users.

### Emoji Picker
Shared widget spec at `docs/emoji-picker/problem-statement.md`. Searchable grid, categories/tabs, recent/frequent section, skin tone selection. Used in compose toolbar, calendar event descriptions, contact notes, and anywhere text input supports emoji. Separate widget crate or module — not compose-specific.

**Depends on:** Nothing (standalone widget).
**Unblocks:** Polish across multiple text input surfaces. Not a hard blocker for any single feature.

### Read Receipts (Outgoing)
No implementation spec needed. Direct implementation: add `Disposition-Notification-To` header in provider send functions. Track as a task, not a document.

## Tier 5 — Major Independent Workstream

### Calendar Phased Implementation Spec
The largest remaining spec effort. Needs phased implementation covering: new SQLite schema (calendars, events, attendees, recurrence rules, reminders), provider calendar API integration (Google Calendar, Microsoft Graph Calendar, CalDAV), mode switcher UI, calendar sidebar (mini-month, view switcher, calendar list), four main views (day, work week, week, month — each a custom iced widget), event blocks, event interaction (popover → modal), event CRUD, RSVP, recurrence expansion (rrule crate), time picker with timezone support, email ↔ calendar integration, and eventually pop-out calendar window.

**Depends on:** Core app shell (Tier 1) should be solid first.
**Unblocks:** Enterprise calendar workflows. Does not block the email product.

## Dependency Graph

```
Tier 1 (parallel cluster):
  Command Palette Slice 6
    └── Sidebar Phase 2
  Sidebar Phase 1 (independent)
  Accounts (independent)
  Status Bar (independent, pairs with Accounts)

Tier 2:
  Contacts Autocomplete (independent)
    └── Pop-Out Compose (Tier 3)
  Search App Integration
    └── Pinned Searches (Tier 3)

Tier 3:
  Pop-Out Compose (depends on: Editor + Contacts Autocomplete + Signatures)
  Pop-Out Message View (mostly independent)
  Signatures (depends on: Editor)
  Pinned Searches (depends on: Search App Integration)

Tier 4:
  Contacts Management + Import (depends on: Contacts Autocomplete)
  Emoji Picker (independent)

Tier 5:
  Calendar (depends on: Tier 1 shell being solid)
```

## Cross-Cutting Notes

- **Editor** is in progress and not tracked here. Together with contacts autocomplete, it unblocks serious compose work.
- **Generational load tracking** (bloom pattern) should be treated as a foundational primitive — it appears in nearly every spec (search, calendar, main layout, sidebar, command palette, pinned searches, status bar, contacts).
- **Pop-out windows** are deliberately split into compose and message-view specs because the compose half has heavy dependencies (editor, autocomplete) while message-view is comparatively self-contained.
- **Contacts** are deliberately split into autocomplete (core email loop) and management (additive) because the token widget is a compose blocker while the settings UI is not.
