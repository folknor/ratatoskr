# Focus: Problem Statement

## Overview

Focus is the substrate every keyboard-driven feature in the app sits on. The command palette, region-scoped shortcuts, search-bar interaction, Tab traversal, and any future screen-reader story all depend on a coherent answer to two questions at every moment: which region of the UI is the user "in", and which specific widget will receive their next keystroke.

There are two layers here. Region focus is an app-level concept - one of `Sidebar`, `ThreadList`, `ReadingPane`, `Composer`, `SearchBar`, plus any overlays. Widget focus is iced's concept - which `Focusable` widget owns the text cursor. They are not the same thing, they evolve on different timescales, and they have to be designed together. Today neither is implemented to a degree that the user can rely on.

Command palette work is downstream of this. Widget-definition work upstream of this. The point of this document is to describe the shape of the gap, what good looks like, and what has to land before any of it can be built.

## Current State

### Region focus is phantom infrastructure

`FocusedRegion` enum exists in `crates/cmdk/src/context.rs` with five variants (`ThreadList`, `ReadingPane`, `Composer`, `SearchBar`, `Sidebar`). `App.focused_region: Option<FocusedRegion>` exists in `crates/app/src/app.rs`. `CommandContext` reads it for command scoring (`scoring.rs` boosts e.g. compose commands when the composer is focused).

The field is initialized to `None` and never written to anywhere in the codebase. The scoring branches that depend on it can fire only as dead code. A user who clicks into the reading pane, the sidebar, or the composer leaves the app's idea of "where I am" untouched.

### Iced's focus model is text-input-only

The fork at `https://github.com/folknor/iced` exposes a small set of focus operations via `iced::widget::operation`: `focus(id)`, `unfocus()`, `focus_next()`, `focus_previous()`, `count()`, `find_focused()`, `is_focused(id)`. These walk the widget tree and act on nodes that implement the `Focusable` trait.

The only widgets that implement `Focusable` are `text_input` and `text_editor`. Buttons, mouse_areas, list rows, containers, the composer body, navigation items - none of them participate. Click-to-focus is hard-coded inside `text_input::update`; there is no generic mechanism. There are no focus events: a widget that gains or loses focus does not emit a message, so the app cannot react to focus changes through the normal `update()` flow. To know whether something is focused the app must run an operation that reads the state.

Tab/Shift-Tab is not wired anywhere in iced itself. The Tab key arrives as a regular key event; turning it into traversal is something each app has to write.

### There are no focus rings

The app draws no visible indicator on focusable widgets. A keyboard-only user cannot tell which widget would receive their input. This is true even for text inputs that do have iced-level focus state - the focused state has no associated styling in our theme. Discoverability of the keyboard model is effectively zero.

### The palette does not preserve focus

When the palette opens, it programmatically focuses its own text input. When it closes, the previously-focused widget is not restored. The user has to click somewhere to resume keyboard interaction with the underlying view. The same problem applies to any future overlay (settings sheet, compose modal, pop-out windows when re-attaching).

## The Two Layers

### Region focus (app-level)

One of a fixed set of named regions is "active". The active region:

- Drives command scoring (already partially wired in `cmdk::registry::scoring::focused_region_boost`).
- Should drive region-scoped keybindings: `j`/`k` for thread-list movement, composer-only chords, navigation chords in the sidebar.
- Should drive a visible affordance - a border, accent, or other indicator that tells the user where their keystrokes are headed at the region granularity.
- Should be remembered across overlay transitions so that focus can be restored on close.

The set of regions is short and stable. Region focus changes are infrequent (clicks, deliberate key presses) and the app should always have an answer to "which region is focused?" once boot is complete.

### Widget focus (iced-level)

One specific widget owns the text cursor. The active widget:

- Receives typed characters, Enter, Backspace, arrow keys for in-widget movement.
- Has a visible focus ring (today: missing).
- Is what `focus_next` / `focus_previous` advances or rewinds.

Widget focus is what iced already understands, but only for text inputs. To make widget focus useful for the rest of the UI, the inventory of widgets that participate in focus has to be expanded - and that means either every interactive widget must implement `Focusable`, or we need our own focusable-widget concept layered on top.

These two layers are related but distinct. The region tells the app where the user is; the widget tells iced where the keystrokes go. A region can be focused without any widget inside it being focused (e.g. the sidebar is active but no item is "highlighted" for keyboard movement). A widget can be focused without changing the region (Tab moves between two text inputs in the composer; the region stays `Composer`).

## What Focus Should Do

### Track which region is active

Clicking inside a region marks that region as focused. Programmatic focus changes (palette opens, overlay opens, app launches) update the region too. The previous region is remembered when an overlay takes over, so that closing the overlay restores it.

### Route key events

The cmdk `BindingTable` currently fires globally - any chord that matches a registered binding triggers, regardless of where the user is. Some chords are genuinely global (open palette, undo). Others should belong to a region: `j`/`k` for thread-list movement, composer formatting chords, sidebar navigation. The binding table needs a region scope, and the keyboard handler needs to consult the focused region before dispatching.

### Visible focus rings on every focusable widget

This is the biggest gap with iced upstream and the hardest to retrofit. Every interactive widget the user can act on with the keyboard should have a discoverable focused state - a ring, an outline, a background shift, whichever the design picks. The user pressing Tab should see exactly which widget they are now on.

The widgets that need to participate include, at minimum: buttons (every variant), nav rows in the sidebar, thread list items, message list items inside a thread, dropdown triggers, dropdown menu items, list-picker rows in the palette, settings rows, the composer body, file/attachment chips, action toolbar buttons, context menu items.

