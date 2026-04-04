# Overlay Surfaces Glossary

Canonical terminology for overlay-like UI surfaces in Ratatoskr.

This glossary entry defines the semantic surface types used across the app.
Implementation plans and refactors should reference these terms rather than
inventing local names like "popup" or overloading primitive names.

## Canonical Types

Use these terms consistently:

- `Tooltip`
  - Small explanatory surface
  - Hover/focus triggered
  - Non-blocking
  - Does not take focus

- `Dropdown`
  - Anchored chooser opened from a control
  - Primarily used to select a value
  - Selecting an item usually closes it

- `ContextMenu`
  - Anchored action menu
  - Primarily used to act on an object or context
  - Dismisses on outside click, Escape, or selection

- `Popover`
  - Anchored richer panel
  - More substantial than a dropdown
  - Interactive but not globally blocking

- `Modal`
  - Blocking surface
  - Disables interaction behind it
  - Centered card or dialog stack

- `Sheet`
  - Large panel sliding over content from an edge
  - Full blocking surface while open
  - Distinct from a centered modal by presentation and layout

## Semantic Distinctions

The key distinction between `Dropdown` and `ContextMenu` is purpose, not input
method:

- `Dropdown` = anchored selection surface
- `ContextMenu` = anchored action surface

A context menu does not need to be opened by right-click. A trigger-opened
overflow action list is still a `ContextMenu` if it presents actions rather
than choices.

## Canonical Behavior Table

These are the fixed behavioral expectations for each semantic type. If an
implementation deviates, that is a bug or misclassification rather than a cue
to make the type system more configurable.

| Type | Positioning | Blocks Background | Dismiss | Focus |
|---|---|---|---|---|
| `Tooltip` | Anchored | No | Unhover, unfocus | None |
| `Dropdown` | Anchored | Click-dismiss | Outside click, Escape, selection | Menu items |
| `ContextMenu` | Anchored | Click-dismiss | Outside click, Escape, selection | Menu items |
| `Popover` | Anchored | Click-dismiss | Outside click, Escape | Content |
| `Modal` | Centered | Dimming + blocking | Escape, explicit button | Trapped in content |
| `Sheet` | Edge-slide | Full blocking | Explicit close only | Trapped in content |

## Primitive Layer vs Semantic Type

Semantic surface type and rendering primitive are separate concepts.

Current primitive layer terms:

- `AnchoredOverlay`
  - Lower-level anchored placement primitive
  - Used by higher-level semantic types like `Dropdown`, `ContextMenu`, and
    some `Popover` or tooltip-like surfaces

- `modal_overlay()`
  - Lower-level blocking stack primitive
  - Used by `Modal` and `Sheet`

- `iced::widget::tooltip`
  - Native tooltip primitive

Do not call a semantic surface by the name of its primitive unless they are the
same thing. For example:

- do not call a `ContextMenu` a "popover" just because it uses `AnchoredOverlay`
- do not call a `Modal` a generic "overlay" when its semantic contract is known
- avoid the word `popup` in new naming because it is too ambiguous

## Current Examples

Representative current examples in the codebase:

- `Tooltip`
  - Settings help surface
  - Calendar pop-out button tooltip
  - Emoji picker hover tooltips

- `Dropdown`
  - Sidebar scope selector
  - Generic app dropdown widget
  - Compose From-account selector
  - Calendar availability selector

- `ContextMenu`
  - Compose recipient token context menu
  - Pop-out message action context menu
  - Future email-link right-click menu

- `Popover`
  - Calendar event detail quick-glance card

- `Modal`
  - Add Account modal
  - Compose discard confirmation
  - Compose link insertion dialog
  - Calendar full event modal
  - Calendar event editor
  - Calendar delete confirmation
  - Command palette

- `Sheet`
  - Settings slide-in panel

## Naming Rule

When introducing or refactoring a surface:

1. Classify it as one of the six canonical semantic types.
2. Choose the primitive layer separately.
3. Name code after the semantic type where possible.
4. Avoid introducing new generic terms like `popup`.
