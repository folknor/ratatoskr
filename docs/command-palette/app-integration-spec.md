# Command Palette: App Integration

The original Slice 6 spec was a ~1500-line implementation plan. Slices 6a, 6b, 6c, and 6e are shipped; 6d is partial; 6f is deferred. See `roadmap.md` for live status and `discrepancies.md` for outstanding gaps.

This file keeps only the design rationale that doesn't appear in code comments and that a future contributor would need to make changes safely.

## Why the dispatch is split in two

`dispatch_command(id) -> Option<Message>` handles direct (non-parameterized) commands. `dispatch_parameterized(id, args) -> Option<Message>` handles the parameterized ones once stage 2 has produced typed `CommandArgs`. Both live in `crates/app/src/command_dispatch.rs`.

Why two functions: parameterized commands return `None` from `dispatch_command` because they have nothing to dispatch *until* stage 2 completes. Returning `None` from the direct-dispatch path is what triggers the palette to enter `OptionPick` instead of executing immediately. Don't try to unify them - the `None` is load-bearing signal, not a missing case.

## Why the palette UI talks to the resolver via `Task::perform`

Resolver calls touch the DB through `with_conn_sync` (the `Arc<Mutex<Connection>>` pattern). The palette is a high-frequency UI path; even small mutex contention could cause keystroke jank. So `Confirm` returning a parameterized command schedules an async resolver call, and the result arrives via `OptionsLoaded`.

Each resolver call carries a generation counter so stale results are discarded if the user switches commands or types between calls. This matters more than it looks: `get_options` for "All Accounts" navigation can return hundreds of items across multiple accounts, and a slow-account stragglers landing after the user has moved on would otherwise overwrite the visible list.

## Why keyboard dispatch lives in a global subscription

The palette subscribes via `iced::event::listen_with`, which receives events before widgets process them. This is necessary for two reasons:

1. **Modifier chords like `Ctrl+K` must work even when a `text_input` has focus.** Letting iced's normal widget event flow handle them would mean Ctrl+K gets eaten by whatever input is focused.
2. **Two-key sequences (`g then i`) need a state machine that survives between key events.** That state (`pending_chord`) lives on `App`, and the timeout subscription is conditionally added when `pending_chord.is_some()`.

The flow inside `handle_key_pressed`: if the palette is open, route to the palette's own key handler (only Escape/arrows/Enter intercepted; everything else flows to the text input). Otherwise, if a widget already captured the event AND there's no command modifier, skip. Otherwise convert to a `cmdk::Chord`, then either resolve as second-of-sequence, single chord, or pending-first.

## Why `CommandArgs` is in `cmdk` rather than `app`

The natural place for `CommandArgs` would be `crates/app/` - it's used by the app's dispatch. It lives in `crates/cmdk/` instead because it's part of the parameterized command contract: the registry says "this command takes these parameters," the resolver provides options, the app builds typed `CommandArgs`, and dispatch consumes them. Putting `CommandArgs` in `cmdk` lets the type system enforce that the variants match the parameterized command IDs at the contract layer, not just inside the app.

Trade-off: `cmdk` ends up depending on `crates/types/` for `FolderId` and `TagId`. That's accepted - `types` is the lightweight shared-IDs crate (serde only) precisely so other crates can use typed IDs without pulling in heavy deps.

## Why `PaletteStage` is a flat enum with state on the parent

The original spec had `PaletteStage::CommandSearch { query, results, selected_index }` as a data-carrying enum. The implementation has a bare unit enum with the fields stored on `Palette` directly. This is a deliberate shape choice, not a design slip:

- Most of the state (query text, results vec, option items, selected index, generation counter) is shared between stages and would have to be duplicated or moved through `Option`s in a data-carrying variant.
- The flat fields let stage-2-specific data (`option_items`, `option_matches`) coexist with stage-1 data (`results`) without `match` arms in every accessor.
- Equivalence is preserved: `is_option_pick()` plus the fields acts as the same state machine.

Don't refactor this back to data-carrying without good reason - it'll grow boilerplate.

## Escape behavior in stage 2

The spec said Escape always closes. The implementation makes Escape in `OptionPick` go back to `CommandSearch` instead, only closing from `CommandSearch`. This is intentional UX: backing out of a wrong command into the search list is the common case; closing entirely is a less common intent that another Escape (or click-outside) handles.

## Cross-account undo wart

`dispatch_plan_with_undo` splits cross-account plans (one journal row per account) into one plan per account. Each split pushes its own undo-stack entry. An N-account bulk action therefore takes N `Ctrl+Z` presses to fully undo. Documented in code comments. Not currently planned for fix - when a real user complains, fold the splits into a single composite undo entry.

## What's still unbuilt

See `roadmap.md` and `discrepancies.md` for the live list. The big-ticket items as of 2026-05-15:

- `scroll_to_selected` - needs `scrollable::scroll_to()` on the iced fork.
- Slice 6d expansion (context menus, more toolbars using `command_button` / `command_icon_button`).
- Slice 6f keybinding management UI.
- `AppAskAi` - dispatch returns `None` until the feature lands.
