# TODO

## Layout & Interaction

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

- [ ] **Restore OS-based theme and 1.0 scale** — `SettingsState::default()` currently hardcodes `theme: "Light"` for development convenience. Revert to `theme: "System"` once UI prototyping is done, and persist user preferences to disk.
