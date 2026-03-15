# TODO

## Widget Design Rule Violations

Remaining violations from the CLAUDE.md audit.

### Widget constructors accept data, not UI elements (5)

- [ ] `widgets.rs` — `DropdownEntry.icon` is `Element`, should accept data describing the icon
- [ ] `widgets.rs` — `dropdown()` `trigger_icon` param is `Element`, same fix
- [ ] `widgets.rs` — `action_icon_button()` accepts `iced::widget::Text`, should accept icon identifier
- [ ] `widgets.rs` — `reply_button()` accepts `iced::widget::Text`, should accept icon identifier
- [ ] `settings.rs` — `setting_row()` accepts `Element` for control param

### All widgets belong in widgets.rs (5)

- [ ] `settings.rs` — `section()` is a reusable widget, move to widgets.rs
- [ ] `settings.rs` — `setting_row()` / `settings_row_container()` are reusable, move to widgets.rs
- [ ] `settings.rs` — `toggle_row()` is a reusable widget, move to widgets.rs
- [ ] `settings.rs` — `info_row()` is a reusable widget, move to widgets.rs
- [ ] `settings.rs` — `accent_color_row()` is a swatch picker widget, move to widgets.rs

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

## Done

- [x] Persist window state across restarts
- [x] Every slot gets its own container (10 violations fixed)
- [x] No magic numbers (8 violations fixed)
- [x] No raw colors outside theme.rs (5 violations fixed)
- [x] Move `settings_section_container` style to theme.rs
- [x] Replace `pick_list` with custom `select` widget
- [x] Unify nav buttons across sidebar and settings
