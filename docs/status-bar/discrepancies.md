# Status Bar: Spec vs. Implementation Discrepancies

Audit date: 2026-03-21
Updated: 2026-03-21 (post-implementation pass)

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

10. **View integration** -- Status bar rendered at bottom of main layout in both mail and calendar modes via `column![layout, status_bar]`. Also present in settings view. Not present in pop-out windows (correct per problem statement).

11. **update() implementation** -- CycleTick advances both cycle indices with wrapping_add, expires confirmations. WarningClicked emits RequestReauth for TokenExpiry, no-ops for ConnectionFailure.

12. **subscription()** -- Conditional tick: fires only when warnings > 1, sync_progress > 1, or confirmation is Some. Matches spec.

13. **View helper** -- `build_status_row()` extracts repeated row construction into a helper. Uses `icon::alert_triangle()`, `icon::refresh()`, `icon::check()` with `ICON_MD` size and `TEXT_SM` text. Matches spec icon choices.

14. **format_number()** -- Thousands separator helper matches spec.

15. **Idle state** -- Renders a fixed-height container with `STATUS_BAR_HEIGHT` and `ContainerClass::StatusBar` styling, maintaining consistent layout. Matches spec section 13.

16. **ResolvedContent::Warning has account_id** -- The `account_id` is embedded in the resolved content, matching the spec's approach. This avoids the race condition of re-deriving from `self.warnings` via cycle index between `view()` and `update()`.

17. **Sync progress pipeline** -- `SyncEvent` enum, `IcedProgressReporter` (implements `ProgressReporter` trait), and `create_sync_progress_channel()` factory are implemented. `Message::SyncProgress(SyncEvent)` variant wired in the app. `handle_sync_event()` routes Progress/Complete/Error events to the status bar methods.

18. **Warning pipeline** -- `SyncEvent::Error` sets `ConnectionFailure` warnings. `SyncEvent::Complete` clears warnings. Token expiry warnings can be set via `set_warning()` from auth error paths.

19. **Settings toggle** -- `sync_status_bar` from `SettingsState` is read in `status_bar_view()` helper. When false, the status bar is hidden (zero-height element).

20. **Generational tracking** -- `sync_generations` map with `begin_sync_generation()`, `is_sync_stale()`, and `prune_stale_sync()` methods. `SyncAccountProgress` carries a `generation` field. Stale entries from dead sync tasks can be detected and pruned.

---

## Divergences

### D1. `warnings` uses `BTreeMap` instead of `HashMap`

**Spec:** `warnings: HashMap<String, AccountWarning>`.
**Code:** `warnings: BTreeMap<String, AccountWarning>`.

The `BTreeMap` provides deterministic iteration order (sorted by account ID), which makes cycling through multiple warnings stable and predictable. This is an improvement over the spec's `HashMap` (which would produce arbitrary cycling order). Not a bug -- an intentional upgrade.

**Severity:** None (improvement).

### D2. Clickable `mouse_area` wraps all warnings, not just TokenExpiry

**Spec (section 14):** "ConnectionFailure warnings render without the mouse_area wrapper (no on_press, no cursor change)."
**Code:** The `mouse_area` wrapper with `on_press` and `Interaction::Pointer` is applied whenever `clickable` is true. The `clickable` flag is correctly set to true only for `TokenExpiry`. When the currently displayed warning is `ConnectionFailure`, `clickable` is false and no `mouse_area` is applied. This matches the spec.

**Severity:** None -- on closer inspection this matches the spec.

---

## Remaining Work

### R1. Confirmation dispatch points not wired

`show_confirmation()` is ready to be called from action handlers (archive, trash, label, star, etc.) but `Message::EmailAction` is currently a stub. Confirmations will be wired when email actions are implemented. The pipeline is structurally complete -- only the call sites are missing.

### R2. Token expiry warnings not wired to auth errors

Token expiry is detected during sync when an OAuth refresh fails. The auth error handling path does not yet exist (accounts UI is not implemented). `set_warning()` with `WarningKind::TokenExpiry` is ready to be called from that path.

### R3. `RequestReauth` shows a placeholder confirmation

The handler emits an `eprintln` log and shows a temporary confirmation ("not yet implemented") instead of opening a re-authentication flow. This is expected -- the accounts re-auth UI does not exist yet.

### R4. Sync progress subscription not connected to sync orchestrator

`IcedProgressReporter` and `create_sync_progress_channel()` are implemented, but the sync orchestrator does not yet use them. The receiver needs to be polled via an iced subscription that produces `Message::SyncProgress` events. This requires the sync orchestrator to accept an `IcedProgressReporter` instance, which is a cross-cutting change outside the status bar's scope.

### R5. `prune_stale_sync` not called automatically

The generational tracking methods (`begin_sync_generation`, `is_sync_stale`, `prune_stale_sync`) are available but not yet called from the sync orchestrator. A periodic prune or prune-on-generation-bump should be added when the sync pipeline is fully wired.

---

## Cross-Cutting Concern Status

### a. Generational load tracking

**Implemented.** Per-account `sync_generations` map with `begin_sync_generation()`, `is_sync_stale()`, and `prune_stale_sync()`. Each `SyncAccountProgress` entry carries a `generation` field stamped at insertion time. Not yet called from the sync orchestrator (see R5).

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

Previously all five inbound data methods were unreachable. Now:
- **`handle_sync_event`**: Routes `SyncEvent` to `report_sync_progress`, `report_sync_complete`, `set_warning`, `clear_warning`. All reachable once the sync orchestrator is connected.
- **`show_confirmation`**: Called from `handle_status_bar_event` for the placeholder reauth message. Will gain more call sites when email actions are wired.
- **`format_number()`**: Reachable through `resolve_sync_progress()` once sync data flows.
- **`sync_status_bar` settings field**: Now read by `status_bar_view()` in main.rs.

---

## Summary

The status bar scaffold and all integration plumbing are complete:
- Component architecture, types, state machine, view, theme tokens, layout constants all correct.
- `SyncEvent` enum, `IcedProgressReporter`, and `create_sync_progress_channel()` implement the sync progress pipeline.
- `Message::SyncProgress` variant and `handle_sync_event()` wire events from the reporter into the status bar.
- Warning pipeline routes `SyncEvent::Error` to `set_warning()` and `SyncEvent::Complete` to `clear_warning()`.
- Settings toggle (`sync_status_bar`) controls visibility.
- Idle state renders at fixed height (no layout shift).
- `ResolvedContent::Warning` embeds `account_id` to avoid race conditions.
- Generational tracking API ready for stale sync detection.
- Pop-out windows correctly do not include the status bar.

Remaining work is connecting the sync orchestrator to the `IcedProgressReporter` and wiring `show_confirmation()` calls into email action handlers -- both are outside the status bar's scope and depend on features not yet implemented.
