# Overlay Inventory

First-pass catalogue of all overlay-like UI surfaces currently implemented in the app crate.

Purpose:
- classify each surface by actual UI type
- record current naming in code, even when inaccurate
- identify shared behavior contracts
- prepare a rename/refactor pass before unifying implementations

## Canonical Types

Use these terms consistently going forward:

- `Tooltip`
  - small explanatory surface
  - hover/focus triggered
  - non-blocking
  - does not take focus

- `Dropdown`
  - anchored chooser opened from a control
  - selecting an item usually closes it
  - light interaction surface

- `Context Menu`
  - anchored action menu, usually right-click triggered
  - transient action list
  - dismisses on outside click / Escape

- `Popover`
  - anchored richer panel
  - more substantial than a dropdown
  - interactive but not globally blocking

- `Modal Dialog`
  - blocking surface
  - disables interaction behind it
  - centered card or dialog stack

- `Sheet`
  - large panel sliding over content from an edge
  - may be modal or non-modal, but that must be explicit

## Shared Contract Axes

Every surface should eventually declare:

- `surface_type`
- `anchor_strategy`
  - anchored
  - centered
  - edge-sheet
- `focus_policy`
  - takes focus
  - preserves prior focus
  - traps focus
- `backdrop_policy`
  - none
  - click-through
  - click-blocking
  - dimming + blocking
- `dismiss_policy`
  - outside click
  - Escape
  - explicit button only
  - route/state change
- `interaction_policy`
  - informational only
  - single-select
  - multi-control interactive
- `ownership`
  - app-global
  - screen-local
  - widget-local

## Inventory

### Tooltips

1. Settings help tooltip
Current code name:
`popover`

Actual type:
`Tooltip`

Implementation:
- [crates/app/src/ui/settings/row_widgets.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/settings/row_widgets.rs)
- built with [crates/app/src/ui/popover.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/popover.rs)

Notes:
- currently implemented with the generic popover widget
- likely needs a dedicated tooltip contract even if it reuses the same placement engine

2. Calendar pop-out button tooltip
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

3. Emoji picker hover tooltips
Implementation:
- [crates/app/src/ui/emoji_picker.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/emoji_picker.rs)

Notes:
- these use `iced::widget::tooltip` directly rather than the custom popover path
- already evidence of split implementation

### Dropdowns

1. Sidebar scope selector
Current code name:
`dropdown`

Actual type:
`Dropdown`

Implementation:
- [crates/app/src/ui/sidebar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/sidebar.rs)
- [crates/app/src/ui/widgets.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/widgets.rs)
- built on [crates/app/src/ui/popover.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/popover.rs)

2. Generic app dropdown widget
Implementation:
- [crates/app/src/ui/widgets.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/widgets.rs)

3. Settings-style select dropdown
Implementation:
- [crates/app/src/ui/widgets.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/widgets.rs)

4. Compose From-account selector
Implementation:
- [crates/app/src/pop_out/compose.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/compose.rs)

5. Calendar availability dropdown and similar editor selects
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

Notes:
- dropdowns are partly standardized through `widgets.rs`
- they still rely on the generic popover substrate

### Context Menus

1. Compose recipient token context menu
Current code name:
`context_menu`

Actual type:
`Context Menu`

Implementation:
- [crates/app/src/pop_out/compose.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/compose.rs)
- token event source in [crates/app/src/ui/token_input.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/token_input.rs)

Current issue:
- rendered inline into the compose column instead of as a proper anchored overlay
- naming is correct, behavior is not

2. Right-click “Search here” sidebar actions
Current code name:
not a menu yet, only direct right-click action

Actual type:
should probably become `Context Menu` eventually if expanded

Implementation:
- [crates/app/src/ui/sidebar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/sidebar.rs)

Notes:
- currently a right-click shortcut, not a true context menu

3. Future email-link right-click menu
Status:
not yet implemented

Notes:
- should be classified as a `Context Menu`, not a popup or dropdown

### Popovers

1. Shared generic popover engine
Implementation:
- [crates/app/src/ui/popover.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/popover.rs)

Current role:
- shared anchor-positioned overlay primitive
- used by dropdowns and some help surfaces

Concern:
- currently named `popover`, but in practice serves as the lower-level anchored overlay primitive for multiple higher-level surface types

2. Pop-out message overflow menu
Current code name:
`overflow_menu`

