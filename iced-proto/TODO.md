# TODO

## Widget Design Rule Violations

Audit performed against the rules in CLAUDE.md. Originally 35 violations.

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
- [x] `settings.rs` — `settings_section_container()` is a style function — moved to theme.rs

### Every slot gets its own container (10)

- [x] `widgets.rs` — `nav_item_with_badge` label text bare in row
- [x] `widgets.rs` — `label_nav_item` text bare in row
- [x] `widgets.rs` — `collapsible_section` header title and chevron bare in row
- [x] `widgets.rs` — dropdown trigger chevron_slot has no container
- [x] `widgets.rs` — `compose_button` icon and text bare in row
- [x] `widgets.rs` — `settings_button` icon and text bare in row
- [x] `widgets.rs` — `action_icon_button` icon and text bare in row
- [x] `widgets.rs` — `reply_button` icon and text bare in row
- [x] `settings.rs` — back button icon and text bare in row
- [x] `settings.rs` — tab nav button icon and text bare in row

### No magic numbers (8)

- [x] `settings.rs` — settings nav width hardcoded as `200` — now `SETTINGS_NAV_WIDTH`
- [x] `settings.rs` — `max_width(600)` hardcoded in general tab — now `SETTINGS_CONTENT_MAX_WIDTH`
- [x] `settings.rs` — `max_width(600)` hardcoded in about tab — same
- [x] `settings.rs` — `SWATCH_SIZE` defined locally — moved to layout.rs
- [x] `settings.rs` — swatch border radius computed inline — now `RADIUS_ROUND`
- [x] `settings.rs` — inline rule style with hardcoded alpha — now `theme::subtle_divider_rule`
- [x] `settings.rs` — `Padding::from([SPACE_XXS, SPACE_SM])` in pick_list — now `PAD_PICK_LIST`
- [x] `settings.rs` — `Padding::from([SPACE_SM, SPACE_MD])` repeated — now `PAD_SETTINGS_ROW`

### No raw colors outside theme.rs (5)

- [x] `settings.rs` — `ACCENT_COLORS` array — moved to `theme::ACCENT_COLORS`
- [x] `settings.rs` — `settings_pick_list()` inline style closure — now `theme::ghost_pick_list`
- [x] `settings.rs` — `iced::Color::TRANSPARENT` in pick_list style — moved to theme.rs
- [x] `settings.rs` — accent swatch button inline style — now `theme::swatch_button()`
- [x] `settings.rs` — `Color::BLACK.scale_alpha(0.15)` — moved to `theme::settings_section_container`