Most of these are not even widgets yet - they are inlined view code that builds the right elements at the call site. The focus-ring requirement is therefore upstream-blocked by the widget-definition plan: there is no "the button widget" to add a focus ring to.

### Tab/Shift-Tab traversal

Tab moves to the next focusable widget; Shift-Tab to the previous. The shape of "next" is an open question - traverse only within the focused region, or cross region boundaries, or both with a different chord? In any case, the iced operations `focus_next` / `focus_previous` only know about `Focusable` widgets in the tree; getting non-text-input widgets into that tree is a prerequisite.

### Focus restoration on overlay close

The palette is the immediate motivator, but every overlay has the same problem. When the user opens the palette from the reading pane and dismisses it without executing a command, the reading pane should regain focus (region and, if applicable, widget). Same for settings, compose modal, any pop-out window returning to the main app.

This requires saving (region, widget id) on overlay open and restoring on close. Iced provides no scaffolding for this; we have to build it.

## Dependencies

### Widget inventory must exist first

The "focus rings on every focusable widget" requirement assumes there is a set of widgets to attach rings to. Today many interactive UI elements are built inline at call sites rather than as named widgets:

- Buttons exist as theme styles applied to iced's `button` rather than as widget functions with consistent behaviour.
- List rows in the sidebar, thread list, and message list are built ad-hoc.
- Dropdown triggers, dropdown menus, context menus, and tooltip surfaces are partially-formed.
- The composer toolbar, attachment chips, and reading-pane action toolbar exist but not as focusable units.

Until the widget inventory is named, defined, and built as proper widgets (and centralized per `UI.md`'s "all widgets belong in widgets.rs"), focus rings cannot be attached to them. This work is therefore downstream of widget-definition plans for buttons, list items, nav rows, dropdowns, context menus, and overlay surfaces.

### Other plans this gates on

- Per the overlay-surfaces glossary (`reference/glossary/overlay-surfaces.md`), tooltips, dropdowns, context menus, popovers, modals, and sheets are not all built or unified. Focus interaction with overlays needs that surface model settled.
- The keybindings settings UI (command palette slice 6f) is downstream of regional binding tables, which is downstream of this.
- Pop-out windows and their focus/restoration semantics (`docs/pop-out-windows/`) interact with the cross-window focus story.

### Plans this gates

- Command palette UX (`docs/command-palette/`) cannot finish: region-aware scoring is half-built, focus restoration on close is unbuilt, traversal in the result list depends on the widget-focus story.
- Keyboard-driven workflows generally (j/k thread navigation, sidebar arrow keys, in-thread message movement) cannot get strict region semantics until the focused region is actually tracked.

## Ecosystem Patterns

We have not surveyed how comparable iced apps handle region and widget focus. Before committing to a design, the following should be examined:

- **Halloy** (`https://github.com/squidowl/halloy`) - same iced fork, similar panel-based layout (server list, channel list, message pane, input). Probably the most directly relevant prior art. Should be the first thing we read.
- **libcosmic** - already in the research checkout. Shows the `focus_next()` operation pattern via Tab. Worth reading for traversal mechanics but it is a desktop-environment shell with different ergonomic constraints.
- **Other iced apps** with serious keyboard models - check the iced ecosystem list and `awesome-iced` for examples.

Outside iced, the mature focus models worth referencing for the region/widget split and focus-ring conventions:

- **Web platform** - `:focus`, `:focus-visible`, `tabindex`, `focus()`, `blur()`, focus events, `activeElement`. The `:focus-visible` heuristic (focus ring only after keyboard navigation, not after mouse click) is particularly relevant.
- **GTK** - explicit focus chain, `can-focus` property, focus rings on every interactive widget by default.
- **AppKit** - first responder chain, key view loop, focus ring style.

A short survey of these (one paragraph each, what they do, what we should steal) should precede the design doc.

## Scope and Constraints

### In scope

- Region focus tracking, including click-to-focus and programmatic transitions.
- Region-scoped key event routing in cmdk's binding table.
- Focus rings on every focusable widget, with a consistent style.
- Tab/Shift-Tab traversal.
- Focus restoration on overlay close.
- Documenting the focused-region invariant: when is it allowed to be `None`, and when must it have a value.

### Out of scope (for the first design)

- Accessibility tree integration (screen readers, ARIA-equivalent semantics) - depends on iced upstream support that does not exist today.
- Global hotkeys outside the app window - separate concern, OS-level.
- IME composition state and how it interacts with focus changes.

## Open Questions

- Does Tab traverse across regions, only within the focused region, or both with different chords (e.g. `Tab` within, `Ctrl+Tab` across)?
- Does the palette occupy a region slot of its own, or does it suspend the underlying region's focus and restore it on close?
- Where does focus go on app launch - the thread list, the sidebar, nowhere?
- How do we model the state "focused region but no widget"? Is that always valid, or must every region have a default focusable child?
- Should focus rings always be visible when a widget is focused, or only after the first keyboard interaction in a given activation (the web `:focus-visible` pattern)? The web pattern is more polished but harder to specify; the always-visible pattern is simpler and more discoverable.
- Multiple windows (pop-out compose, pop-out calendar): does each window have its own focused region, or is there a single global focused region across all windows? What happens when a pop-out window loses OS-level focus to another app?
- Region-scoped keybindings vs global ones: where is the line drawn, and who decides? Is the region scope part of the descriptor in cmdk, or a separate routing layer?
- How does focus interact with disabled widgets - skipped during Tab traversal, or focusable but unactivatable?
