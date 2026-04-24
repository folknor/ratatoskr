# Command Palette: Spec vs. Code Discrepancies

Audit date: 2026-03-22

---

## Divergences

### PaletteStage carries no inline data

**Spec**: `PaletteStage::CommandSearch` and `OptionPick` carry query/results/selected_index inline.

**Code** (`ui/palette.rs`): `PaletteStage` is a bare enum with unit variants. All state is flat fields on `PaletteState`. Functionally equivalent.

### Escape in palette: stage 2 goes back instead of closing

**Spec** (section 2.3): `Close` always closes.

**Code**: In stage 2, `PaletteMessage::Close` calls `back_to_stage1()` instead of closing. Better UX.

### scroll_to_selected is a no-op

`scroll_to_selected()` returns `Task::none()`. Blocked by iced fork lacking `scroll_to()` API. Arrow keys change selection index but do not scroll the results scrollable.

---

## Not implemented

### Keybinding management UI (Slice 6f)
No settings panel for viewing/rebinding shortcuts. Lower priority - default bindings work out of the box. Deferred past V1.

---

## Dead code

None remaining. `PendingChord.started` field removed (2026-03-22).

---

## Resolved

### NavMsgNext / NavMsgPrev ✅ (2026-03-22)
Now dispatch to `ReadingPaneMessage::NextMessage` / `PrevMessage`. ReadingPane tracks `focused_message` index, expands the target message on navigation.

### EmailSelectAll ✅
Dispatches to `ThreadListMessage::SelectAll`.

### EmailSelectFromHere ✅ (2026-03-22)
Now dispatches to `ThreadListMessage::SelectFromHere`. Selects from current thread to end of list.

### PaletteState is a Component ✅
Implements `Component` trait with `PaletteEvent::ExecuteCommand` / `ExecuteParameterized` / `Dismissed` / `Error`.

### is_muted and is_pinned populated ✅
`command_dispatch.rs` reads from `Thread.is_muted` / `Thread.is_pinned`.

### Keybinding override persistence (Slice 6e) ✅
Overrides loaded at boot from `keybindings.json`, saved on mutations.

### UsageTracker persistence ✅ (2026-03-22)
Usage counts saved to `command_usage.json` after each command execution. Loaded at boot via `registry.usage.load_from_map()`.

### PendingChord.started removed ✅ (2026-03-22)
Dead field removed from `PendingChord` struct.

### Undo tokens (Slice 5) ✅ (2026-03-22)
`UndoToken` enum with 12 variants (Archive, Trash, MoveToFolder, ToggleRead/Star/Pin/Mute/Spam, AddLabel, RemoveLabel, Snooze). `UndoStack` bounded FIFO queue (capacity 20). `CommandId::Undo` registered with `Ctrl+Z`. `is_undoable` flag on `CommandDescriptor`. 13 email commands marked undoable. App holds `undo_stack: UndoStack`, Undo handler pops and logs compensation (actual provider API calls deferred until email actions are wired).