Actual type:
closer to `Popover` or `Context Menu` depending on final contract

Implementation:
- [crates/app/src/pop_out/message_view.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/message_view.rs)

Notes:
- anchored action list opened from a trigger
- semantically it behaves more like a context/action menu than a dropdown

3. Calendar event detail quick-glance card
Current code name:
`EventDetail` popover

Actual type:
`Popover`

Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

Notes:
- separate stack implementation, not using `ui/popover.rs`
- this is an important duplication point

### Modal Dialogs

1. Add Account modal
Implementation:
- [crates/app/src/main.rs](/home/folk/Programs/ratatoskr/crates/app/src/main.rs)

Variants:
- first-launch centered modal
- modal over existing main layout with blocker/backdrop

Current issue:
- TODO notes indicate background interaction contracts are inconsistent

2. Compose discard confirmation
Current code name:
`discard_confirmation`

Actual type:
intended `Modal Dialog`, currently rendered inline

Implementation:
- [crates/app/src/pop_out/compose.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/compose.rs)

Current issue:
- not actually modal

3. Compose link insertion dialog
Current code name:
`link_dialog`

Actual type:
intended `Modal Dialog`, currently rendered inline

Implementation:
- [crates/app/src/pop_out/compose.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/compose.rs)

Current issue:
- not actually modal

4. Calendar full event modal
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

5. Calendar event editor modal
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

6. Calendar delete confirmation dialog
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

7. Command palette
Current code name:
`Palette`

Actual type:
`Modal Dialog` or specialized modal command surface

Implementation:
- [crates/app/src/main.rs](/home/folk/Programs/ratatoskr/crates/app/src/main.rs)
- [crates/app/src/ui/palette.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/palette.rs)

Notes:
- modal-like backdrop and focus ownership
- probably should not be grouped with generic dialogs in code, but it does share the same blocking/dismiss contracts

### Sheets

1. Settings slide-in panel
Current code name:
`overlay`

Actual type:
`Sheet`

Implementation:
- [crates/app/src/ui/settings/types.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/settings/types.rs)
- [crates/app/src/ui/settings/update.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/settings/update.rs)
- [crates/app/src/ui/settings/tabs.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/settings/tabs.rs)

Examples inside the sheet system:
- account editor
- signature editor
- contact editor
- group editor
- import contacts
- create filter

Current issue:
- code uses `overlay`, but semantically this is a right-edge sheet system
- backdrop dismiss semantics are currently wrong per `TODO.md`

## Inline-Rendered Surfaces That Should Be Reclassified

These are especially important because their current name and behavior diverge:

1. Compose token context menu
- named as context menu
- rendered inline
- should become anchored context menu

2. Compose discard confirmation
- named/treated as dialog
- rendered inline
- should become modal dialog

3. Compose link insertion dialog
- named/treated as dialog
- rendered inline
- should become modal dialog

4. Compose autocomplete dropdown
- semantically a dropdown
- rendered inline in the header flow
- may still be acceptable visually, but contractually it behaves more like an anchored overlay

## Current Implementation Buckets

Today the codebase appears to have at least these separate implementation paths:

1. Native `iced::widget::tooltip`
2. Custom anchored overlay primitive in [ui/popover.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/popover.rs)
3. Manual `stack![]` + backdrop modal composition in `main.rs` and `calendar.rs`
4. Manual inline pseudo-overlays in `pop_out/compose.rs`
5. Manual slide-in sheet composition in `ui/settings/tabs.rs`

This is the fragmentation we need to normalize.

## Proposed Rename Direction

Before refactoring behavior, the code should move toward these naming conventions:

- `popover.rs`
  - consider renaming to something like `anchored_surface.rs` or `anchored_overlay.rs`
  - reserve `Popover` for the higher-level semantic type, not the generic placement primitive

- Settings `overlay`
  - rename toward `sheet`

- Compose inline “dialog” helpers
  - keep dialog names, but only after they become true modal surfaces

- Overflow “menu”
  - classify either as `Context Menu` or `Popover Action Menu`
  - avoid ambiguous `popup`

## Immediate Next Step

After review of this catalogue:

1. agree canonical names for the six surface types
2. map every current surface to one of those types
3. decide which generic primitives are actually needed
4. rename the obviously wrong code-level terms
5. only then unify behavior contracts
