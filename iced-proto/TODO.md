# TODO

## Layout & Interaction

- [ ] **Audit rendered widths vs layout constants** — At scale 1.0 on a 4K display, observed widths (nav ~420px, sections ~1069px) don't match what the constants predict (nav 208px, sections max 600px). System scale confirmed 1.0. Need to understand why before setting min-width on sections/rows. Add a debug overlay or instrument the layout to see actual rendered sizes.

- [ ] **Per-pane minimum resize limits** — PaneGrid uses a uniform `min_size(120)` for all panes. Should have per-pane minimums (e.g., sidebar can't go below 150px, thread list below 200px). Requires clamping ratios in the `PaneResized` handler since PaneGrid only supports a single global minimum.

- [ ] **`responsive` for adaptive layout** — Wrap PaneGrid in `iced::widget::responsive` to collapse panels at narrow window sizes (e.g., hide contact sidebar below 900px, stack sidebar over thread list below 600px).

- [ ] **Keybinding display and edit UI** - Need to redo the Settings/Shortcuts UI. Take a look at https://nyaa.place/blog/libadwaita-1-8/

## Bugs

- [ ] **UI freezes after ~20 minutes with settings open** — App hangs completely with no stdout/stderr. Prime suspect is the `mundy` subscription (`appearance.rs`) holding a D-Bus connection that may drop or block over time. Bisect by disabling subscriptions one-by-one to isolate.

## Research

- [ ] **Investigate iced ecosystem projects** — Review for patterns, widget implementations, and architecture ideas:
  - https://github.com/hecrj/iced_fontello — Icon font integration
  - https://github.com/hecrj/iced_palace — Hecrj's iced showcase/playground
  - https://github.com/pop-os/cosmic-edit — COSMIC text editor (large real-world iced app)
  - https://github.com/pop-os/iced/blob/master/widget/src/markdown.rs — COSMIC fork's markdown widget

## Settings Row Types

- [ ] **License display/multiline static text display row** - Need to be able to click the link there, and also the text should be selectable/copyable in these widgets. Needs its own base type.

## Dev Defaults

- [ ] **Restore OS-based theme and 1.0 scale** — `SettingsState::default()` currently hardcodes `theme: "Light"` and `scale: 1.5` for development convenience. Revert to `theme: "System"` and `scale: 1.0` once UI prototyping is done, and persist user preferences to disk.
