# Command Palette: Spec vs. Code Discrepancies

Audit date: 2026-05-15 (supersedes 2026-03-22).

The backend and most app integration is shipped. This is a fresh sweep of the remaining spec/code mismatches.

---

## Outstanding gaps

### Recency not folded into fuzzy ranking

`crates/cmdk/src/registry/core.rs::query_fuzzy` reads `recency_score` into each `CommandMatch` but sorts only by the combined `raw_fuzzy + context_boost + availability_bonus`. `query_empty` does use recency. Spec calls for recency in both paths. Folding `usage_count` (probably log-scaled) into the fuzzy score is the remaining work.

### `scroll_to_selected()` is a no-op

`crates/app/src/ui/palette.rs:620` returns `Task::none()`. Blocked on the iced fork not exposing `scrollable::scroll_to()`. Arrow keys change `selected_index` correctly but the scrollable doesn't follow when the highlight goes off-screen.

### Slice 6d — UI surface migration is partial

`command_button` / `command_icon_button` (`crates/app/src/ui/widgets/buttons.rs`) are used in the reading-pane toolbar (5 places, `crates/app/src/ui/reading_pane.rs`). Not yet adopted by the thread list, sidebar, or any context-menu surface. Context menus don't exist as a primitive yet.

### Slice 6f — keybinding management UI

No settings panel for view/search/rebind. Deferred past V1 by the original spec; default bindings work out of the box.

### `AppAskAi` returns `None` from `dispatch_command`

Genuine stub — the Ask AI feature isn't built. The `None` return is correct for now; rip the variant when Ask AI lands or wire it.

---

## Intentional divergences (kept)

### `PaletteStage` is a flat unit enum

Spec describes `CommandSearch { query, results, selected_index }` and `OptionPick { ... }` as data-carrying. Code (`crates/app/src/ui/palette.rs:65`) has bare unit variants and stores the fields on the parent `Palette` struct. Functionally equivalent; doesn't warrant a refactor.

### Escape in stage 2 returns to stage 1

Spec says `Close` always closes. Code (`crates/app/src/ui/palette.rs:311`) returns to stage 1 from `OptionPick` and only closes from `CommandSearch`. Better UX — kept.

### `CommandArgs::NavigateToLabel` split into `NavigateToFolder` + `NavigateToTag`

Spec showed a single `NavigateToLabel { label_id, account_id }`. Code has two variants (`crates/cmdk/src/args.rs`) using the typed-IDs convention from `crates/types/src/typed_ids.rs` (`FolderId` vs `TagId`). `CommandId::NavigateToLabel` is the single user-visible command; the resolver picks the variant.

### Command count: 70, not 55

Problem-statement.md was written when there were 55 commands. The current 70 add 7 calendar commands (`CalendarToggle`, view modes, today, create, pop-out, switch-to-mail/calendar), plus `Undo`, `SmartFolderSave`, `AppRebuildSearchIndex`, `EmailSelectAll`, `EmailSelectFromHere`, and `AppOpenPalette`. The architecture handles the growth without strain.

---

## Cross-account undo wart

`dispatch_plan_with_undo` (`crates/app/src/handlers/commands.rs`) splits cross-account plans into one plan per account; each split pushes its own undo-stack entry. An N-account bulk undo therefore takes N `Ctrl+Z` presses. Documented in code comments; not currently planned for fix.
