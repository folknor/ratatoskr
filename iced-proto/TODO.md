# TODO

## Widget Design Rule Violations

Remaining violations from the CLAUDE.md audit.

### Widget constructors accept data, not UI elements

All resolved. `DropdownEntry.icon` and `dropdown()` `trigger_icon` now use `DropdownIcon` enum.

## Layout & Interaction

- [ ] **Per-pane minimum resize limits** — PaneGrid uses a uniform `min_size(120)` for all panes. Should have per-pane minimums (e.g., sidebar can't go below 150px, thread list below 200px). Requires clamping ratios in the `PaneResized` handler since PaneGrid only supports a single global minimum.

- [ ] **`responsive` for adaptive layout** — Wrap PaneGrid in `iced::widget::responsive` to collapse panels at narrow window sizes (e.g., hide contact sidebar below 900px, stack sidebar over thread list below 600px).

- [ ] **Animated toggler widget** — Port libcosmic's slerp-based toggle animation for smooth sliding pill togglers. Current iced built-in toggler snaps instantly. libcosmic's version uses `anim::slerp()` with configurable duration (200ms default). ~150-200 LOC to port.

## Research

- [ ] **Investigate iced ecosystem projects** — Review for patterns, widget implementations, and architecture ideas:
  - https://github.com/hecrj/iced_fontello — Icon font integration
  - https://github.com/hecrj/iced_palace — Hecrj's iced showcase/playground
  - https://github.com/pop-os/cosmic-edit — COSMIC text editor (large real-world iced app)
  - https://github.com/pop-os/iced/blob/master/widget/src/markdown.rs — COSMIC fork's markdown widget

## Dev Defaults

- [ ] **Restore OS-based theme and 1.0 scale** — `SettingsState::default()` currently hardcodes `theme: "Light"` and `scale: 1.5` for development convenience. Revert to `theme: "System"` and `scale: 1.0` once UI prototyping is done, and persist user preferences to disk.

## Done

- [x] Persist window state across restarts
- [x] Every slot gets its own container (10 violations fixed)
- [x] No magic numbers (8 violations fixed)
- [x] No raw colors outside theme.rs (5 violations fixed)
- [x] Move `settings_section_container` style to theme.rs
- [x] Replace `pick_list` with custom `select` widget
- [x] Unify nav buttons across sidebar and settings
