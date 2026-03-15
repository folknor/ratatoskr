# TODO

## Widget Design Rule Violations

Audit performed against the rules in CLAUDE.md. 35 violations across the codebase.

### Widget constructors accept data, not UI elements (5)

- [ ] `widgets.rs` — `DropdownEntry.icon` is `Element`, should accept data describing the icon
- [ ] `widgets.rs` — `dropdown()` `trigger_icon` param is `Element`, same fix
- [ ] `widgets.rs` — `action_icon_button()` accepts `iced::widget::Text`, should accept icon identifier
- [ ] `widgets.rs` — `reply_button()` accepts `iced::widget::Text`, should accept icon identifier
- [ ] `settings.rs` — `setting_row()` accepts `Element` for control param

### All widgets belong in widgets.rs (7)

- [ ] `settings.rs` — `section()` is a reusable widget, move to widgets.rs
- [ ] `settings.rs` — `setting_row()` is a reusable widget, move to widgets.rs
- [ ] `settings.rs` — `toggle_row()` is a reusable widget, move to widgets.rs
- [ ] `settings.rs` — `info_row()` is a reusable widget, move to widgets.rs
- [ ] `settings.rs` — `settings_pick_list()` is a styled widget, move to widgets.rs
- [ ] `settings.rs` — `accent_color_row()` is a swatch picker widget, move to widgets.rs
- [ ] `settings.rs` — `settings_section_container()` is a style function, move to theme.rs

### Every slot gets its own container (10)

- [ ] `widgets.rs` — `nav_item_with_badge` label text bare in row
- [ ] `widgets.rs` — `label_nav_item` text bare in row
- [ ] `widgets.rs` — `collapsible_section` header title and chevron bare in row
- [ ] `widgets.rs` — dropdown trigger chevron_slot has no container
- [ ] `widgets.rs` — `compose_button` icon and text bare in row
- [ ] `widgets.rs` — `settings_button` icon and text bare in row
- [ ] `widgets.rs` — `action_icon_button` icon and text bare in row
- [ ] `widgets.rs` — `reply_button` icon and text bare in row
- [ ] `settings.rs` — back button icon and text bare in row
- [ ] `settings.rs` — tab nav button icon and text bare in row

### No magic numbers (8)

- [ ] `settings.rs` — settings nav width hardcoded as `200`, needs layout constant
- [ ] `settings.rs` — `max_width(600)` hardcoded in general tab, needs layout constant
- [ ] `settings.rs` — `max_width(600)` hardcoded in about tab, same
- [ ] `settings.rs` — `SWATCH_SIZE` defined locally, move to layout.rs
- [ ] `settings.rs` — swatch border radius computed inline (`SWATCH_SIZE / 2.0`), needs constant
- [ ] `settings.rs` — inline rule style with hardcoded alpha, should use `theme::divider_rule`
- [ ] `settings.rs` — `Padding::from([SPACE_XXS, SPACE_SM])` in pick_list, needs named preset
- [ ] `settings.rs` — `Padding::from([SPACE_SM, SPACE_MD])` repeated 6+ times, needs `PAD_SETTINGS_ROW`

### No raw colors outside theme.rs (5)

- [ ] `settings.rs` — `ACCENT_COLORS` array with 6 raw `Color::from_rgb()` values, move to theme.rs
- [ ] `settings.rs` — `settings_pick_list()` inline style closure, move to theme.rs
- [ ] `settings.rs` — `iced::Color::TRANSPARENT` in pick_list style
- [ ] `settings.rs` — accent swatch button inline style with raw color
- [ ] `settings.rs` — `Color::BLACK.scale_alpha(0.15)` in section container style, move to theme.rs
