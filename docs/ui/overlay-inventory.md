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
  - anchored action menu
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

## Canonical Behavior Table

These are the fixed behavioral expectations for each canonical surface type.
If an implementation deviates, that is a bug or misclassification, not a cue
to make the type system more configurable.

Key semantic distinction:
- `Dropdown` = anchored selection surface
- `ContextMenu` = anchored action surface

The difference is purpose, not input method. A context menu does not need to be
opened by right-click; a trigger-opened overflow action list is still a
`ContextMenu` if it presents actions rather than choices.

| Type | Positioning | Blocks Background | Dismiss | Focus |
|---|---|---|---|---|
| `Tooltip` | Anchored | No | Unhover, unfocus | None |
| `Dropdown` | Anchored | Click-dismiss | Outside click, Escape, selection | Menu items |
| `ContextMenu` | Anchored | Click-dismiss | Outside click, Escape, selection | Menu items |
| `Popover` | Anchored | Click-dismiss | Outside click, Escape | Content |
| `Modal` | Centered | Dimming + blocking | Escape, explicit button | Trapped in content |
| `Sheet` | Edge-slide | Dimming + blocking | Escape, explicit close | Trapped in content |

## Inventory

### Tooltips

1. Settings help tooltip
Current code name:
`popover`

Actual type:
`Tooltip`

Implementation:
- [crates/app/src/ui/settings/row_widgets.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/settings/row_widgets.rs)
- built with [crates/app/src/ui/anchored_overlay.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/anchored_overlay.rs)

Notes:
- currently implemented with the generic anchored overlay primitive
- likely needs a dedicated tooltip contract even if it reuses the same placement engine
- lifecycle is not just hover-only; settings also tracks help visibility/pinning state in
  [crates/app/src/ui/settings/types.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/settings/types.rs)
  so this is currently the most stateful tooltip-family surface in the app
- intended target is still `Tooltip`, not `Popover`
- pinned/sticky help behavior should be removed during cleanup rather than preserved as a separate hybrid surface type

2. Calendar pop-out button tooltip
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

Current code name:
`pop_out_btn`

Actual type:
`Tooltip`

Notes:
- implemented directly with `iced::widget::tooltip`
- anchored to the calendar sidebar’s bottom `Pop Out` button
- tooltip text is `Open calendar in a separate window`

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
- built on [crates/app/src/ui/anchored_overlay.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/anchored_overlay.rs)

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
- they still rely on the generic anchored overlay substrate

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

1. Shared anchored overlay engine
Implementation:
- [crates/app/src/ui/anchored_overlay.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/anchored_overlay.rs)

Current role:
- shared anchor-positioned overlay primitive
- used by dropdowns and some help surfaces

Concern:
- now correctly named as the lower-level anchored overlay primitive for multiple higher-level surface types

2. Pop-out message overflow menu
Current code name:
`overflow_menu`

Actual type:
`ContextMenu`

Implementation:
- [crates/app/src/pop_out/message_view.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/message_view.rs)

Notes:
- anchored action list opened from a trigger
- semantically it is a `ContextMenu`, because it presents actions rather than choices
- trigger-opened is still compatible with `ContextMenu`; right-click is not required by the taxonomy

3. Calendar event detail quick-glance card
Current code name:
`EventDetail` popover

Actual type:
`Popover`

Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

Notes:
- separate stack implementation, not using `ui/anchored_overlay.rs`
- this is an important duplication point
- now represented in calendar state as `CalendarPopover::EventDetail`

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
`Modal Dialog`

Implementation:
- [crates/app/src/pop_out/compose.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/compose.rs)

Notes:
- now rendered as a true blocking modal in compose

3. Compose link insertion dialog
Current code name:
`link_dialog`

Actual type:
`Modal Dialog`

Implementation:
- [crates/app/src/pop_out/compose.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/compose.rs)

Notes:
- now rendered as a true blocking modal in compose

4. Calendar full event modal
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

Notes:
- now represented in calendar state as `CalendarModal::EventFull`

5. Calendar event editor modal
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

Notes:
- now represented in calendar state as `CalendarModal::EventEditor`

6. Calendar delete confirmation dialog
Implementation:
- [crates/app/src/ui/calendar.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/calendar.rs)

Notes:
- now represented in calendar state as `CalendarModal::ConfirmDelete`

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
`active_sheet`

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
2. Custom anchored overlay primitive in [ui/anchored_overlay.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/anchored_overlay.rs)
3. Manual `stack![]` + backdrop modal composition in `main.rs` and `calendar.rs`
4. Manual inline pseudo-overlays in `pop_out/compose.rs`
5. Manual slide-in sheet composition in `ui/settings/tabs.rs`

This is the fragmentation we need to normalize.

## Proposed Rename Direction

Before refactoring behavior, the code should move toward these naming conventions:

- `anchored_overlay.rs`
  - keep `AnchoredOverlay` as the lower-level placement primitive
  - reserve `Popover` for the higher-level semantic type, not the generic placement primitive

- Settings sheet state/messages
  - keep `sheet` terminology

- Compose inline “dialog” helpers
  - keep dialog names, but only after they become true modal surfaces

- Overflow “menu”
  - classify as `ContextMenu`
  - avoid ambiguous `popup`

## Phase 1: Naming Cleanup

Phase 1 naming cleanup was completed in commit `18ea25e0`.

It established:
- `AnchoredOverlay` as the primitive anchored-surface layer in [crates/app/src/ui/anchored_overlay.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/anchored_overlay.rs)
- `SettingsSheetPage`, `active_sheet`, and `sheet_anim` as the settings sheet terminology in [crates/app/src/ui/settings/types.rs](/home/folk/Programs/ratatoskr/crates/app/src/ui/settings/types.rs)
- the former mixed `CalendarOverlay` state has since been split into `CalendarPopover` and `CalendarModal`

## Phase 2: Behavioral Fixes

These are semantically named surfaces whose implementation does not yet match
their canonical type behavior.

| Surface | Location | Semantic Type | Current Problem | Notes |
|---|---|---|---|---|
| `overflow_menu` | [crates/app/src/pop_out/message_view.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/message_view.rs) | `ContextMenu` | action surface built on anchored overlay primitive | It presents actions, not choices, so it is a context menu even though it is trigger-opened rather than right-click-opened. |

## Immediate Next Step

After review of this catalogue:

1. Continue Phase 2 with the remaining misimplemented surfaces
   - most obviously [crates/app/src/pop_out/message_view.rs](/home/folk/Programs/ratatoskr/crates/app/src/pop_out/message_view.rs) `overflow_menu`

2. Resolve remaining feature-level naming drift for context menus and dropdowns
   - make feature state/messages read semantically, not just the shared primitive layer
