# Status Bar: Spec vs. Implementation Discrepancies

Audit date: 2026-03-21

Spec files reviewed:
- `docs/status-bar/problem-statement.md`
- `docs/status-bar/implementation-spec.md`

Code files reviewed:
- `crates/app/src/ui/status_bar.rs`
- `crates/app/src/main.rs` (integration points)
- `crates/app/src/ui/theme.rs` (ContainerClass::StatusBar, TextClass::Warning)
- `crates/app/src/ui/layout.rs` (STATUS_BAR_HEIGHT, PAD_STATUS_BAR)
- `crates/app/src/ui/settings/` (sync_status_bar toggle)
- `crates/app/src/component.rs` (Component trait)

---

## What Matches the Spec

1. **Component structure** -- `StatusBar` implements `Component` trait with `Message = StatusBarMessage`, `Event = StatusBarEvent`. File location matches (`crates/app/src/ui/status_bar.rs`). Module registered in `ui/mod.rs`.

2. **Types** -- `AccountWarning`, `WarningKind` (TokenExpiry, ConnectionFailure), `SyncAccountProgress`, `Confirmation` all match the spec. `StatusBarMessage` (CycleTick, WarningClicked) and `StatusBarEvent` (RequestReauth) match.

3. **Inbound data methods** -- `report_sync_progress()`, `report_sync_complete()`, `set_warning()`, `clear_warning()`, `show_confirmation()` all present with correct signatures.

4. **Priority resolution** -- `resolve()` logic matches the spec exactly: warnings first, then confirmation-preempts-sync, then sync progress, then standalone confirmation, then idle.

5. **Warning resolution** -- Single/multi-warning formatting matches spec. Cycle index with modulo wrapping matches.

6. **Sync progress resolution** -- Single/multi-account formatting with cycling matches spec.

7. **Constants** -- `CONFIRMATION_DURATION` (3s) and `CYCLE_INTERVAL` (3s) match. `STATUS_BAR_HEIGHT` (28.0) and `PAD_STATUS_BAR` (4/12/4/12) match.

8. **Theme additions** -- `ContainerClass::StatusBar` uses `background.weaker` with 10% alpha border. `TextClass::Warning` uses `theme.palette().warning.base.color`. Both use named style classes via `fn style()` -- no inline closures.

9. **App integration** -- `status_bar` field on `App`, initialized via `StatusBar::new()` in boot. `Message::StatusBar` variant present. Subscription wired. `handle_status_bar()` and `handle_status_bar_event()` match spec structure.

10. **View integration** -- Status bar rendered at bottom of main layout in both mail and calendar modes via `column![layout, status_bar]`. Also present in settings view. Present in pop-out message view (see divergence below).

11. **update() implementation** -- CycleTick advances both cycle indices with wrapping_add, expires confirmations. WarningClicked emits RequestReauth for TokenExpiry, no-ops for ConnectionFailure.

12. **subscription()** -- Conditional tick: fires only when warnings > 1, sync_progress > 1, or confirmation is Some. Matches spec.

13. **View helper** -- `build_status_row()` extracts repeated row construction into a helper. Uses `icon::alert_triangle()`, `icon::refresh()`, `icon::check()` with `ICON_MD` size and `TEXT_SM` text. Matches spec icon choices.

14. **format_number()** -- Thousands separator helper matches spec.

---

## Divergences

### D1. `ResolvedContent::Warning` missing `account_id` field

**Spec:** `ResolvedContent::Warning` carries `account_id: String` for the click handler.
**Code:** The `account_id` field is absent from `ResolvedContent::Warning`. The `WarningClicked` handler re-derives the warning from `self.warnings` using `warning_cycle_index`, so the field is not functionally needed. However, there is a subtle race: if a warning is added or removed between `view()` (which resolved the displayed warning) and the `WarningClicked` message arriving in `update()`, the cycle index could point to a different warning. The spec's approach of embedding `account_id` in the resolved content would avoid this, but in practice the race window is negligible.

**Severity:** Low -- functionally correct but architecturally divergent from spec.

### D2. `warnings` uses `BTreeMap` instead of `HashMap`

**Spec:** `warnings: HashMap<String, AccountWarning>`.
**Code:** `warnings: BTreeMap<String, AccountWarning>`.

The `BTreeMap` provides deterministic iteration order (sorted by account ID), which makes cycling through multiple warnings stable and predictable. This is an improvement over the spec's `HashMap` (which would produce arbitrary cycling order). Not a bug -- an intentional upgrade.

**Severity:** None (improvement).

### D3. Idle state renders zero-height Space, not fixed-height container

**Spec (section 13):** "When no warnings, sync progress, or confirmations are active, the status bar renders an empty container at its fixed height. [...] the bar's physical presence is constant."
**Code:** `ResolvedContent::Idle` renders `Space::new().width(0).height(0)` -- the bar collapses to zero height.

The code has a comment: "Nothing to show -- collapse to zero height. 'Absence means nothing to say' per the problem statement." This contradicts the implementation spec's explicit requirement for a constant-height container. The main content area shifts vertically when the status bar transitions between idle and active.

**Severity:** Medium -- layout shift is noticeable and the spec explicitly called this out.

### D4. Clickable `mouse_area` wraps all warnings, not just TokenExpiry

**Spec (section 14):** "ConnectionFailure warnings render without the mouse_area wrapper (no on_press, no cursor change)."
**Code:** The `mouse_area` wrapper with `on_press` and `Interaction::Pointer` is applied whenever `clickable` is true. The `clickable` flag is correctly set to true only for `TokenExpiry`. However, when cycling through multiple warnings, if the currently displayed warning is `ConnectionFailure`, `clickable` is false and no `mouse_area` is applied -- this matches the spec. But if `TokenExpiry` is displayed, the entire bar becomes clickable even though the `WarningClicked` handler correctly no-ops for `ConnectionFailure`. This is actually correct behavior.

