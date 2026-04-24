# Overlay Standardization Plan

Architectural plan for unifying overlay code paths across the app crate.
The [overlay surfaces glossary](../glossary/overlay-surfaces.md) defines the
canonical semantic surface types and naming rules. This document defines the
target architecture, decides which primitives survive, and lays out the
migration.

## Current State

Five separate implementation paths exist today:

1. **`iced::widget::tooltip`** - used by calendar pop-out button, emoji picker
2. **`AnchoredOverlay`** (`ui/anchored_overlay.rs`) - custom iced overlay widget
   for dropdowns, context menus, compose autocomplete
3. **Hand-rolled `stack![]` + blocker** - modal composition in `main.rs`
   (add-account, palette), `calendar.rs` (full event, editor, delete confirm),
   `compose.rs` (discard, link dialog)
4. **Hand-rolled sheet** - settings slide-in panel in `settings/tabs.rs`
   (animated offset + blocker `mouse_area`)
5. **Calendar `popover_stack`** - one-off stack-based popover for event detail
   quick-glance, separate from `AnchoredOverlay`

Every `stack![]` + blocker site re-implements the same pattern: base,
blocking `mouse_area`, positioned content. The differences are: whether the
blocker is visually dimmed (modals) or invisible (sheet), content positioning
(centered vs edge-slide), and dismiss behavior (Escape + button vs button
only).

## Target Architecture

### Three primitives

1. **`iced::widget::tooltip`** - keep as-is for simple hover tooltips
2. **`AnchoredOverlay`** - keep as-is for anchored surfaces (dropdowns, context
   menus, popovers). Already well-factored with `AnchorPosition` and
   `anchor_point` support.
3. **`modal_overlay()`** - new helper that replaces all hand-rolled
   `stack![]` + blocker composition for blocking surfaces (modals and sheets).

### Why not more primitives

- Tooltips are native iced and work fine. No reason to wrap them.
- Anchored surfaces all go through `AnchoredOverlay` already. The calendar
  event detail popover (`popover_stack`) is the only anchored surface that
  doesn't - it should migrate to `AnchoredOverlay` (see deferred work).
- Every blocking surface (modal, sheet, palette) uses the same stack+blocker
  pattern. The variation between them is small enough to parameterize.

### Why not fewer

- Tooltips and anchored overlays use fundamentally different iced mechanisms
  (`iced::widget::tooltip` vs `Widget::overlay()`). Merging them would mean
  reimplementing tooltip from scratch for no benefit.
- Modals use `stack![]` (layout-level layering), anchored overlays use
  `Widget::overlay()` (overlay-level layering). These are different iced
  concepts and should stay separate.

## `modal_overlay()` Design

A free function in `ui/modal_overlay.rs` that takes a base element and a
surface specification, and returns the composed `stack![]` element.

```rust
pub enum ModalSurface {
    /// Centered card with dimmed backdrop. Used by confirmation dialogs,
    /// add-account, palette, calendar modals.
    Modal,
    /// Opaque edge-slide panel with invisible event blocker underneath.
    /// Used by settings sheet. The `offset` is horizontal translation in
    /// pixels applied as left padding: 0.0 means the sheet is flush with
    /// the right edge (fully visible), larger values push it offscreen to
    /// the right.
    Sheet { offset: f32 },
}

pub fn modal_overlay<'a, Message: Clone + 'a>(
    base: Element<'a, Message>,
    content: Element<'a, Message>,
    surface: ModalSurface,
) -> Element<'a, Message>
```

The surface type determines all behavior:

- **`Modal`** - inserts a `mouse_area` blocker styled with `ModalBackdrop`
  (dimmed, semi-transparent). The blocker consumes all clicks but does not
  dismiss. Content is centered. `modal_overlay()` is a layering and
  event-blocking primitive, not a complete modal-layout system - callers
  that need custom positioning (e.g., palette top offset) apply it to the
  content element before passing it in.
- **`Sheet`** - inserts an unstyled `mouse_area` blocker that consumes all
  clicks. Content is positioned with left padding based on `offset`. The
  sheet is opaque and covers the full area.

Neither surface type dismisses on blocker click. Modals dismiss via Escape
or an explicit button. Sheets dismiss via an explicit close button only.

**What `modal_overlay()` does NOT own:**
- Dismiss wiring - the caller wires Escape handling in `update()` and
  provides close buttons in the content. `modal_overlay()` only handles
  layout and event blocking.
- Animation - the caller computes the offset for `Sheet` (e.g., from
  `sheet_anim`) and passes it in. `modal_overlay()` applies it.
- Focus trapping - iced does not natively support focus trapping. This is
  a known gap between the behavioral contract (the glossary says Modal and
  Sheet trap focus) and what iced can enforce.

### What `modal_overlay()` replaces

- `calendar::modal_stack()`
- `compose::compose_modal_stack()`
- `main.rs::view_with_add_account_modal()` inline stack
- `main.rs` palette stack composition
- `settings/tabs.rs` sheet stack composition

