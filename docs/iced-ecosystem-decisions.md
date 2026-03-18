# Iced Ecosystem Decisions

Research and decisions on dependencies, forks, and widget strategies for the iced migration. Conducted March 2026.

## Iced Fork: squidowl/iced (Halloy's fork)

**Decision: Stay on Halloy's fork.**

We pin to `squidowl/iced` rev `b201e4f` (the `arboard-less-patch` branch). Updated March 2026 — the fork was rebased on top of iced master (merged via Halloy PR #1666), so we're now current with upstream. The fork carries 7 patches on top:

1. Reverts PR #3238 "Rich Clipboard" — uses `window_clipboard` instead of `arboard`
2. Primary clipboard paste for TextInput (Linux middle-click paste)
3. Primary clipboard copy for TextInput (auto-copy selection)
4. Primary clipboard for TextEditor
5. Drag updates for double/triple click (word/line selection expansion)
6. Keyboard modifiers on mouse::Event::ButtonPressed (needed for shift-click)
7. Shift-click selection in TextEditor

**Why not upstream?** Upstream iced (iced-rs/iced) has no primary clipboard support. Issue #904 has been open since 2022 with no plan to address it. An email client on Linux absolutely needs middle-click paste and proper text selection. These patches touch 27 files across core/widget/winit/test — porting them to upstream's new `arboard`-based clipboard API would be a significant effort.

**API changes from the March 2026 rebase:**
- `palette::Extended` renamed to `Palette`, old `Palette` renamed to `Seed`
- `Font::with_name()` → `Font::new()`
- `Theme::custom()` now takes `Seed` instead of `Palette`
- `theme.extended_palette()` → `theme.palette()`
- New: `font::set_defaults` task, `Color::mix()`, scale factor in window opened event
- macOS Tahoe `objc2` panic fix

**Revisit when:** upstream addresses issue #904, or Halloy's patches get upstreamed.

## Theme System: iced's built-in palette derivation

**Decision: Use `Theme::custom(name, Seed)` with 6 seed colors.**

iced has built-in palette derivation via `Palette::generate()`. A `Seed` struct with 6 fields (background, text, primary, success, warning, danger) automatically generates:
- 8 background levels (weakest → strongest) with auto-paired text colors
- primary/secondary/success/warning/danger with base/weak/strong variants
- All using OKLCh perceptual color math internally

All built-in widget Catalog implementations (button, container, text, scrollable, etc.) work automatically. We only need ~10 custom style helper functions for email-specific widgets (thread cards, nav items, badges, etc.).

Theme files are 8-line TOMLs with 6 hex colors. Everything else is derived.

**Previous approach (deleted):** ~2,100 lines of custom Catalog implementations adapted from Halloy (GPL-3.0). Replaced with ~250 lines using iced's built-in system.

## Spacing/Layout: Geometric scale with named presets

**Decision: Bootstrap-inspired constraint system.**

Spacing scale (geometric progression): 0, 2, 4, 8, 12, 16, 24, 32, 48, 64px. Every padding/spacing value in the app must land on this scale — no ad-hoc numbers.

Padding presets named by role: `PAD_BUTTON`, `PAD_NAV_ITEM`, `PAD_THREAD_CARD`, etc. Views pick the preset matching their role. This makes it impossible to choose wrong values — there's only one valid answer for any component.

Panel widths: Sidebar 180px, Thread List 280px, Contact Sidebar 240px.

**Research context:** Sniffnet uses scattered magic numbers. Halloy uses config-driven structs but no unified grid. COSMIC uses a 10-level scale (4→128px). hecrj's own apps have no centralized spacing at all. Our approach is more structured than all of them.

## iced_fontello: Don't adopt

**Decision: Keep our hand-rolled `icon.rs`.**

- Fontello does not include Lucide icons — dealbreaker
- Requires network access at build time (hits fontello.com API)
- Our 105-line `icon.rs` is simpler, zero build dependencies, works perfectly
- MIT licensed but irrelevant since we won't use it

## iced_palace: Add as dependency (when needed)

**Decision: Depend on iced_palace for text widgets.**

- MIT licensed, maintained by hecrj (iced's author)
- Same iced fork compatibility (0.15-dev)
- **EllipsizedText** — truncating sender names, subjects, snippets in thread list
- **labeled_slider** — useful for settings UI (font size, panel widths)
- Typewriter/DiffusedText — optional, fun for future UX
- Add as git dep when we actually need EllipsizedText in the thread list

## libcosmic: Reference only, don't depend

**Decision: Study patterns, don't import.**

- **License:** MPL-2.0 (file-level copyleft, not viral like GPL)
- **Blocker:** Ships its own iced fork (pop-os/iced) which conflicts with ours (squidowl/iced). Can't depend on both.
- **Approach:** Reimplement patterns we need against our iced fork. Most of what COSMIC offers are patterns using iced's built-in widgets, not COSMIC-specific widgets.

### Useful patterns identified from COSMIC apps:

| Pattern | Source | How to use |
|---------|--------|-----------|
| Scrollable item lists | cosmic-store | `column` + `scrollable` + `button::custom()` per item |
| Grid layout with dynamic columns | cosmic-store | `GridMetrics` calculates columns from available width |
| Pane grid with drag resize | halloy (iced built-in) | `PaneGrid` — already in iced, no libcosmic needed |
| Search bar with clear button | cosmic-edit | `text_input` with trailing icon button |
| Filter dropdown | cosmic-store | `widget::dropdown()` — available in iced |
| Dialog/modal stack | cosmic-store | `VecDeque<DialogPage>` for modal queue |
| Tab bar with drag reorder | cosmic-edit | Tab model + drag MIME type |
| Keyboard shortcut wrapper | halloy | `shortcut(element, bindings, msg)` pattern |
| Responsive layout | cosmic-store | `widget::responsive(\|size\| { ... })` — built into iced |
| Context menu | halloy | `widget::popover()` positioned at click point |
| Settings page layout | cosmic-settings | Nav sidebar + scrollable page, max-width centered |

### Key iced built-in widgets we should use (no external deps needed):

- `PaneGrid` — 4-panel layout with drag-to-resize
- `responsive` — dynamic layout based on available size
- `scrollable` — all lists
- `text_input` / `text_editor` — search, compose
- `button::primary/secondary/text/danger/subtle` — all button styles
- `container::bordered_box/rounded_box/dark` — panel containers
- `canvas` — avatars, custom drawing

## Color System Research Summary

Six systems were studied to inform our approach:

| System | Seeds | Derivation | Color Space | License |
|--------|-------|-----------|-------------|---------|
| COSMIC | ~30 palette | Full algorithmic via 100-step OKLCh ramps + WCAG contrast | OKLCh | MPL-2.0 |
| Bootstrap | 8 semantic | `mix(white/black, color, weight)` at 20% steps | Linear RGB | MIT |
| Oomox/themix | 6 + fallbacks | `mix(fg, bg, ratio)` for text hierarchy | Linear RGB | GPL-3.0 |
| Base16/tinted | 16 hand-picked | Zero derivation | N/A | MIT |
| Kitty | ~30 configurable | Mostly manual, HSLuv for contrast | HSLuv | GPL-3.0 |
| iced built-in | 6 | `deviate()` function, OKLCh lightness | OKLCh | MIT |

**Our choice:** iced's built-in system (6 seeds, OKLCh derivation) — it's already there, MIT licensed, and produces good results. No need to build our own color math.

## Font Strategy

- **Text:** Inter variable (regular + italic TTFs, SIL OFL 1.1 license)
- **Icons:** Lucide icon font (custom TTF, ISC license)
- Both bundled in `iced-proto/fonts/` with license files
- Loaded at startup via `include_bytes!` + `app.font()`
- `default_font` set to Inter

## Open Questions

- **When to add iced_palace?** When we implement proper text truncation in the thread list.
- **Will we need to fork iced ourselves?** Halloy covers our needs for now. If we need patches they don't carry (e.g., specific email-related widget behavior), we'd fork from theirs, not upstream.
- **CEF vs litehtml for email body rendering?** Still needs testing against real email corpus (see iced-migration-research.md).