**Severity:** None -- on closer inspection this matches the spec.

---

## What's Missing

### M1. Sync progress pipeline not wired

**Spec (sections 9.1-9.5):** `IcedProgressReporter`, `SyncEvent` enum, channel-based subscription, `Message::SyncProgress` variant, and the `update()` handler that calls `report_sync_progress()` / `report_sync_complete()` / `set_warning()`.
**Code:** None of this exists. The `report_sync_progress()`, `report_sync_complete()`, `set_warning()`, and `clear_warning()` methods exist on `StatusBar` but are never called from `main.rs`. The `SyncEvent` type and `IcedProgressReporter` are not implemented anywhere in the app crate.

**Impact:** The status bar scaffold is complete but receives no real data. It will always show idle state.

### M2. Confirmation pipeline not wired

**Spec (section 11.1):** `show_confirmation()` should be called from action handlers (move to trash, archive, apply label, etc.).
**Code:** `show_confirmation()` exists on `StatusBar` but is never called from `main.rs` or any other file.

**Impact:** Transient confirmations never appear.

### M3. Warning pipeline not wired

**Spec (sections 10.1-10.3):** Token expiry and connection failure warnings should be set from sync error handling.
**Code:** `set_warning()` and `clear_warning()` exist but are never called.

**Impact:** Warnings never appear.

### M4. Settings toggle not connected

**Code:** `sync_status_bar: bool` exists in `SettingsState` with a UI toggle ("Show Sync Status Bar"), but this flag is never read in `main.rs` to conditionally show/hide the status bar.
**Spec:** Does not mention a settings toggle at all.

**Impact:** The toggle is dead UI -- changing it has no effect.

### M5. `RequestReauth` handler is a stub

**Spec (section 8.3):** `handle_status_bar_event` logs the request as a TODO placeholder.
**Code:** Matches -- the handler drops the `account_id` with `let _ = account_id` and returns `Task::none()`. This is expected (spec notes it will be wired with the accounts UI), but worth tracking.

### M6. Pop-out window includes status bar (spec says it should not)

**Problem statement:** "The status bar exists in both mail and calendar modes. It does not appear in pop-out windows."
**Code:** `view_message_detail_window()` (line ~2235) includes `column![layout, status_bar]`, rendering the status bar in the pop-out message detail window.

**Severity:** Medium -- contradicts the problem statement.

---

## Cross-Cutting Concern Status

### a. Generational load tracking

**Not used.** The spec's problem statement references bloom's generational tracking pattern for stale sync state, but the implementation does not use per-account generation counters. Sync progress is simply inserted/removed from a `HashMap`. There is no staleness detection -- if a sync task dies without calling `report_sync_complete()`, its progress entry will persist indefinitely, showing stale numbers.

### b. Component trait

**Fully implemented.** `StatusBar` implements the project's `Component` trait (`crates/app/src/component.rs`) with `type Message = StatusBarMessage` and `type Event = StatusBarEvent`. The trait provides `update()`, `view()`, and `subscription()`. Integration in `main.rs` follows the standard pattern: `handle_status_bar()` dispatches messages, maps tasks, and forwards events to `handle_status_bar_event()`.

### c. Token-to-Catalog theming

**Fully compliant.** All styling uses named style classes: `ContainerClass::StatusBar.style()`, `TextClass::Warning.style()`, `TextClass::Muted.style()`. No inline style closures anywhere in `status_bar.rs`. Layout constants (`STATUS_BAR_HEIGHT`, `PAD_STATUS_BAR`, `ICON_MD`, `TEXT_SM`, `SPACE_XS`) come from the shared layout module.

### d. iced_drop drag-and-drop

**N/A.** The status bar has no drag-and-drop interaction.

### e. Subscription orchestration

**Correctly implemented.** The status bar uses `iced::time::every(CYCLE_INTERVAL)` for cycling ticks, conditionally activated only when cycling or confirmation expiry is needed. The subscription is batched into the app's subscription list via `self.status_bar.subscription().map(Message::StatusBar)`.

### f. Core CRUD bypassed

**N/A.** The status bar does not perform any database operations. All data flows inward via direct method calls from the app.

### g. Dead code

- **Inbound data methods** (`report_sync_progress`, `report_sync_complete`, `set_warning`, `clear_warning`, `show_confirmation`): All five methods are defined but never called from outside `status_bar.rs`. They are public API awaiting integration, not truly dead -- but currently unreachable.
- **`StatusBarEvent::RequestReauth`**: Defined and handled, but can never be emitted because `set_warning()` is never called, so `warnings` is always empty, so `WarningClicked` always early-returns.
- **`StatusBarMessage::WarningClicked`**: Same -- defined and handled but unreachable in practice.
- **`sync_status_bar` settings field**: Defined, toggled in settings UI, but never read by any consumer.
- **`format_number()`**: Defined but never executed (no sync progress data ever arrives).

---

## Summary

The status bar **scaffold is complete and faithfully implements the spec's architecture**: Component trait, types, priority state machine, subscription, view, theme tokens, layout constants, and app integration are all present and correct. The code quality is high -- the `BTreeMap` upgrade for deterministic cycling and the `build_status_row()` extraction are improvements over the spec.

However, **all three data pipelines (sync progress, warnings, confirmations) are unwired** -- the status bar receives no data and permanently shows idle state. The idle state itself diverges from spec (zero-height collapse vs. fixed-height container). The status bar incorrectly appears in pop-out windows. A settings toggle exists but is disconnected.

The remaining work is integration, not architecture: connecting the sync layer's `ProgressReporter` events, wiring action confirmations, and connecting the settings toggle.
