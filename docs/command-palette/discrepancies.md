# Command Palette: Spec vs. Code Discrepancies

Audit date: 2026-03-21

---

## Divergences

### PaletteState is not a Component

**Spec** (app-integration-spec.md section 2.1): Palette should implement the `Component` trait with `PaletteEvent::ExecuteCommand` / `PaletteEvent::Dismissed`.

**Code**: `PaletteState` is a raw state struct. All update logic is inline in `handlers/palette.rs`. No `PaletteEvent` enum exists. The palette does not implement `Component`. Other non-sidebar components (reading pane, thread list) also appear to use direct message passing, so this may reflect an architectural preference rather than an omission.

### PaletteStage carries no data

**Spec** (app-integration-spec.md): `PaletteStage::CommandSearch` and `OptionPick` carry query/results/selected_index inline.

**Code** (`ui/palette.rs:12-18`): `PaletteStage` is a bare enum with unit variants. All state is flat fields on `PaletteState`. Functionally equivalent.

### NavMsgNext, NavMsgPrev, EmailSelectAll, EmailSelectFromHere return None

**Spec**: All mapped to concrete `Message` variants (e.g., `ReadingPaneMessage::NextMessage`).

**Code** (`command_dispatch.rs:319,393-394`): `NavMsgNext`/`NavMsgPrev` return `None` (no `ReadingPaneMessage::NextMessage` variant). `EmailSelectAll`/`EmailSelectFromHere` return `None` (no `ThreadListMessage::SelectAll` variant). These require component-side additions.

### Escape in palette: stage 2 goes back instead of closing

**Spec** (app-integration-spec.md section 2.3): `Close` always closes.

**Code** (`handlers/palette.rs:28-37`): In stage 2, `PaletteMessage::Close` calls `back_to_stage1()` instead of closing. Better UX but diverges from spec.

### scroll_to_selected is a no-op

**Code** (`ui/palette.rs:380-382`): `scroll_to_selected()` returns `Task::none()` with a TODO comment about iced fork lacking `scroll_to()`. Arrow keys change selection index but do not scroll the results scrollable. Long result lists will scroll the selection out of view.

---

## Not implemented

### UsageTracker persistence (Slice 4)
`UsageTracker` counts in-memory only. Not persisted across sessions. Roadmap defers to Slice 6e.
- Code: `crates/command-palette/src/registry.rs` (UsageTracker struct)

### Undo tokens (Slice 5)
No `UndoToken` type, no undo stack, no `is_undoable` flag. Separate future slice.
- Spec: `docs/command-palette/app-integration-spec.md`

### Keybinding override persistence (Slice 6e)
`BindingTable` supports overrides in memory but they are not saved/loaded. Boot has no override loading.
- Code: `crates/command-palette/src/keybinding.rs:377` (BindingTable struct)

### Keybinding management UI (Slice 6f)
No settings panel for rebinding.
- Spec: `docs/command-palette/app-integration-spec.md`

### Thread state: is_muted and is_pinned
`is_muted` and `is_pinned` are hardcoded `None` in `selected_thread_state()`. The app-layer `Thread` struct does not expose these fields yet.
- Code: `crates/app/src/command_dispatch.rs:288-289`

---

## Dead code

### PendingChord.started field
Has `#[allow(dead_code)]`. Set on creation but never read. The timeout uses `iced::time::every` subscription, not elapsed-time checks.
- Code: `crates/app/src/main.rs:143`