### What `modal_overlay()` does NOT replace

- `AnchoredOverlay` - different iced mechanism entirely.
- `calendar::popover_stack()` - despite the name, this is an anchored
  surface, not a modal. It does not dim or block background interaction.
  See deferred work section.

### `view_first_launch_modal()`

The first-launch add-account view (`main.rs:2447`) has no base element -
it IS the entire screen (a centered card on a styled background). Because
`modal_overlay()` takes `base: Element`, this case doesn't fit the API.
It stays as-is: a centered container with no stack composition needed.

## Contract Rules Per Surface Type

| Canonical Type | Primitive | Blocker | Dismiss | Positioning |
|---|---|---|---|---|
| Tooltip | `iced::widget::tooltip` | None | Unhover | Anchored (native) |
| Dropdown | `AnchoredOverlay` | Click-dismiss (via `on_dismiss`) | Outside click, Escape, selection | `AnchorPosition::Below` / `BelowRight` |
| ContextMenu | `AnchoredOverlay` | Click-dismiss (via `on_dismiss`) | Outside click, Escape, selection | `AnchorPosition::Below` / `BelowRight` / `anchor_point` |
| Popover | `AnchoredOverlay` | Click-dismiss (via `on_dismiss`) | Outside click, Escape | `AnchorPosition::Below` / `BelowRight` |
| Modal | `modal_overlay()` | Dimmed, non-dismissing | Escape, explicit button | Centered |
| Sheet | `modal_overlay()` | Invisible, non-dismissing | Explicit close only | `Sheet { offset }` |

## Migration - Completed

All five migration phases have been implemented. The `modal_overlay()`
primitive in `crates/app/src/ui/modal_overlay.rs` now handles all blocking
overlay composition. Hand-rolled `stack![] + mouse_area` blocker patterns
have been removed from:

- `calendar.rs` - `modal_stack()` deleted, replaced with `modal_overlay()`
  using `CalendarMessage::Noop` as blocker event sink. **Bug fix:** calendar
  modals no longer dismiss on backdrop click.
- `compose.rs` - `compose_modal_stack()` deleted, replaced with
  `modal_overlay()`.
- `main.rs` - add-account modal and palette inline stacks replaced with
  `modal_overlay()`. `PaletteBackdrop` unified into `ModalBackdrop`.
  **Product behavior change:** palette no longer dismisses on backdrop click.
- `settings/tabs.rs` - sheet inline stack replaced with
  `modal_overlay(ModalSurface::Sheet { offset })`.

**`view_first_launch_modal()`** remains as-is - it has no base element to
overlay on (it IS the entire screen).

**Note on `blocker_msg` parameter:** iced's `mouse_area` only captures
click events when `on_press` is set. `modal_overlay()` requires a
`blocker_msg: Message` parameter for this reason. The message should be a
no-op in the caller's update loop.

### Remaining hand-rolled `stack![]` patterns (verified acceptable)

- `calendar.rs:popover_stack()` - deferred anchored surface (see below)
- `calendar_time_grid.rs` - layout stacking, not overlay
- `widgets.rs` - tooltip stack
- `main.rs` - chord indicator (non-blocking, no mouse_area)

## Deferred Work

### Calendar event detail popover

`calendar::popover_stack()` is the only anchored surface not using
`AnchoredOverlay`. It currently right-aligns the event detail card in the
calendar view using a hand-rolled stack.

The target behavior is anchored near the clicked event, using
`AnchoredOverlay` with click coordinates. This requires capturing the
click position in `CalendarPopover::EventDetail` (not currently stored)
and wiring it through to `anchor_point`. The popover and modal states are
mutually exclusive - only one surface is open at a time - so this is not
a stacking concern.

Deferred because it's independent of the `modal_overlay()` extraction and
can be done at any time.

### Settings help tooltip

The settings help surface currently uses `AnchoredOverlay`. The legacy
pinned/sticky help behavior has already been removed. This should
eventually migrate to a Ratatoskr Tooltip primitive (whether that ends
up being a thin wrapper around `iced::widget::tooltip` or a custom
implementation is a separate decision). Not required for the
standardization effort.

### Focus trapping

The overlay surfaces glossary specifies that Modal and Sheet
surfaces trap focus within their content. iced does not natively support
focus trapping. This is a known gap. If iced adds focus trapping support
in the future, `modal_overlay()` would be the single place to wire it in.

### Escape key audit

Each Modal surface should dismiss on Escape. Sheets do not dismiss on
Escape. The actual keyboard routing lives in `handlers/keyboard.rs` and
each component's update loop. This plan standardizes the layout/blocking
primitive but does not audit Escape handling. After `modal_overlay()`
lands, a mechanical verification pass should confirm: every Modal
dismisses on Escape, no Sheet dismisses on Escape, and the calendar
backdrop-click dismiss bug (see Phase 2) is fixed.
